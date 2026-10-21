use ferrum_kv::{server, DEFAULT_PORT};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let port = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    println!("Servidor escuchando en 127.0.0.1:{port}");

    server::run(listener, tokio::signal::ctrl_c()).await;
    Ok(())
}
