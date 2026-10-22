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

#[path = "support/transmission.rs"]
mod transmission_fixture;
use transmission_fixture::{configured_service, transmission_rpc};

const FEED: &str = include_str!("fixtures/releases.xml");
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

#[actix_web::test]
async fn parses_rss_enclosures_and_filter_reasons() {
    let releases = feeds::parse(
        FEED.as_bytes(),
        &"https://example.invalid/".parse().unwrap(),
        &input("https://example.invalid", false),
    )
    .unwrap();
    assert_eq!(releases[0].episode, Some(3));
    assert_eq!(releases[0].decision, Decision::Ready);
    assert_eq!(releases[2].decision, Decision::NeedsReview);
    assert_eq!(releases[3].decision, Decision::Filtered);
    assert!(releases[0]
        .download_url
        .as_ref()
        .unwrap()
        .starts_with("magnet:"));
}

#[actix_web::test]
async fn restart_and_repeated_scans_do_not_resubmit() {
    for protocol in ["qbittorrent", "legacy", "modern"] {
        restart_and_repeated_scans_do_not_resubmit_contract(protocol).await;
    }
}

async fn restart_and_repeated_scans_do_not_resubmit_contract(protocol: &str) {
    let (url, remote, server) = mock_protocol(protocol).await;
    let path = std::env::temp_dir().join(format!("rust-project-{}.db", uuid::Uuid::new_v4()));
    let db = format!("sqlite://{}", path.display());
    let service = configured_service(Store::open(&db).await.unwrap(), &config(&url))
        .await
        .unwrap();
    let sub = service.store.create(input(&url, false)).await.unwrap();
    let preview = service.preview_subscription(&sub.id).await.unwrap();
    assert_eq!(preview[1].decision, Decision::AlreadyQueued);
    assert!(service.store.jobs().await.unwrap().is_empty());
    assert!(service.scan_subscription(&sub.id).await.is_err());
    service
        .store
        .update(&sub.id, input(&url, true))
        .await
        .unwrap();
    assert_eq!(service.cycle().await.unwrap().queued, 1);
    assert_eq!(service.cycle().await.unwrap().queued, 0);
    assert_eq!(remote.lock().unwrap().adds, 1);
    let jobs = service.store.jobs().await.unwrap();
    assert_eq!(jobs[0].state, "submitted");
    service.store.pool.close().await;
    drop(service);

    remote.lock().unwrap().rpc_id = 812;
    let recovered = configured_service(Store::open(&db).await.unwrap(), &config(&url))
        .await
        .unwrap();
    recovered.cycle().await.unwrap();
    assert_eq!(remote.lock().unwrap().adds, 1);
    // Removing a task remotely does not silently recreate it.
    remote.lock().unwrap().torrents.clear();
    recovered.cycle().await.unwrap();
    assert_eq!(recovered.store.jobs().await.unwrap()[0].state, "missing");
    assert_eq!(remote.lock().unwrap().adds, 1);
    recovered.store.retry(&jobs[0].id).await.unwrap();
    recovered.cycle().await.unwrap();
    assert_eq!(remote.lock().unwrap().adds, 2);
    remote.lock().unwrap().torrents[0]["progress"] = json!(1.0);
    recovered.cycle().await.unwrap();
    assert_eq!(recovered.store.jobs().await.unwrap()[0].state, "completed");
    recovered.store.pool.close().await;
    drop(recovered);
    std::fs::remove_file(path).unwrap();
    server.stop(true).await;
}

#[actix_web::test]
async fn reconciles_uncertain_submission_and_respects_disabled_subscriptions() {
    for protocol in ["qbittorrent", "legacy", "modern"] {
        reconciles_uncertain_submission_and_respects_disabled_subscriptions_contract(protocol)
            .await;
    }
}

async fn reconciles_uncertain_submission_and_respects_disabled_subscriptions_contract(
    protocol: &str,
) {
    let (url, remote, server) = mock_protocol(protocol).await;
    let service = configured_service(Store::open("sqlite::memory:").await.unwrap(), &config(&url))
        .await
        .unwrap();
    let sub = service.store.create(input(&url, true)).await.unwrap();
    remote.lock().unwrap().fail_list = true;
    assert!(!service.cycle().await.unwrap().errors.is_empty());
    assert_eq!(remote.lock().unwrap().adds, 0);
    remote.lock().unwrap().fail_list = false;
    service
        .store
        .update(&sub.id, input(&url, false))
        .await
        .unwrap();
    service.cycle().await.unwrap();
    assert_eq!(remote.lock().unwrap().adds, 0);
    service
        .store
        .update(&sub.id, input(&url, true))
        .await
        .unwrap();
    remote.lock().unwrap().fail_after_add = true;
    service.cycle().await.unwrap();
    assert_eq!(remote.lock().unwrap().adds, 1);
    let job = &service.store.jobs().await.unwrap()[0];
    assert_eq!(job.state, "submitting");
    assert!(job.last_error.is_some());
    sqlx::query("UPDATE download_jobs SET next_attempt_at = 0")
        .execute(service.store.pool.sqlite().unwrap())
        .await
        .unwrap();
    service.cycle().await.unwrap();
    assert_eq!(remote.lock().unwrap().adds, 1);
    assert_eq!(service.store.jobs().await.unwrap()[0].state, "submitted");
    service.store.delete(&sub.id).await.unwrap();
    assert!(service.store.subscriptions().await.unwrap().is_empty());
    assert_eq!(service.store.jobs().await.unwrap().len(), 1);
    server.stop(true).await;
}

