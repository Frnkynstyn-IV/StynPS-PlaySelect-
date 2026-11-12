//! Talking to the rest of the system: lightbar LEDs, battery, Steam, active window, permissions.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

const SONY_HEX: &str = "054C";

fn read_trim(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn is_sony(path: &Path) -> bool {
    fs::canonicalize(path).map(|p| p.to_string_lossy().to_uppercase().contains(SONY_HEX)).unwrap_or(false)
}

pub fn set_lightbar(rgb: [u8; 3], brightness: f32) -> Result<(), String> {
    let k = (brightness / 100.0).clamp(0.0, 1.0);
    let [r, g, b] = rgb.map(|c| (c as f32 * k).round() as u8);
    let entries = fs::read_dir("/sys/class/leds").map_err(|e| format!("cannot read /sys/class/leds: {e}"))?;
    let mut wrote = false;
    let mut denied = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_sony(&path) { continue; }
        let result: std::io::Result<()> = (|| {
            if name.contains("rgb") && path.join("multi_intensity").exists() {
                let max = read_trim(path.join("max_brightness")).unwrap_or_else(|| "255".into());
                fs::write(path.join("brightness"), max)?;
                fs::write(path.join("multi_intensity"), format!("{r} {g} {b}"))?;
                Ok(())
            } else if let Some(base) = name.strip_suffix(":red") {
                let dir = entry.path().parent().map(Path::to_path_buf).unwrap_or_default();
                for (color, val) in [("red", r), ("green", g), ("blue", b)] {
                    fs::write(dir.join(format!("{base}:{color}")).join("brightness"), val.to_string())?;
                }
                let global = dir.join(format!("{base}:global"));
                if global.exists() {
                    let max = read_trim(global.join("max_brightness")).unwrap_or_else(|| "255".into());
                    fs::write(global.join("brightness"), max)?;
                }
                Ok(())
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "not a lightbar LED"))
            }
        })();
        match result {
            Ok(()) => wrote = true,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => denied = true,
            Err(_) => {}
        }
    }

    if wrote { Ok(()) } else if denied {
        Err("Permission denied writing the lightbar. Use Settings > Permissions to install the udev rule.".into())
    } else {
        Err("No lightbar LED found. Is the controller connected?".into())
    }
}

pub fn read_battery() -> Option<(u8, String)> {
    for entry in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("ps-controller-battery") || name.starts_with("sony_controller_battery") {
            let pct = read_trim(entry.path().join("capacity"))?.parse::<u8>().ok()?;
            let status = read_trim(entry.path().join("status")).unwrap_or_default();
            return Some((pct.min(100), status));
        }
    }
    None
}

