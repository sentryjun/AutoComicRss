// SPDX-License-Identifier: GPL-2.0-only
use super::State;
use crate::error::Result;
use actix_web::HttpResponse;

pub(super) async fn status(state: State) -> Result<HttpResponse> {
    let active = state.store.active_downloader_id().await?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "downloader_configured": active.is_some(),
        "active_downloader_id": active,
        "active_destination_id": state.store.active_destination_id().await?,
        "legacy_unassigned_jobs": state.store.legacy_unassigned_count().await?,
        "version": env!("CARGO_PKG_VERSION")
    })))
}

pub(super) async fn run(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.cycle().await?))
}

pub(super) async fn health() -> HttpResponse {
    HttpResponse::Ok()
        .json(serde_json::json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")}))
}

pub(super) async fn ready(state: State) -> HttpResponse {
    // Health checks must not wait for scans or contact websites/downloaders/media.
    match tokio::time::timeout(std::time::Duration::from_secs(2), state.store.ping()).await {
        Ok(Ok(())) => HttpResponse::Ok().json(serde_json::json!({"status": "ready"})),
        _ => HttpResponse::ServiceUnavailable().json(serde_json::json!({"status": "unavailable"})),
    }
}
