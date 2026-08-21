//! 主入口 - HTTP 服务启动
//!
//! This binary only compiles with `http-server` feature enabled.

#[cfg(feature = "http-server")]
mod http_server {
    use tokio::net::TcpListener;

    pub async fn run() -> anyhow::Result<()> {
        let app = desktop_api::api::http::create_router();

        let listener = TcpListener::bind("127.0.0.1:8888").await?;
        println!("DesktopAPI v2.0 启动于 http://localhost:8888");

        axum::serve(listener, app).await?;
        Ok(())
    }
}

#[cfg(not(feature = "http-server"))]
fn main() {
    println!("DesktopAPI v2.0 — This binary requires the 'http-server' feature.");
    println!("Use: cargo run --features http-server");
}

#[cfg(feature = "http-server")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter("desktop_api=debug")
        .init();

    http_server::run().await?;

    Ok(())
}
