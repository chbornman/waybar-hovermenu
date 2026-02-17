use anyhow::Result;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::menu::MenuManager;
use crate::modules::get_status;

/// Start all watchers for real-time status updates
pub async fn start_watchers(
    config: Arc<Config>,
    menu_manager: Arc<MenuManager>,
    status_tx: broadcast::Sender<(String, String)>,
) {
    // Audio watcher (PulseAudio)
    let tx = status_tx.clone();
    let mm = Arc::clone(&menu_manager);
    tokio::spawn(async move {
        if let Err(e) = watch_audio(tx, mm).await {
            tracing::error!("Audio watcher error: {}", e);
        }
    });
    
    // Bluetooth watcher (dbus-monitor)
    let tx = status_tx.clone();
    let mm = Arc::clone(&menu_manager);
    tokio::spawn(async move {
        if let Err(e) = watch_bluetooth(tx, mm).await {
            tracing::error!("Bluetooth watcher error: {}", e);
        }
    });
    
    // Network watcher (dbus-monitor)
    let tx = status_tx.clone();
    let mm = Arc::clone(&menu_manager);
    tokio::spawn(async move {
        if let Err(e) = watch_network(tx, mm).await {
            tracing::error!("Network watcher error: {}", e);
        }
    });
    
    // CPU poller
    let tx = status_tx.clone();
    let mm = Arc::clone(&menu_manager);
    let interval = config.modules.get("cpu")
        .and_then(|m| m.poll_interval)
        .unwrap_or(3);
    tokio::spawn(async move {
        poll_module("cpu", Duration::from_secs(interval), tx, mm).await;
    });
    
    // Battery watcher (UPower) + fallback poller
    let tx = status_tx.clone();
    let mm = Arc::clone(&menu_manager);
    tokio::spawn(async move {
        if let Err(e) = watch_battery(tx, mm).await {
            tracing::error!("Battery watcher error: {}", e);
        }
    });
    
    // Mail watcher (inotify)
    let tx = status_tx.clone();
    let mm = Arc::clone(&menu_manager);
    let mail_dir = config.modules.get("mail")
        .and_then(|m| m.watch_dir.clone())
        .unwrap_or_else(|| "~/.local/share/mail".to_string());
    tokio::spawn(async move {
        if let Err(e) = watch_mail(&mail_dir, tx, mm).await {
            tracing::error!("Mail watcher error: {}", e);
        }
    });
    
    // Calendar/clock poller (every 30 seconds - updates on the minute)
    let tx = status_tx.clone();
    let mm = Arc::clone(&menu_manager);
    tokio::spawn(async move {
        poll_module("calendar", Duration::from_secs(30), tx, mm).await;
    });
}

