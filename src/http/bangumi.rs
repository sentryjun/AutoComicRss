// SPDX-License-Identifier: GPL-2.0-only
use super::State;
use crate::error::Result;
use actix_web::{web, HttpResponse};

#[derive(serde::Deserialize)]
pub(super) struct Search {
    q: String,
}
pub(super) async fn search(state: State, query: web::Query<Search>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.bangumi.search(&query.q).await?))
}
pub(super) async fn subject(state: State, id: web::Path<i64>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.bangumi.subject(*id).await?))
}
pub(super) async fn cover(state: State, id: web::Path<i64>) -> Result<HttpResponse> {
    let (mime, bytes) = state.bangumi.cover(*id).await?;
    Ok(HttpResponse::Ok()
        .content_type(mime)
        .insert_header(("X-Content-Type-Options", "nosniff"))
        .insert_header(("Cache-Control", "private, max-age=300"))
        .body(bytes))
}
