//! StynPS (PlaySelect) backend daemon.

use std::{
    collections::{HashMap, HashSet},
    io,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use evdev::{
    uinput::{UInputEvent, VirtualDevice, VirtualDeviceBuilder, VirtualEventStream},
    AbsInfo, AttributeSet, BusType, Device, EventStream, EventType, FFEffect, FFEffectData,
    FFEffectKind, FFEffectType, FFReplay, FFTrigger, InputEvent, InputId, Key, RelativeAxisType,
    UInputEventType, UinputAbsSetup,
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{mpsc, watch, RwLock},
    time::MissedTickBehavior,
};
use tracing::{info, warn};

use crate::{
    remap::{RemapConfig, Remapper},
    system,
};

const SONY_VENDOR: u16 = 0x054c;
const DEFAULT_PROFILE: &str = "Default";

pub fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("stynps.sock"),
        _ => PathBuf::from(format!("/tmp/stynps-{}.sock", std::env::var("USER").unwrap_or_default())),
    }
}

static OWNS_SOCKET: AtomicBool = AtomicBool::new(false);

pub fn cleanup_socket() {
    if OWNS_SOCKET.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(socket_path());
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub button_map: HashMap<String, String>,
    #[serde(default)]
    pub match_rules: Vec<MatchRule>,

    #[serde(default = "default_sens")]
    pub tp_sensitivity: f32,
    #[serde(default)]
    pub tp_mouse_enabled: bool,

    #[serde(default = "default_sens")]
    pub gyro_sensitivity: f32,
    #[serde(default)]
    pub gyro_mouse_enabled: bool,
    #[serde(default)]
    pub gyro_invert_x: bool,
    #[serde(default)]
    pub gyro_invert_y: bool,
}

fn default_sens() -> f32 { 1.0 }

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: DEFAULT_PROFILE.into(),
            button_map: HashMap::new(),
            match_rules: Vec::new(),
            tp_sensitivity: 1.0,
            tp_mouse_enabled: false,
            gyro_sensitivity: 1.0,
            gyro_mouse_enabled: false,
            gyro_invert_x: false,
            gyro_invert_y: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MatchRule {
    #[serde(default)]
    pub window_class: Option<String>,
    #[serde(default)]
    pub process_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalSettings {
    pub sync_lightbar_battery: bool,
    pub lb_color: [u8; 3],
    pub lb_brightness: f32,
    pub rumble_strength_small: u8,
    pub rumble_strength_large: u8,
    pub rumble_passthrough: bool,
    pub steam_input_enabled: bool,
    pub auto_steam_handover: bool,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            sync_lightbar_battery: false,
            lb_color: [0, 112, 209],
            lb_brightness: 80.0,
            rumble_strength_small: 255,
            rumble_strength_large: 255,
            rumble_passthrough: true,
            steam_input_enabled: false,
            auto_steam_handover: false,
        }
    }
}

#[derive(Default)]
pub struct State {
    profiles: HashMap<String, Profile>,
    settings: GlobalSettings,
    active_profile: String,
    auto_profile: bool,
    battery: Option<(u8, String)>,
    controller_connected: bool,
    controller_ids: Option<(u16, u16)>,
    bluetooth: bool,
    active_app: Option<String>,
    running_game: Option<system::RunningGame>,
    last_error: Option<String>,
    rumble_error: Option<String>,
    lightbar_error: Option<String>,
}

fn effective_steam_mode(st: &State) -> bool {
    let is_steam_game = st.running_game.as_ref().map_or(false, |g| g.steam_id.is_some());
    st.settings.steam_input_enabled || (st.settings.auto_steam_handover && is_steam_game)
}

#[derive(Clone)]
struct Runtime {
    remap: Arc<RemapConfig>,
    tp_enabled: bool,
    tp_sens: f32,
    gyro_enabled: bool,
    gyro_sens: f32,
    gyro_invert_x: bool,
    gyro_invert_y: bool,
    steam_mode: bool,
    rumble_passthrough: bool,
    strength_small: u8,
    strength_large: u8,
}

