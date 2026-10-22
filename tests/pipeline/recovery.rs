// SPDX-License-Identifier: GPL-2.0-only
use super::*;

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
