use anyhow::{Context, Result};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tracing::debug;

use crate::config::{Config, ModuleConfig};

/// Manages the state of open menus
pub struct MenuManager {
    config: Arc<Config>,
    /// Currently pinned module (if any)
    pinned: Mutex<Option<String>>,
    /// Currently open module (if any) - tracks which module's menu is open
    open_module: Mutex<Option<String>>,
    /// Generation counter to cancel old cursor watchers
    watcher_generation: AtomicU64,
}

impl MenuManager {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            pinned: Mutex::new(None),
            open_module: Mutex::new(None),
            watcher_generation: AtomicU64::new(0),
        }
    }
    
    /// Check if a module is currently pinned
    pub async fn is_pinned(&self, module: &str) -> bool {
        let pinned = self.pinned.lock().await;
        pinned.as_deref() == Some(module)
    }
    
    /// Check if any module is pinned
    pub async fn has_pinned(&self) -> bool {
        self.pinned.lock().await.is_some()
    }
    
    /// Check if a specific module's menu is currently open
    pub async fn is_menu_open(&self, module: &str) -> bool {
        let open = self.open_module.lock().await;
        open.as_deref() == Some(module)
    }
    
    /// Handle hover event - open menu for module (only if hover is enabled)
    pub async fn hover(self: &Arc<Self>, module: &str) -> Result<()> {
        // No-op if hover is disabled globally
        if !self.config.daemon.hover {
            return Ok(());
        }

        // If this module's menu is already open, do nothing
        if self.is_menu_open(module).await {
            return Ok(());
        }
        
        // Get module config
        let module_config = self.config.get_module(module)
            .context("Module not found")?;
        
        if !module_config.enabled {
            return Ok(());
        }
        
        // Close any existing menu first
        self.close_all_menus().await?;
        
        // Clear pin state when opening new menu via hover
        {
            let mut pinned = self.pinned.lock().await;
            *pinned = None;
        }
        
        // Open the new menu
        self.open_menu(module, module_config).await?;
        
        Ok(())
    }
    
    /// Handle leave event - close menu if not pinned and cursor not over menu
    /// Uses debouncing: checks multiple times over 300ms before closing
    /// Only active when hover mode is enabled.
    pub async fn leave(&self) -> Result<()> {
        // No-op if hover is disabled — menus are managed by click only
        if !self.config.daemon.hover {
            return Ok(());
        }

        // Don't close if pinned
        if self.has_pinned().await {
            return Ok(());
        }
        
        // Check cursor position multiple times over 300ms
        // Only close if cursor stays outside the safe zone
        for _ in 0..6 {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            
            let (cursor_x, cursor_y) = self.get_cursor_pos().await;
            
            // If cursor is in waybar, don't close
            if cursor_y <= self.config.daemon.waybar_height as i32 {
                return Ok(());
            }
            
            // If cursor is over menu, don't close
            if self.is_cursor_over_menu(cursor_x, cursor_y).await {
                return Ok(());
            }
        }
        
        // Cursor stayed outside safe zone for 300ms - close
        self.close_all_menus().await?;
        
        Ok(())
    }
    
    /// Handle click event.
    /// When hover is disabled: simple toggle — click opens, click again closes.
    /// When hover is enabled: original pin-based behavior.
    pub async fn click(self: &Arc<Self>, module: &str) -> Result<()> {
        let is_open = self.is_menu_open(module).await;

        if !self.config.daemon.hover {
            // Hover disabled — click is a simple open/close toggle
            if is_open {
                self.close_all_menus().await?;
            } else {
                let module_config = self.config.get_module(module)
                    .context("Module not found")?;

                if !module_config.enabled {
                    return Ok(());
                }

                // Close any other open menu first
                self.close_all_menus().await?;

                // Open the menu (no pin, no cursor watcher)
                self.open_menu(module, module_config).await?;
            }
        } else {
            // Hover enabled — original pin-based behavior
            let is_pinned = self.is_pinned(module).await;

            if is_pinned {
                // Already pinned - unpin and close
                {
                    let mut pinned = self.pinned.lock().await;
                    *pinned = None;
                }
                self.close_all_menus().await?;
            } else if is_open {
                // Menu is open but not pinned - pin it
                {
                    let mut pinned = self.pinned.lock().await;
                    *pinned = Some(module.to_string());
                }
                self.set_menu_border_gold(module).await?;
            } else {
                // Menu not open - open it and pin it
                let module_config = self.config.get_module(module)
                    .context("Module not found")?;

                if !module_config.enabled {
                    return Ok(());
                }

                // Close any existing menu first
                self.close_all_menus().await?;

                // Open and pin
                self.open_menu(module, module_config).await?;
                {
                    let mut pinned = self.pinned.lock().await;
                    *pinned = Some(module.to_string());
                }
                self.set_menu_border_gold(module).await?;
            }
        }

        // Jiggle the mouse slightly to reset waybar's click target state,
        // allowing the same widget to be clicked again without moving the mouse.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let _ = Command::new("ydotool")
            .args(["mousemove", "-x", "1", "-y", "0"])
            .output();
        let _ = Command::new("ydotool")
            .args(["mousemove", "-x", "-1", "-y", "0"])
            .output();

        Ok(())
    }
    
    /// Open a menu for a module
    async fn open_menu(self: &Arc<Self>, module: &str, config: &ModuleConfig) -> Result<()> {
        let command = config.command.as_ref()
            .context("Module has no command configured")?;
        
        let expanded_command = shellexpand::tilde(command);
        
        if config.kind == "gui" {
            // GUI app - just launch it, with GTK dark theme forced
            // Use tokio::process so the child is auto-reaped (avoids zombies)
            let gui_cmd = format!("GTK_THEME=Adwaita:dark {}", expanded_command);
            tokio::process::Command::new("sh")
                .args(["-c", &gui_cmd])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            
            // Mouse jiggle to prevent hover-leave issues
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let _ = Command::new("ydotool")
                .args(["mousemove", "-x", "1", "-y", "0"])
                .output();
            let _ = Command::new("ydotool")
                .args(["mousemove", "-x", "-1", "-y", "0"])
                .output();
        } else {
            // TUI app - launch in terminal with special title
            let title = format!("WAYBAR-MENU: {}", module);
            
            // Build command from template: replace {title} and {command}
            let cmd = self.config.daemon.terminal_cmd
                .replace("{title}", &title)
                .replace("{command}", &expanded_command);
            
            // Use tokio::process so the child is auto-reaped (avoids zombies)
            tokio::process::Command::new("sh")
                .args(["-c", &cmd])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
        }
        
        // Track which module is open
        {
            let mut open_module = self.open_module.lock().await;
            *open_module = Some(module.to_string());
        }
        
        // Only spawn cursor watcher when hover mode is enabled.
        // In click-only mode, menus stay open until explicitly closed by another click.
        if self.config.daemon.hover {
            // Increment generation to cancel any previous cursor watcher
            let generation = self.watcher_generation.fetch_add(1, Ordering::SeqCst) + 1;

            // Spawn cursor watcher task
            let manager = Arc::clone(self);
            let waybar_height = self.config.daemon.waybar_height;
            tokio::spawn(async move {
                // Wait for window to appear
                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

                let mut outside_count = 0;
                const CHECKS_BEFORE_CLOSE: u32 = 5; // 500ms outside safe zone
                loop {
                    // Check if this watcher is still valid (not superseded by a new menu)
                    if manager.watcher_generation.load(Ordering::SeqCst) != generation {
                        debug!("Cursor watcher cancelled (new generation)");
                        return;
                    }

                    // Check if menu is pinned - if so, stop watching
                    if manager.has_pinned().await {
                        debug!("Cursor watcher stopped (menu pinned)");
                        return;
                    }

                    // Check if menu is still open
                    if manager.open_module.lock().await.is_none() {
                        debug!("Cursor watcher stopped (menu closed)");
                        return;
                    }

                    let (cursor_x, cursor_y) = manager.get_cursor_pos().await;

                    // Safe zone: waybar area OR over menu window
                    let in_waybar = cursor_y <= waybar_height as i32;
                    let over_menu = manager.is_cursor_over_menu(cursor_x, cursor_y).await;

                    tracing::debug!("Cursor at ({}, {}), in_waybar={}, over_menu={}", cursor_x, cursor_y, in_waybar, over_menu);

                    if in_waybar || over_menu {
                        // Cursor is in safe zone - reset counter
                        outside_count = 0;
                    } else {
                        // Cursor is outside safe zone
                        outside_count += 1;

                        if outside_count >= CHECKS_BEFORE_CLOSE {
                            let _ = manager.close_all_menus().await;
                            return;
                        }
                    }

                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            });
        }
        
        Ok(())
    }
    
    /// Close all waybar menus with slide-up animation, then kill
    async fn close_all_menus(&self) -> Result<()> {
        // Collect all GUI window classes from config
        let gui_classes: Vec<String> = self.config.modules.values()
            .filter(|m| m.kind == "gui")
            .filter_map(|m| m.window_class.clone())
            .collect();

        // Find all menu windows
        let output = Command::new("hyprctl")
            .args(["clients", "-j"])
            .output()?;
        
        let clients: serde_json::Value = serde_json::from_slice(&output.stdout)
            .unwrap_or(serde_json::Value::Array(vec![]));
        
        // Collect windows to animate
        let mut windows: Vec<(String, i32)> = Vec::new(); // (address, pid)
        
        if let Some(clients) = clients.as_array() {
            for client in clients {
                let title = client.get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let class = client.get("class")
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                let pid = client.get("pid")
                    .and_then(|p| p.as_i64())
                    .unwrap_or(0) as i32;
                let addr = client.get("address")
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .to_string();
                
                let is_tui_menu = title.starts_with("WAYBAR-MENU:");
                let is_gui_menu = gui_classes.iter().any(|c| c == class);
                
                if is_tui_menu || is_gui_menu {
                    windows.push((addr, pid));
                }
            }
        }
        
        // Animate: slide up and fade out
        for step in 1i32..=8 {
            let move_y = step * -60; // Move up 60px per step
            let alpha = 1.0 - (step as f32 * 0.12);
            
            for (addr, _) in &windows {
                let _ = Command::new("hyprctl")
                    .args(["--batch", &format!(
                        "dispatch movewindowpixel 0 {},address:{} ; dispatch setprop address:{} alpha {:.2} lock",
                        move_y, addr, addr, alpha
                    )])
                    .output();
            }
            
            tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;
        }
        
        // Now kill the processes
        for (_, pid) in &windows {
            if *pid > 0 {
                unsafe {
                    libc::kill(*pid, libc::SIGTERM);
                }
            }
        }
        
        // Clear open menu tracking
        {
            let mut open_module = self.open_module.lock().await;
            *open_module = None;
        }
        
        Ok(())
    }
    
    /// Find a menu window's address
    async fn find_menu_window(&self, module: &str, config: &ModuleConfig) -> Option<String> {
        let output = Command::new("hyprctl")
            .args(["clients", "-j"])
            .output()
            .ok()?;
        
        let clients: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        
        if let Some(clients) = clients.as_array() {
            for client in clients {
                if config.kind == "gui" {
                    // Match by window class for GUI apps
                    if let Some(window_class) = &config.window_class {
                        let class = client.get("class")
                            .and_then(|c| c.as_str())
                            .unwrap_or("");
                        if class == window_class {
                            return client.get("address")
                                .and_then(|a| a.as_str())
                                .map(|s| s.to_string());
                        }
                    }
                } else {
                    // Match by title for TUI apps
                    let title = client.get("title")
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    let expected_title = format!("WAYBAR-MENU: {}", module);
                    if title.contains(&expected_title) || title == expected_title {
                        return client.get("address")
                            .and_then(|a| a.as_str())
                            .map(|s| s.to_string());
                    }
                }
            }
        }
        
        None
    }
    
    /// Set gold border on menu window for a module
    async fn set_menu_border_gold(&self, module: &str) -> Result<()> {
        // Give window time to appear
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        let module_config = self.config.get_module(module);
        if let Some(config) = module_config {
            if let Some(addr) = self.find_menu_window(module, config).await {
                let _ = Command::new("hyprctl")
                    .args(["dispatch", "setprop", &format!("address:{}", addr), "activebordercolor", "0xffd4a366"])
                    .output();
            }
        }
        Ok(())
    }
    
    /// Get cursor position (x, y)
    async fn get_cursor_pos(&self) -> (i32, i32) {
        let output = Command::new("hyprctl")
            .args(["cursorpos", "-j"])
            .output()
            .ok();
        
        if let Some(output) = output {
            if let Ok(pos) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                let x = pos.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = pos.get("y").and_then(|v| v.as_i64()).unwrap_or(100) as i32;
                return (x, y);
            }
        }
        
        (0, 100) // Default to below waybar
    }
    
    /// Check if cursor is inside any open menu window
    async fn is_cursor_over_menu(&self, cursor_x: i32, cursor_y: i32) -> bool {
        let gui_classes: Vec<String> = self.config.modules.values()
            .filter(|m| m.kind == "gui")
            .filter_map(|m| m.window_class.clone())
            .collect();

        let output = Command::new("hyprctl")
            .args(["clients", "-j"])
            .output()
            .ok();
        
        if let Some(output) = output {
            if let Ok(clients) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                if let Some(clients) = clients.as_array() {
                    for client in clients {
                        let title = client.get("title")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        let class = client.get("class")
                            .and_then(|c| c.as_str())
                            .unwrap_or("");
                        
                        // Check if this is a menu window
                        let is_tui_menu = title.starts_with("WAYBAR-MENU:");
                        let is_gui_menu = gui_classes.iter().any(|c| c == class);
                        if !is_tui_menu && !is_gui_menu {
                            continue;
                        }
                        
                        // Get window position and size
                        let at = client.get("at").and_then(|a| a.as_array());
                        let size = client.get("size").and_then(|s| s.as_array());
                        
                        if let (Some(at), Some(size)) = (at, size) {
                            let win_x = at.get(0).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            let win_y = at.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            let win_w = size.get(0).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            let win_h = size.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                            
                            // Check if cursor is inside this window (with 10px buffer)
                            let buffer = 10;
                            if cursor_x >= win_x - buffer && cursor_x < win_x + win_w + buffer &&
                               cursor_y >= win_y - buffer && cursor_y < win_y + win_h + buffer {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        
        false
    }
}
