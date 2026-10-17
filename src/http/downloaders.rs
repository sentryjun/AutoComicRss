// SPDX-License-Identifier: GPL-2.0-only
use super::State;
use crate::{
    domain::DownloaderInput,
    error::{Error, Result},
};
use actix_web::{web, HttpResponse};

pub(super) async fn downloaders(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.downloaders().await?))
}

pub(super) async fn create_downloader(
    state: State,
    body: web::Json<DownloaderInput>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Created().json(state.store.create_downloader(body.into_inner()).await?))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DownloaderSelection {
    downloader_id: String,
}

pub(super) async fn activate_downloader(
    state: State,
    body: web::Json<DownloaderSelection>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    state.store.activate_downloader(&body.downloader_id).await?;
    Ok(HttpResponse::NoContent().finish())
}

pub(super) async fn claim_legacy_jobs(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Ok()
        .json(serde_json::json!({"claimed": state.store.claim_legacy_jobs(&id).await?})))
}
