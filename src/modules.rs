use anyhow::Result;
use serde::Serialize;
use std::path::Path;
use std::process::Command;
use walkdir::WalkDir;

/// JSON output format for waybar
#[derive(Debug, Clone, Serialize)]
pub struct ModuleStatus {
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub class: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tooltip: String,
}

impl ModuleStatus {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            class: String::new(),
            tooltip: String::new(),
        }
    }

    pub fn with_class(mut self, class: impl Into<String>) -> Self {
        self.class = class.into();
        self
    }

    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = tooltip.into();
        self
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"text":"error"}"#.to_string())
    }
}

/// Get status for a specific module
pub fn get_status(module: &str, pinned: bool) -> ModuleStatus {
    let mut status = match module {
        "audio" => get_audio_status(),
        "bluetooth" => get_bluetooth_status(),
        "network" => get_network_status(),
        "cpu" => get_cpu_status(),
        "battery" => get_battery_status(),
        "mail" => get_mail_status(),
        "calendar" => get_calendar_status(),
        "localsend" => get_localsend_status(),
        "vpn" => get_vpn_status(),
        "surfshark" => get_surfshark_status(),
        _ => ModuleStatus::new("?"),
    };

    if pinned {
        status.class = "pinned".to_string();
    }

    status
}

fn get_audio_status() -> ModuleStatus {
    // Get mute status
    let muted = Command::new("pactl")
        .args(["get-sink-mute", "@DEFAULT_SINK@"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("yes"))
        .unwrap_or(false);

    if muted {
        return ModuleStatus::new("\u{f6a9}"); // volume-xmark
    }

    // Get volume using the vol script (handles remapping)
    let vol_path = shellexpand::tilde("~/.local/bin/vol").to_string();
    let volume: u32 = Command::new(&vol_path)
        .arg("get")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .unwrap_or(0)
        })
        .unwrap_or(0);

    let icon = if volume == 0 {
        "\u{f026}" // volume-off
    } else if volume < 50 {
        "\u{f027}" // volume-low
    } else {
        "\u{f028}" // volume-high
    };

    ModuleStatus::new(format!("{} {}%", icon, volume))
}

fn get_bluetooth_status() -> ModuleStatus {
    // Check if bluetooth is powered on
    let powered = Command::new("bluetoothctl")
        .arg("show")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("Powered: yes"))
        .unwrap_or(false);

    let bt_icon = "\u{f293}"; // bluetooth-b

    if !powered {
        return ModuleStatus::new(format!("{} off", bt_icon));
    }

    // Check for connected devices
    let connected = Command::new("bluetoothctl")
        .args(["devices", "Connected"])
        .output()
        .ok();

    if let Some(output) = connected {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(line) = stdout.lines().next() {
            // Line format: "Device XX:XX:XX:XX:XX:XX DeviceName"
            if let Some(name) = line
                .split_whitespace()
                .skip(2)
                .collect::<Vec<_>>()
                .join(" ")
                .into()
            {
                let name: String = name;
                if !name.is_empty() {
                    return ModuleStatus::new(format!("{} {}", bt_icon, name));
                }
            }
        }
    }

    ModuleStatus::new(format!("{} on", bt_icon))
}

fn get_network_status() -> ModuleStatus {
    let wifi_icon = "\u{f1eb}"; // wifi
    let eth_icon = "\u{f796}"; // ethernet

    // Check for wifi connection via iwctl
    let wifi_output = Command::new("iwctl")
        .args(["station", "wlan0", "show"])
        .output()
        .ok();

    if let Some(output) = wifi_output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut connected = false;
        let mut ssid = String::new();
        for line in stdout.lines() {
            if line.contains("State") && line.contains("connected") {
                connected = true;
            }
            if line.contains("Connected network") {
                ssid = line.split_whitespace().last().unwrap_or("").to_string();
            }
        }
        if connected && !ssid.is_empty() {
            return ModuleStatus::new(format!("{} {}", wifi_icon, ssid));
        }
    }

    // Check for ethernet via ip — look for physical ethernet interfaces (en*) with state UP
    let eth_output = Command::new("ip")
        .args(["-o", "link", "show", "up"])
        .output()
        .ok();

    if let Some(output) = eth_output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            // Extract interface name (first field after index)
            let iface = line
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .trim_end_matches(':');
            if iface.starts_with("en") && line.contains("state UP") {
                return ModuleStatus::new(eth_icon.to_string());
            }
        }
    }

    ModuleStatus::new(format!("{} off", wifi_icon))
}

