// SPDX-License-Identifier: GPL-2.0-only
//! Embedded management UI; no separate frontend server or runtime required.
use actix_web::{web, HttpResponse};

fn asset(content_type: &'static str, body: &'static str) -> HttpResponse {
    HttpResponse::Ok()
        .content_type(content_type)
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("X-Content-Type-Options", "nosniff"))
        .insert_header(("Referrer-Policy", "no-referrer"))
        .insert_header((
            "Content-Security-Policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
        ))
        .body(body)
}

pub(super) fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/", web::get().to(index))
        .route("/assets/app.js", web::get().to(script))
        .route("/assets/app.css", web::get().to(style));
}
async fn index() -> HttpResponse {
    asset(
        "text/html; charset=utf-8",
        include_str!("../../web/index.html"),
    )
}
async fn script() -> HttpResponse {
    asset(
        "text/javascript; charset=utf-8",
        include_str!("../../web/app.js"),
    )
}
async fn style() -> HttpResponse {
    asset("text/css; charset=utf-8", include_str!("../../web/app.css"))
}
