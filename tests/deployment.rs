// SPDX-License-Identifier: GPL-2.0-only
use actix_web::{test, web, App};
use autocomic_rss::{config::Config, http, service::Service, storage::Store};
use std::{process::Command, sync::Arc, time::Duration};

#[actix_web::test]
async fn readiness_checks_database_without_waiting_for_the_scan_gate() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let config = Config {
        database_url: "sqlite::memory:".into(),
        host: "127.0.0.1".parse().unwrap(),
        port: 8080,
        poll_interval: Duration::from_secs(300),
        qbit: None,
        bangumi_base_url: "https://api.bgm.tv".into(),
        mikan_base_url: "https://example.invalid".into(),
        website: Default::default(),
    };
    let service = Arc::new(Service::new(store.clone(), &config).await.unwrap());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(service.clone()))
            .configure(http::configure),
    )
    .await;
    let _busy = service.gate.lock().await;
    let response =
        test::call_service(&app, test::TestRequest::get().uri("/ready").to_request()).await;
    assert_eq!(response.status(), 200);
    store.pool.close().await;
    let response =
        test::call_service(&app, test::TestRequest::get().uri("/ready").to_request()).await;
    assert_eq!(response.status(), 503);
    assert_eq!(
        test::read_body(response).await.as_ref(),
        br#"{"status":"unavailable"}"#
    );
    let response =
        test::call_service(&app, test::TestRequest::get().uri("/health").to_request()).await;
    assert_eq!(response.status(), 200);
}

#[actix_web::test]
async fn deployment_host_must_be_an_ip_address() {
    let output = Command::new(env!("CARGO_BIN_EXE_autocomic-rss"))
        .env("APP_HOST", "https://not-an-ip.invalid")
        .env("DATABASE_URL", "sqlite::memory:")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("APP_HOST must be an IPv4 or IPv6 address"));
}
