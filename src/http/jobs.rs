// SPDX-License-Identifier: GPL-2.0-only
use super::State;
use crate::error::{Error, Result};
use actix_web::{web, HttpResponse};

pub(super) async fn job_files(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.file_report(&id).await?))
}

pub(super) async fn check_job_files(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.check_job_files(&id).await?))
}

pub(super) async fn jobs(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.jobs().await?))
}
pub(super) async fn retry(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    state.store.retry(&id).await?;
    Ok(HttpResponse::NoContent().finish())
}