pub fn battery_color(pct: u8) -> [u8; 3] {
    match pct {
        51..=100 => [0, 255, 0],
        21..=50 => [255, 255, 0],
        _ => [255, 0, 0],
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunningGame {
    pub name: String,
    pub steam_id: Option<u32>,
}

fn find_steam_game_name(app_id: u32) -> Option<String> {
    let home = dirs::home_dir()?;
    let mut paths = vec![
        home.join(".steam/steam/steamapps"),
        home.join(".local/share/Steam/steamapps"),
        home.join(".steam/root/steamapps"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps"),
    ];

    for base_path in &paths.clone() {
        let vdf_path = base_path.join("libraryfolders.vdf");
        if let Ok(contents) = std::fs::read_to_string(vdf_path) {
            for line in contents.lines() {
                let line = line.trim();
                if line.starts_with("\"path\"") {
                    let parts: Vec<&str> = line.split('"').collect();
                    if parts.len() >= 4 {
                        let custom_path = std::path::PathBuf::from(parts[3]).join("steamapps");
                        if !paths.contains(&custom_path) {
                            paths.push(custom_path);
                        }
                    }
                }
            }
        }
    }

    let filename = format!("appmanifest_{}.acf", app_id);
    for path in paths {
        let file_path = path.join(&filename);
        if let Ok(contents) = std::fs::read_to_string(file_path) {
            for line in contents.lines() {
                let line = line.trim();
                if line.starts_with("\"name\"") {
                    let parts: Vec<&str> = line.split('"').collect();
                    if parts.len() >= 4 {
                        return Some(parts[3].to_string());
                    }
                }
            }
        }
    }
    None
}

fn get_running_steam_app() -> Option<RunningGame> {
    let me = std::process::id();
    let Ok(dir) = fs::read_dir("/proc") else { return None };
    let mut buf = vec![0u8; 32 * 1024];

    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if pid == me { continue; }

        let comm = read_trim(format!("/proc/{pid}/comm")).unwrap_or_default().to_lowercase();
        if !comm.contains("wine") && !comm.contains("proton") && !comm.ends_with(".exe") {
            continue;
        }

        let Ok(mut f) = fs::File::open(format!("/proc/{pid}/environ")) else { continue };
        let n = f.read(&mut buf).unwrap_or(0);
        for item in buf[..n].split(|b| *b == 0) {
            if let Some(id_bytes) = item.strip_prefix(b"SteamAppId=") {
                if !id_bytes.is_empty() && id_bytes != b"0" {
                    if let Ok(id_str) = std::str::from_utf8(id_bytes) {
                        if let Ok(app_id) = id_str.parse::<u32>() {
                            let name = find_steam_game_name(app_id).unwrap_or_else(|| "Unknown Steam Game".to_string());
                            return Some(RunningGame { name, steam_id: Some(app_id) });
                        }
                    }
                }
            }
        }
    }
    None
}

fn get_running_wine_app() -> Option<RunningGame> {
    let me = std::process::id();
    let Ok(dir) = fs::read_dir("/proc") else { return None };
    let ignored = [
        "explorer.exe", "services.exe", "winedevice.exe", "plugplay.exe",
        "rpcss.exe", "svchost.exe", "steam.exe", "steamwebhelper.exe",
        "tabtip.exe", "conhost.exe", "wineboot.exe", "start.exe",
        "cmd.exe", "rundll32.exe", "iexplore.exe", "steamerrorreporter.exe"
    ];

    for entry in dir.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        if pid == me { continue; }

        let comm = read_trim(format!("/proc/{pid}/comm")).unwrap_or_default().to_lowercase();

        if !comm.contains("wine") && !comm.contains("proton") && !comm.ends_with(".exe") {
            continue;
        }

        for ignored_proc in ignored {
            if comm == ignored_proc {
                continue;
            }
        }

        if comm.ends_with(".exe") || comm.contains(".exe") {
            let name = comm.replace(".exe", "");
            let display_name = if name.chars().all(|c| !c.is_whitespace()) && name.len() > 3 {
                let mut c = name.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            } else {
                name
            };
            return Some(RunningGame { name: display_name, steam_id: None });
        }
    }
    None
}

pub fn get_running_game() -> Option<RunningGame> {
    get_running_steam_app().or_else(get_running_wine_app)
}

pub fn ignore_devices_value(ids: &[(u16, u16)]) -> String {
    ids.iter().map(|(v, p)| format!("0x{v:04X}/0x{p:04X}")).collect::<Vec<_>>().join(",")
}

pub fn active_window_class() -> Option<String> {
    use x11rb::{connection::Connection, protocol::xproto::*};
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let atom = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW").ok()?.reply().ok()?.atom;
    let reply = conn.get_property(false, root, atom, AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
    let win = reply.value32()?.next()?;
    if win == 0 { return None; }
    let cls = conn.get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256).ok()?.reply().ok()?;
    let text = String::from_utf8_lossy(&cls.value).to_string();
    let joined = text.split('\0').filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    (!joined.is_empty()).then_some(joined)
}

pub const UDEV_RULES: &str = r#"# StynPS (PlaySelect)
KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"
ACTION=="add", SUBSYSTEM=="leds", KERNELS=="*:054C:*", RUN+="/bin/sh -c 'chmod a+w /sys%p/brightness /sys%p/multi_intensity 2>/dev/null'"
# Disable USB autosuspend for Sony controllers to prevent random disconnects
ACTION=="add", SUBSYSTEM=="usb", ATTR{idVendor}=="054c", TEST=="power/control", ATTR{power/control}="on"
"#;

pub fn install_permissions() -> Result<String, String> {
    let script = format!(
        "set -e\ncat > /etc/udev/rules.d/70-stynps.rules <<'STYNPS_EOF'\n{UDEV_RULES}STYNPS_EOF\n\
echo uinput > /etc/modules-load.d/stynps.conf\nmodprobe uinput\nudevadm control --reload\nudevadm trigger\n"
    );
    let out = Command::new("pkexec").args(["sh", "-c", &script]).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "pkexec was not found. Install polkit, or copy the udev rule by hand.".to_string()
        } else {
            e.to_string()
        }
    })?;
    match out.status.code() {
        Some(0) => Ok("Permissions installed. Reconnect the controller (or log out and in) once.".into()),
        Some(126) | Some(127) => Err("Authorization was cancelled.".into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("stynps")
}
