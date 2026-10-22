// SPDX-License-Identifier: GPL-2.0-only
use actix_web::{test, web, App};
use autocomic_rss::{config::Config, http, service::Service, storage::Store};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

#[actix_web::test]
async fn directory_api_rejects_traversal_and_does_not_follow_links_outside_root() {
    let directory = std::env::temp_dir().join(format!("media-browse-{}", uuid::Uuid::new_v4()));
    let root = directory.join("root");
    std::fs::create_dir_all(root.join("Series")).unwrap();
    std::fs::create_dir(directory.join("outside")).unwrap();
    std::fs::write(root.join("Series/ep01.mkv"), b"abc").unwrap();
    std::fs::write(directory.join("outside/secret"), b"outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(directory.join("outside"), root.join("escape")).unwrap();
    let config = Config {
        database_url: "sqlite::memory:".into(),
        host: "127.0.0.1".parse().unwrap(),
        port: 8080,
        poll_interval: Duration::from_secs(300),
        qbit: None,
        mikan_base_url: "https://mikanani.me".into(),
        website: Default::default(),
    };
    let service = Arc::new(
        Service::new(Store::open("sqlite::memory:").await.unwrap(), &config)
            .await
            .unwrap(),
    );
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(service))
            .configure(http::configure),
    )
    .await;
    for body in [
        json!({"name":"bad", "kind":"local", "root_path":"relative"}),
        json!({"name":"bad", "kind":"smb", "root_path":"/share"}),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/destinations")
                .set_json(body)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);
    }
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/destinations")
            .set_json(json!({"name":"Local", "kind":"local", "root_path":root.to_str().unwrap()}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 201);
    let destination: Value = test::read_body_json(response).await;
    let id = destination["id"].as_str().unwrap();
    let response = test::call_service(
        &app,
        test::TestRequest::put()
            .uri("/api/destinations/active")
            .set_json(json!({"destination_id":id}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 400); // No downloader mapping: never silently use a fallback.
    let downloader: Value = test::call_and_read_body_json(&app, test::TestRequest::post().uri("/api/downloaders")
        .set_json(json!({"name":"Local qBit", "kind":"qbittorrent", "connection":{
            "url":"http://127.0.0.1:1", "username":"", "password":"", "save_path":"/downloads"
        }})).to_request()).await;
    let response = test::call_service(
        &app,
        test::TestRequest::put()
            .uri("/api/downloaders/active")
            .set_json(json!({"downloader_id":downloader["id"]}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 204);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri(&format!("/api/destinations/{id}/mappings"))
            .set_json(json!({"downloader_id":downloader["id"],"download_root":"/downloads/local"}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 201);
    let mapping: Value = test::read_body_json(response).await;
    assert_eq!(mapping["version"], 1);
    let response = test::call_service(
        &app,
        test::TestRequest::put()
            .uri("/api/destinations/active")
            .set_json(json!({"destination_id":id}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 204);
    let status: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get().uri("/api/status").to_request(),
    )
    .await;
    assert_eq!(status["active_destination_id"], id);
    let base = format!("/api/destinations/{id}/entries");
    let listing: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri(&format!("{base}?path=Series"))
            .to_request(),
    )
    .await;
    assert_eq!(listing["status"], "available");
    assert_eq!(listing["entries"][0]["name"], "ep01.mkv");
    assert_eq!(listing["entries"][0]["size"], 3);
    for path in [
        "../outside",
        "%2Fetc",
        "C%3A%2FWindows",
        "Series%5C..%5Coutside",
        "Series/../outside",
        "Series//bad",
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("{base}?path={path}"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400, "{path}");
    }
    #[cfg(unix)]
    {
        let listing: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::get()
                .uri(&format!("{base}?path=escape"))
                .to_request(),
        )
        .await;
        assert_eq!(listing["status"], "unavailable");
        assert_eq!(listing["entries"], json!([]));
    }
    std::fs::rename(&root, directory.join("offline")).unwrap();
    let listing: Value =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri(&base).to_request()).await;
    assert_eq!(listing["status"], "unavailable");
    std::fs::remove_dir_all(directory).unwrap();
}