#[actix_web::test]
async fn api_validates_input_and_persists_crud() {
    use actix_web::{http::StatusCode, test};
    let mut settings = config("http://127.0.0.1:1");
    settings.qbit = None;
    let service = Arc::new(
        configured_service(Store::open("sqlite::memory:").await.unwrap(), &settings)
            .await
            .unwrap(),
    );
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(service.clone()))
            .configure(http::configure),
    )
    .await;
    let mut invalid = input("http://example.invalid", false);
    invalid.rss_url = "file:///etc/passwd".into();
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/subscriptions")
            .set_json(invalid)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/subscriptions")
            .set_json(input("https://example.invalid", false))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = test::read_body_json(response).await;
    let uri = format!("/api/subscriptions/{}", body["id"].as_str().unwrap());
    let mut changed = input("https://example.invalid", false);
    changed.title = "Updated".into();
    let response = test::call_service(
        &app,
        test::TestRequest::put()
            .uri(&uri)
            .set_json(changed)
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = test::call_service(&app, test::TestRequest::get().uri(&uri).to_request()).await;
    let body: Value = test::read_body_json(response).await;
    assert_eq!(body["title"], "Updated");
    let response =
        test::call_service(&app, test::TestRequest::delete().uri(&uri).to_request()).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = test::call_service(&app, test::TestRequest::get().uri(&uri).to_request()).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn downloads_relative_torrent_enclosure_and_submits_original_bytes() {
    for protocol in ["qbittorrent", "legacy", "modern"] {
        downloads_relative_torrent_enclosure_and_submits_original_bytes_contract(protocol).await;
    }
}

async fn downloads_relative_torrent_enclosure_and_submits_original_bytes_contract(protocol: &str) {
    let (url, remote, server) = mock_protocol(protocol).await;
    remote.lock().unwrap().feed = Some(
        r#"<rss version="2.0"><channel>
    <title>Test</title><link>https://example.invalid</link><description>Test</description>
    <item><guid>torrent</guid><title>Example - 05 [1080p]</title>
    <enclosure url="/release.torrent" type="application/x-bittorrent" length="1"/>
    </item></channel></rss>"#
            .into(),
    );
    let service = configured_service(Store::open("sqlite::memory:").await.unwrap(), &config(&url))
        .await
        .unwrap();
    service.store.create(input(&url, true)).await.unwrap();
    let report = service.cycle().await.unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let jobs = service.store.jobs().await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, "submitted", "{:?}", jobs[0]);
    assert_eq!(
        jobs[0].info_hash.as_deref(),
        Some(
            autocomic_rss::downloaders::torrent::torrent_hash(TORRENT)
                .unwrap()
                .as_str()
        )
    );
    assert_eq!(
        service
            .store
            .torrent_bytes(&jobs[0].id)
            .await
            .unwrap()
            .unwrap(),
        TORRENT
    );
    service.cycle().await.unwrap();
    assert_eq!(remote.lock().unwrap().adds, 1);
    server.stop(true).await;
}

#[actix_web::test]
async fn switching_keeps_queued_jobs_and_retries_with_their_original_downloader_after_restart() {
    for (protocol_a, protocol_b) in [
        ("qbittorrent", "qbittorrent"),
        ("qbittorrent", "legacy"),
        ("modern", "qbittorrent"),
    ] {
        switching_contract(protocol_a, protocol_b).await;
    }
}

