// SPDX-License-Identifier: GPL-2.0-only
use super::*;

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
