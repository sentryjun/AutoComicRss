// SPDX-License-Identifier: GPL-2.0-only
use actix_web::{dev::ServerHandle, web, App, HttpRequest, HttpResponse, HttpServer};
use autocomic_rss::{
    config::{Config, QbitConfig},
    domain::{Decision, SubscriptionInput},
    http,
    service::Service,
    sources::feeds,
    storage::Store,
};
use serde_json::{json, Value};
use std::{
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};

mod transmission_fixture;
use transmission_fixture::{configured_service, transmission_rpc};

const FEED: &str = include_str!("../fixtures/releases.xml");
const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TORRENT: &[u8] =
    b"d4:infod6:lengthi1e4:name1:a12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";

#[derive(Default)]
struct Remote {
    adds: usize,
    torrents: Vec<Value>,
    fail_list: bool,
    fail_after_add: bool,
    feed: Option<String>,
    files: Option<Vec<Value>>,
    save_paths: Vec<String>,
    rpc_modern: bool,
    rpc_requests: Vec<String>,
    rpc_conflicts: usize,
    rpc_always_conflict: bool,
    rpc_auth_failure: bool,
    rpc_duplicate_path: Option<String>,
    rpc_id: u64,
    rpc_rotate_on_add: bool,
    rpc_rotated: bool,
    rpc_fail_method: Option<String>,
}

async fn login() -> HttpResponse {
    HttpResponse::Ok()
        .insert_header(("Set-Cookie", "SID=test; Path=/"))
        .body("Ok.")
}

async fn rss(state: web::Data<Arc<Mutex<Remote>>>) -> HttpResponse {
    HttpResponse::Ok().content_type("application/rss+xml").body(
        state
            .lock()
            .unwrap()
            .feed
            .clone()
            .unwrap_or_else(|| FEED.into()),
    )
}

fn authenticated(req: &HttpRequest) -> bool {
    req.cookie("SID").is_some_and(|c| c.value() == "test")
}

async fn info(req: HttpRequest, state: web::Data<Arc<Mutex<Remote>>>) -> HttpResponse {
    if !authenticated(&req) {
        return HttpResponse::Forbidden().finish();
    }
    let remote = state.lock().unwrap();
    if remote.fail_list {
        return HttpResponse::ServiceUnavailable().finish();
    }
    HttpResponse::Ok().json(&remote.torrents)
}

async fn add(
    req: HttpRequest,
    body: web::Bytes,
    state: web::Data<Arc<Mutex<Remote>>>,
) -> HttpResponse {
    if !authenticated(&req) {
        return HttpResponse::Forbidden().finish();
    }
    let text = String::from_utf8_lossy(&body);
    let hash = if text.contains("name=\"torrents\"") {
        assert!(body.windows(TORRENT.len()).any(|w| w == TORRENT));
        autocomic_rss::downloaders::torrent::torrent_hash(TORRENT).unwrap()
    } else {
        assert!(text.contains("name=\"urls\""));
        regex::Regex::new("urn:btih:([a-f0-9]{40})")
            .unwrap()
            .captures(&text)
            .unwrap()[1]
            .to_owned()
    };
    assert!(text.contains("name=\"savepath\""));
    let save_path = text
        .split("name=\"savepath\"")
        .nth(1)
        .unwrap()
        .split("\r\n\r\n")
        .nth(1)
        .unwrap()
        .split("\r\n")
        .next()
        .unwrap()
        .to_owned();
    let mut remote = state.lock().unwrap();
    remote.adds += 1;
    remote.save_paths.push(save_path.clone());
    remote
        .torrents
        .push(json!({"hash": hash, "progress": 0.0, "state": "metaDL", "save_path": save_path}));
    if remote.fail_after_add {
        remote.fail_after_add = false;
        return HttpResponse::InternalServerError().finish();
    }
    HttpResponse::Ok().body("Ok.")
}

async fn mock() -> (String, Arc<Mutex<Remote>>, ServerHandle) {
    mock_protocol("qbittorrent").await
}

async fn mock_protocol(protocol: &str) -> (String, Arc<Mutex<Remote>>, ServerHandle) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let remote = Arc::new(Mutex::new(Remote::default()));
    remote.lock().unwrap().rpc_modern = protocol == "modern";
    let data = remote.clone();
    let server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(data.clone()))
            .route("/feed", web::get().to(rss))
            .route("/rpc/feed", web::get().to(rss))
            .route("/rpc", web::post().to(transmission_rpc))
            .route(
                "/release.torrent",
                web::get().to(|| async { HttpResponse::Ok().body(TORRENT) }),
            )
            .route("/api/v2/auth/login", web::post().to(login))
            .route("/api/v2/torrents/info", web::get().to(info))
            .route(
                "/api/v2/torrents/files",
                web::get().to(
                    |req: HttpRequest, state: web::Data<Arc<Mutex<Remote>>>| async move {
                        assert!(authenticated(&req));
                        HttpResponse::Ok().json(state.lock().unwrap().files.clone().unwrap_or_else(
                            || {
                                vec![
                                    json!({"name":"Series/ep03.mkv", "size":3, "priority":1}),
                                    json!({"name":"skipped.mkv", "size":10, "priority":0}),
                                ]
                            },
                        ))
                    },
                ),
            )
            .route("/api/v2/torrents/add", web::post().to(add))
    })
    .workers(1)
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    (
        if protocol == "qbittorrent" {
            url
        } else {
            format!("{url}/rpc")
        },
        remote,
        handle,
    )
}

fn input(url: &str, enabled: bool) -> SubscriptionInput {
    SubscriptionInput {
        bangumi_id: None,
        title: "Example".into(),
        rss_url: format!("{url}/feed"),
        season: 1,
        episode_offset: 0,
        include: vec!["1080p".into()],
        exclude: vec![],
        enabled,
    }
}

fn config(url: &str) -> Config {
    Config {
        database_url: "sqlite::memory:".into(),
        host: "127.0.0.1".parse().unwrap(),
        port: 8080,
        poll_interval: Duration::from_secs(300),
        bangumi_base_url: "https://api.bgm.tv".into(),
        mikan_base_url: "https://mikanani.me".into(),
        website: Default::default(),
        qbit: Some(QbitConfig {
            url: url.into(),
            username: "test".into(),
            password: "test".into(),
            save_path: "/downloads".into(),
        }),
    }
}

mod downloads;
mod files;
mod mysql;
mod ownership;
mod recovery;
mod subscriptions;