async fn switching_contract(protocol_a: &str, protocol_b: &str) {
    use actix_web::test;
    let (url_a, remote_a, server_a) = mock_protocol(protocol_a).await;
    let (url_b, remote_b, server_b) = mock_protocol(protocol_b).await;
    let path = std::env::temp_dir().join(format!("downloader-switch-{}.db", uuid::Uuid::new_v4()));
    let db = format!("sqlite://{}", path.display());
    let settings = config(&url_a);
    let service = Arc::new(
        configured_service(Store::open(&db).await.unwrap(), &settings)
            .await
            .unwrap(),
    );
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(service.clone()))
            .configure(http::configure),
    )
    .await;
    let instances: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/api/downloaders")
            .to_request(),
    )
    .await;
    let id_a = instances[0]["id"].as_str().unwrap().to_owned();
    let response = test::call_service(&app, test::TestRequest::post().uri("/api/downloaders")
    .set_json(json!({"name":"Downloader B", "kind":if protocol_b == "qbittorrent" { "qbittorrent" } else { "transmission" }, "connection":{
        "url":url_b, "username":"test", "password":"B-private-password", "save_path":"/downloads"
    }})).to_request()).await;
    assert_eq!(response.status(), 201);
    let body = String::from_utf8(test::read_body(response).await.to_vec()).unwrap();
    assert!(!body.contains("B-private-password"));
    let instance: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(instance["password_configured"], true);
    assert_eq!(instance["is_active"], false);
    let id_b = instance["id"].as_str().unwrap().to_owned();
    let sub_a: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/api/subscriptions")
            .set_json(input(&url_a, true))
            .to_request(),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/subscriptions/{}/scan",
                sub_a["id"].as_str().unwrap()
            ))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(remote_a.lock().unwrap().adds, 0); // Still queued when switching.
    let response = test::call_service(
        &app,
        test::TestRequest::put()
            .uri("/api/downloaders/active")
            .set_json(json!({"downloader_id":id_b}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 204);
    let sub_b: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/api/subscriptions")
            .set_json(input(&url_a, true))
            .to_request(),
    )
    .await;
    let run: Value =
        test::call_and_read_body_json(&app, test::TestRequest::post().uri("/api/run").to_request())
            .await;
    assert_eq!(run["errors"], json!([]));
    assert_eq!(remote_a.lock().unwrap().adds, 1);
    assert_eq!(remote_b.lock().unwrap().adds, 1); // Same hash in different instances must stay separate.
    let jobs: Vec<Value> =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri("/api/jobs").to_request())
            .await;
    let job_a = jobs
        .iter()
        .find(|j| j["subscription_id"] == sub_a["id"])
        .unwrap()
        .clone();
    assert_eq!(job_a["downloader_id"], id_a);
    assert_eq!(
        jobs.iter()
            .find(|j| j["subscription_id"] == sub_b["id"])
            .unwrap()["downloader_id"],
        id_b
    );

    remote_a.lock().unwrap().fail_list = true;
    remote_b.lock().unwrap().torrents[0]["progress"] = json!(0.5);
    let run: Value =
        test::call_and_read_body_json(&app, test::TestRequest::post().uri("/api/run").to_request())
            .await;
    assert!(run["errors"][0].as_str().unwrap().contains(&id_a));
    let jobs: Vec<Value> =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri("/api/jobs").to_request())
            .await;
    assert_eq!(
        jobs.iter().find(|j| j["id"] == job_a["id"]).unwrap()["state"],
        "submitted"
    );
    assert_eq!(
        jobs.iter()
            .find(|j| j["subscription_id"] == sub_b["id"])
            .unwrap()["state"],
        "downloading"
    );

    remote_a.lock().unwrap().fail_list = false;
    remote_a.lock().unwrap().torrents.clear();
    test::call_service(&app, test::TestRequest::post().uri("/api/run").to_request()).await;
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/jobs/{}/retry",
                job_a["id"].as_str().unwrap()
            ))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 204);
    test::call_service(&app, test::TestRequest::post().uri("/api/run").to_request()).await;
    assert_eq!(remote_a.lock().unwrap().adds, 2);
    assert_eq!(remote_b.lock().unwrap().adds, 1);
    service.store.pool.close().await;
    drop(app);
    drop(service);

    // Startup's original QBIT_URL must not overwrite the persisted global selection.
    let recovered = Arc::new(
        configured_service(Store::open(&db).await.unwrap(), &settings)
            .await
            .unwrap(),
    );
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(recovered.clone()))
            .configure(http::configure),
    )
    .await;
    let status: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get().uri("/api/status").to_request(),
    )
    .await;
    assert_eq!(status["active_downloader_id"], id_b);
    remote_a.lock().unwrap().torrents[0]["progress"] = json!(1.0);
    remote_b.lock().unwrap().torrents[0]["progress"] = json!(1.0);
    test::call_service(&app, test::TestRequest::post().uri("/api/run").to_request()).await;
    let jobs: Vec<Value> =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri("/api/jobs").to_request())
            .await;
    assert!(jobs.iter().all(|j| j["state"] == "completed"));
    assert_eq!(remote_a.lock().unwrap().adds, 2);
    assert_eq!(remote_b.lock().unwrap().adds, 1);
    recovered.store.pool.close().await;
    drop(app);
    drop(recovered);
    std::fs::remove_file(path).unwrap();
    server_a.stop(false).await;
    server_b.stop(false).await;
}

