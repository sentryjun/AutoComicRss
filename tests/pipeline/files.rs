// SPDX-License-Identifier: GPL-2.0-only
use super::*;

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
