use std::net::SocketAddr;

use anyhow::Result;

use crate::services::web;

pub async fn run_web(port: u16) -> Result<()> {
    let app = web::router();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("dongshan web running at http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