#[actix_web::test]
async fn upgrading_without_original_config_requires_explicit_legacy_claim() {
    use actix_web::test;
    use sqlx::sqlite::SqliteConnectOptions;
    let (url, remote, server) = mock().await;
    let directory =
        std::env::temp_dir().join(format!("legacy-downloader-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let migration_dir = directory.join("migrations");
    std::fs::create_dir(&migration_dir).unwrap();
    std::fs::write(
        migration_dir.join("0001_initial.sql"),
        include_str!("../migrations/0001_initial.sql"),
    )
    .unwrap();
    let db_path = directory.join("old.db");
    // Build an actual v1 database with its migration checksum, then upgrade through Store::open.
    let old = sqlx::SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::migrate::Migrator::new(migration_dir.as_path())
        .await
        .unwrap()
        .run(&old)
        .await
        .unwrap();
    sqlx::query("INSERT INTO subscriptions(id, spec) VALUES ('legacy-sub', ?)")
        .bind(serde_json::to_string(&input(&url, true)).unwrap())
        .execute(&old)
        .await
        .unwrap();
    sqlx::query("INSERT INTO download_jobs(id, subscription_id, resource_key, title, season, episode, download_url, info_hash, state) VALUES ('legacy-job', 'legacy-sub', 'legacy-resource', 'Example - 03', 1, 3, ?, ?, 'submitted')")
        .bind(format!("magnet:?xt=urn:btih:{HASH}")).bind(HASH).execute(&old).await.unwrap();
    old.close().await;
    let mut settings = config(&url);
    settings.qbit = None;
    let service = Arc::new(
        configured_service(
            Store::open(&format!("sqlite://{}", db_path.display()))
                .await
                .unwrap(),
            &settings,
        )
        .await
        .unwrap(),
    );
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(service.clone()))
            .configure(http::configure),
    )
    .await;
    let status: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get().uri("/api/status").to_request(),
    )
    .await;
    assert_eq!(status["legacy_unassigned_jobs"], 1);
    let sub: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/api/subscriptions")
            .set_json(input(&url, true))
            .to_request(),
    )
    .await;
    test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!(
                "/api/subscriptions/{}/scan",
                sub["id"].as_str().unwrap()
            ))
            .to_request(),
    )
    .await;
    let instance: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/api/downloaders")
            .set_json(
                json!({"name":"Restored original", "kind":"qbittorrent", "connection":{
                    "url":url, "username":"test", "password":"test", "save_path":"/downloads"
                }}),
            )
            .to_request(),
    )
    .await;
    let id = instance["id"].as_str().unwrap();
    let response = test::call_service(
        &app,
        test::TestRequest::put()
            .uri("/api/downloaders/active")
            .set_json(json!({"downloader_id":id}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 204);
    let jobs: Vec<Value> =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri("/api/jobs").to_request())
            .await;
    assert!(jobs.iter().find(|j| j["id"] == "legacy-job").unwrap()["downloader_id"].is_null());
    assert_eq!(
        jobs.iter()
            .find(|j| j["subscription_id"] == sub["id"])
            .unwrap()["downloader_id"],
        id
    );
    // Restore the original remote task: claiming must reconcile it, not add it again.
    remote
        .lock()
        .unwrap()
        .torrents
        .push(json!({"hash": HASH, "progress": 1.0, "state":"uploading"}));
    let claimed: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/downloaders/{id}/claim-legacy-jobs"))
            .to_request(),
    )
    .await;
    assert_eq!(claimed["claimed"], 1);
    test::call_service(&app, test::TestRequest::post().uri("/api/run").to_request()).await;
    let jobs: Vec<Value> =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri("/api/jobs").to_request())
            .await;
    let legacy = jobs.iter().find(|j| j["id"] == "legacy-job").unwrap();
    assert_eq!(legacy["downloader_id"], id);
    assert_eq!(legacy["state"], "completed");
    assert_eq!(legacy["legacy_unassigned"], false);
    assert_eq!(remote.lock().unwrap().adds, 0);
    let response = test::call_service(
        &app,
        test::TestRequest::put()
            .uri("/api/downloaders/active")
            .set_json(json!({"downloader_id":"does-not-exist"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 404);
    let status: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get().uri("/api/status").to_request(),
    )
    .await;
    assert_eq!(status["active_downloader_id"], id);
    assert_eq!(status["legacy_unassigned_jobs"], 0);
    service.store.pool.close().await;
    drop(app);
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
    server.stop(false).await;
}

#[actix_web::test]
async fn destination_and_mapping_changes_preserve_queued_paths_and_restart_snapshots() {
    for protocol in ["qbittorrent", "legacy", "modern"] {
        destination_and_mapping_changes_preserve_queued_paths_and_restart_snapshots_contract(
            protocol,
        )
        .await;
    }
}

async fn destination_and_mapping_changes_preserve_queued_paths_and_restart_snapshots_contract(
    protocol: &str,
) {
    use autocomic_rss::media::{DestinationInput, DestinationKind, MappingInput};
    let (url, remote, server) = mock_protocol(protocol).await;
    let path =
        std::env::temp_dir().join(format!("destination-snapshots-{}.db", uuid::Uuid::new_v4()));
    let db = format!("sqlite://{}", path.display());
    let settings = config(&url);
    let service = configured_service(Store::open(&db).await.unwrap(), &settings)
        .await
        .unwrap();
    let owner = service.store.active_downloader_id().await.unwrap().unwrap();
    let a = service
        .store
        .create_destination(DestinationInput {
            name: "A".into(),
            kind: DestinationKind::Local,
            smb: None,
            root_path: "/media/a".into(),
        })
        .await
        .unwrap();
    let b = service
        .store
        .create_destination(DestinationInput {
            name: "B".into(),
            kind: DestinationKind::Local,
            smb: None,
            root_path: "/media/b".into(),
        })
        .await
        .unwrap();
    assert!(service.store.activate_destination(&a.id).await.is_err());
    let v1 = service
        .store
        .create_mapping(
            &a.id,
            MappingInput {
                downloader_id: owner.clone(),
                download_root: "/downloads/one".into(),
            },
        )
        .await
        .unwrap();
    service.store.activate_destination(&a.id).await.unwrap();
    let first = service.store.create(input(&url, true)).await.unwrap();
    service.scan_subscription(&first.id).await.unwrap();
    let v2 = service
        .store
        .create_mapping(
            &a.id,
            MappingInput {
                downloader_id: owner.clone(),
                download_root: "/downloads/revised".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(v2.version, 2);
    remote.lock().unwrap().feed = Some(FEED.replace(HASH, &"b".repeat(40)));
    let second = service.store.create(input(&url, true)).await.unwrap();
    service.scan_subscription(&second.id).await.unwrap();
    service
        .store
        .create_mapping(
            &b.id,
            MappingInput {
                downloader_id: owner.clone(),
                download_root: "/downloads/two".into(),
            },
        )
        .await
        .unwrap();
    service.store.activate_destination(&b.id).await.unwrap();
    remote.lock().unwrap().feed = Some(FEED.replace(HASH, &"c".repeat(40)));
    let third = service.store.create(input(&url, true)).await.unwrap();
    service.scan_subscription(&third.id).await.unwrap();
    let report = service.cycle().await.unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(
        remote.lock().unwrap().save_paths,
        vec!["/downloads/one", "/downloads/revised", "/downloads/two"]
    );
    let jobs = service.store.jobs().await.unwrap();
    let original = jobs
        .iter()
        .find(|job| job.subscription_id == first.id)
        .unwrap();
    assert_eq!(original.destination_id.as_deref(), Some(a.id.as_str()));
    assert_eq!(original.mapping_id.as_deref(), Some(v1.id.as_str()));
    let revised = jobs
        .iter()
        .find(|job| job.subscription_id == second.id)
        .unwrap();
    assert_eq!(revised.mapping_id.as_deref(), Some(v2.id.as_str()));
    service.store.pool.close().await;
    drop(service);
    let recovered = configured_service(Store::open(&db).await.unwrap(), &settings)
        .await
        .unwrap();
    assert_eq!(
        recovered
            .store
            .active_destination_id()
            .await
            .unwrap()
            .as_deref(),
        Some(b.id.as_str())
    );
    let original = recovered.store.job(&original.id).await.unwrap();
    assert_eq!(
        original.download_save_path.as_deref(),
        Some("/downloads/one")
    );
    assert_eq!(original.destination_id.as_deref(), Some(a.id.as_str()));
    recovered.cycle().await.unwrap();
    assert_eq!(remote.lock().unwrap().adds, 3);
    // Identical hash in the same instance but another destination cannot be reused or moved.
    remote.lock().unwrap().feed = Some(FEED.into());
    let collision = recovered.store.create(input(&url, true)).await.unwrap();
    recovered.cycle().await.unwrap();
    let collision = recovered
        .store
        .jobs()
        .await
        .unwrap()
        .into_iter()
        .find(|j| j.subscription_id == collision.id)
        .unwrap();
    assert_eq!(collision.state, "failed");
    assert!(collision.last_error.unwrap().contains("save path"));
    assert_eq!(remote.lock().unwrap().adds, 3);
    let other = recovered
        .store
        .create_downloader(autocomic_rss::domain::DownloaderInput {
            name: "unmapped".into(),
            kind: autocomic_rss::domain::DownloaderKind::Qbittorrent,
            connection: config("http://127.0.0.1:1").qbit.unwrap(),
        })
        .await
        .unwrap();
    assert!(recovered
        .store
        .activate_downloader(&other.id)
        .await
        .is_err());
    assert_eq!(
        recovered
            .store
            .active_downloader_id()
            .await
            .unwrap()
            .as_deref(),
        Some(owner.as_str())
    );
    recovered.store.pool.close().await;
    drop(recovered);
    std::fs::remove_file(path).unwrap();
    server.stop(false).await;
}

#[actix_web::test]
async fn file_checks_keep_download_completion_and_last_success_when_storage_is_unavailable() {
    for protocol in ["qbittorrent", "legacy", "modern"] {
        file_checks_keep_download_completion_and_last_success_when_storage_is_unavailable_contract(
            protocol,
        )
        .await;
    }
}

async fn file_checks_keep_download_completion_and_last_success_when_storage_is_unavailable_contract(
    protocol: &str,
) {
    use actix_web::test;
    use autocomic_rss::media::{DestinationInput, DestinationKind, MappingInput};
    let (url, remote, server) = mock_protocol(protocol).await;
    let directory = std::env::temp_dir().join(format!("local-media-{}", uuid::Uuid::new_v4()));
    let root = directory.join("media");
    std::fs::create_dir_all(root.join("Series")).unwrap();
    std::fs::write(root.join("Series/ep03.mkv"), b"abc").unwrap();
    let db = format!("sqlite://{}", directory.join("app.db").display());
    let settings = config(&url);
    let service = Arc::new(
        configured_service(Store::open(&db).await.unwrap(), &settings)
            .await
            .unwrap(),
    );
    let owner = service.store.active_downloader_id().await.unwrap().unwrap();
    let destination = service
        .store
        .create_destination(DestinationInput {
            name: "local".into(),
            kind: DestinationKind::Local,
            smb: None,
            root_path: root.to_str().unwrap().into(),
        })
        .await
        .unwrap();
    service
        .store
        .create_mapping(
            &destination.id,
            MappingInput {
                downloader_id: owner,
                download_root: "/downloads/media".into(),
            },
        )
        .await
        .unwrap();
    service
        .store
        .activate_destination(&destination.id)
        .await
        .unwrap();
    service.store.create(input(&url, true)).await.unwrap();
    service.cycle().await.unwrap();
    remote.lock().unwrap().torrents[0]["progress"] = json!(1.0);
    service.cycle().await.unwrap();
    let job = service.store.jobs().await.unwrap().remove(0);
    assert_eq!(job.state, "completed");
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(service.clone()))
            .configure(http::configure),
    )
    .await;
    let report_url = format!("/api/jobs/{}/files", job.id);
    let check_url = format!("{report_url}/check");
    let initial: Value =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri(&report_url).to_request())
            .await;
    assert_eq!(initial["status"], "unchecked");
    let present: Value =
        test::call_and_read_body_json(&app, test::TestRequest::post().uri(&check_url).to_request())
            .await;
    assert_eq!(present["status"], "present");
    assert_eq!(present["files"].as_array().unwrap().len(), 1); // Skip priority-zero files.
    assert_eq!(present["files"][0]["observed_size"], 3);
    std::fs::remove_file(root.join("Series/ep03.mkv")).unwrap();
    let absent: Value =
        test::call_and_read_body_json(&app, test::TestRequest::post().uri(&check_url).to_request())
            .await;
    assert_eq!(absent["status"], "absent");
    std::fs::write(root.join("Series/ep03.mkv"), b"x").unwrap();
    let partial: Value =
        test::call_and_read_body_json(&app, test::TestRequest::post().uri(&check_url).to_request())
            .await;
    assert_eq!(partial["status"], "present"); // Presence is not integrity or download completion.
    assert_eq!(partial["files"][0]["expected_size"], 3);
    assert_eq!(partial["files"][0]["observed_size"], 1);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    std::fs::rename(&root, directory.join("offline")).unwrap();
    let unavailable: Value =
        test::call_and_read_body_json(&app, test::TestRequest::post().uri(&check_url).to_request())
            .await;
    assert_eq!(unavailable["status"], "unavailable");
    assert_eq!(unavailable["last_success_at"], partial["last_success_at"]);
    assert!(
        unavailable["checked_at"].as_i64().unwrap() > partial["last_success_at"].as_i64().unwrap()
    );
    assert_eq!(service.store.job(&job.id).await.unwrap().state, "completed");
    assert_eq!(remote.lock().unwrap().adds, 1);
    service.store.pool.close().await;
    drop(app);
    drop(service);
    server.stop(false).await;
    let recovered = configured_service(Store::open(&db).await.unwrap(), &settings)
        .await
        .unwrap();
    assert_eq!(
        recovered.store.file_report(&job.id).await.unwrap().status,
        "unavailable"
    );
    std::fs::rename(directory.join("offline"), &root).unwrap();
    let report = recovered.check_job_files(&job.id).await.unwrap();
    assert_eq!(report.status, "present"); // Saved paths remain checkable with downloader offline.
    assert_eq!(
        recovered.store.job(&job.id).await.unwrap().state,
        "completed"
    );
    recovered.store.pool.close().await;
    drop(recovered);
    std::fs::remove_dir_all(directory).unwrap();
}

