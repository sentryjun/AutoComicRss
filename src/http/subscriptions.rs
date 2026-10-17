// SPDX-License-Identifier: GPL-2.0-only
use super::State;
use crate::{
    domain::SubscriptionInput,
    error::{Error, Result},
};
use actix_web::{web, HttpResponse};

pub(super) async fn list(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.subscriptions().await?))
}
pub(super) async fn get(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.subscription(&id).await?))
}
pub(super) async fn create(
    state: State,
    body: web::Json<SubscriptionInput>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Created().json(state.store.create(body.into_inner()).await?))
}
pub(super) async fn update(
    state: State,
    id: web::Path<String>,
    body: web::Json<SubscriptionInput>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Ok().json(state.store.update(&id, body.into_inner()).await?))
}
pub(super) async fn delete(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    state.store.delete(&id).await?;
    Ok(HttpResponse::NoContent().finish())
}
pub(super) async fn preview(
    state: State,
    body: web::Json<SubscriptionInput>,
) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.preview(body.into_inner()).await?))
}
pub(super) async fn preview_saved(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.preview_subscription(&id).await?))
}
pub(super) async fn scan(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(serde_json::json!({"queued": state.scan_subscription(&id).await?})))
}
pub(super) async fn releases(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.releases(&id).await?))
}
