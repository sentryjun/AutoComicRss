// SPDX-License-Identifier: GPL-2.0-only
use super::State;
use crate::{
    error::{Error, Result},
    scraping::SettingsInput,
};
use actix_web::{web, HttpResponse};
pub(super) async fn settings(state: State) -> Result<HttpResponse> {
    Ok(HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .json(state.store.scraping_settings().await?.view()))
}
pub(super) async fn save_settings(
    state: State,
    body: web::Json<SettingsInput>,
) -> Result<HttpResponse> {
    let _guard = state.gate.try_lock().map_err(|_| Error::Busy)?;
    Ok(HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-store"))
        .json(
            state
                .store
                .save_scraping_settings(body.into_inner())
                .await?
                .view(),
        ))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TestTitle {
    title: String,
    season: i32,
    episode_offset: i32,
}
pub(super) async fn test_title(state: State, body: web::Json<TestTitle>) -> Result<HttpResponse> {
    if !(1..=999).contains(&body.season) || body.episode_offset.abs_diff(0) > 10000 {
        return Err(Error::Invalid("invalid season or offset".into()));
    }
    let settings = state.store.scraping_settings().await?;
    let parsed = state
        .scraper
        .parse(&settings, &body.title, body.season)
        .await?;
    let episode = parsed.episode(body.season, body.episode_offset);
    Ok(HttpResponse::Ok().json(serde_json::json!({"metadata":parsed,"episode":episode.as_ref().ok(),"reason":episode.err()})))
}