#[actix_web::test]
async fn unsafe_downloader_paths_and_unreadable_files_are_unavailable_not_missing() {
    for protocol in ["qbittorrent", "legacy", "modern"] {
        unsafe_downloader_paths_and_unreadable_files_are_unavailable_not_missing_contract(protocol)
            .await;
    }
}

async fn unsafe_downloader_paths_and_unreadable_files_are_unavailable_not_missing_contract(
    protocol: &str,
) {
    use autocomic_rss::media::{DestinationInput, DestinationKind, MappingInput};
    let (url, remote, server) = mock_protocol(protocol).await;
    let directory =
        std::env::temp_dir().join(format!("media-confinement-{}", uuid::Uuid::new_v4()));
    let root = directory.join("root");
    std::fs::create_dir_all(root.join("restricted")).unwrap();
    std::fs::create_dir_all(directory.join("outside")).unwrap();
    std::fs::write(directory.join("outside/secret"), b"secret").unwrap();
    std::fs::write(root.join("restricted/file.mkv"), b"abc").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(directory.join("outside"), root.join("escape")).unwrap();
    let service = configured_service(Store::open("sqlite::memory:").await.unwrap(), &config(&url))
        .await
        .unwrap();
    let owner = service.store.active_downloader_id().await.unwrap().unwrap();
    let destination = service
        .store
        .create_destination(DestinationInput {
            name: "local".into(),
            kind: DestinationKind::Local,
            smb: None,
            root_path: root.to_str().unwrap().into(),
        })
        .await
        .unwrap();
    service
        .store
        .create_mapping(
            &destination.id,
            MappingInput {
                downloader_id: owner,
                download_root: "/downloads/media".into(),
            },
        )
        .await
        .unwrap();
    service
        .store
        .activate_destination(&destination.id)
        .await
        .unwrap();
    // Each subscription is a distinct task pointing to the same already-complete test torrent.
    remote.lock().unwrap().torrents.push(
        json!({"hash":HASH, "state":"uploading", "progress":1.0, "save_path":"/downloads/media"}),
    );
    for path in [
        "../outside/secret",
        "/absolute",
        "C:\\outside",
        "folder/../outside",
        "",
    ] {
        remote.lock().unwrap().files = Some(vec![json!({"name":path,"size":3,"priority":1})]);
        let sub = service.store.create(input(&url, true)).await.unwrap();
        service.cycle().await.unwrap();
        let job = service
            .store
            .jobs()
            .await
            .unwrap()
            .into_iter()
            .find(|j| j.subscription_id == sub.id)
            .unwrap();
        let report = service.check_job_files(&job.id).await.unwrap();
        assert_eq!(report.status, "unavailable", "{path}");
        assert!(report.files.is_empty()); // No partial or unsafe manifest is stored.
        assert_eq!(service.store.job(&job.id).await.unwrap().state, "completed");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in ["escape/secret", "restricted/file.mkv", "restricted"] {
            remote.lock().unwrap().files = Some(vec![json!({"name":path,"size":3,"priority":1})]);
            let sub = service.store.create(input(&url, true)).await.unwrap();
            service.cycle().await.unwrap();
            let job = service
                .store
                .jobs()
                .await
                .unwrap()
                .into_iter()
                .find(|j| j.subscription_id == sub.id)
                .unwrap();
            let restricted = root.join("restricted");
            let permissions = std::fs::metadata(&restricted).unwrap().permissions();
            if path == "restricted/file.mkv" {
                std::fs::set_permissions(&restricted, std::fs::Permissions::from_mode(0o000))
                    .unwrap();
            }
            let permission_enforced = std::fs::read_dir(&restricted).is_err();
            let report = service.check_job_files(&job.id).await.unwrap();
            std::fs::set_permissions(&restricted, permissions).unwrap();
            // Root-owned CI processes may bypass chmod; other cases always enforce confinement.
            if path != "restricted/file.mkv" || permission_enforced {
                assert_eq!(report.status, "unavailable", "{path}");
                assert_eq!(report.files[0].status, "unavailable");
            }
            assert_eq!(service.store.job(&job.id).await.unwrap().state, "completed");
        }
    }
    assert_eq!(remote.lock().unwrap().adds, 0);
    service.store.pool.close().await;
    drop(service);
    std::fs::remove_dir_all(directory).unwrap();
    server.stop(false).await;
}