/// Watch for PulseAudio changes
async fn watch_audio(
    tx: broadcast::Sender<(String, String)>,
    menu_manager: Arc<MenuManager>,
) -> Result<()> {
    loop {
        let mut child = TokioCommand::new("pactl")
            .args(["subscribe"])
            .stdout(Stdio::piped())
            .spawn()?;
        
        let stdout = child.stdout.take().expect("stdout");
        let mut reader = BufReader::new(stdout).lines();
        
        while let Ok(Some(line)) = reader.next_line().await {
            if line.contains("'change' on sink") {
                let pinned = menu_manager.is_pinned("audio").await;
                let status = tokio::task::spawn_blocking(move || {
                    get_status("audio", pinned)
                }).await.unwrap_or_else(|_| crate::modules::ModuleStatus::new("error"));
                let _ = tx.send(("audio".to_string(), status.to_json()));
            }
        }
        
        // Reconnect after a short delay if pactl exits
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Watch for Bluetooth changes via dbus-monitor
async fn watch_bluetooth(
    tx: broadcast::Sender<(String, String)>,
    menu_manager: Arc<MenuManager>,
) -> Result<()> {
    loop {
        let mut child = TokioCommand::new("dbus-monitor")
            .args(["--system", "type='signal',sender='org.bluez'"])
            .stdout(Stdio::piped())
            .spawn()?;
        
        let stdout = child.stdout.take().expect("stdout");
        let mut reader = BufReader::new(stdout).lines();
        
        while let Ok(Some(_)) = reader.next_line().await {
            let pinned = menu_manager.is_pinned("bluetooth").await;
            let status = tokio::task::spawn_blocking(move || {
                get_status("bluetooth", pinned)
            }).await.unwrap_or_else(|_| crate::modules::ModuleStatus::new("error"));
            let _ = tx.send(("bluetooth".to_string(), status.to_json()));
        }
        
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Watch for NetworkManager changes via dbus-monitor
async fn watch_network(
    tx: broadcast::Sender<(String, String)>,
    menu_manager: Arc<MenuManager>,
) -> Result<()> {
    loop {
        let mut child = TokioCommand::new("dbus-monitor")
            .args(["--system", "type='signal',interface='org.freedesktop.NetworkManager'"])
            .stdout(Stdio::piped())
            .spawn()?;
        
        let stdout = child.stdout.take().expect("stdout");
        let mut reader = BufReader::new(stdout).lines();
        
        while let Ok(Some(_)) = reader.next_line().await {
            let pinned = menu_manager.is_pinned("network").await;
            let status = tokio::task::spawn_blocking(move || {
                get_status("network", pinned)
            }).await.unwrap_or_else(|_| crate::modules::ModuleStatus::new("error"));
            let _ = tx.send(("network".to_string(), status.to_json()));
        }
        
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Watch for battery changes via UPower
async fn watch_battery(
    tx: broadcast::Sender<(String, String)>,
    menu_manager: Arc<MenuManager>,
) -> Result<()> {
    loop {
        let mut child = TokioCommand::new("upower")
            .args(["--monitor"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdout = child.stdout.take().expect("stdout");
        let mut reader = BufReader::new(stdout).lines();

        while let Ok(Some(line)) = reader.next_line().await {
            if line.contains("battery") || line.contains("line_power") || line.contains("DisplayDevice") {
                let pinned = menu_manager.is_pinned("battery").await;
                let status = tokio::task::spawn_blocking(move || {
                    get_status("battery", pinned)
                }).await.unwrap_or_else(|_| crate::modules::ModuleStatus::new("error"));
                let _ = tx.send(("battery".to_string(), status.to_json()));
            }
        }

        // Reconnect after a short delay if upower exits
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Poll a module at a fixed interval
async fn poll_module(
    module: &str,
    interval: Duration,
    tx: broadcast::Sender<(String, String)>,
    menu_manager: Arc<MenuManager>,
) {
    let module = module.to_string();
    loop {
        tokio::time::sleep(interval).await;
        let pinned = menu_manager.is_pinned(&module).await;
        let module_clone = module.clone();
        let status = tokio::task::spawn_blocking(move || {
            get_status(&module_clone, pinned)
        }).await.unwrap_or_else(|_| crate::modules::ModuleStatus::new("error"));
        let _ = tx.send((module.clone(), status.to_json()));
    }
}

/// Watch mail directory for changes
async fn watch_mail(
    mail_dir: &str,
    tx: broadcast::Sender<(String, String)>,
    menu_manager: Arc<MenuManager>,
) -> Result<()> {
    let expanded = shellexpand::tilde(mail_dir).to_string();
    let path = Path::new(&expanded);
    
    if !path.exists() {
        tracing::warn!("Mail directory does not exist: {}", expanded);
        return Ok(());
    }
    
    // Use inotifywait for recursive watching
    loop {
        let mut child = TokioCommand::new("inotifywait")
            .args(["-m", "-r", "-e", "create,delete,moved_to,moved_from", &expanded])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        
        let stdout = child.stdout.take().expect("stdout");
        let mut reader = BufReader::new(stdout).lines();
        
        while let Ok(Some(_)) = reader.next_line().await {
            let pinned = menu_manager.is_pinned("mail").await;
            let status = tokio::task::spawn_blocking(move || {
                get_status("mail", pinned)
            }).await.unwrap_or_else(|_| crate::modules::ModuleStatus::new("error"));
            let _ = tx.send(("mail".to_string(), status.to_json()));
        }
        
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
