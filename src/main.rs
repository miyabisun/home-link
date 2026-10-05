mod logging;
mod port;

use std::{error::Error, net::SocketAddr};

use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    logging::init();

    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "home-link.db".into());
    let db = home_link::open_db(&db_path)?;
    info!(%db_path, "database opened");

    let bind_addr = SocketAddr::from(([0, 0, 0, 0], port::from_env()?));
    let listener = TcpListener::bind(bind_addr).await?;
    info!(%bind_addr, "server listening");

    let matter_url = std::env::var("MATTER_SERVER_URL")
        .ok()
        .filter(|url| !url.is_empty());
    info!(
        matter_url = matter_url.as_deref().unwrap_or("unset"),
        "matterjs-server"
    );

    axum::serve(listener, home_link::app(db, matter_url))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("server stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    info!("shutdown signal received");
}
