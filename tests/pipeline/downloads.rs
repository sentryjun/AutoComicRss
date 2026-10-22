// SPDX-License-Identifier: GPL-2.0-only
use super::*;

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
