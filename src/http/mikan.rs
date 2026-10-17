// SPDX-License-Identifier: GPL-2.0-only
use super::State;
use crate::error::Result;
use actix_web::{web, HttpResponse};

#[derive(serde::Deserialize)]
pub(super) struct SearchQuery {
    q: String,
}

pub(super) async fn mikan_search(
    state: State,
    query: web::Query<SearchQuery>,
) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.mikan.search(&query.q).await?))
}

pub(super) async fn mikan_series(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.mikan.series(&id).await?))
}

pub(super) async fn mikan_cover(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    let (mime, bytes) = state.mikan.cover(&id).await?;
    Ok(HttpResponse::Ok()
        .content_type(mime)
        .insert_header(("X-Content-Type-Options", "nosniff"))
        .insert_header(("Cache-Control", "private, max-age=60"))
        .body(bytes))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResolveInput {
    url: String,
}

pub(super) async fn mikan_resolve(
    state: State,
    body: web::Json<ResolveInput>,
) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.mikan.resolve(&body.url).await?))
}
