// SPDX-License-Identifier: GPL-2.0-only
use super::*;

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
        include_str!("../../migrations/sqlite/0001_initial.sql"),
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
