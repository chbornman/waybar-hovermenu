mod config;
mod ipc;
mod menu;
mod modules;
mod watchers;

use std::sync::Arc;
use anyhow::Result;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive("waybar_hovermenu=info".parse()?)
        )
        .init();
    
    tracing::info!("Starting waybar-hovermenu");
    
    // Load configuration
    let config = Arc::new(config::Config::load()?);
    tracing::info!("Loaded config with {} modules", config.modules.len());
    
    // Create menu manager
    let menu_manager = Arc::new(menu::MenuManager::new(Arc::clone(&config)));
    
    // Create IPC server
    let ipc_server = Arc::new(ipc::IpcServer::new(
        Arc::clone(&config),
        Arc::clone(&menu_manager),
    ));
    
    // Start watchers for real-time updates
    watchers::start_watchers(
        Arc::clone(&config),
        Arc::clone(&menu_manager),
        ipc_server.status_sender(),
    ).await;
    
    // Handle shutdown signals
    let shutdown = async {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("Received shutdown signal");
    };
    
    // Run IPC server until shutdown
    tokio::select! {
        result = ipc_server.run() => {
            if let Err(e) = result {
                tracing::error!("IPC server error: {}", e);
            }
        }
        _ = shutdown => {}
    }
    
    // Cleanup
    let _ = std::fs::remove_file(&config.daemon.socket_path);
    tracing::info!("Shutdown complete");
    
    Ok(())
}
