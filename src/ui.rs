//! StynPS PlaySelect: egui/eframe frontend.

use std::{
    collections::{HashMap, HashSet},
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, vec2, Align, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Frame, Layout, Margin, Painter, Pos2, Rect,
    RichText, Rounding, Sense, Stroke, Vec2,
};
use serde_json::{json, Value};

use crate::{
    daemon::{self, GlobalSettings, MatchRule, Profile},
    system,
};

const SUPPORT_URL: &str = "https://github.com/Frnkynstyn-IV/StynPS-PlaySelect-/issues";
const TELEGRAM_URL: &str = "https://t.me/StynPS";
const BOOT_SECS: f64 = 2.4;
const BOOT_FADE: f64 = 0.45;

const BTN_SOUTH: u16 = 304;
const BTN_EAST: u16 = 305;
const BTN_NORTH: u16 = 307;
const BTN_WEST: u16 = 308;
const BTN_TL: u16 = 310;
const BTN_TR: u16 = 311;
const BTN_SELECT: u16 = 314;
const BTN_START: u16 = 315;
const BTN_MODE: u16 = 316;
const BTN_THUMBL: u16 = 317;
const BTN_THUMBR: u16 = 318;

const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_Z: u16 = 2;
const ABS_RX: u16 = 3;
const ABS_RY: u16 = 4;
const ABS_RZ: u16 = 5;
const ABS_HAT0X: u16 = 16;
const ABS_HAT0Y: u16 = 17;

const BUTTONS: [(&str, &str, u16); 15] = [
    ("D-Pad Up", "ABS_HAT0Y_UP", ABS_HAT0Y),
    ("D-Pad Down", "ABS_HAT0Y_DOWN", ABS_HAT0Y),
    ("D-Pad Left", "ABS_HAT0X_LEFT", ABS_HAT0X),
    ("D-Pad Right", "ABS_HAT0X_RIGHT", ABS_HAT0X),
    ("Cross", "BTN_SOUTH", BTN_SOUTH),
    ("Circle", "BTN_EAST", BTN_EAST),
    ("Square", "BTN_WEST", BTN_WEST),
    ("Triangle", "BTN_NORTH", BTN_NORTH),
    ("L1", "BTN_TL", BTN_TL),
    ("R1", "BTN_TR", BTN_TR),
    ("L3", "BTN_THUMBL", BTN_THUMBL),
    ("R3", "BTN_THUMBR", BTN_THUMBR),
    ("Create", "BTN_SELECT", BTN_SELECT),
    ("Options", "BTN_START", BTN_START),
    ("PS", "BTN_MODE", BTN_MODE),
];

const AXES: [(&str, &str, u16); 6] = [
    ("Left Trigger (L2)", "ABS_Z", ABS_Z),
    ("Right Trigger (R2)", "ABS_RZ", ABS_RZ),
    ("Left Stick X", "ABS_X", ABS_X),
    ("Left Stick Y", "ABS_Y", ABS_Y),
    ("Right Stick X", "ABS_RX", ABS_RX),
    ("Right Stick Y", "ABS_RY", ABS_RY),
];

/// Touchpad and lightbar outline fractions tailored to standard DualSense layout.
struct ControllerLayout {
    hw_top: f32,
    hw_bot: f32,
    y_top: f32,
    y_bot: f32,
    corner: f32,   // horizontal radius of the bottom corners
    corner_v: f32, // vertical radius of the bottom corners
}

impl Default for ControllerLayout {
    fn default() -> Self {
        Self {
            hw_top: 0.1724,
            hw_bot: 0.1508,
            y_top: 0.255,
            y_bot: 0.414,
            corner: 0.055,
            corner_v: 0.040,
        }
    }
}

fn get_icon(name: &str, is_dark: bool) -> Option<egui::Image<'static>> {
    let (uri, bytes): (&str, &[u8]) = match (name, is_dark) {
        ("Cross", true) | ("BTN_SOUTH", true) => ("bytes://cross_d.png", include_bytes!("../assets/cross_dark.png") as &[u8]),
        ("Cross", false) | ("BTN_SOUTH", false) => ("bytes://cross_l.png", include_bytes!("../assets/cross_light.png") as &[u8]),
        ("Circle", true) | ("BTN_EAST", true) => ("bytes://circle_d.png", include_bytes!("../assets/circle_dark.png") as &[u8]),
        ("Circle", false) | ("BTN_EAST", false) => ("bytes://circle_l.png", include_bytes!("../assets/circle_light.png") as &[u8]),
        ("Square", true) | ("BTN_WEST", true) => ("bytes://square_d.png", include_bytes!("../assets/square_dark.png") as &[u8]),
        ("Square", false) | ("BTN_WEST", false) => ("bytes://square_l.png", include_bytes!("../assets/square_light.png") as &[u8]),
        ("Triangle", true) | ("BTN_NORTH", true) => ("bytes://triangle_d.png", include_bytes!("../assets/triangle_dark.png") as &[u8]),
        ("Triangle", false) | ("BTN_NORTH", false) => ("bytes://triangle_l.png", include_bytes!("../assets/triangle_light.png") as &[u8]),
        ("D-Pad Up", true) | ("ABS_HAT0Y_UP", true) => ("bytes://du_d.png", include_bytes!("../assets/dpad_up_dark.png") as &[u8]),
        ("D-Pad Up", false) | ("ABS_HAT0Y_UP", false) => ("bytes://du_l.png", include_bytes!("../assets/dpad_up_light.png") as &[u8]),
        ("D-Pad Down", true) | ("ABS_HAT0Y_DOWN", true) => ("bytes://dd_d.png", include_bytes!("../assets/dpad_down_dark.png") as &[u8]),
        ("D-Pad Down", false) | ("ABS_HAT0Y_DOWN", false) => ("bytes://dd_l.png", include_bytes!("../assets/dpad_down_light.png") as &[u8]),
        ("D-Pad Left", true) | ("ABS_HAT0X_LEFT", true) => ("bytes://dl_d.png", include_bytes!("../assets/dpad_left_dark.png") as &[u8]),
        ("D-Pad Left", false) | ("ABS_HAT0X_LEFT", false) => ("bytes://dl_l.png", include_bytes!("../assets/dpad_left_light.png") as &[u8]),
        ("D-Pad Right", true) | ("ABS_HAT0X_RIGHT", true) => ("bytes://dr_d.png", include_bytes!("../assets/dpad_right_dark.png") as &[u8]),
        ("D-Pad Right", false) | ("ABS_HAT0X_RIGHT", false) => ("bytes://dr_l.png", include_bytes!("../assets/dpad_right_light.png") as &[u8]),
        ("L1", true) | ("BTN_TL", true) => ("bytes://l1_d.png", include_bytes!("../assets/l1_dark.png") as &[u8]),
        ("L1", false) | ("BTN_TL", false) => ("bytes://l1_l.png", include_bytes!("../assets/l1_light.png") as &[u8]),
        ("R1", true) | ("BTN_TR", true) => ("bytes://r1_d.png", include_bytes!("../assets/r1_dark.png") as &[u8]),
        ("R1", false) | ("BTN_TR", false) => ("bytes://r1_l.png", include_bytes!("../assets/r1_light.png") as &[u8]),
        ("Left Trigger (L2)", true) | ("ABS_Z", true) => ("bytes://l2_d.png", include_bytes!("../assets/l2_dark.png") as &[u8]),
        ("Left Trigger (L2)", false) | ("ABS_Z", false) => ("bytes://l2_l.png", include_bytes!("../assets/l2_light.png") as &[u8]),
        ("Right Trigger (R2)", true) | ("ABS_RZ", true) => ("bytes://r2_d.png", include_bytes!("../assets/r2_dark.png") as &[u8]),
        ("Right Trigger (R2)", false) | ("ABS_RZ", false) => ("bytes://r2_l.png", include_bytes!("../assets/r2_light.png") as &[u8]),
        ("L3", true) | ("BTN_THUMBL", true) | ("Left Stick X", true) | ("Left Stick Y", true) | ("ABS_X", true) | ("ABS_Y", true) => ("bytes://ls_d.png", include_bytes!("../assets/ls_dark.png") as &[u8]),
        ("L3", false) | ("BTN_THUMBL", false) | ("Left Stick X", false) | ("Left Stick Y", false) | ("ABS_X", false) | ("ABS_Y", false) => ("bytes://ls_l.png", include_bytes!("../assets/ls_light.png") as &[u8]),
        ("R3", true) | ("BTN_THUMBR", true) | ("Right Stick X", true) | ("Right Stick Y", true) | ("ABS_RX", true) | ("ABS_RY", true) => ("bytes://rs_d.png", include_bytes!("../assets/r3_dark.png") as &[u8]),
        ("R3", false) | ("BTN_THUMBR", false) | ("Right Stick X", false) | ("Right Stick Y", false) | ("ABS_RX", false) | ("ABS_RY", false) => ("bytes://rs_l.png", include_bytes!("../assets/r3_light.png") as &[u8]),
        ("Create", true) | ("BTN_SELECT", true) => ("bytes://create_d.png", include_bytes!("../assets/start_dark.png") as &[u8]),
        ("Create", false) | ("BTN_SELECT", false) => ("bytes://create_l.png", include_bytes!("../assets/start_light.png") as &[u8]),
        ("Options", true) | ("BTN_START", true) => ("bytes://options_d.png", include_bytes!("../assets/options_dark.png") as &[u8]),
        ("Options", false) | ("BTN_START", false) => ("bytes://options_l.png", include_bytes!("../assets/options_light.png") as &[u8]),
        ("PS", true) | ("BTN_MODE", true) => ("bytes://ps_d.png", include_bytes!("../assets/ps_dark.png") as &[u8]),
        ("PS", false) | ("BTN_MODE", false) => ("bytes://ps_l.png", include_bytes!("../assets/ps_light.png") as &[u8]),
        _ => return None,
    };
    Some(egui::Image::from_bytes(uri, bytes).fit_to_exact_size(vec2(24.0, 24.0)))
}

