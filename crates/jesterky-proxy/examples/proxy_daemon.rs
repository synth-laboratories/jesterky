use jesterky_proxy::ChatProxy;
use std::io::{self, Write};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = std::env::args()
        .nth(1)
        .ok_or("usage: cargo run -p jesterky-proxy --example proxy_daemon -- <model>")?;
    let proxy = ChatProxy::spawn(&model)
        .await?
        .ok_or("the requested model does not require a chat proxy")?;

    println!("CODEX_HOME={}", proxy.codex_home().display());
    io::stdout().flush()?;

    // The caller owns the process lifetime and terminates this sidecar after codex exits.
    std::future::pending::<()>().await;
    Ok(())
}
