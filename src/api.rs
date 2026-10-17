// SPDX-License-Identifier: GPL-2.0-only
use crate::{
    domain::{DownloaderInput, SubscriptionInput},
    error::{Error, Result},
    media::{self, DestinationInput, MappingInput},
    service::Service,
};
use actix_web::{web, HttpResponse};
use std::sync::Arc;

type State = web::Data<Arc<Service>>;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
            .route("/sources/mikan/search", web::get().to(mikan_search))
            .route("/sources/mikan/series/{id}", web::get().to(mikan_series))
            .route(
                "/sources/mikan/series/{id}/cover",
                web::get().to(mikan_cover),
            )
            .route("/sources/mikan/resolve", web::post().to(mikan_resolve))
            .route("/status", web::get().to(status))
            .route("/destinations", web::get().to(destinations))
            .route("/destinations", web::post().to(create_destination))
            .route("/destinations/active", web::put().to(activate_destination))
            .route("/destinations/{id}/mappings", web::get().to(mappings))
            .route(
                "/destinations/{id}/mappings",
                web::post().to(create_mapping),
            )
            .route(
                "/destinations/{id}/entries",
                web::get().to(directory_entries),
            )
            .route("/downloaders", web::get().to(downloaders))
            .route("/downloaders", web::post().to(create_downloader))
            .route("/downloaders/active", web::put().to(activate_downloader))
            .route(
                "/downloaders/{id}/claim-legacy-jobs",
                web::post().to(claim_legacy_jobs),
            )
            .route("/preview", web::post().to(preview))
            .route("/subscriptions", web::get().to(list))
            .route("/subscriptions", web::post().to(create))
            .route("/subscriptions/{id}", web::get().to(get))
            .route("/subscriptions/{id}", web::put().to(update))
            .route("/subscriptions/{id}", web::delete().to(delete))
            .route("/subscriptions/{id}/preview", web::get().to(preview_saved))
            .route("/subscriptions/{id}/scan", web::post().to(scan))
            .route("/subscriptions/{id}/releases", web::get().to(releases))
            .route("/jobs", web::get().to(jobs))
            .route("/jobs/{id}/files", web::get().to(job_files))
            .route("/jobs/{id}/files/check", web::post().to(check_job_files))
            .route("/jobs/{id}/retry", web::post().to(retry))
            .route("/run", web::post().to(run)),
    );
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    q: String,
}

async fn mikan_search(state: State, query: web::Query<SearchQuery>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.mikan.search(&query.q).await?))
}

async fn mikan_series(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.mikan.series(&id).await?))
}

async fn mikan_cover(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    let (mime, bytes) = state.mikan.cover(&id).await?;
    Ok(HttpResponse::Ok()
        .content_type(mime)
        .insert_header(("X-Content-Type-Options", "nosniff"))
        .insert_header(("Cache-Control", "private, max-age=60"))
        .body(bytes))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveInput {
    url: String,
}

async fn mikan_resolve(state: State, body: web::Json<ResolveInput>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.mikan.resolve(&body.url).await?))
}

async fn status(state: State) -> Result<HttpResponse> {
    let active = state.store.active_downloader_id().await?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "downloader_configured": active.is_some(),
        "active_downloader_id": active,
        "active_destination_id": state.store.active_destination_id().await?,
        "legacy_unassigned_jobs": state.store.legacy_unassigned_count().await?,
        "version": env!("CARGO_PKG_VERSION")
    })))
}

async fn destinations(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.destinations().await?))
}

async fn create_destination(
    state: State,
    body: web::Json<DestinationInput>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Created().json(state.store.create_destination(body.into_inner()).await?))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DestinationSelection {
    destination_id: String,
}

async fn activate_destination(
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

async fn mappings(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.mappings(&id).await?))
}

async fn create_mapping(
    state: State,
    id: web::Path<String>,
    body: web::Json<MappingInput>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Created().json(state.store.create_mapping(&id, body.into_inner()).await?))
}

#[derive(serde::Deserialize)]
struct DirectoryQuery {
    #[serde(default)]
    path: String,
}

async fn directory_entries(
    state: State,
    id: web::Path<String>,
    query: web::Query<DirectoryQuery>,
) -> Result<HttpResponse> {
    media::validate_relative(&query.path, true)?;
    let root = state.store.destination(&id).await?.root_path;
    let path = query.into_inner().path;
    let entries = tokio::task::spawn_blocking(move || media::list_local(&root, &path))
        .await
        .map_err(|_| Error::Upstream("local directory query failed".into()))?;
    Ok(HttpResponse::Ok().json(entries))
}

async fn job_files(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.file_report(&id).await?))
}

async fn check_job_files(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.check_job_files(&id).await?))
}

async fn downloaders(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.downloaders().await?))
}

async fn create_downloader(state: State, body: web::Json<DownloaderInput>) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Created().json(state.store.create_downloader(body.into_inner()).await?))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloaderSelection {
    downloader_id: String,
}

async fn activate_downloader(
    state: State,
    body: web::Json<DownloaderSelection>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    state.store.activate_downloader(&body.downloader_id).await?;
    Ok(HttpResponse::NoContent().finish())
}

async fn claim_legacy_jobs(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Ok()
        .json(serde_json::json!({"claimed": state.store.claim_legacy_jobs(&id).await?})))
}
async fn list(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.subscriptions().await?))
}
async fn get(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.subscription(&id).await?))
}
async fn create(state: State, body: web::Json<SubscriptionInput>) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Created().json(state.store.create(body.into_inner()).await?))
}
async fn update(
    state: State,
    id: web::Path<String>,
    body: web::Json<SubscriptionInput>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Ok().json(state.store.update(&id, body.into_inner()).await?))
}
async fn delete(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    state.store.delete(&id).await?;
    Ok(HttpResponse::NoContent().finish())
}
async fn preview(state: State, body: web::Json<SubscriptionInput>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.preview(body.into_inner()).await?))
}
async fn preview_saved(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.preview_subscription(&id).await?))
}
async fn scan(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(serde_json::json!({"queued": state.scan_subscription(&id).await?})))
}
async fn releases(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.releases(&id).await?))
}
async fn jobs(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.store.jobs().await?))
}
async fn retry(state: State, id: web::Path<String>) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    state.store.retry(&id).await?;
    Ok(HttpResponse::NoContent().finish())
}
async fn run(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok().json(state.cycle().await?))
}
