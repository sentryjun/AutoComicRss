// SPDX-License-Identifier: GPL-2.0-only
use super::State;
use crate::{
    error::{Error, Result},
    media::{self, DestinationInput, MappingInput},
};
use actix_web::{web, HttpResponse};

pub(super) async fn destinations(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.destinations().await?))
}

pub(super) async fn create_destination(
    state: State,
    body: web::Json<DestinationInput>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Created().json(state.store.create_destination(body.into_inner()).await?))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DestinationSelection {
    destination_id: String,
}

pub(super) async fn activate_destination(
    state: State,
    body: web::Json<DestinationSelection>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    state
        .store
        .activate_destination(&body.destination_id)
        .await?;
    Ok(HttpResponse::NoContent().finish())
}

pub(super) async fn mappings(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.mappings(&id).await?))
}

pub(super) async fn create_mapping(
    state: State,
    id: web::Path<String>,
    body: web::Json<MappingInput>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Created().json(state.store.create_mapping(&id, body.into_inner()).await?))
}

#[derive(serde::Deserialize)]
pub(super) struct DirectoryQuery {
    #[serde(default)]
    path: String,
}

pub(super) async fn directory_entries(
    state: State,
    id: web::Path<String>,
    query: web::Query<DirectoryQuery>,
) -> Result<HttpResponse> {
    media::validate_relative(&query.path, true)?;
    let access = state.store.media_access(&id).await?;
    let path = query.into_inner().path;
    let entries = access.entries(&path).await?;
    Ok(HttpResponse::Ok().json(entries))
}
