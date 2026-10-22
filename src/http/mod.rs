// SPDX-License-Identifier: GPL-2.0-only
//! HTTP routes and request/response translation.
mod bangumi;
mod destinations;
mod downloaders;
mod error;
mod jobs;
mod mikan;
mod scraping;
mod subscriptions;
mod system;
mod ui;

use crate::service::Service;
use actix_web::web;
use std::sync::Arc;

type State = web::Data<Arc<Service>>;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.configure(ui::configure);
    cfg.route("/health", web::get().to(system::health));
    cfg.route("/ready", web::get().to(system::ready));
    cfg.service(
        web::scope("/api")
            .route("/scraping/settings", web::get().to(scraping::settings))
            .route("/scraping/settings", web::put().to(scraping::save_settings))
            .route("/scraping/test", web::post().to(scraping::test_title))
            .route("/sources/bangumi/search", web::get().to(bangumi::search))
            .route(
                "/sources/bangumi/subjects/{id}",
                web::get().to(bangumi::subject),
            )
            .route(
                "/sources/bangumi/subjects/{id}/cover",
                web::get().to(bangumi::cover),
            )
            .route("/sources/mikan/search", web::get().to(mikan::mikan_search))
            .route(
                "/sources/mikan/series/{id}",
                web::get().to(mikan::mikan_series),
            )
            .route(
                "/sources/mikan/series/{id}/cover",
                web::get().to(mikan::mikan_cover),
            )
            .route(
                "/sources/mikan/resolve",
                web::post().to(mikan::mikan_resolve),
            )
            .route("/status", web::get().to(system::status))
            .route("/destinations", web::get().to(destinations::destinations))
            .route(
                "/destinations",
                web::post().to(destinations::create_destination),
            )
            .route(
                "/destinations/active",
                web::put().to(destinations::activate_destination),
            )
            .route(
                "/destinations/{id}/mappings",
                web::get().to(destinations::mappings),
            )
            .route(
                "/destinations/{id}/mappings",
                web::post().to(destinations::create_mapping),
            )
            .route(
                "/destinations/{id}/entries",
                web::get().to(destinations::directory_entries),
            )
            .route("/downloaders", web::get().to(downloaders::downloaders))
            .route(
                "/downloaders",
                web::post().to(downloaders::create_downloader),
            )
            .route(
                "/downloaders/active",
                web::put().to(downloaders::activate_downloader),
            )
            .route(
                "/downloaders/{id}/claim-legacy-jobs",
                web::post().to(downloaders::claim_legacy_jobs),
            )
            .route("/preview", web::post().to(subscriptions::preview))
            .route("/subscriptions", web::get().to(subscriptions::list))
            .route("/subscriptions", web::post().to(subscriptions::create))
            .route("/subscriptions/{id}", web::get().to(subscriptions::get))
            .route("/subscriptions/{id}", web::put().to(subscriptions::update))
            .route(
                "/subscriptions/{id}",
                web::delete().to(subscriptions::delete),
            )
            .route(
                "/subscriptions/{id}/preview",
                web::get().to(subscriptions::preview_saved),
            )
            .route(
                "/subscriptions/{id}/scan",
                web::post().to(subscriptions::scan),
            )
            .route(
                "/subscriptions/{id}/releases",
                web::get().to(subscriptions::releases),
            )
            .route("/jobs", web::get().to(jobs::jobs))
            .route("/jobs/{id}/files", web::get().to(jobs::job_files))
            .route(
                "/jobs/{id}/files/check",
                web::post().to(jobs::check_job_files),
            )
            .route("/jobs/{id}/retry", web::post().to(jobs::retry))
            .route("/run", web::post().to(system::run)),
    );
}