fn paint_logo(ui: &egui::Ui, rect: Rect, is_dark: bool) {
    let (uri, bytes): (&str, &[u8]) = if is_dark {
        ("bytes://logo_d.png", include_bytes!("../assets/logo_dark.png") as &[u8])
    } else {
        ("bytes://logo_l.png", include_bytes!("../assets/logo_light.png") as &[u8])
    };
    let img = egui::Image::from_bytes(uri, bytes).fit_to_exact_size(rect.size());
    img.paint_at(ui, rect);
}

fn draw_arrow(p: &Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    let stroke = Stroke::new(2.0_f32, color);
    p.line_segment([center + vec2(-4.0, 0.0), center + vec2(4.0, 0.0)], stroke);
    p.line_segment([center + vec2(1.0, -3.0), center + vec2(4.0, 0.0)], stroke);
    p.line_segment([center + vec2(1.0, 3.0), center + vec2(4.0, 0.0)], stroke);
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn paint_beeping_dot(p: &Painter, c: Pos2, time: f64, color: Color32, speed: f64) {
    let beat = ((time * speed) % 1.0) as f32;
    let alpha = if beat < 0.5 { 1.0 - (beat / 0.5).powi(2) } else { 0.0 };
    let radius = 3.0 + (1.0 - alpha) * 8.0;
    p.circle_filled(c, 3.0, color);
    if alpha > 0.0 {
        p.circle_stroke(c, radius, Stroke::new(1.5_f32, color.gamma_multiply(alpha)));
    }
}

fn ease_out(x: f64) -> f64 {
    1.0 - (1.0 - x).powi(3)
}

fn glow(p: &Painter, c: Pos2, r: f32, col: Color32, k: f32) {
    for i in 1..=3 {
        let f = i as f32;
        p.circle_filled(c, r + f * r * 0.15, col.gamma_multiply(0.1 * k / f));
    }
}

fn paint_controller(ui: &mut egui::Ui, lb_color: Color32, lb_brightness: f32, is_rumbling: bool) {
    let is_dark = ui.ctx().style().visuals.dark_mode;
    let img_bytes = if is_dark { include_bytes!("../assets/controller_white.png") as &[u8] } else { include_bytes!("../assets/controller_black.png") as &[u8] };
    let img_uri = if is_dark { "bytes://controller_white.png" } else { "bytes://controller_black.png" };

    let w = ui.available_width();
    let mut rect = Rect::from_center_size(ui.cursor().center_top() + vec2(0.0, w * 0.5), vec2(w, w));

    if is_rumbling {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
        let shake = if (nanos / 50_000_000) % 2 == 0 { 4.0 } else { -4.0 };
        rect = rect.translate(vec2(shake, shake));
        ui.ctx().request_repaint();
    }

    ui.allocate_exact_size(vec2(w, w), Sense::hover());
    let p = ui.painter_at(rect);

    let img = egui::Image::from_bytes(img_uri, img_bytes).fit_to_exact_size(rect.size()).maintain_aspect_ratio(true);
    img.paint_at(ui, rect);

    // --- Lightbar ---
    let k = (lb_brightness / 100.0).clamp(0.0, 1.0);
    let lb_c = Color32::from_rgb(
        (lb_color.r() as f32 * k) as u8,
                                 (lb_color.g() as f32 * k) as u8,
                                 (lb_color.b() as f32 * k) as u8,
    );

    let at = |f: (f32, f32)| Pos2::new(rect.min.x + f.0 * w, rect.min.y + f.1 * rect.height());

    let layout = ControllerLayout::default();
    let (cx, hwt, hwb) = (0.5_f32, layout.hw_top, layout.hw_bot);
    let (yt, yb, cr, cv) = (layout.y_top, layout.y_bot, layout.corner, layout.corner_v);
    let ye = yb - cv;

    let mut pts: Vec<Pos2> = Vec::new();
    let mut add = |c: [(f32, f32); 4]| {
        let c = c.map(|f| at(f));
        let mut seg = egui::epaint::CubicBezierShape::from_points_stroke(c, false, Color32::TRANSPARENT, Stroke::new(0.0_f32, Color32::TRANSPARENT)).flatten(Some(0.2_f32));
        if !pts.is_empty() && !seg.is_empty() { seg.remove(0); }
        pts.extend(seg);
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;

    add([
        (cx - hwt, yt),
        (cx - lerp(hwt, hwb, 1.0 / 3.0), lerp(yt, ye, 1.0 / 3.0)),
        (cx - lerp(hwt, hwb, 2.0 / 3.0), lerp(yt, ye, 2.0 / 3.0)),
        (cx - hwb, ye),
    ]);
    let left_corner = [(cx - hwb, ye), (cx - hwb + 0.002, ye + 0.55 * cv), (cx - hwb + 0.45 * cr, yb), (cx - hwb + cr, yb)];
    add(left_corner);
    add([(cx - hwb + cr, yb), (cx - 0.06, yb + 0.004), (cx + 0.06, yb + 0.004), (cx + hwb - cr, yb)]);
    let right_corner = [(cx + hwb - cr, yb), (cx + hwb - 0.45 * cr, yb), (cx + hwb - 0.002, ye + 0.55 * cv), (cx + hwb, ye)];
    add(right_corner);
    add([
        (cx + hwb, ye),
        (cx + lerp(hwb, hwt, 1.0 / 3.0), lerp(ye, yt, 1.0 / 3.0)),
        (cx + lerp(hwb, hwt, 2.0 / 3.0), lerp(ye, yt, 2.0 / 3.0)),
        (cx + hwt, yt),
    ]);

    let s = (w / 723.0).max(0.4);
    let hot = Color32::from_rgb(
        (lb_c.r() as f32 + (255.0 - lb_c.r() as f32) * 0.45 * k) as u8,
                                (lb_c.g() as f32 + (255.0 - lb_c.g() as f32) * 0.45 * k) as u8,
                                (lb_c.b() as f32 + (255.0 - lb_c.b() as f32) * 0.45 * k) as u8,
    );

    let layers: [(f32, Color32, bool); 9] = [
        (34.0, lb_c.gamma_multiply(0.035), false),
        (25.0, lb_c.gamma_multiply(0.06), false),
        (18.0, lb_c.gamma_multiply(0.11), false),
        (13.0, lb_c.gamma_multiply(0.20), false),
        (10.0, lb_c, true),
        (6.5, lb_c, true),
        (4.0, hot.gamma_multiply(0.85), false),
        (2.0, hot, false),
        (0.9, Color32::WHITE.gamma_multiply(0.35 * k), false),
    ];
    for (px, col, caps) in layers {
        let width = px * s;
        p.add(egui::Shape::line(pts.clone(), Stroke::new(width, col)));
        if caps {
            if let (Some(&a), Some(&b)) = (pts.first(), pts.last()) {
                p.circle_filled(a, width * 0.5, col);
                p.circle_filled(b, width * 0.5, col);
            }
        }
    }
}

#[derive(Clone, Default)]
struct Live {
    connected: bool,
    have_status: bool,
    controller: bool,
    battery: Option<u8>,
    battery_status: Option<String>,
    active_profile: String,
    auto_profile: bool,
    active_app: Option<String>,
    profiles: Vec<Profile>,
    buttons: HashSet<u16>,
    axes: HashMap<u16, i32>,
    settings: GlobalSettings,
    running_game_name: Option<String>,
    running_game_steam_id: Option<u32>,
    steam_mode_active: bool,
    controller_ids: Option<[u16; 2]>,
    bluetooth: bool,
    last_error: Option<String>,
    rumble_error: Option<String>,
    lightbar_error: Option<String>,
    cmd_error: Option<String>,
}

fn request(w: &mut UnixStream, r: &mut BufReader<UnixStream>, cmd: &Value) -> io::Result<Value> {
    writeln!(w, "{cmd}")?;
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 { return Err(io::ErrorKind::UnexpectedEof.into()); }
    Ok(serde_json::from_str(&line).unwrap_or(Value::Null))
}

fn opt_str(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn session(live: &Arc<Mutex<Live>>, ctx: &egui::Context, rx: &mpsc::Receiver<Value>, stream: UnixStream) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut w = stream.try_clone()?;
    let mut r = BufReader::new(stream);
    let mut force_slow = true;
    let mut last_slow = Instant::now();

    loop {
        while let Ok(cmd) = rx.try_recv() {
            let resp = request(&mut w, &mut r, &cmd)?;
            if resp["status"].as_str() == Some("error") { lock(live).cmd_error = opt_str(&resp["message"]); }
            force_slow = true;
        }

        let inp = request(&mut w, &mut r, &json!({"cmd": "get_input"}))?;
        let buttons: HashSet<u16> = inp["buttons"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64().map(|x| x as u16)).collect()).unwrap_or_default();
        let axes: HashMap<u16, i32> = inp["axes"].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_i64()? as i32))).collect()).unwrap_or_default();

        let mut changed = {
            let mut l = lock(live);
            let c = l.buttons != buttons || l.axes != axes;
            l.buttons = buttons;
            l.axes = axes;
            c
        };

        if force_slow || last_slow.elapsed() > Duration::from_millis(400) {
            let st = request(&mut w, &mut r, &json!({"cmd": "get_status"}))?;
            let pr = request(&mut w, &mut r, &json!({"cmd": "list_profiles"}))?;
            let mut profiles: Vec<Profile> = serde_json::from_value(pr["profiles"].clone()).unwrap_or_default();
            profiles.sort_by(|a, b| a.name.cmp(&b.name));
            let mut l = lock(live);
            l.have_status = st["status"].as_str() == Some("status");
            l.controller = st["connected"].as_bool().unwrap_or(false);
            l.battery = st["battery"].as_u64().map(|b| b as u8);
            l.battery_status = opt_str(&st["battery_status"]);
            l.active_profile = st["active_profile"].as_str().unwrap_or("").to_string();
            l.auto_profile = st["auto_profile"].as_bool().unwrap_or(false);
            l.active_app = opt_str(&st["active_app"]);
            if let Ok(settings) = serde_json::from_value(st["settings"].clone()) { l.settings = settings; }
            l.running_game_name = opt_str(&st["running_game_name"]);
            l.running_game_steam_id = st["running_game_steam_id"].as_u64().map(|x| x as u32);
            l.steam_mode_active = st["steam_mode_active"].as_bool().unwrap_or(false);
            l.controller_ids = st["controller_ids"].as_array().and_then(|a| Some([a.first()?.as_u64()? as u16, a.get(1)?.as_u64()? as u16]));
            l.bluetooth = st["bluetooth"].as_bool().unwrap_or(false);
            l.last_error = opt_str(&st["last_error"]);
            l.rumble_error = opt_str(&st["rumble_error"]);
            l.lightbar_error = opt_str(&st["lightbar_error"]);
            l.profiles = profiles;
            force_slow = false;
            last_slow = Instant::now();
            changed = true;
        }

        if changed { ctx.request_repaint(); }
        thread::sleep(Duration::from_millis(16));
    }
}

