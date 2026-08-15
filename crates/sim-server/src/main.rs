use anyhow::{Context, Result};
use sim_server::{router, ServerState};
use std::net::SocketAddr;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let bind = std::env::var("TRANSFER_WINDOW_BIND")
        .unwrap_or_else(|_| "127.0.0.1:3000".into())
        .parse::<SocketAddr>()
        .context("TRANSFER_WINDOW_BIND must be a socket address")?;
    if !bind.ip().is_loopback()
        && std::env::var("TRANSFER_WINDOW_ALLOW_REMOTE").as_deref() != Ok("1")
    {
        anyhow::bail!(
            "refusing a non-loopback bind without TRANSFER_WINDOW_ALLOW_REMOTE=1; add authentication at the reverse proxy before public exposure"
        );
    }
    let data_directory = std::env::var_os("TRANSFER_WINDOW_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("var/transfer-window"));
    let allowed_origin = std::env::var("TRANSFER_WINDOW_WEB_ORIGIN")
        .unwrap_or_else(|_| "http://localhost:1420".into());
    let state = ServerState::new(data_directory)?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!("Transfer Window API listening on http://{bind}/api/v1");
    axum::serve(listener, router(state, &allowed_origin)?).await?;
    Ok(())
}