fn get_cpu_status() -> ModuleStatus {
    // Read /proc/stat for CPU usage
    let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();

    if let Some(cpu_line) = stat.lines().next() {
        let parts: Vec<u64> = cpu_line
            .split_whitespace()
            .skip(1) // skip "cpu"
            .filter_map(|s| s.parse().ok())
            .collect();

        if parts.len() >= 4 {
            let user = parts[0];
            let system = parts[2];
            let idle = parts[3];
            let total = user + system + idle;

            if total > 0 {
                let usage = ((user + system) * 100) / total;
                return ModuleStatus::new(format!("\u{f2db} {}%", usage)); // microchip
            }
        }
    }

    ModuleStatus::new("\u{f2db} ?%") // microchip
}

fn get_battery_status() -> ModuleStatus {
    // Find the first battery in /sys/class/power_supply/
    let ps_dir = Path::new("/sys/class/power_supply");
    let battery_path = std::fs::read_dir(ps_dir)
        .ok()
        .and_then(|entries| {
            entries.filter_map(|e| e.ok()).find(|e| {
                let type_path = e.path().join("type");
                std::fs::read_to_string(type_path)
                    .map(|t| t.trim().eq_ignore_ascii_case("battery"))
                    .unwrap_or(false)
            })
        })
        .map(|e| e.path());

    let battery_path = match battery_path {
        Some(p) => p,
        None => return ModuleStatus::new("".to_string()), // no battery — hide module
    };

    let capacity = std::fs::read_to_string(battery_path.join("capacity"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "?".to_string());

    let status = std::fs::read_to_string(battery_path.join("status"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "Unknown".to_string());

    let cap_num: u32 = capacity.parse().unwrap_or(0);
    let bat_icon = match status.as_str() {
        "Charging" => "\u{f0e7}",        // bolt
        "Full" => "\u{f1e6}",            // plug
        _ if cap_num > 75 => "\u{f240}", // battery-full
        _ if cap_num > 50 => "\u{f241}", // battery-three-quarters
        _ if cap_num > 25 => "\u{f242}", // battery-half
        _ if cap_num > 10 => "\u{f243}", // battery-quarter
        _ => "\u{f244}",                 // battery-empty
    };

    let text = match status.as_str() {
        "Full" => bat_icon.to_string(),
        "Charging" => format!("{} {}%", bat_icon, capacity),
        _ => format!("{} {}%", bat_icon, capacity),
    };

    ModuleStatus::new(text)
}

fn get_mail_status() -> ModuleStatus {
    let mail_dir = shellexpand::tilde("~/.local/share/mail").to_string();
    let mail_path = Path::new(&mail_dir);

    let mut unread = 0;

    if mail_path.exists() {
        // Count files in */INBOX/new/
        for entry in WalkDir::new(mail_path).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if let Some(parent) = path.parent() {
                    if parent.ends_with("new") {
                        if let Some(grandparent) = parent.parent() {
                            if grandparent.ends_with("INBOX") {
                                unread += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // Unicode envelope
    let envelope = "\u{f0e0}";

    if unread > 0 {
        ModuleStatus::new(format!("{} {}", envelope, unread))
    } else {
        ModuleStatus::new(envelope.to_string())
    }
}

fn get_calendar_status() -> ModuleStatus {
    // Show current date and time
    let output = Command::new("date")
        .args(["+%a %d %b %H:%M"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "???".to_string());

    ModuleStatus::new(format!("\u{f073} {}", output)) // calendar
}

fn get_localsend_status() -> ModuleStatus {
    ModuleStatus::new("\u{2191}\u{2193}") // ↑↓
}

fn get_vpn_status() -> ModuleStatus {
    let shield_icon = "\u{f3ed}"; // shield-halved
    let up = std::process::Command::new("ip")
        .args(["link", "show", "wg0"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("UP"))
        .unwrap_or(false);
    if up {
        ModuleStatus::new(shield_icon.to_string())
    } else {
        ModuleStatus::new(format!("{} off", shield_icon))
    }
}

fn get_surfshark_status() -> ModuleStatus {
    ModuleStatus::new("\u{f21b}") // user-secret (spy)
}

/// Execute a quick action for a module
pub fn execute_action(action: &str) -> Result<()> {
    let expanded = shellexpand::tilde(action);
    Command::new("sh").args(["-c", &expanded]).spawn()?;
    Ok(())
}