#[actix_web::test]
async fn transmission_negotiation_duplicate_paths_and_error_states() {
    for protocol in ["legacy", "modern"] {
        let (url, remote, server) = mock_protocol(protocol).await;
        let service =
            configured_service(Store::open("sqlite::memory:").await.unwrap(), &config(&url))
                .await
                .unwrap();
        service.store.create(input(&url, true)).await.unwrap();
        remote.lock().unwrap().rpc_always_conflict = true;
        let report = service.cycle().await.unwrap();
        assert!(report.errors[0].contains("retry limit"));
        assert_eq!(remote.lock().unwrap().rpc_conflicts, 3);
        assert_eq!(remote.lock().unwrap().adds, 0);
        remote.lock().unwrap().rpc_always_conflict = false;
        remote.lock().unwrap().rpc_auth_failure = true;
        let report = service.cycle().await.unwrap();
        assert!(report.errors[0].contains("401"));
        assert!(!report.errors[0].contains("private-server-detail"));
        assert!(remote.lock().unwrap().rpc_requests.is_empty());
        remote.lock().unwrap().rpc_auth_failure = false;
        // A daemon-side RPC error never triggers protocol fallback or submission.
        remote.lock().unwrap().rpc_fail_method = Some(
            if protocol == "modern" {
                "session_get"
            } else {
                "session-get"
            }
            .into(),
        );
        let report = service.cycle().await.unwrap();
        assert!(report.errors[0].contains("rejected"));
        assert!(!report.errors[0].contains("private-server-detail"));
        assert_eq!(remote.lock().unwrap().adds, 0);
        remote.lock().unwrap().rpc_fail_method = None;
        remote.lock().unwrap().rpc_rotate_on_add = true;
        remote.lock().unwrap().rpc_duplicate_path = Some("/elsewhere".into());
        service.cycle().await.unwrap();
        let job = service.store.jobs().await.unwrap().remove(0);
        assert_eq!(job.state, "failed");
        assert!(job.last_error.unwrap().contains("save path"));
        assert_eq!(remote.lock().unwrap().adds, 1);
        assert!(remote.lock().unwrap().rpc_rotated);
        service.cycle().await.unwrap();
        assert_eq!(remote.lock().unwrap().adds, 1);
        // Explicit retry may adopt a concurrent duplicate only when its path agrees.
        remote.lock().unwrap().torrents.clear();
        remote.lock().unwrap().rpc_duplicate_path = Some("/downloads".into());
        service.store.retry(&job.id).await.unwrap();
        service.cycle().await.unwrap();
        assert_eq!(service.store.job(&job.id).await.unwrap().state, "submitted");
        assert_eq!(remote.lock().unwrap().adds, 2);
        remote.lock().unwrap().torrents[0]["progress"] = json!(1.0);
        remote.lock().unwrap().torrents[0]["metadata"] = json!(0.0);
        service.cycle().await.unwrap();
        assert_ne!(service.store.job(&job.id).await.unwrap().state, "completed");
        remote.lock().unwrap().torrents[0]["error"] = json!(3);
        service.cycle().await.unwrap();
        assert_eq!(service.store.job(&job.id).await.unwrap().state, "failed");
        service.store.retry(&job.id).await.unwrap();
        remote.lock().unwrap().torrents[0]["error"] = json!(2); // Tracker error doesn't invalidate local completion.
        remote.lock().unwrap().torrents[0]["metadata"] = json!(1.0);
        service.cycle().await.unwrap();
        assert_eq!(service.store.job(&job.id).await.unwrap().state, "completed");
        assert_eq!(remote.lock().unwrap().adds, 2);
        let calls = remote.lock().unwrap().rpc_requests.clone();
        assert!(!calls
            .iter()
            .any(|m| m.contains("remove") || m.contains("location")));
        server.stop(false).await;
    }
}