fn spawn_worker(ctx: egui::Context, live: Arc<Mutex<Live>>, rx: mpsc::Receiver<Value>) {
    thread::spawn(move || loop {
        match UnixStream::connect(daemon::socket_path()) {
            Ok(stream) => {
                lock(&live).connected = true;
                ctx.request_repaint();
                let _ = session(&live, &ctx, &rx, stream);
                *lock(&live) = Live::default();
                ctx.request_repaint();
            }
            Err(_) => thread::sleep(Duration::from_secs(1)),
        }
    });
}

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
enum Theme {
    Dark,
    Light,
    MinimalLight,
    MinimalDark,
}

impl Theme {
    fn is_dark(self) -> bool {
        matches!(self, Theme::Dark | Theme::MinimalDark)
    }
}

#[derive(Clone, Copy)]
struct Pal {
    bg: Color32,
    panel: Color32,
    card: Color32,
    text: Color32,
    sub: Color32,
    accent: Color32,
    line: Color32,
    error_bg: Color32,
    error_line: Color32,
}

fn palette(theme: Theme, accent: Color32) -> Pal {
    match theme {
        Theme::Dark => Pal {
            bg: Color32::BLACK,
            panel: Color32::BLACK,
            card: Color32::BLACK,
            text: Color32::WHITE,
            sub: Color32::from_rgb(180, 180, 180),
            accent: if accent == Color32::BLACK { Color32::WHITE } else { accent },
            line: Color32::from_rgb(60, 60, 60),
            error_bg: Color32::from_rgb(80, 20, 20),
            error_line: Color32::RED,
        },
        Theme::Light => Pal {
            bg: Color32::WHITE,
            panel: Color32::from_rgb(240, 240, 240),
            card: Color32::from_rgb(250, 250, 250),
            text: Color32::BLACK,
            sub: Color32::from_rgb(80, 80, 80),
            accent: if accent == Color32::WHITE { Color32::BLACK } else { accent },
            line: Color32::from_rgb(200, 200, 200),
            error_bg: Color32::from_rgb(255, 220, 220),
            error_line: Color32::RED,
        },
        Theme::MinimalLight => Pal {
            bg: Color32::WHITE,
            panel: Color32::WHITE,
            card: Color32::WHITE,
            text: Color32::BLACK,
            sub: Color32::from_rgb(50, 50, 50),
            accent: Color32::BLACK,
            line: Color32::BLACK,
            error_bg: Color32::WHITE,
            error_line: Color32::BLACK,
        },
        Theme::MinimalDark => Pal {
            bg: Color32::BLACK,
            panel: Color32::BLACK,
            card: Color32::BLACK,
            text: Color32::WHITE,
            sub: Color32::from_rgb(200, 200, 200),
            accent: Color32::WHITE,
            line: Color32::WHITE,
            error_bg: Color32::BLACK,
            error_line: Color32::WHITE,
        },
    }
}

fn apply_style(ctx: &egui::Context, pal: &Pal, theme: Theme) {
    let mut v = if theme.is_dark() { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = pal.bg;
    v.window_fill = pal.card;
    v.override_text_color = Some(pal.text);
    v.selection.bg_fill = pal.accent.gamma_multiply(0.55);
    v.extreme_bg_color = pal.panel;
    let r = Rounding::same(14.0);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.rounding = r;
    }
    v.widgets.inactive.weak_bg_fill = pal.card;
    v.widgets.inactive.bg_fill = Color32::TRANSPARENT;
    v.widgets.hovered.weak_bg_fill = pal.accent.gamma_multiply(0.35);
    v.widgets.hovered.bg_fill = Color32::TRANSPARENT;
    v.widgets.active.weak_bg_fill = pal.accent.gamma_multiply(0.6);
    v.widgets.active.bg_fill = Color32::TRANSPARENT;

    v.widgets.inactive.fg_stroke = Stroke::new(1.5_f32, pal.text);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, pal.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.5_f32, pal.accent);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.5_f32, pal.text);

    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, pal.text.gamma_multiply(0.5));
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, pal.accent);

    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = vec2(12.0, 12.0);
        s.spacing.button_padding = vec2(16.0, 8.0);
        s.spacing.combo_width = 120.0;
    });
}

fn card<R>(ui: &mut egui::Ui, pal: &Pal, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    Frame::none()
    .fill(pal.card)
    .stroke(Stroke::new(1.0_f32, pal.accent.gamma_multiply(0.6)))
    .rounding(Rounding::same(26.0_f32))
    .inner_margin(Margin::same(20.0))
    .show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    })
    .inner
}

fn pill(ui: &mut egui::Ui, pal: &Pal, text: &str, on: bool) -> egui::Response {
    let (fill, text_col) = if on {
        let contrast = if pal.accent.r() > 128 && pal.accent.g() > 128 && pal.accent.b() > 128 {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        (pal.accent, contrast)
    } else {
        (pal.card, pal.text)
    };

    ui.add(
        egui::Button::new(RichText::new(text).color(text_col).size(14.5))
        .fill(fill)
        .rounding(Rounding::same(8.0))
        .min_size(vec2(0.0, 36.0)),
    )
}

fn nav_button(ui: &mut egui::Ui, pal: &Pal, text: &str, selected: bool) -> egui::Response {
    let (fill, col, stroke) = if selected {
        let text_color = if pal.accent.r() > 128 && pal.accent.g() > 128 && pal.accent.b() > 128 {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        (pal.accent, text_color, Stroke::new(1.0_f32, pal.accent))
    } else {
        (Color32::TRANSPARENT, pal.sub, Stroke::NONE)
    };

    let btn = egui::Button::new(RichText::new(text).color(col).size(16.0).family(FontFamily::Name("LuxuryTitle".into())))
    .fill(fill)
    .rounding(Rounding::same(12.0))
    .min_size(vec2(ui.available_width(), 40.0));
    let resp = ui.add(btn);
    if selected {
        ui.painter().rect_stroke(resp.rect, Rounding::same(12.0), stroke);
    }
    resp
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Overview,
    Mapping,
    AutoProfiles,
    Lightbar,
    Rumble,
    Touchpad,
    Steam,
    Settings,
    About,
}

const TABS: [(Tab, &str); 9] = [
    (Tab::Overview, "Overview"),
    (Tab::Mapping, "Controller Mapping"),
    (Tab::AutoProfiles, "Auto-Profiles"),
    (Tab::Lightbar, "Lightbar"),
    (Tab::Rumble, "Rumble"),
    (Tab::Touchpad, "Touchpad and Gyro"),
    (Tab::Steam, "Steam Input"),
    (Tab::Settings, "Settings"),
    (Tab::About, "About"),
];

const DS4_IDS: [(u16, u16); 2] = [(0x054c, 0x05c4), (0x054c, 0x09cc)];

fn banner(ui: &mut egui::Ui, pal: &Pal, text: &str, dismissable: bool) -> bool {
    let mut dismissed = false;
    Frame::none()
    .fill(pal.error_bg)
    .stroke(Stroke::new(1.0_f32, pal.error_line))
    .rounding(Rounding::same(10.0))
    .inner_margin(Margin::symmetric(12.0, 8.0))
    .show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(text).color(pal.text).size(13.0));
            if dismissable {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("X").clicked() {
                        dismissed = true;
                    }
                });
            }
        });
    });
    ui.add_space(8.0);
    dismissed
}

