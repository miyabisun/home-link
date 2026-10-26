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

    let sesame = home_link::sesame::Config::from_env();
    match &sesame {
        Ok(config) => info!(?config, "SESAME"),
        Err(reason) => info!(reason, "SESAME unlock disabled"),
    }

    let outside = |name, default: &str| {
        std::env::var(name)
            .ok()
            .filter(|url| !url.is_empty())
            .unwrap_or_else(|| default.into())
    };
    let mut home = home_link::Home::new(db, matter_url.clone())
        .with_sesame(sesame)
        .with_outside(home_link::Outside {
            gsi_url: outside("GSI_URL", home_link::outside::GSI_URL),
            open_meteo_url: outside("OPEN_METEO_URL", home_link::outside::OPEN_METEO_URL),
        });
    match std::env::var("SWITCHBOT_METER_ADDRESS")
        .ok()
        .filter(|text| !text.is_empty())
    {
        None => info!("SwitchBot meter unset"),
        Some(text) => match home_link::meter::address(&text) {
            Some(address) => {
                // The address stays out of the logs, as out of the repository.
                info!("SwitchBot meter set");
                home = home.with_meter(address);
            }
            None => {
                tracing::warn!(
                    "SWITCHBOT_METER_ADDRESS is not a Bluetooth address; meter disabled"
                );
            }
        },
    }
    let discord = std::env::var(home_link::discord::ENV)
        .ok()
        .filter(|value| !value.is_empty());
    match discord.as_deref().map(home_link::discord::Discord::new) {
        // The URL holds the webhook's token and stays out of the logs.
        Some(Some(discord)) => {
            info!("light notices go to Discord");
            home = home.with_discord(Some(discord));
        }
        Some(None) => tracing::warn!(
            "{} is neither a webhook URL nor discord://token@id; light notices are not sent",
            home_link::discord::ENV
        ),
        None => info!(
            "{} unset; light notices are not sent",
            home_link::discord::ENV
        ),
    }
    home.start();

    axum::serve(
        listener,
        home.router()
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
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