fn runtime_of(st: &State) -> Runtime {
    let p = st.profiles.get(&st.active_profile);
    Runtime {
        remap: RemapConfig::from_names(&p.map(|p| p.button_map.clone()).unwrap_or_default()),
        tp_enabled: p.map_or(false, |p| p.tp_mouse_enabled),
        tp_sens: p.map_or(1.0, |p| p.tp_sensitivity),
        gyro_enabled: p.map_or(false, |p| p.gyro_mouse_enabled),
        gyro_sens: p.map_or(1.0, |p| p.gyro_sensitivity),
        gyro_invert_x: p.map_or(false, |p| p.gyro_invert_x),
        gyro_invert_y: p.map_or(false, |p| p.gyro_invert_y),
        steam_mode: effective_steam_mode(st),
        rumble_passthrough: st.settings.rumble_passthrough,
        strength_small: st.settings.rumble_strength_small,
        strength_large: st.settings.rumble_strength_large,
    }
}

type Shared = Arc<RwLock<State>>;

enum InputCmd { TestRumble { ms: u16 }, StopRumble }

struct Ctx {
    shared: Shared,
    rt_tx: watch::Sender<Runtime>,
    cmd_tx: mpsc::UnboundedSender<InputCmd>,
}

fn publish(st: &State, tx: &watch::Sender<Runtime>) { tx.send_replace(runtime_of(st)); }

#[derive(Default)]
struct InputSnapshot { buttons: HashSet<u16>, axes: HashMap<u16, i32> }

fn snapshot() -> std::sync::MutexGuard<'static, InputSnapshot> {
    static S: OnceLock<Mutex<InputSnapshot>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(InputSnapshot::default())).lock().unwrap_or_else(|e| e.into_inner())
}

fn record_snapshot(ev: &InputEvent) {
    let mut s = snapshot();
    match ev.event_type() {
        EventType::KEY => {
            if ev.value() == 0 { s.buttons.remove(&ev.code()); } else { s.buttons.insert(ev.code()); }
        }
        EventType::ABSOLUTE => { s.axes.insert(ev.code(), ev.value()); }
        _ => {}
    }
}

fn config_path() -> PathBuf { system::config_dir().join("config.json") }

fn load_state() -> State {
    let mut st = State { auto_profile: true, ..Default::default() };
    let path = config_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        match serde_json::from_str::<serde_json::Value>(&data) {
            Ok(val) => {
                if let Some(obj) = val["profiles"].as_object() {
                    for (key, v) in obj {
                        match serde_json::from_value::<Profile>(v.clone()) {
                            Ok(mut p) => {
                                if p.name.trim().is_empty() { p.name = key.clone(); }
                                if p.name == "default" || p.name == "Fallback" { p.name = DEFAULT_PROFILE.into(); }
                                st.profiles.insert(p.name.clone(), p);
                            }
                            Err(e) => warn!("skipping unreadable profile {key:?}: {e}"),
                        }
                    }
                }
                st.settings = serde_json::from_value(val["settings"].clone()).unwrap_or_default();
                if let Some(a) = val["active"].as_str() {
                    st.active_profile = if a == "default" || a == "Fallback" { DEFAULT_PROFILE.into() } else { a.to_string() };
                }
                if let Some(a) = val["auto"].as_bool() { st.auto_profile = a; }
            }
            Err(e) => {
                warn!("config.json is not valid JSON ({e}); keeping a backup and starting fresh");
                let _ = std::fs::copy(&path, path.with_extension("json.bak"));
            }
        }
    }
    st.profiles.entry(DEFAULT_PROFILE.into()).or_default();
    if !st.profiles.contains_key(&st.active_profile) { st.active_profile = DEFAULT_PROFILE.into(); }
    st
}

fn save_state(st: &State) {
    let path = config_path();
    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    let val = serde_json::json!({ "profiles": st.profiles, "settings": st.settings, "active": st.active_profile, "auto": st.auto_profile });
    if let Ok(data) = serde_json::to_string_pretty(&val) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, data).is_ok() { let _ = std::fs::rename(&tmp, &path); }
    }
}

