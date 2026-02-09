use anyhow::Result;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

use crate::config::Config;
use crate::menu::MenuManager;
use crate::modules::{execute_action, get_status};

/// IPC server that listens on a Unix socket
pub struct IpcServer {
    config: Arc<Config>,
    menu_manager: Arc<MenuManager>,
    /// Broadcast channel for status updates
    status_tx: broadcast::Sender<(String, String)>, // (module, json)
}

impl IpcServer {
    pub fn new(config: Arc<Config>, menu_manager: Arc<MenuManager>) -> Self {
        let (status_tx, _) = broadcast::channel(100);
        Self {
            config,
            menu_manager,
            status_tx,
        }
    }
    
    /// Get a sender for broadcasting status updates
    pub fn status_sender(&self) -> broadcast::Sender<(String, String)> {
        self.status_tx.clone()
    }
    
    /// Start the IPC server
    pub async fn run(&self) -> Result<()> {
        let socket_path = &self.config.daemon.socket_path;
        
        // Remove existing socket if present
        let _ = std::fs::remove_file(socket_path);
        
        let listener = UnixListener::bind(socket_path)?;
        tracing::info!("IPC server listening on {}", socket_path);
        
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let config = Arc::clone(&self.config);
                    let menu_manager = Arc::clone(&self.menu_manager);
                    let status_tx = self.status_tx.clone();
                    
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, config, menu_manager, status_tx).await {
                            tracing::error!("Client error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("Accept error: {}", e);
                }
            }
        }
    }
    
    /// Broadcast a status update for a module
    pub fn broadcast_status(&self, module: &str) {
        let pinned = futures::executor::block_on(self.menu_manager.is_pinned(module));
        let status = get_status(module, pinned);
        let json = status.to_json();
        let _ = self.status_tx.send((module.to_string(), json));
    }
}

async fn handle_client(
    stream: UnixStream,
    config: Arc<Config>,
    menu_manager: Arc<MenuManager>,
    status_tx: broadcast::Sender<(String, String)>,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    
    // Read the first line to determine the command
    reader.read_line(&mut line).await?;
    let line = line.trim();
    
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(());
    }
    
    let command = parts[0];
    let module = parts.get(1).copied();
    
    match command {
        "follow" => {
            // Stream status updates for a module
            if let Some(module) = module {
                let mut rx = status_tx.subscribe();
                
                // Send initial status (use spawn_blocking since get_status does blocking I/O)
                let pinned = if config.daemon.hover {
                    menu_manager.is_pinned(module).await
                } else {
                    menu_manager.is_menu_open(module).await
                };
                let module_owned = module.to_string();
                let status = tokio::task::spawn_blocking(move || {
                    get_status(&module_owned, pinned)
                }).await.unwrap_or_else(|_| crate::modules::ModuleStatus::new("error"));
                writer.write_all(status.to_json().as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
                
                // Stream updates
                loop {
                    match rx.recv().await {
                        Ok((update_module, json)) => {
                            if update_module == module {
                                writer.write_all(json.as_bytes()).await?;
                                writer.write_all(b"\n").await?;
                                writer.flush().await?;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // Missed some updates, continue
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            }
        }
        
        "status" => {
            // One-shot status query (use spawn_blocking since get_status does blocking I/O)
            if let Some(module) = module {
                let pinned = if config.daemon.hover {
                    menu_manager.is_pinned(module).await
                } else {
                    menu_manager.is_menu_open(module).await
                };
                let module_owned = module.to_string();
                let status = tokio::task::spawn_blocking(move || {
                    get_status(&module_owned, pinned)
                }).await.unwrap_or_else(|_| crate::modules::ModuleStatus::new("error"));
                writer.write_all(status.to_json().as_bytes()).await?;
                writer.write_all(b"\n").await?;
            }
        }
        
        "hover" => {
            if let Some(module) = module {
                if let Err(e) = MenuManager::hover(&menu_manager, module).await {
                    tracing::error!("Hover error: {}", e);
                }
            }
        }
        
        "leave" => {
            if let Err(e) = menu_manager.leave().await {
                tracing::error!("Leave error: {}", e);
            }
        }
        
        "click" => {
            if let Some(module) = module {
                if let Err(e) = MenuManager::click(&menu_manager, module).await {
                    tracing::error!("Click error: {}", e);
                }
                // Broadcast status update to reflect active state
                // When hover is disabled, highlight based on menu being open
                // When hover is enabled, highlight based on pin state
                let highlighted = if config.daemon.hover {
                    menu_manager.is_pinned(module).await
                } else {
                    menu_manager.is_menu_open(module).await
                };
                let status = get_status(module, highlighted);
                let _ = status_tx.send((module.to_string(), status.to_json()));
            }
        }
        
        "action" => {
            if let Some(module) = module {
                if let Some(module_config) = config.get_module(module) {
                    if let Some(action) = &module_config.action {
                        if let Err(e) = execute_action(action) {
                            tracing::error!("Action error: {}", e);
                        }
                        // Give the action time to complete, then broadcast update
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        let pinned = menu_manager.is_pinned(module).await;
                        let status = get_status(module, pinned);
                        let _ = status_tx.send((module.to_string(), status.to_json()));
                    }
                }
            }
        }
        
        _ => {
            tracing::warn!("Unknown command: {}", command);
        }
    }
    
    Ok(())
}