fn animated_red_banner(ui: &mut egui::Ui, id_salt: &str, is_visible: bool, text: &str) {
    let id = ui.make_persistent_id(id_salt);
    let t = ui.ctx().animate_bool_with_time(id, is_visible, 0.3);

    if t > 0.0 {
        let bg = Color32::from_rgb(60, 10, 10).gamma_multiply(t);
        let stroke_col = Color32::from_rgb(255, 80, 80).gamma_multiply(t);
        let text_col = Color32::from_rgb(255, 100, 100).gamma_multiply(t);

        Frame::none()
        .fill(bg)
        .stroke(Stroke::new(1.5_f32, stroke_col))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(12.0, 8.0 * t))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).color(text_col).size(13.0).strong());
            });
        });
        ui.add_space(8.0 * t);
    }
}

fn map_combo(ui: &mut egui::Ui, draft: &mut Profile, name: &str, dark: bool, width: f32) {
    let cur = draft.button_map.get(name).cloned().unwrap_or_else(|| name.to_string());
    let cur_label = BUTTONS.iter().find(|b| b.1 == cur).map_or(cur.as_str(), |b| b.0).to_string();
    ui.push_id(name, |ui| {
        ui.scope(|ui| {
            let r = Rounding::same(8.0);
            let v = ui.visuals_mut();
            v.widgets.inactive.rounding = r;
            v.widgets.hovered.rounding = r;
            v.widgets.active.rounding = r;
            v.widgets.open.rounding = r;
            v.window_rounding = r;

            egui::ComboBox::from_id_source("map_combo")
            .selected_text(RichText::new(cur_label).size(14.0))
            .width(width)
            .show_ui(ui, |ui| {
                for (l2, n2, _) in BUTTONS {
                    let mut clicked = false;
                    ui.horizontal(|ui| {
                        if let Some(img) = get_icon(n2, dark) {
                            ui.add(img);
                        }
                        ui.add_space(4.0);
                        if ui.selectable_label(cur == n2, RichText::new(l2).size(14.0)).clicked() {
                            clicked = true;
                        }
                    });
                    if clicked {
                        if n2 == name {
                            draft.button_map.remove(name);
                        } else {
                            draft.button_map.insert(name.to_string(), n2.to_string());
                        }
                    }
                }
            });
        });
    });
}

pub struct App {
    live: Arc<Mutex<Live>>,
    tx: mpsc::Sender<Value>,
    tab: Tab,
    theme: Theme,
    accent: Color32,
    boot_t0: Option<f64>,
    skip_boot: bool,
    draft: Profile,
    draft_for: String,
    ap_draft: Option<Vec<Profile>>,
    new_profile: String,

    lb_color: Color32,
    lb_hex: String,
    lb_brightness: f32,
    lb_sync_battery: bool,
    lb_loaded: bool,

    rumble_small: f32,
    rumble_large: f32,
    rumble_pass: bool,
    rumble_loaded: bool,
    rumble_until: Option<Instant>,
    perm_msg: Arc<Mutex<Option<String>>>,
    rumble_warning_until: Option<Instant>,
    lightbar_warning_until: Option<Instant>,

    prev_controller: bool,
    connected_rumble_seq: Option<u8>,
    connected_rumble_timer: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut fonts = FontDefinitions::default();

        fonts.font_data.insert(
            "inter".to_owned(),
                               FontData::from_static(include_bytes!("../assets/Inter-VariableFont_opsz,wght.ttf")),
        );
        fonts.font_data.insert(
            "inter_italic".to_owned(),
                               FontData::from_static(include_bytes!("../assets/Inter-Italic-VariableFont_opsz,wght.ttf")),
        );

        fonts.font_data.insert(
            "playfair".to_owned(),
                               FontData::from_static(include_bytes!("../assets/PlayfairDisplay-VariableFont_wght.ttf")),
        );

        fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "inter".to_owned());
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(1, "inter_italic".to_owned());

        fonts.families.insert(FontFamily::Name("LuxuryTitle".into()), vec!["playfair".to_owned()]);

        cc.egui_ctx.set_fonts(fonts);

        // THIS LINE LOADS THE IMAGE DECODERS
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let theme = if let Some(storage) = cc.storage {
            eframe::get_value(storage, "theme").unwrap_or(Theme::MinimalDark)
        } else {
            Theme::MinimalDark
        };

        let live = Arc::new(Mutex::new(Live::default()));
        let (tx, rx) = mpsc::channel();
        spawn_worker(cc.egui_ctx.clone(), live.clone(), rx);
        Self {
            live,
            tx,
            tab: Tab::Overview,
            theme,
            accent: Color32::from_rgb(0, 112, 209),
            boot_t0: None,
            skip_boot: false,
            draft: Profile::default(),
            draft_for: String::new(),
            ap_draft: None,
            new_profile: String::new(),
            lb_color: Color32::from_rgb(180, 180, 180),
            lb_hex: "#B4B4B4".to_string(),
            lb_brightness: 80.0,
            lb_sync_battery: false,
            lb_loaded: false,
            rumble_small: 255.0,
            rumble_large: 255.0,
            rumble_pass: true,
            rumble_loaded: false,
            rumble_until: None,
            perm_msg: Arc::new(Mutex::new(None)),
            rumble_warning_until: None,
            lightbar_warning_until: None,
            prev_controller: false,
            connected_rumble_seq: None,
            connected_rumble_timer: Instant::now(),
        }
    }

    fn send(&self, v: Value) {
        let _ = self.tx.send(v);
    }

    fn rumbling(&self) -> bool {
        self.rumble_until.map_or(false, |t| Instant::now() < t)
    }

    fn sync_from_daemon(&mut self, live: &Live) {
        if !live.connected || !live.have_status {
            self.lb_loaded = false;
            self.rumble_loaded = false;
            return;
        }
        let s = &live.settings;
        if !self.lb_loaded {
            self.lb_color = Color32::from_rgb(s.lb_color[0], s.lb_color[1], s.lb_color[2]);
            self.lb_hex = format!("#{:02X}{:02X}{:02X}", s.lb_color[0], s.lb_color[1], s.lb_color[2]);
            self.lb_brightness = s.lb_brightness;
            self.lb_sync_battery = s.sync_lightbar_battery;
            self.lb_loaded = true;
        }
        if !self.rumble_loaded {
            self.rumble_small = s.rumble_strength_small as f32;
            self.rumble_large = s.rumble_strength_large as f32;
            self.rumble_pass = s.rumble_passthrough;
            self.rumble_loaded = true;
        }
    }

    fn send_rumble_strength(&self) {
        self.send(json!({ "cmd": "set_rumble_strength", "small": self.rumble_small as u8, "large": self.rumble_large as u8, "passthrough": self.rumble_pass }));
    }

    fn boot_screen(&self, ctx: &egui::Context, pal: &Pal, t: f64) {
        egui::CentralPanel::default().frame(Frame::none().fill(pal.bg)).show(ctx, |ui| {
            let p = ui.painter();
            let c = ui.max_rect().center() - vec2(0.0, 30.0);
            let e = ease_out((t / 1.1).min(1.0)) as f32;
            for i in 0..3 {
                let ph = ((t * 0.7 + i as f64 * 0.33) % 1.0) as f32;
                p.circle_stroke(c, 60.0 + ph * 200.0, Stroke::new(2.0_f32, pal.accent.gamma_multiply((1.0 - ph) * 0.4 * e)));
            }
            glow(p, c, 62.0 * e, pal.accent, 0.8 * e);

            let logo_rect = Rect::from_center_size(c, vec2(140.0 * e, 140.0 * e));
            paint_logo(ui, logo_rect, self.theme.is_dark());

            let a = (((t - 0.6) / 0.6).clamp(0.0, 1.0)) as f32;
            p.text(c + vec2(0.0, 108.0), Align2::CENTER_CENTER, "StynPS PlaySelect", FontId::new(28.0, FontFamily::Name("LuxuryTitle".into())), pal.text.gamma_multiply(a));
            p.text(c + vec2(0.0, 138.0), Align2::CENTER_CENTER, "Starting up...", FontId::proportional(14.0), pal.sub.gamma_multiply(a));
            let bar = Rect::from_center_size(c + vec2(0.0, 175.0), vec2(260.0, 6.0));
            p.rect_filled(bar, Rounding::same(3.0), pal.line.gamma_multiply(0.6));
            let mut fill = bar;
            fill.set_width(bar.width() * ((t / BOOT_SECS).clamp(0.0, 1.0) as f32));
            p.rect_filled(fill, Rounding::same(3.0), pal.accent);
        });
    }

    fn sidebar(&mut self, ctx: &egui::Context, pal: &Pal, live: &Live) {
        egui::SidePanel::left("sidebar")
        .exact_width(240.0)
        .resizable(false)
        .frame(Frame::none().fill(pal.panel).inner_margin(Margin::same(16.0)))
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                paint_logo(ui, r, self.theme.is_dark());
                ui.add_space(4.0);
                ui.label(RichText::new("StynPS").size(28.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
            });
            ui.add_space(20.0);

            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                let theme_icon = if self.theme.is_dark() { "☀" } else { "🌙" };
                let btn = egui::Button::new(RichText::new(theme_icon).size(24.0))
                .fill(pal.card)
                .rounding(Rounding::same(8.0))
                .min_size(Vec2::splat(44.0));
                if ui.add(btn).clicked() {
                    self.theme = match self.theme {
                        Theme::Dark => Theme::Light,
                        Theme::Light => Theme::Dark,
                        Theme::MinimalLight => Theme::MinimalDark,
                        Theme::MinimalDark => Theme::MinimalLight,
                    };
                }

                ui.add_space(16.0);

                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                    let p = ui.painter();
                    let c = rect.center();
                    let time = ui.input(|i| i.time);

                    let (col, status_text) = if !live.connected {
                        (Color32::from_rgb(200, 50, 50), "Offline")
                    } else if !live.controller {
                        (Color32::from_rgb(200, 150, 50), "Searching")
                    } else {
                        (Color32::from_rgb(50, 200, 50), "Connected")
                    };

                    if live.connected && live.controller {
                        for i in 0..3 {
                            let phase = ((time * 0.3 + i as f64 * 0.33) % 1.0) as f32;
                            let radius = 3.0 + phase * 7.0;
                            let alpha = 1.0 - phase;
                            p.circle_stroke(c, radius, Stroke::new(1.5_f32, col.gamma_multiply(alpha)));
                        }
                        p.circle_filled(c, 3.0, col);
                    } else if live.connected && !live.controller {
                        paint_beeping_dot(p, c, time, col, 2.0);
                    } else {
                        p.circle_filled(c, 3.0, col);
                    }

                    ui.add_space(6.0);
                    ui.label(RichText::new(status_text).color(pal.sub).size(13.0));
                });

                ui.add_space(16.0);

                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        for (tab, label) in TABS {
                            if nav_button(ui, pal, label, self.tab == tab).clicked() {
                                self.tab = tab;
                                self.ap_draft = None;
                            }
                            ui.add_space(4.0);
                        }
                    });
                });
            });
        });
    }

    fn overview_device_card(&self, ui: &mut egui::Ui, pal: &Pal, live: &Live) {
        card(ui, pal, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Device:").color(pal.sub));
                ui.add_space(4.0);

                ui.scope(|ui| {
                    ui.spacing_mut().interact_size.y = 36.0;
                    let r = Rounding::same(8.0);
                    let v = ui.visuals_mut();
                    v.widgets.inactive.rounding = r;
                    v.widgets.hovered.rounding = r;
                    v.widgets.active.rounding = r;
                    v.widgets.open.rounding = r;
                    v.window_rounding = r;

                    egui::ComboBox::from_id_source("device_sel")
                    .width(140.0)
                    .selected_text(RichText::new("Controller 1").strong())
                    .show_ui(ui, |_ui| {});
                });
            });
            ui.add_space(10.0);
            paint_controller(ui, self.lb_color, self.lb_brightness, live.controller && self.rumbling());
        });
    }

    fn overview_info_cards(ui: &mut egui::Ui, pal: &Pal, live: &Live) {
        card(ui, pal, |ui| {
            ui.label(RichText::new("Controller Info").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
            ui.add_space(8.0);
            let conn = if live.bluetooth { "Bluetooth" } else { "USB" };
            ui.label(RichText::new(format!("Connection: {}", if live.controller { conn } else { "Disconnected" })).size(14.0));
            let bat_text = match live.battery {
                Some(b) => format!("{}% {}", b, live.battery_status.as_deref().unwrap_or("")),
             None => "Unknown".to_string(),
            };
            ui.label(RichText::new(format!("Battery: {}", bat_text)).size(14.0));
            ui.label(RichText::new(format!("Active Profile: {}", live.active_profile)).size(14.0));
        });
        ui.add_space(10.0);
        card(ui, pal, |ui| {
            ui.label(RichText::new("Game Status").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
            ui.add_space(8.0);
            let (status_text, status_col) = if let Some(name) = &live.running_game_name {
                let text = if let Some(id) = live.running_game_steam_id {
                    format!("Running: {} ({})", name, id)
                } else {
                    format!("Running: {} (Wine)", name)
                };
                (text, Color32::from_rgb(0, 200, 100))
            } else {
                ("No game running".to_string(), Color32::from_rgb(232, 72, 120))
            };

            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                let p = ui.painter();
                let time = ui.input(|i| i.time);
                paint_beeping_dot(p, rect.center(), time, status_col, 1.5);
                ui.add_space(4.0);
                ui.label(RichText::new(status_text).color(status_col).strong());
            });

            ui.add_space(4.0);
            ui.label(RichText::new(format!("Steam Mode Active: {}", if live.steam_mode_active { "Yes" } else { "No" })).size(14.0));
        });
    }

    fn tab_overview(&mut self, ui: &mut egui::Ui, pal: &Pal, live: &Live) {
        ui.label(RichText::new("Overview").size(30.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if ui.available_width() >= 720.0 {
                ui.columns(2, |cols| {
                    self.overview_device_card(&mut cols[0], pal, live);
                    Self::overview_info_cards(&mut cols[1], pal, live);
                });
            } else {
                self.overview_device_card(ui, pal, live);
                ui.add_space(10.0);
                Self::overview_info_cards(ui, pal, live);
            }
        });
    }

    fn tab_mapping(&mut self, ui: &mut egui::Ui, pal: &Pal, live: &Live) {
        if self.draft_for != live.active_profile {
            if let Some(p) = live.profiles.iter().find(|p| p.name == live.active_profile) {
                self.draft = p.clone();
                self.draft_for = live.active_profile.clone();
            }
        }

        if ui.available_width() < 700.0 {
            ui.label(RichText::new("Controller Mapping").size(30.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_sized([60.0, 36.0], egui::Label::new(RichText::new("Profile:").color(pal.sub)));

                let mut sel = live.active_profile.clone();

                ui.scope(|ui| {
                    ui.spacing_mut().interact_size.y = 36.0;
                    let r = Rounding::same(8.0);
                    let v = ui.visuals_mut();
                    v.widgets.inactive.rounding = r;
                    v.widgets.hovered.rounding = r;
                    v.widgets.active.rounding = r;
                    v.widgets.open.rounding = r;
                    v.window_rounding = r;

                    egui::ComboBox::from_id_source("profile_sel")
                    .width(120.0)
                    .selected_text(RichText::new(&sel).strong())
                    .show_ui(ui, |ui| {
                        for p in &live.profiles {
                            ui.selectable_value(&mut sel, p.name.clone(), &p.name);
                        }
                    });
                });

                if sel != live.active_profile {
                    self.send(json!({"cmd": "set_profile", "name": sel}));
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if pill(ui, pal, "Apply & Save", true).clicked() {
                        self.send(json!({"cmd": "upsert_profile", "profile": self.draft}));
                    }
                    if pill(ui, pal, "Reset", false).clicked() {
                        self.draft.button_map.clear();
                    }
                });
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Controller Mapping").size(30.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if pill(ui, pal, "Apply & Save", true).clicked() {
                        self.send(json!({"cmd": "upsert_profile", "profile": self.draft}));
                    }
                    if pill(ui, pal, "Reset", false).clicked() {
                        self.draft.button_map.clear();
                    }
                    ui.add_space(10.0);
                    let mut sel = live.active_profile.clone();

                    ui.scope(|ui| {
                        ui.spacing_mut().interact_size.y = 36.0;
                        let r = Rounding::same(8.0);
                        let v = ui.visuals_mut();
                        v.widgets.inactive.rounding = r;
                        v.widgets.hovered.rounding = r;
                        v.widgets.active.rounding = r;
                        v.widgets.open.rounding = r;
                        v.window_rounding = r;

                        egui::ComboBox::from_id_source("profile_sel")
                        .width(120.0)
                        .selected_text(RichText::new(&sel).strong())
                        .show_ui(ui, |ui| {
                            for p in &live.profiles {
                                ui.selectable_value(&mut sel, p.name.clone(), &p.name);
                            }
                        });
                    });

                    if sel != live.active_profile {
                        self.send(json!({"cmd": "set_profile", "name": sel}));
                    }

                    ui.add_space(8.0);
                    ui.add_sized([60.0, 36.0], egui::Label::new(RichText::new("Profile:").color(pal.sub)));
                });
            });
        }

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);

        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            card(ui, pal, |ui| {
                ui.label(RichText::new("Live input").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                ui.label(RichText::new("Press any button on your controller to test.").color(pal.sub));
                ui.add_space(10.0);
                paint_controller(ui, self.lb_color, self.lb_brightness, self.rumbling());
            });

            let ui = &mut cols[1];
            card(ui, pal, |ui| {
                ui.label(RichText::new("Assignments").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                ui.add_space(8.0);
                egui::ScrollArea::vertical().max_height(ui.available_height() - 20.0).show(ui, |ui| {
                    ui.label(RichText::new("Buttons").color(pal.sub).size(13.0));
                    ui.add_space(4.0);
                    for (label, name, code) in BUTTONS {
                        let hat = |axis: u16| live.axes.get(&axis).copied().unwrap_or(0);
                        let lit = match name {
                            "ABS_HAT0Y_UP" => hat(ABS_HAT0Y) < 0,
                                                                                           "ABS_HAT0Y_DOWN" => hat(ABS_HAT0Y) > 0,
                                                                                           "ABS_HAT0X_LEFT" => hat(ABS_HAT0X) < 0,
                                                                                           "ABS_HAT0X_RIGHT" => hat(ABS_HAT0X) > 0,
                                                                                           _ => live.buttons.contains(&code),
                        };
                        let (fill, stroke) = if lit {
                            (pal.accent.gamma_multiply(0.15), Stroke::new(1.0_f32, pal.accent))
                        } else {
                            (Color32::TRANSPARENT, Stroke::NONE)
                        };

                        Frame::none()
                        .fill(fill)
                        .stroke(stroke)
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(10.0, 8.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.allocate_ui_with_layout(vec2(120.0, 24.0), Layout::left_to_right(Align::Center), |ui| {
                                    if let Some(img) = get_icon(name, self.theme.is_dark()) {
                                        ui.add(img);
                                    }
                                    ui.add_space(6.0);
                                    ui.label(RichText::new(label).strong().color(if lit { pal.accent } else { pal.text }));
                                });
                                let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                                draw_arrow(ui.painter(), rect, pal.sub);
                                map_combo(ui, &mut self.draft, name, self.theme.is_dark(), 130.0);
                            });
                        });
                        ui.add_space(2.0);
                    }

                    ui.add_space(16.0);
                    ui.separator();
                    ui.add_space(16.0);

                    ui.label(RichText::new("Axes (Sticks & Triggers)").color(pal.sub).size(13.0));
                    ui.label(RichText::new("Triggers can act as a button. Sticks always pass through unchanged.").color(pal.sub).size(11.0));
                    ui.add_space(4.0);
                    for (label, name, code) in AXES {
                        let is_trigger = code == ABS_Z || code == ABS_RZ;
                        let val = live.axes.get(&code).copied().unwrap_or(0);
                        let is_active = if is_trigger { val > 2 } else { (val - 128).abs() > 20 };
                        let (fill, stroke) = if is_active {
                            (pal.accent.gamma_multiply(0.15), Stroke::new(1.0_f32, pal.accent))
                        } else {
                            (Color32::TRANSPARENT, Stroke::NONE)
                        };

                        Frame::none()
                        .fill(fill)
                        .stroke(stroke)
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(10.0, 8.0))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.allocate_ui_with_layout(vec2(140.0, 24.0), Layout::left_to_right(Align::Center), |ui| {
                                    if let Some(img) = get_icon(name, self.theme.is_dark()) {
                                        ui.add(img);
                                    }
                                    ui.add_space(6.0);
                                    ui.label(RichText::new(label).strong().color(if is_active { pal.accent } else { pal.text }));
                                });
                                let pct = (val as f32 / 255.0).clamp(0.0, 1.0);
                                let (rect, _) = ui.allocate_exact_size(vec2(60.0, 10.0), Sense::hover());
                                let p = ui.painter();
                                p.rect_filled(rect, Rounding::same(5.0), pal.panel);
                                let mut fill_rect = rect;
                                fill_rect.set_width(rect.width() * pct);
                                if is_active {
                                    for i in 1..=5 {
                                        let f = i as f32;
                                        p.rect_filled(rect.expand(f * 2.0), Rounding::same(5.0 + f), pal.accent.gamma_multiply(0.3 / f));
                                    }
                                }
                                p.rect_filled(fill_rect, Rounding::same(5.0), if is_active { pal.accent } else { pal.line });
                                ui.label(RichText::new(format!("{val}")).color(pal.sub).size(11.0).monospace());
                                if is_trigger {
                                    let (rect, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                                    draw_arrow(ui.painter(), rect, pal.sub);
                                    map_combo(ui, &mut self.draft, name, self.theme.is_dark(), 120.0);
                                } else {
                                    ui.label(RichText::new("pass-through").color(pal.sub).size(12.0));
                                }
                            });
                        });
                        ui.add_space(2.0);
                    }
                });
            });
        });
    }

    fn tab_auto(&mut self, ui: &mut egui::Ui, pal: &Pal, live: &Live) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Auto-Profiles").size(30.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut auto = live.auto_profile;
                if ui.checkbox(&mut auto, "Switch automatically").changed() {
                    self.send(json!({"cmd": "set_auto_profile", "enabled": auto}));
                }
            });
        });
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);

        let drafts = self.ap_draft.get_or_insert_with(|| live.profiles.clone()).clone();
        let mut drafts = drafts;
        let mut to_send: Vec<Value> = Vec::new();
        let mut deleted: Option<String> = None;

        let tile_height = 110.0;

        card(ui, pal, |ui| {
            ui.set_min_height(tile_height);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Active Window:").color(pal.sub));
                ui.label(RichText::new(live.active_app.as_deref().unwrap_or("None detected")).strong());
            });
            ui.label(RichText::new("A profile activates when its conditions match.").color(pal.sub).size(13.0));
        });
        ui.add_space(10.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for prof in drafts.iter_mut() {
                card(ui, pal, |ui| {
                    ui.set_min_height(tile_height);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&prof.name).size(16.0).strong());
                        if prof.name == live.active_profile {
                            ui.label(RichText::new("IN USE").color(pal.accent).size(12.0).strong());
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if prof.name != "Default" && pill(ui, pal, "Delete", false).clicked() {
                                deleted = Some(prof.name.clone());
                            }
                            if pill(ui, pal, "Save", true).clicked() {
                                to_send.push(json!({"cmd": "upsert_profile", "profile": prof}));
                            }
                        });
                    });
                    ui.add_space(8.0);
                    let mut remove = None;
                    for (i, rule) in prof.match_rules.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            let mut c = rule.window_class.clone().unwrap_or_default();
                            let mut pn = rule.process_name.clone().unwrap_or_default();
                            ui.label("Window:");
                            ui.add(egui::TextEdit::singleline(&mut c).desired_width(140.0).hint_text("e.g. steam_app"));
                            ui.label("Process:");
                            ui.add(egui::TextEdit::singleline(&mut pn).desired_width(140.0).hint_text("e.g. eldenring"));
                            rule.window_class = (!c.is_empty()).then_some(c);
                            rule.process_name = (!pn.is_empty()).then_some(pn);
                            if ui.small_button("X").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        prof.match_rules.remove(i);
                    }
                    if prof.name != "Default" && ui.small_button("+ Add condition").clicked() {
                        prof.match_rules.push(MatchRule::default());
                    }
                    if prof.name == "Default" {
                        ui.label(RichText::new("Default profile, used when no other rules match.").color(pal.sub).size(12.0));
                    }
                });
                ui.add_space(8.0);
            }
            card(ui, pal, |ui| {
                ui.set_min_height(tile_height);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Create new profile:").strong());

                    ui.scope(|ui| {
                        ui.spacing_mut().interact_size.y = 36.0;
                        let r = Rounding::same(8.0);
                        let v = ui.visuals_mut();
                        v.widgets.inactive.rounding = r;
                        v.widgets.hovered.rounding = r;
                        v.widgets.active.rounding = r;
                        v.widgets.open.rounding = r;

                        ui.add(egui::TextEdit::singleline(&mut self.new_profile)
                        .hint_text("Profile Name")
                        .desired_width(200.0)
                        .margin(Margin::symmetric(12.0, 8.0)));
                    });

                    if pill(ui, pal, "Create", true).clicked() && !self.new_profile.trim().is_empty() && !drafts.iter().any(|d| d.name == self.new_profile.trim()) {
                        let p = Profile { name: self.new_profile.trim().to_string(), ..Default::default() };
                        to_send.push(json!({"cmd": "upsert_profile", "profile": p}));
                        drafts.push(p);
                        self.new_profile.clear();
                    }
                });
            });
        });
        if let Some(name) = deleted {
            to_send.push(json!({"cmd": "delete_profile", "name": name}));
            drafts.retain(|p| p.name != name);
        }
        self.ap_draft = Some(drafts);
        for v in to_send {
            self.send(v);
        }
    }

    fn tab_touchpad(&mut self, ui: &mut egui::Ui, pal: &Pal, live: &Live) {
        ui.label(RichText::new("Touchpad and Gyro").size(30.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);

        let Some(mut prof) = live.profiles.iter().find(|p| p.name == live.active_profile).cloned() else {
            ui.label(RichText::new("Waiting for the daemon...").color(pal.sub));
            return;
        };
        let mut changed = false;

        card(ui, pal, |ui| {
            ui.label(RichText::new("Touchpad to Mouse").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
            ui.add_space(8.0);
            changed |= ui.checkbox(&mut prof.tp_mouse_enabled, "Enable Touchpad Mouse (click the pad to left-click)").changed();
            ui.add_enabled_ui(prof.tp_mouse_enabled, |ui| {
                changed |= ui.add(egui::Slider::new(&mut prof.tp_sensitivity, 0.1..=5.0).text("Sensitivity")).changed();
            });
        });
        ui.add_space(10.0);
        card(ui, pal, |ui| {
            ui.label(RichText::new("Gyro Aiming").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
            ui.add_space(8.0);
            changed |= ui.checkbox(&mut prof.gyro_mouse_enabled, "Enable Gyro Mouse").changed();
            ui.add_enabled_ui(prof.gyro_mouse_enabled, |ui| {
                changed |= ui.add(egui::Slider::new(&mut prof.gyro_sensitivity, 0.1..=5.0).text("Sensitivity")).changed();
                changed |= ui.checkbox(&mut prof.gyro_invert_x, "Invert horizontal").changed();
                changed |= ui.checkbox(&mut prof.gyro_invert_y, "Invert vertical").changed();
            });
        });
        if changed {
            self.send(json!({"cmd": "upsert_profile", "profile": prof}));
        }
    }

    fn tab_lightbar(&mut self, ui: &mut egui::Ui, pal: &Pal, live: &Live) {
        ui.label(RichText::new("Lightbar").size(30.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let show_warn = self.lightbar_warning_until.map_or(false, |t| Instant::now() < t);
            if self.lightbar_warning_until.is_some() && !show_warn {
                self.lightbar_warning_until = None;
            }
            animated_red_banner(ui, "lightbar_warn_banner", show_warn, "No lightbar LED found. Is the controller connected?");

            ui.columns(2, |cols| {
                let ui = &mut cols[0];
                card(ui, pal, |ui| {
                    ui.label(RichText::new("Controller Preview").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                    ui.add_space(10.0);
                    paint_controller(ui, self.lb_color, self.lb_brightness, self.rumbling());
                });

                let ui = &mut cols[1];
                card(ui, pal, |ui| {
                    ui.label(RichText::new("Color & Brightness").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("Color:");
                        let mut rgb = [self.lb_color.r(), self.lb_color.g(), self.lb_color.b()];
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            self.lb_color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                            self.lb_hex = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
                        }
                        ui.add_space(10.0);
                        ui.label("Hex:");
                        if ui.add(egui::TextEdit::singleline(&mut self.lb_hex).desired_width(80.0)).lost_focus() {
                            let hex = self.lb_hex.trim_start_matches('#');
                            if hex.len() == 6 {
                                if let Ok(r) = u8::from_str_radix(&hex[0..2], 16) {
                                    if let Ok(g) = u8::from_str_radix(&hex[2..4], 16) {
                                        if let Ok(b) = u8::from_str_radix(&hex[4..6], 16) {
                                            self.lb_color = Color32::from_rgb(r, g, b);
                                        }
                                    }
                                }
                            }
                        }
                    });
                    ui.add_space(8.0);
                    ui.label("Brightness:");
                    ui.add(egui::Slider::new(&mut self.lb_brightness, 0.0..=100.0).suffix("%"));
                    ui.add_space(8.0);
                    ui.checkbox(&mut self.lb_sync_battery, "Sync with Battery Level (Green/Yellow/Red)");
                });
                ui.add_space(20.0);
                if pill(ui, pal, "Apply to Controller", true).clicked() {
                    if !live.controller {
                        self.lightbar_warning_until = Some(Instant::now() + Duration::from_secs(3));
                    } else {
                        self.send(json!({
                            "cmd": "set_lightbar",
                            "color": [self.lb_color.r(), self.lb_color.g(), self.lb_color.b()],
                                        "brightness": self.lb_brightness,
                                        "sync_battery": self.lb_sync_battery,
                        }));
                    }
                }
            });
        });
    }

    fn tab_rumble(&mut self, ui: &mut egui::Ui, pal: &Pal, live: &Live) {
        ui.label(RichText::new("Rumble").size(30.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            let show_warn = self.rumble_warning_until.map_or(false, |t| Instant::now() < t);
            if self.rumble_warning_until.is_some() && !show_warn {
                self.rumble_warning_until = None;
            }
            animated_red_banner(ui, "rumble_warn_banner", show_warn, "No rumble support found. Is the controller connected?");

            let is_visually_rumbling = live.controller && self.rumbling();

            ui.columns(2, |cols| {
                let ui = &mut cols[0];
                card(ui, pal, |ui| {
                    ui.label(RichText::new("Live input").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                    ui.label(RichText::new("The controller shakes here while the test runs.").color(pal.sub));
                    ui.add_space(10.0);
                    paint_controller(ui, self.lb_color, self.lb_brightness, is_visually_rumbling);
                });

                let ui = &mut cols[1];
                card(ui, pal, |ui| {
                    ui.label(RichText::new("Motor Strength").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                    ui.add_space(12.0);

                    let mut commit = false;
                    for (value, text) in [(&mut self.rumble_large, "Large Motor (Low Freq)"), (&mut self.rumble_small, "Small Motor (High Freq)")] {
                        let r = ui.add(egui::Slider::new(value, 0.0..=255.0).text(text).step_by(1.0));
                        commit |= r.drag_stopped() || (r.changed() && !r.dragged());
                    }
                    ui.add_space(16.0);
                    if ui.checkbox(&mut self.rumble_pass, "Forward rumble from games to the controller").changed() {
                        commit = true;
                    }
                    if commit {
                        self.send_rumble_strength();
                    }

                    ui.add_space(20.0);
                    let test_btn = egui::Button::new(RichText::new("Test Rumble").color(Color32::BLACK).size(18.0).strong())
                    .fill(pal.accent)
                    .rounding(Rounding::same(8.0))
                    .min_size(vec2(ui.available_width(), 50.0));
                    if ui.add(test_btn).clicked() {
                        if !live.controller {
                            self.rumble_warning_until = Some(Instant::now() + Duration::from_secs(3));
                        } else {
                            self.send_rumble_strength();
                            self.send(json!({"cmd": "test_rumble"}));
                            self.rumble_until = Some(Instant::now() + Duration::from_millis(700));
                        }
                    }
                });
            });

            ui.add_space(20.0);
            card(ui, pal, |ui| {
                ui.label(RichText::new("How it works").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                ui.label(RichText::new("Games vibrate the virtual controller StynPS creates. StynPS passes that on to your real controller, scaled by the strengths above.").color(pal.sub).size(13.0));
                if live.steam_mode_active {
                    ui.add_space(4.0);
                    ui.label(RichText::new("Steam Input has the controller right now, so game rumble goes through Steam. The test still works.").color(pal.sub).size(13.0));
                }
            });
        });
    }

    fn tab_steam(&mut self, ui: &mut egui::Ui, pal: &Pal, live: &Live) {
        ui.label(RichText::new("Steam Input").size(30.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut enabled = live.settings.steam_input_enabled;
            let mut auto = live.settings.auto_steam_handover;
            let mut changed = false;

            card(ui, pal, |ui| {
                ui.label(RichText::new("Who controls the controller?").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if pill(ui, pal, "StynPS controls it", !enabled).clicked() {
                        enabled = false;
                        changed = true;
                    }
                    if pill(ui, pal, "Steam Input controls it", enabled).clicked() {
                        enabled = true;
                        changed = true;
                    }
                });
                ui.add_space(6.0);
                let desc = if enabled {
                    "StynPS is paused and lets go of the controller, so Steam Input has it alone."
                } else {
                    "StynPS takes over the controller and shows games one virtual controller."
                };
                ui.label(RichText::new(desc).color(pal.sub).size(13.0));
                ui.add_space(8.0);
                if ui.checkbox(&mut auto, "Hand the controller to Steam Input automatically while a Steam game is running").changed() {
                    changed = true;
                }
                ui.add_space(8.0);

                let (who, col) = if !live.controller {
                    ("No controller connected", pal.sub)
                } else if live.steam_mode_active {
                    ("Steam Input has the controller (StynPS is paused)", pal.accent)
                } else {
                    ("StynPS has the controller", pal.accent)
                };
                ui.label(RichText::new(format!("* {who}")).color(col).strong());

                let (status_text, status_col) = if let Some(name) = &live.running_game_name {
                    if let Some(id) = live.running_game_steam_id {
                        (format!("Running: {} (Steam App {})", name, id), Color32::from_rgb(0, 200, 100))
                    } else {
                        (format!("Running: {} (Non-Steam)", name), Color32::from_rgb(200, 150, 50))
                    }
                } else {
                    ("No game running".to_string(), Color32::from_rgb(232, 72, 120))
                };

                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                    let p = ui.painter();
                    let time = ui.input(|i| i.time);
                    paint_beeping_dot(p, rect.center(), time, status_col, 1.5);
                    ui.add_space(4.0);
                    ui.label(RichText::new(status_text).color(status_col).strong().size(13.0));
                });
            });
            if changed {
                self.send(json!({"cmd": "set_steam_input", "enabled": enabled, "auto_handover": auto}));
            }

            ui.add_space(10.0);
            card(ui, pal, |ui| {
                ui.label(RichText::new("Fix doubled inputs in one game").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                ui.add_space(6.0);
                ui.label(RichText::new("If a game reacts twice to every press, it can see both the real controller and the virtual one.").color(pal.sub).size(13.0));
                ui.add_space(8.0);
                let ids: Vec<(u16, u16)> = match live.controller_ids {
                    Some([v, p]) => vec![(v, p)],
                 None => DS4_IDS.to_vec(),
                };
                let opt = format!("SDL_GAMECONTROLLER_IGNORE_DEVICES=\"{}\" %command%", system::ignore_devices_value(&ids));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&opt).monospace().color(pal.accent));
                });
                ui.add_space(6.0);
                if pill(ui, pal, "Copy", false).clicked() {
                    ui.ctx().copy_text(opt);
                }
            });

            ui.add_space(10.0);
            card(ui, pal, |ui| {
                ui.label(RichText::new("Steam checklist").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                ui.add_space(6.0);
                for line in [
                    "1. Steam, Settings, Controller: turn off Steam Input for PlayStation controllers.",
                    "2. For a game you want StynPS to handle: Properties, Controller, set it to disable Steam Input.",
                    "3. Still doubled? Use the launch option above for that game.",
                    "4. The launch option only affects SDL and Wine/Proton games.",
                ] {
                    ui.label(RichText::new(line).color(pal.sub).size(13.0));
                    ui.add_space(2.0);
                }
            });
        });
    }

    fn tab_settings(&mut self, ui: &mut egui::Ui, pal: &Pal, live: &Live) {
        ui.label(RichText::new("Settings").size(30.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            card(ui, pal, |ui| {
                ui.label(RichText::new("Permissions").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                ui.add_space(6.0);
                ui.label(RichText::new("StynPS needs access to /dev/uinput and the lightbar. Install the udev rule once.").color(pal.sub).size(13.0));
                ui.add_space(8.0);
                let busy = lock(&self.perm_msg).as_deref() == Some("Working...");
                let clicked = ui.add_enabled_ui(!busy, |ui| pill(ui, pal, "Install permissions...", true).clicked()).inner;
                if clicked {
                    let slot = self.perm_msg.clone();
                    let ctx = ui.ctx().clone();
                    *lock(&slot) = Some("Working...".to_string());
                    thread::spawn(move || {
                        let msg = match system::install_permissions() {
                            Ok(m) => m,
                                  Err(e) => format!("Failed: {e}"),
                        };
                        *lock(&slot) = Some(msg);
                        ctx.request_repaint();
                    });
                }
                if let Some(m) = lock(&self.perm_msg).clone() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(m).color(pal.sub).size(13.0));
                }
            });

            ui.add_space(10.0);

            card(ui, pal, |ui| {
                ui.label(RichText::new("Appearance").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                ui.horizontal(|ui| {
                    ui.label("Theme:");
                    if pill(ui, pal, "Dark", self.theme == Theme::Dark).clicked() { self.theme = Theme::Dark; }
                    if pill(ui, pal, "Light", self.theme == Theme::Light).clicked() { self.theme = Theme::Light; }
                    if pill(ui, pal, "Minimal-Light", self.theme == Theme::MinimalLight).clicked() { self.theme = Theme::MinimalLight; }
                    if pill(ui, pal, "Minimal-Dark", self.theme == Theme::MinimalDark).clicked() { self.theme = Theme::MinimalDark; }
                });
                ui.add_space(8.0);

                let is_minimal = matches!(self.theme, Theme::MinimalLight | Theme::MinimalDark);

                ui.horizontal(|ui| {
                    ui.label("Accent Color:");
                    ui.add_enabled_ui(!is_minimal, |ui| {
                        for (name, col) in [
                            ("Blue", Color32::from_rgb(0, 112, 209)),
                                      ("Violet", Color32::from_rgb(124, 92, 255)),
                                      ("Teal", Color32::from_rgb(0, 168, 150)),
                                      ("Rose", Color32::from_rgb(232, 72, 120)),
                        ] {
                            if pill(ui, pal, name, self.accent == col).clicked() {
                                self.accent = col;
                            }
                        }
                    });
                });

                if is_minimal {
                    ui.add_space(4.0);
                    ui.label(RichText::new("To use accent colours switch theme to Dark or Light").color(pal.sub).size(12.0));
                }
            });
            ui.add_space(10.0);
            card(ui, pal, |ui| {
                ui.label(RichText::new("Daemon Status").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
                ui.label(format!("Socket: {}", daemon::socket_path().display()));
                ui.label(if live.connected { "Status: Connected" } else { "Status: Offline" });
                let how = if live.bluetooth { "Bluetooth" } else { "USB" };
                ui.label(if live.controller { format!("Controller: Detected ({how})") } else { "Controller: Not Found".to_string() });
            });
        });
    }

    fn tab_about(&mut self, ui: &mut egui::Ui, pal: &Pal, _live: &Live) {
        ui.label(RichText::new("About StynPS").size(30.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.text));
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(16.0);

        card(ui, pal, |ui| {
            ui.label(RichText::new("StynPS PlaySelect").size(22.0).family(FontFamily::Name("LuxuryTitle".into())).color(pal.accent));
            ui.label(RichText::new(format!("Version: {}", env!("CARGO_PKG_VERSION"))).size(14.0));
            ui.add_space(8.0);
            ui.label(RichText::new("A modern, high-performance Linux controller remapper for the DualShock 4 and DualSense (PS5) controllers. Built entirely in Rust with a native egui interface.").size(14.0));
            ui.add_space(16.0);

            ui.horizontal(|ui| {
                if pill(ui, pal, "Support (GitHub Issues)", true).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(SUPPORT_URL));
                }
                if pill(ui, pal, "Telegram Channel", true).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(TELEGRAM_URL));
                }
            });
        });

        ui.add_space(10.0);

        card(ui, pal, |ui| {
            ui.label(RichText::new("Contributors").size(18.0).family(FontFamily::Name("LuxuryTitle".into())));
            ui.add_space(8.0);
            ui.label(RichText::new("This project is heavily vibecoded (AI-assisted).").color(pal.sub).size(13.0));
            ui.label(RichText::new("All contributors are credited on GitHub.").color(pal.sub).size(13.0));
        });
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "theme", &self.theme);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        let t0 = *self.boot_t0.get_or_insert(now);
        let boot_t = if self.skip_boot { BOOT_SECS + BOOT_FADE } else { now - t0 };
        let pal = palette(self.theme, self.accent);
        apply_style(ctx, &pal, self.theme);

        if boot_t < BOOT_SECS {
            self.boot_screen(ctx, &pal, boot_t);
            ctx.request_repaint();
            return;
        }

        let live = lock(&self.live).clone();
        self.sync_from_daemon(&live);

        if live.controller && !self.prev_controller {
            self.prev_controller = true;
            self.connected_rumble_seq = Some(0);
            self.connected_rumble_timer = Instant::now();
            self.send(json!({"cmd": "test_rumble"}));
            self.rumble_until = Some(Instant::now() + Duration::from_millis(200));
        } else if !live.controller && self.prev_controller {
            self.prev_controller = false;
        }

        if let Some(seq) = self.connected_rumble_seq {
            if self.connected_rumble_timer.elapsed() > Duration::from_millis(250) {
                match seq {
                    0 => {
                        self.send(json!({"cmd": "stop_rumble"}));
                        self.connected_rumble_seq = Some(1);
                        self.connected_rumble_timer = Instant::now();
                    }
                    1 => {
                        self.send(json!({"cmd": "test_rumble"}));
                        self.connected_rumble_seq = Some(2);
                        self.connected_rumble_timer = Instant::now();
                        self.rumble_until = Some(Instant::now() + Duration::from_millis(200));
                    }
                    2 => {
                        self.send(json!({"cmd": "stop_rumble"}));
                        self.connected_rumble_seq = Some(3);
                    }
                    _ => {}
                }
            }
        }

        self.sidebar(ctx, &pal, &live);

        egui::CentralPanel::default()
        .frame(Frame::none().fill(pal.bg).inner_margin(Margin::same(24.0)))
        .show(ctx, |ui| {
            if let Some(e) = &live.last_error {
                banner(ui, &pal, e, false);
            }
            if let Some(e) = &live.cmd_error {
                if banner(ui, &pal, &format!("The daemon refused a command: {e}"), true) {
                    lock(&self.live).cmd_error = None;
                }
            }
            match self.tab {
                Tab::Overview => self.tab_overview(ui, &pal, &live),
              Tab::Mapping => self.tab_mapping(ui, &pal, &live),
              Tab::AutoProfiles => self.tab_auto(ui, &pal, &live),
              Tab::Lightbar => self.tab_lightbar(ui, &pal, &live),
              Tab::Rumble => self.tab_rumble(ui, &pal, &live),
              Tab::Touchpad => self.tab_touchpad(ui, &pal, &live),
              Tab::Steam => self.tab_steam(ui, &pal, &live),
              Tab::Settings => self.tab_settings(ui, &pal, &live),
              Tab::About => self.tab_about(ui, &pal, &live),
            }
        });

        let fade = 1.0 - ((boot_t - BOOT_SECS) / BOOT_FADE).clamp(0.0, 1.0);
        if fade > 0.0 {
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("boot_fade")))
            .rect_filled(ctx.screen_rect(), Rounding::ZERO, pal.bg.gamma_multiply(fade as f32));
            ctx.request_repaint();
        } else if matches!(self.tab, Tab::Overview | Tab::Mapping | Tab::Lightbar | Tab::Rumble | Tab::Touchpad)
            || self.rumbling()
            || live.connected
            || self.connected_rumble_seq.is_some()
            {
                ctx.request_repaint_after(Duration::from_millis(33));
            }
    }
}