#[derive(Deserialize, Debug)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    GetStatus, GetInput, ListProfiles,
    SetProfile { name: String }, SetAutoProfile { enabled: bool }, UpsertProfile { profile: Profile }, DeleteProfile { name: String },
    SetLightbar { color: [u8; 3], brightness: f32, sync_battery: bool }, SetRumbleStrength { small: u8, large: u8, passthrough: bool },
    TestRumble, StopRumble, SetSteamInput { enabled: bool, auto_handover: bool },
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Response {
    Ok,
    Status { connected: bool, battery: Option<u8>, battery_status: Option<String>, active_profile: String, auto_profile: bool, active_app: Option<String>, settings: GlobalSettings, running_game_name: Option<String>, running_game_steam_id: Option<u32>, steam_mode_active: bool, controller_ids: Option<[u16; 2]>, bluetooth: bool, last_error: Option<String>, rumble_error: Option<String>, lightbar_error: Option<String> },
    Profiles { profiles: Vec<Profile> },
    Input { buttons: Vec<u16>, axes: HashMap<u16, i32> },
    Error { message: String },
}

pub async fn run() -> Result<()> {
    let listener = bind_socket().await?;
    let st = load_state();
    let (rt_tx, _rt_rx) = watch::channel(runtime_of(&st));
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let ctx = Arc::new(Ctx { shared: Arc::new(RwLock::new(st)), rt_tx, cmd_tx });

    tokio::spawn(input_task(ctx.clone(), cmd_rx));
    tokio::spawn(battery_task(ctx.clone()));
    tokio::spawn(game_detect_task(ctx.clone()));
    tokio::spawn(autoprofile_task(ctx.clone()));
    ipc_loop(ctx, listener).await
}

async fn bind_socket() -> Result<UnixListener> {
    let path = socket_path();
    if UnixStream::connect(&path).await.is_ok() { bail!("another StynPS is already running (socket {} is in use)", path.display()); }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("cannot bind {}", path.display()))?;
    OWNS_SOCKET.store(true, Ordering::SeqCst);
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    Ok(listener)
}

fn find_pad() -> Option<(PathBuf, Device)> {
    evdev::enumerate().find(|(_, d)| {
        let name = d.name().unwrap_or_default();
        d.input_id().vendor() == SONY_VENDOR && !name.contains("Motion Sensors") && !name.contains("Touchpad") && d.supported_keys().map_or(false, |k| k.contains(Key::BTN_SOUTH))
    })
}

fn find_sibling(pad: &Device, needle: &str) -> Option<Device> {
    let id = pad.input_id();
    let uniq = pad.unique_name().map(str::to_string);
    evdev::enumerate().map(|(_, d)| d).find(|d| {
        d.input_id().vendor() == id.vendor() && d.input_id().product() == id.product() && d.name().unwrap_or_default().contains(needle) && d.unique_name().map(str::to_string) == uniq
    })
}

const PAD_BUTTONS: [Key; 11] = [
    Key::BTN_SOUTH, Key::BTN_EAST, Key::BTN_NORTH, Key::BTN_WEST, Key::BTN_TL, Key::BTN_TR, Key::BTN_SELECT, Key::BTN_START, Key::BTN_MODE, Key::BTN_THUMBL, Key::BTN_THUMBR,
];

fn build_gamepad(src: &Device) -> Result<VirtualDevice> {
    let mut keys = AttributeSet::<Key>::new();
    if let Some(src_keys) = src.supported_keys() { src_keys.iter().for_each(|k| keys.insert(k)); }
    PAD_BUTTONS.iter().for_each(|k| keys.insert(*k));

    let input_id = InputId::new(BusType::BUS_USB, 0x045e, 0x028e, 0x0110);
    let mut b = VirtualDeviceBuilder::new()?.name("Microsoft X-Box 360 pad").input_id(input_id).with_keys(&keys)?;

    if let Some(axes) = src.supported_absolute_axes() {
        let state = src.get_abs_state()?;
        for axis in axes.iter() {
            let i = state[axis.0 as usize];
            let info = AbsInfo::new(i.value, i.minimum, i.maximum, i.fuzz, i.flat, i.resolution);
            b = b.with_absolute_axis(&UinputAbsSetup::new(axis, info))?;
        }
    }
    let mut ff = AttributeSet::<FFEffectType>::new();
    ff.insert(FFEffectType::FF_RUMBLE);
    Ok(b.with_ff(&ff)?.with_ff_effects_max(16).build()?)
}

fn build_mouse() -> Result<VirtualDevice> {
    let mut keys = AttributeSet::<Key>::new();
    keys.insert(Key::BTN_LEFT); keys.insert(Key::BTN_RIGHT);
    let mut rel = AttributeSet::<RelativeAxisType>::new();
    rel.insert(RelativeAxisType::REL_X); rel.insert(RelativeAxisType::REL_Y);
    Ok(VirtualDeviceBuilder::new()?.name("StynPS Virtual Mouse").with_keys(&keys)?.with_relative_axes(&rel)?.build()?)
}

