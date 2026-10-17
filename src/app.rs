// SPDX-License-Identifier: GPL-2.0-only
//! Application startup and graceful shutdown.
use crate::{
    config::Config,
    http,
    service::{worker, Service},
    storage::Store,
};
use actix_web::{web, App, HttpServer};
use std::sync::Arc;

pub async fn run(config: Config) -> std::io::Result<()> {
    std::fs::create_dir_all("data")?;
    let store = Store::open(&config.database_url)
        .await
        .map_err(std::io::Error::other)?;
    let service = Arc::new(
        Service::new(store, &config)
            .await
            .map_err(std::io::Error::other)?,
    );
    let app_service = service.clone();
    let server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(app_service.clone()))
            .app_data(web::JsonConfig::default().limit(32 * 1024))
            .configure(http::configure)
    })
    .bind(("127.0.0.1", config.port))?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let background = tokio::spawn(worker(service, config.poll_interval, shutdown_rx));

    tracing::info!(address = %format!("127.0.0.1:{}", config.port), "AutoComicRss started");

    let result = server.run().await;
    let _ = shutdown_tx.send(true);
    let _ = background.await;
    result
}
