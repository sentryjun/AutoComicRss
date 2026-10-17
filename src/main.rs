// SPDX-License-Identifier: GPL-2.0-only
use autocomic_rss::{app, config::Config};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let config = Config::from_env().map_err(std::io::Error::other)?;
    app::run(config).await
}