fn is_gone(e: &io::Error) -> bool { matches!(e.raw_os_error(), Some(19) | Some(5)) }

async fn apply_lightbar(ctx: &Ctx) {
    let (color, brightness) = {
        let st = ctx.shared.read().await;
        let color = if st.settings.sync_lightbar_battery { system::battery_color(st.battery.as_ref().map_or(100, |b| b.0)) } else { st.settings.lb_color };
        (color, st.settings.lb_brightness)
    };
    let result = tokio::task::spawn_blocking(move || system::set_lightbar(color, brightness)).await;
    let error = match result { Ok(Ok(())) => None, Ok(Err(e)) => Some(e), Err(e) => Some(e.to_string()) };
    ctx.shared.write().await.lightbar_error = error;
}

enum Outcome { NoController, Restart, Disconnected }

async fn input_task(ctx: Arc<Ctx>, mut cmd_rx: mpsc::UnboundedReceiver<InputCmd>) {
    loop {
        let res = run_input(&ctx, &mut cmd_rx).await;
        {
            let mut st = ctx.shared.write().await;
            st.controller_connected = false; st.controller_ids = None;
            if let Err(e) = &res { warn!("input session ended: {e:#}"); st.last_error = Some(format!("{e:#}")); }
        }
        { let mut s = snapshot(); s.buttons.clear(); s.axes.clear(); }
        while cmd_rx.try_recv().is_ok() {}
        let wait = match res {
            Ok(Outcome::Restart) => Duration::from_millis(150), Ok(Outcome::Disconnected) => Duration::from_millis(500), Ok(Outcome::NoController) => Duration::from_millis(1500), Err(_) => Duration::from_secs(3),
        };
        tokio::time::sleep(wait).await;
    }
}

async fn next_ev(s: &mut Option<EventStream>) -> io::Result<InputEvent> { match s { Some(s) => s.next_event().await, None => std::future::pending().await } }
async fn next_vev(s: &mut Option<VirtualEventStream>) -> io::Result<UInputEvent> { match s { Some(s) => s.next_event().await, None => std::future::pending().await } }

#[derive(Default)]
struct Pointer { dx: f32, dy: f32, slot: i32, last: [Option<f32>; 2], left_down: bool }

impl Pointer {
    fn flush(&mut self, mouse: &mut Option<VirtualDevice>) {
        let (ix, iy) = (self.dx.trunc() as i32, self.dy.trunc() as i32);
        if ix == 0 && iy == 0 { return; }
        self.dx -= ix as f32; self.dy -= iy as f32;
        if let Some(m) = mouse {
            let mut evs = Vec::with_capacity(2);
            if ix != 0 { evs.push(InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_X.0, ix)); }
            if iy != 0 { evs.push(InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_Y.0, iy)); }
            let _ = m.emit(&evs);
        }
    }
    fn click(&mut self, down: bool, mouse: &mut Option<VirtualDevice>) {
        if self.left_down == down { return; }
        self.left_down = down;
        if let Some(m) = mouse { let _ = m.emit(&[InputEvent::new(EventType::KEY, Key::BTN_LEFT.code(), down as i32)]); }
    }
}

const ABS_MT_SLOT: u16 = 0x2f; const ABS_MT_POSITION_X: u16 = 0x35; const ABS_MT_POSITION_Y: u16 = 0x36; const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_RX: u16 = 3; const ABS_RY: u16 = 4; const GYRO_DEADZONE: i32 = 300; const GYRO_SCALE: f32 = 4.0e-5;

fn handle_touchpad(ev: InputEvent, ptr: &mut Pointer, rt: &Runtime, mouse: &mut Option<VirtualDevice>) {
    if !rt.tp_enabled || mouse.is_none() { ptr.click(false, mouse); ptr.last = [None, None]; return; }
    match ev.event_type() {
        EventType::ABSOLUTE => match ev.code() {
            ABS_MT_SLOT => ptr.slot = ev.value(),
            ABS_MT_TRACKING_ID if ptr.slot == 0 => ptr.last = [None, None],
            ABS_MT_POSITION_X | ABS_MT_POSITION_Y if ptr.slot == 0 => {
                let i = (ev.code() == ABS_MT_POSITION_Y) as usize;
                let v = ev.value() as f32;
                if let Some(prev) = ptr.last[i] {
                    let d = (v - prev) * rt.tp_sens;
                    if i == 0 { ptr.dx += d; } else { ptr.dy += d; }
                }
                ptr.last[i] = Some(v);
            }
            _ => {}
        },
        EventType::KEY if ev.code() == Key::BTN_LEFT.code() => ptr.click(ev.value() != 0, mouse),
        EventType::SYNCHRONIZATION => ptr.flush(mouse),
        _ => {}
    }
}