#[actix_web::test]
async fn transmission_control_is_independent_of_website_proxy_and_legacy_claims_are_rejected() {
    for protocol in ["legacy", "modern"] {
        let (url, remote, server) = mock_protocol(protocol).await;
        let mut settings = config(&url);
        settings.website.proxy_url = Some("http://127.0.0.1:1".into());
        let service = configured_service(Store::open("sqlite::memory:").await.unwrap(), &settings)
            .await
            .unwrap();
        let sub = service.store.create(input(&url, true)).await.unwrap();
        let releases = feeds::parse(FEED.as_bytes(), &url.parse().unwrap(), &sub.input).unwrap();
        service.store.record_scan(&sub, &releases).await.unwrap();
        // Source requests fail via the unreachable website proxy; magnet submission goes directly to RPC.
        let report = service.cycle().await.unwrap();
        assert!(!report.errors.is_empty());
        assert_eq!(remote.lock().unwrap().adds, 1);
        let owner = service.store.active_downloader_id().await.unwrap().unwrap();
        assert!(service.store.claim_legacy_jobs(&owner).await.is_err());
        server.stop(false).await;
    }
}

#[actix_web::test]
#[ignore = "requires a dedicated MySQL 8.4 server; see tests/README.md"]
async fn mysql_download_pipeline_switch_restart_and_files() {
    use autocomic_rss::media::{DestinationInput, DestinationKind, MappingInput};
    let base = std::env::var("AUTOCOMIC_TEST_MYSQL_URL").expect("dedicated test server required");
    let admin = sqlx::MySqlPool::connect(&base).await.unwrap();
    let name = format!("autocomic_test_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut db = reqwest::Url::parse(&base).unwrap();
    db.set_path(&format!("/{name}"));
    let (url_a, remote_a, server_a) = mock().await;
    let (url_b, remote_b, server_b) = mock_protocol("modern").await;
    let settings = config(&url_a);
    let service = Service::new(Store::open(db.as_str()).await.unwrap(), &settings)
        .await
        .unwrap();
    let a = service.store.active_downloader_id().await.unwrap().unwrap();
    let directory = std::env::temp_dir().join(format!("mysql-files-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(directory.join("Series")).unwrap();
    std::fs::write(directory.join("Series/ep03.mkv"), b"abc").unwrap();
    let destination = service
        .store
        .create_destination(DestinationInput {
            name: "media".into(),
            kind: DestinationKind::Local,
            root_path: directory.to_str().unwrap().into(),
            smb: None,
        })
        .await
        .unwrap();
    service
        .store
        .create_mapping(
            &destination.id,
            MappingInput {
                downloader_id: a.clone(),
                download_root: "/downloads".into(),
            },
        )
        .await
        .unwrap();
    service
        .store
        .activate_destination(&destination.id)
        .await
        .unwrap();
    let sub = service.store.create(input(&url_a, true)).await.unwrap();
    service.scan_subscription(&sub.id).await.unwrap();
    let b=service.store.create_downloader(serde_json::from_value(json!({"name":"B","kind":"transmission","connection":{"url":url_b,"username":"test","password":"test","save_path":"/downloads"}})).unwrap()).await.unwrap();
    service
        .store
        .create_mapping(
            &destination.id,
            MappingInput {
                downloader_id: b.id.clone(),
                download_root: "/downloads".into(),
            },
        )
        .await
        .unwrap();
    service.store.activate_downloader(&b.id).await.unwrap();
    service.store.create(input(&url_a, true)).await.unwrap();
    assert!(service.cycle().await.unwrap().errors.is_empty());
    assert_eq!(remote_a.lock().unwrap().adds, 1);
    assert_eq!(remote_b.lock().unwrap().adds, 1);
    service.store.pool.close().await;
    drop(service);
    remote_a.lock().unwrap().torrents[0]["progress"] = json!(1.0);
    remote_b.lock().unwrap().torrents[0]["progress"] = json!(1.0);
    let recovered = Service::new(Store::open(db.as_str()).await.unwrap(), &settings)
        .await
        .unwrap();
    assert_eq!(
        recovered
            .store
            .active_downloader_id()
            .await
            .unwrap()
            .as_deref(),
        Some(b.id.as_str())
    );
    assert!(recovered.cycle().await.unwrap().errors.is_empty());
    for job in recovered.store.jobs().await.unwrap() {
        assert_eq!(job.state, "completed");
        assert_eq!(
            recovered.check_job_files(&job.id).await.unwrap().status,
            "present"
        );
    }
    assert_eq!(remote_a.lock().unwrap().adds, 1);
    assert_eq!(remote_b.lock().unwrap().adds, 1);
    recovered.store.pool.close().await;
    server_a.stop(false).await;
    server_b.stop(false).await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    std::fs::remove_dir_all(directory).unwrap();
}
