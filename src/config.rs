use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub modules: HashMap<String, ModuleConfig>,
}

#[derive(Debug, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_terminal_cmd")]
    pub terminal_cmd: String,
    #[serde(default = "default_waybar_height")]
    pub waybar_height: u32,
    #[serde(default = "default_socket_path")]
    pub socket_path: String,
    /// Global toggle for hover-to-open behavior. When false, menus only open/close via click.
    #[serde(default)]
    pub hover: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            terminal_cmd: default_terminal_cmd(),
            waybar_height: default_waybar_height(),
            socket_path: default_socket_path(),
            hover: false,
        }
    }
}

fn default_terminal_cmd() -> String {
    "foot -T {title} {command}".to_string()
}

fn default_waybar_height() -> u32 {
    32
}

fn default_socket_path() -> String {
    "/tmp/waybar-hovermenu.sock".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModuleConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Menu type: "tui" or "gui"
    #[serde(default = "default_kind")]
    pub kind: String,

    /// Command to run for the menu (e.g., "wiremix", "bluetui")
    pub command: Option<String>,

    /// Window class for GUI apps (e.g., "localsend")
    pub window_class: Option<String>,

    /// Window size [width, height]
    #[serde(default = "default_size")]
    pub size: [u32; 2],

    /// Position: "top-right" or "top-left"
    #[serde(default = "default_position")]
    pub position: String,

    /// Right-click quick action command
    pub action: Option<String>,

    /// Poll interval in seconds (for modules that poll)
    pub poll_interval: Option<u64>,

    /// Watch directory (for mail module)
    pub watch_dir: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_kind() -> String {
    "tui".to_string()
}

fn default_size() -> [u32; 2] {
    [600, 400]
}

fn default_position() -> String {
    "top-right".to_string()
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path();

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config from {:?}", config_path))?;
            let config: Config =
                toml::from_str(&content).with_context(|| "Failed to parse config")?;
            Ok(config)
        } else {
            // Return default config
            Ok(Self::default())
        }
    }

    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("waybar-hovermenu")
            .join("config.toml")
    }

    pub fn get_module(&self, name: &str) -> Option<&ModuleConfig> {
        self.modules.get(name)
    }
}

impl Default for Config {
    fn default() -> Self {
        let mut modules = HashMap::new();

        // Audio
        modules.insert(
            "audio".to_string(),
            ModuleConfig {
                enabled: true,
                kind: "gui".to_string(),
                command: Some("pavucontrol".to_string()),
                window_class: Some("org.pulseaudio.pavucontrol".to_string()),
                size: [600, 400],
                position: "top-right".to_string(),
                action: Some("pactl set-sink-mute @DEFAULT_SINK@ toggle".to_string()),
                poll_interval: None,
                watch_dir: None,
            },
        );

        // Bluetooth
        modules.insert(
            "bluetooth".to_string(),
            ModuleConfig {
                enabled: true,
                kind: "tui".to_string(),
                command: Some("bluetui".to_string()),
                window_class: None,
                size: [600, 400],
                position: "top-right".to_string(),
                action: Some("bluetoothctl power off || bluetoothctl power on".to_string()),
                poll_interval: None,
                watch_dir: None,
            },
        );

        // Network
        modules.insert(
            "network".to_string(),
            ModuleConfig {
                enabled: true,
                kind: "tui".to_string(),
                command: Some("impala".to_string()),
                window_class: None,
                size: [600, 400],
                position: "top-right".to_string(),
                action: Some("nmcli radio wifi off || nmcli radio wifi on".to_string()),
                poll_interval: None,
                watch_dir: None,
            },
        );

        // CPU
        modules.insert(
            "cpu".to_string(),
            ModuleConfig {
                enabled: true,
                kind: "tui".to_string(),
                command: Some("/usr/bin/btop".to_string()),
                window_class: None,
                size: [900, 600],
                position: "top-right".to_string(),
                action: None,
                poll_interval: Some(3),
                watch_dir: None,
            },
        );

        // Battery
        modules.insert(
            "battery".to_string(),
            ModuleConfig {
                enabled: true,
                kind: "tui".to_string(),
                command: Some("~/.local/bin/powertui".to_string()),
                window_class: None,
                size: [600, 400],
                position: "top-right".to_string(),
                action: None,
                poll_interval: Some(30),
                watch_dir: None,
            },
        );

        // Mail
        modules.insert(
            "mail".to_string(),
            ModuleConfig {
                enabled: true,
                kind: "tui".to_string(),
                command: Some("mailtui".to_string()),
                window_class: None,
                size: [600, 400],
                position: "top-left".to_string(),
                action: Some("mbsync -a".to_string()),
                poll_interval: None,
                watch_dir: Some("~/.local/share/mail".to_string()),
            },
        );

        // Calendar
        modules.insert(
            "calendar".to_string(),
            ModuleConfig {
                enabled: true,
                kind: "tui".to_string(),
                command: Some("~/.local/bin/calentui".to_string()),
                window_class: None,
                size: [600, 400],
                position: "top-right".to_string(),
                action: None,
                poll_interval: None,
                watch_dir: None,
            },
        );

        // LocalSend
        modules.insert(
            "localsend".to_string(),
            ModuleConfig {
                enabled: true,
                kind: "gui".to_string(),
                command: Some("flatpak run org.localsend.localsend_app".to_string()),
                window_class: Some("localsend".to_string()),
                size: [400, 500],
                position: "top-left".to_string(),
                action: None,
                poll_interval: None,
                watch_dir: None,
            },
        );

        Self {
            daemon: DaemonConfig::default(),
            modules,
        }
    }
}