fn handle_motion(ev: InputEvent, ptr: &mut Pointer, rt: &Runtime, mouse: &mut Option<VirtualDevice>) {
    if !rt.gyro_enabled || mouse.is_none() { return; }
    match ev.event_type() {
        EventType::ABSOLUTE if ev.value().abs() >= GYRO_DEADZONE => {
            let d = -(ev.value() as f32) * GYRO_SCALE * rt.gyro_sens;
            match ev.code() {
                ABS_RY => ptr.dx += if rt.gyro_invert_x { -d } else { d },
                ABS_RX => ptr.dy += if rt.gyro_invert_y { -d } else { d },
                _ => {}
            }
        }
        EventType::SYNCHRONIZATION => ptr.flush(mouse),
        _ => {}
    }
}

#[derive(Clone, Copy)]
struct GameFx { strong: u16, weak: u16, replay: FFReplay }

#[derive(Default)]
struct Rumble { game: HashMap<i16, GameFx>, playing: HashMap<i16, FFEffect>, test: Option<(FFEffect, Instant)>, reported: bool }

fn scale(v: u16, strength: u8) -> u16 { ((v as u32 * strength as u32) / 255) as u16 }

async fn report_rumble(ctx: &Ctx, rumble: &mut Rumble, error: Option<String>) {
    match error {
        Some(e) => {
            let msg = format!("Rumble failed: {e}. The controller's /dev/input node must be writable by you.");
            let mut st = ctx.shared.write().await;
            if st.rumble_error.as_deref() != Some(msg.as_str()) { st.rumble_error = Some(msg); }
            rumble.reported = true;
        }
        None if rumble.reported => { ctx.shared.write().await.rumble_error = None; rumble.reported = false; }
        None => {}
    }
}

