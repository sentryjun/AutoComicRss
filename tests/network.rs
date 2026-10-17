// SPDX-License-Identifier: GPL-2.0-only
use actix_web::{dev::ServerHandle, test, web, App, HttpRequest, HttpResponse, HttpServer};
use rust_project::{
    api,
    config::{Config, QbitConfig},
    service::Service,
    storage::Store,
};
use serde_json::{json, Value};
use std::{net::TcpListener, sync::Arc, time::Duration};

async fn server(proxy: bool) -> (String, ServerHandle) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = HttpServer::new(move || App::new().default_service(web::to(move |req: HttpRequest| async move {
        if proxy {
            assert_eq!(req.headers().get("Proxy-Authorization").and_then(|h| h.to_str().ok()), Some("Basic dXNlcjpwYXNz"), "request {}", req.uri());
            if req.uri().host() == Some("127.0.0.1") { return HttpResponse::BadGateway().finish(); }
            match req.path() {
                "/to-direct" => {
                    let query = web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string()).unwrap();
                    HttpResponse::Found().insert_header(("Location", query.get("target").unwrap().as_str())).finish()
                },
                "/Home/Search" => HttpResponse::Found().insert_header(("Location", "http://assets.invalid/results")).finish(),
                "/results" => {
                    assert_eq!(req.uri().host(), Some("assets.invalid"));
                    HttpResponse::Ok().content_type("text/html").body(r#"<form action="/Home/Search"><input name="searchstr"></form><ul class="an-ul"><a href="/Home/Bangumi/101"><div class="an-text">经代理搜索</div></a></ul>"#)
                },
                "/feed" => HttpResponse::Ok().body(include_str!("fixtures/releases.xml")),
                _ => HttpResponse::BadGateway().finish(),
            }
        } else {
            assert!(req.headers().get("Proxy-Authorization").is_none());
            match req.path() {
                "/api/v2/auth/login" => HttpResponse::Ok().insert_header(("Set-Cookie", "SID=test; Path=/")).body("Ok."),
                "/api/v2/torrents/info" => HttpResponse::Ok().json(json!([])),
                "/feed" => HttpResponse::Ok().body(include_str!("fixtures/releases.xml")),
                _ => HttpResponse::NotFound().finish(),
            }
        }
    }))).workers(1).listen(listener).unwrap().run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    (base, handle)
}

#[actix_web::test]
async fn direct_exceptions_are_explicit_and_proxy_failure_never_falls_back() {
    let (proxy, proxy_server) = server(true).await;
    let (direct, direct_server) = server(false).await;
    let proxy = proxy.replacen("http://", "http://user:pass@", 1);
    for (direct_hosts, status) in [("127.0.0.1", 200), ("", 502)] {
        let config = Config {
            database_url: "sqlite::memory:".into(),
            port: 8080,
            poll_interval: Duration::from_secs(300),
            qbit: None,
            mikan_base_url: "http://mikan.invalid".into(),
            website: rust_project::config::WebsiteConfig {
                proxy_url: Some(proxy.clone()),
                direct_hosts: direct_hosts.into(),
            },
        };
        let service = Service::new(Store::open("sqlite::memory:").await.unwrap(), &config)
            .await
            .unwrap();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(Arc::new(service)))
                .configure(api::configure),
        )
        .await;
        let mut redirect = reqwest::Url::parse("http://unresolvable.invalid/to-direct").unwrap();
        redirect
            .query_pairs_mut()
            .append_pair("target", &format!("{direct}/feed"));
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/preview")
                .set_json(json!({"title":"Test", "rss_url": redirect.as_str()}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), status);
        if status == 200 {
            let preview: Value = test::read_body_json(response).await;
            assert_eq!(preview[0]["episode"], 3);
        } else {
            let body = String::from_utf8(test::read_body(response).await.to_vec()).unwrap();
            assert!(!body.contains("user:pass"));
        }
    }
    proxy_server.stop(false).await;
    direct_server.stop(false).await;
}

#[actix_web::test]
async fn website_proxy_handles_auth_redirects_and_rss_while_downloader_stays_direct() {
    let (proxy, proxy_server) = server(true).await;
    let (direct, direct_server) = server(false).await;
    let mut proxy_url = reqwest::Url::parse(&proxy).unwrap();
    proxy_url.set_username("user").unwrap();
    proxy_url.set_password(Some("pass")).unwrap();
    let config = Config {
        database_url: "sqlite::memory:".into(),
        port: 8080,
        poll_interval: Duration::from_secs(300),
        mikan_base_url: "http://mikan.invalid".into(),
        website: rust_project::config::WebsiteConfig {
            proxy_url: Some(proxy_url.into()),
            direct_hosts: String::new(),
        },
        qbit: Some(QbitConfig {
            url: direct.clone(),
            username: "test".into(),
            password: "test".into(),
            save_path: "/downloads".into(),
        }),
    };
    let service = Service::new(Store::open("sqlite::memory:").await.unwrap(), &config)
        .await
        .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(Arc::new(service)))
            .configure(api::configure),
    )
    .await;
    let search: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/api/sources/mikan/search?q=test")
            .to_request(),
    )
    .await;
    assert_eq!(search["series"][0]["title"], "经代理搜索");
    let preview: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/api/preview")
            .set_json(json!({"title":"Test", "rss_url":"http://unresolvable.invalid/feed"}))
            .to_request(),
    )
    .await;
    assert_eq!(preview[0]["episode"], 3);
    let cycle: Value =
        test::call_and_read_body_json(&app, test::TestRequest::post().uri("/api/run").to_request())
            .await;
    assert_eq!(cycle["errors"], json!([]));
    assert_eq!(cycle["downloader_configured"], true);
    proxy_server.stop(false).await;
    direct_server.stop(false).await;
}
