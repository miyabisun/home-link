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

    axum::serve(listener, home_link::app(db))
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