async fn handle_ff_event(ev: UInputEvent, gp: &mut VirtualEventStream, phys: &mut EventStream, rumble: &mut Rumble, rt: &Runtime, ctx: &Ctx) -> Result<()> {
    match ev.event_type() {
        EventType::UINPUT if ev.code() == UInputEventType::UI_FF_UPLOAD.0 => {
            let up = gp.device_mut().process_ff_upload(ev)?;
            let data = up.effect();
            if let FFEffectKind::Rumble { strong_magnitude, weak_magnitude } = data.kind {
                rumble.game.insert(up.effect_id(), GameFx { strong: strong_magnitude, weak: weak_magnitude, replay: data.replay });
            }
        }
        EventType::UINPUT if ev.code() == UInputEventType::UI_FF_ERASE.0 => {
            let er = gp.device_mut().process_ff_erase(ev)?;
            let id = er.effect_id() as i16;
            rumble.game.remove(&id); rumble.playing.remove(&id);
        }
        EventType::FORCEFEEDBACK if ev.code() != FFEffectType::FF_GAIN.0 => {
            let id = ev.code() as i16;
            rumble.playing.remove(&id);
            if ev.value() > 0 && rt.rumble_passthrough {
                if let Some(fx) = rumble.game.get(&id).copied() {
                    let strong = scale(fx.strong, rt.strength_large);
                    let weak = scale(fx.weak, rt.strength_small);
                    if strong > 0 || weak > 0 {
                        let data = FFEffectData {
                            direction: 0, trigger: FFTrigger::default(), replay: fx.replay,
                            kind: FFEffectKind::Rumble { strong_magnitude: strong, weak_magnitude: weak },
                        };
                        let result = phys.device_mut().upload_ff_effect(data).and_then(|mut eff| { eff.play(ev.value())?; Ok(eff) });
                        match result {
                            Ok(eff) => { rumble.playing.insert(id, eff); report_rumble(ctx, rumble, None).await; }
                            Err(e) => report_rumble(ctx, rumble, Some(e.to_string())).await,
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn run_input(ctx: &Arc<Ctx>, cmd_rx: &mut mpsc::UnboundedReceiver<InputCmd>) -> Result<Outcome> {
    let Some((_path, mut dev)) = find_pad() else { return Ok(Outcome::NoController) };
    let info = dev.input_id();
    let ids = (info.vendor(), info.product());
    let bluetooth = info.bus_type() == BusType::BUS_BLUETOOTH;
    let has_ff = dev.supported_ff().map_or(false, |f| f.contains(FFEffectType::FF_RUMBLE));

    let mut rt_rx = ctx.rt_tx.subscribe();
    let mut rt = rt_rx.borrow_and_update().clone();
    let steam_mode = rt.steam_mode;
    let grab = !steam_mode;

    let mut tp = find_sibling(&dev, "Touchpad");
    let motion = find_sibling(&dev, "Motion Sensors");

    let mut gp_stream: Option<VirtualEventStream> = None;
    let mut mouse: Option<VirtualDevice> = None;
    if grab {
        gp_stream = Some(build_gamepad(&dev).context("cannot create virtual controller")?.into_event_stream()?);
        mouse = Some(build_mouse().context("cannot create virtual mouse")?);
        dev.grab().context("cannot take over the controller")?;
        if let Some(t) = tp.as_mut() { let _ = t.grab(); }
    }

    let mut stream = dev.into_event_stream()?;
    let mut tp_stream = tp.map(|d| d.into_event_stream()).transpose()?;
    let mut motion_stream = motion.map(|d| d.into_event_stream()).transpose()?;

    {
        let mut st = ctx.shared.write().await;
        st.controller_connected = true; st.controller_ids = Some(ids); st.bluetooth = bluetooth; st.last_error = None;
        st.rumble_error = (!has_ff).then(|| "This controller driver does not report rumble support.".to_string());
    }
    info!("controller ready ({}, {})", if bluetooth { "Bluetooth" } else { "USB" }, if grab { "StynPS mode" } else { "Steam mode" });
    apply_lightbar(ctx).await;

    let mut remapper = Remapper::new(rt.remap.clone());
    let mut batch: Vec<InputEvent> = Vec::with_capacity(64);
    let mut ptr = Pointer::default();
    let mut rumble = Rumble::default();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            r = stream.next_event() => {
                let ev = match r { Ok(ev) => ev, Err(e) if is_gone(&e) => return Ok(Outcome::Disconnected), Err(e) => return Err(e.into()) };
                record_snapshot(&ev);
                if let Some(gp) = gp_stream.as_mut() {
                    match ev.event_type() {
                        EventType::KEY | EventType::ABSOLUTE => remapper.process(ev, &mut batch),
                        EventType::SYNCHRONIZATION => { if !batch.is_empty() { let _ = gp.device_mut().emit(&batch); batch.clear(); } }
                        _ => {}
                    }
                }
            }
            r = next_ev(&mut tp_stream) => match r { Ok(ev) => handle_touchpad(ev, &mut ptr, &rt, &mut mouse), Err(_) => { tp_stream = None; ptr.click(false, &mut mouse); } },
            r = next_ev(&mut motion_stream) => match r { Ok(ev) => handle_motion(ev, &mut ptr, &rt, &mut mouse), Err(_) => motion_stream = None },
            r = next_vev(&mut gp_stream) => {
                let ev = r?;
                if let Some(gp) = gp_stream.as_mut() { handle_ff_event(ev, gp, &mut stream, &mut rumble, &rt, ctx).await?; }
            }
            cmd = cmd_rx.recv() => match cmd {
                Some(InputCmd::TestRumble { ms }) => {
                    let data = FFEffectData {
                        direction: 0, trigger: FFTrigger::default(), replay: FFReplay { length: ms, delay: 0 },
                        kind: FFEffectKind::Rumble { strong_magnitude: rt.strength_large as u16 * 257, weak_magnitude: rt.strength_small as u16 * 257 },
                    };
                    let result = stream.device_mut().upload_ff_effect(data).and_then(|mut eff| { eff.play(1)?; Ok(eff) });
                    match result {
                        Ok(eff) => { rumble.test = Some((eff, Instant::now() + Duration::from_millis(ms as u64 + 100))); report_rumble(ctx, &mut rumble, None).await; }
                        Err(e) => report_rumble(ctx, &mut rumble, Some(e.to_string())).await,
                    }
                }
                Some(InputCmd::StopRumble) => rumble.test = None,
                None => return Ok(Outcome::Restart),
            },
            changed = rt_rx.changed() => {
                if changed.is_err() { return Ok(Outcome::Restart); }
                rt = rt_rx.borrow_and_update().clone();
                if rt.steam_mode != steam_mode { return Ok(Outcome::Restart); }
                let mut release = Vec::new();
                remapper.set_config(rt.remap.clone(), &mut release);
                if let (Some(gp), false) = (gp_stream.as_mut(), release.is_empty()) { let _ = gp.device_mut().emit(&release); }
                if !rt.tp_enabled { ptr.click(false, &mut mouse); }
            }
            _ = tick.tick() => { if matches!(&rumble.test, Some((_, until)) if Instant::now() >= *until) { rumble.test = None; } }
        }
    }
}

async fn ipc_loop(ctx: Arc<Ctx>, listener: UnixListener) -> Result<()> {
    loop {
        let (stream, _) = listener.accept().await?;
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let (r, mut w) = stream.into_split();
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let resp = match serde_json::from_str::<Request>(&line) {
                    Ok(req) => handle_request(req, &ctx).await,
                     Err(e) => Response::Error { message: format!("bad request: {e}") },
                };
                let Ok(mut out) = serde_json::to_vec(&resp) else { break };
                out.push(b'\n');
                if w.write_all(&out).await.is_err() { break; }
            }
        });
    }
}

async fn handle_request(req: Request, ctx: &Arc<Ctx>) -> Response {
    let err = |m: &str| Response::Error { message: m.to_string() };
    match req {
        Request::GetStatus => {
            let st = ctx.shared.read().await;
            let (running_game_name, running_game_steam_id) = match &st.running_game {
                Some(game) => (Some(game.name.clone()), game.steam_id),
                None => (None, None),
            };
            Response::Status {
                connected: st.controller_connected, battery: st.battery.as_ref().map(|b| b.0), battery_status: st.battery.as_ref().map(|b| b.1.clone()),
                active_profile: st.active_profile.clone(), auto_profile: st.auto_profile, active_app: st.active_app.clone(), settings: st.settings.clone(),
                running_game_name, running_game_steam_id, steam_mode_active: effective_steam_mode(&st), controller_ids: st.controller_ids.map(|(v, p)| [v, p]),
                bluetooth: st.bluetooth, last_error: st.last_error.clone(), rumble_error: st.rumble_error.clone(), lightbar_error: st.lightbar_error.clone(),
            }
        }
        Request::GetInput => { let s = snapshot(); Response::Input { buttons: s.buttons.iter().copied().collect(), axes: s.axes.clone() } }
        Request::ListProfiles => Response::Profiles { profiles: ctx.shared.read().await.profiles.values().cloned().collect() },
        Request::SetProfile { name } => {
            let mut st = ctx.shared.write().await;
            if st.profiles.contains_key(&name) { st.active_profile = name; st.auto_profile = false; publish(&st, &ctx.rt_tx); save_state(&st); Response::Ok } else { err("profile not found") }
        }
        Request::SetAutoProfile { enabled } => { let mut st = ctx.shared.write().await; st.auto_profile = enabled; save_state(&st); Response::Ok }
        Request::UpsertProfile { mut profile } => {
            profile.name = profile.name.trim().to_string();
            if profile.name.is_empty() { return err("a profile needs a name"); }
            let mut st = ctx.shared.write().await;
            let name = profile.name.clone();
            st.profiles.insert(name.clone(), profile);
            if st.active_profile == name { publish(&st, &ctx.rt_tx); }
            save_state(&st); Response::Ok
        }
        Request::DeleteProfile { name } => {
            if name == DEFAULT_PROFILE { return err("the default profile cannot be deleted"); }
            let mut st = ctx.shared.write().await;
            st.profiles.remove(&name);
            if st.active_profile == name { st.active_profile = DEFAULT_PROFILE.into(); publish(&st, &ctx.rt_tx); }
            save_state(&st); Response::Ok
        }
        Request::SetLightbar { color, brightness, sync_battery } => {
            { let mut st = ctx.shared.write().await; st.settings.lb_color = color; st.settings.lb_brightness = brightness.clamp(0.0, 100.0); st.settings.sync_lightbar_battery = sync_battery; save_state(&st); }
            apply_lightbar(ctx).await; Response::Ok
        }
        Request::SetRumbleStrength { small, large, passthrough } => {
            let mut st = ctx.shared.write().await;
            st.settings.rumble_strength_small = small; st.settings.rumble_strength_large = large; st.settings.rumble_passthrough = passthrough;
            publish(&st, &ctx.rt_tx); save_state(&st); Response::Ok
        }
        Request::TestRumble => { let _ = ctx.cmd_tx.send(InputCmd::TestRumble { ms: 700 }); Response::Ok }
        Request::StopRumble => { let _ = ctx.cmd_tx.send(InputCmd::StopRumble); Response::Ok }
        Request::SetSteamInput { enabled, auto_handover } => {
            let mut st = ctx.shared.write().await;
            st.settings.steam_input_enabled = enabled; st.settings.auto_steam_handover = auto_handover;
            publish(&st, &ctx.rt_tx); save_state(&st); Response::Ok
        }
    }
}

async fn game_detect_task(ctx: Arc<Ctx>) {
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    loop {
        tick.tick().await;
        let running = tokio::task::spawn_blocking(system::get_running_game).await.unwrap_or(None);
        let mut st = ctx.shared.write().await;
        if st.running_game != running {
            if let Some(game) = &running {
                if let Some(id) = game.steam_id {
                    info!("Game running: {} (Steam {})", game.name, id);
                } else {
                    info!("Game running: {} (Wine/Non-Steam)", game.name);
                }
            } else {
                info!("Game stopped");
            }
            st.running_game = running;
            publish(&st, &ctx.rt_tx);
        }
    }
}

async fn battery_task(ctx: Arc<Ctx>) {
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    let mut last_sync_color: Option<[u8; 3]> = None;
    loop {
        tick.tick().await;
        let battery = tokio::task::spawn_blocking(system::read_battery).await.ok().flatten();
        let resync = {
            let mut st = ctx.shared.write().await;
            st.battery = battery.clone();
            if st.settings.sync_lightbar_battery && st.controller_connected {
                let color = system::battery_color(battery.as_ref().map_or(100, |b| b.0));
                let changed = last_sync_color != Some(color); last_sync_color = Some(color); changed
            } else { last_sync_color = None; false }
        };
        if resync { apply_lightbar(&ctx).await; }
    }
}

fn rule_matches(rule: &MatchRule, class: Option<&str>) -> bool {
    let wc = rule.window_class.as_deref().filter(|s| !s.trim().is_empty());
    let pn = rule.process_name.as_deref().filter(|s| !s.trim().is_empty());
    if wc.is_none() && pn.is_none() { return false; }
    let class_ok = wc.map_or(true, |w| class.map_or(false, |c| c.to_lowercase().contains(&w.trim().to_lowercase())));
    class_ok && pn.is_none()
}

fn pick_profile(profiles: &HashMap<String, Profile>, class: Option<&str>) -> String {
    let mut names: Vec<&String> = profiles.keys().filter(|n| n.as_str() != DEFAULT_PROFILE).collect();
    names.sort();
    for n in names { if profiles[n].match_rules.iter().any(|r| rule_matches(r, class)) { return n.clone(); } }
    DEFAULT_PROFILE.to_string()
}

async fn autoprofile_task(ctx: Arc<Ctx>) {
    let mut tick = tokio::time::interval(Duration::from_millis(1500));
    let mut last_class = None;

    loop {
        tick.tick().await;
        let (auto, has_rules) = {
            let st = ctx.shared.read().await;
            let rules = st.profiles.values().any(|p| !p.match_rules.is_empty());
            (st.auto_profile, rules)
        };

        if !auto || !has_rules { continue; }

        let class = tokio::task::spawn_blocking(system::active_window_class).await.ok().flatten();

        // Skip heavy lookups if the window hasn't changed
        if class == last_class {
            continue;
        }
        last_class = class.clone();

        let mut st = ctx.shared.write().await;
        st.active_app = class.clone();

        let target = pick_profile(&st.profiles, class.as_deref());
        if target != st.active_profile && st.profiles.contains_key(&target) {
            info!("auto-profile: {} -> {target}", st.active_profile);
            st.active_profile = target;
            publish(&st, &ctx.rt_tx);
        }
    }
}
