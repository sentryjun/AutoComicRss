// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[actix_web::test]
async fn exclusive_llm_pipeline_cache_switch_and_immutable_existing_jobs() {
    let (base, remote, server) = mock().await;
    let service = Service::new(
        Store::open("sqlite::memory:").await.unwrap(),
        &config(&base),
    )
    .await
    .unwrap();
    let regex = service.preview(input(&base)).await.unwrap();
    assert_eq!(regex[0].episode, Some(1));
    let metadata = regex[0].metadata.as_ref().unwrap();
    assert_eq!(metadata.metadata.title.as_deref(), Some("Example"));
    assert_eq!(metadata.metadata.group.as_deref(), Some("Group"));
    assert_eq!(remote.lock().unwrap().calls, 0);
    service
        .store
        .save_scraping_settings(settings(&base, Mode::Llm, Some("fixture-secret")))
        .await
        .unwrap();
    let sub = service.store.create(input(&base)).await.unwrap();
    assert_eq!(
        service.preview_subscription(&sub.id).await.unwrap()[0].episode,
        Some(7)
    );
    assert!(service.store.jobs().await.unwrap().is_empty());
    assert_eq!(service.scan_subscription(&sub.id).await.unwrap(), 1);
    assert_eq!(
        remote.lock().unwrap().calls,
        1,
        "cached results avoid a second paid call"
    );
    let job = service.store.jobs().await.unwrap().remove(0);
    assert_eq!(job.episode, 7);
    assert_eq!(
        service.store.releases(&sub.id).await.unwrap()[0]
            .metadata
            .as_ref()
            .unwrap()
            .source,
        Mode::Llm
    );
    service
        .store
        .save_scraping_settings(settings(&base, Mode::Regex, None))
        .await
        .unwrap();
    assert_eq!(
        service.scan_subscription(&sub.id).await.unwrap(),
        0,
        "same resource is not requeued after switching mode"
    );
    assert_eq!(service.store.jobs().await.unwrap()[0].episode, 7);
    assert_eq!(
        service.preview(input(&base)).await.unwrap()[0].episode,
        Some(1)
    );
    server.stop(false).await;
}

#[actix_web::test]
async fn llm_rejects_bad_structures_and_http_errors_without_regex_fallback() {
    let (base, remote, server) = mock().await;
    let service = Service::new(
        Store::open("sqlite::memory:").await.unwrap(),
        &config(&base),
    )
    .await
    .unwrap();
    let good = json!({"title":"Example","season":null,"episode":3,"group":null,"resolution":null,"kind":"single"});
    let mut cases = vec![
        json!({}),
        json!({"choices":[{"finish_reason":"length","message":{"content":good.to_string()}}]}),
        json!({"choices":[{"finish_reason":"stop","message":{"content":"```json\n{}\n```"}}]}),
    ];
    for (field, value) in [
        ("episode", json!(-1)),
        ("episode", json!(3.5)),
        ("season", json!(2)),
        ("kind", json!("batch")),
        ("kind", json!("special")),
        ("kind", json!("unknown")),
        ("extra", json!("instructions")),
    ] {
        let mut metadata = good.clone();
        metadata[field] = value;
        cases.push(completion(metadata));
    }
    for response in cases {
        remote.lock().unwrap().response = Some(response);
        service
            .store
            .save_scraping_settings(settings(&base, Mode::Llm, Some("fixture-secret")))
            .await
            .unwrap();
        let releases = service.preview(input(&base)).await.unwrap();
        assert_eq!(releases[0].decision, Decision::NeedsReview);
        assert_eq!(releases[0].episode, None);
    }
    for status in [401, 429, 500, 302] {
        remote.lock().unwrap().status = status;
        service
            .store
            .save_scraping_settings(settings(&base, Mode::Llm, Some("fixture-secret")))
            .await
            .unwrap();
        let releases = service.preview(input(&base)).await.unwrap();
        assert_eq!(releases[0].decision, Decision::NeedsReview);
        assert!(!serde_json::to_string(&releases)
            .unwrap()
            .contains("fixture-secret"));
        assert!(!releases[0].reason.contains("private-provider-body"));
    }
    assert!(service.store.jobs().await.unwrap().is_empty());
    server.stop(false).await;
}

#[actix_web::test]
async fn request_budget_cache_progress_and_filtered_titles_do_not_call_llm() {
    let (base, remote, server) = mock().await;
    remote.lock().unwrap().count = 22;
    let service = Service::new(
        Store::open("sqlite::memory:").await.unwrap(),
        &config(&base),
    )
    .await
    .unwrap();
    service
        .store
        .save_scraping_settings(settings(&base, Mode::Llm, Some("fixture-secret")))
        .await
        .unwrap();
    let mut sub = input(&base);
    sub.exclude = vec!["Example".into()];
    assert!(service
        .preview(sub)
        .await
        .unwrap()
        .iter()
        .all(|r| r.decision == Decision::Filtered));
    assert_eq!(remote.lock().unwrap().calls, 0);
    let first = service.preview(input(&base)).await.unwrap();
    assert_eq!(
        first
            .iter()
            .filter(|r| r.decision == Decision::Ready)
            .count(),
        20
    );
    assert_eq!(remote.lock().unwrap().calls, 20);
    assert!(service
        .preview(input(&base))
        .await
        .unwrap()
        .iter()
        .all(|r| r.decision == Decision::Ready));
    assert_eq!(remote.lock().unwrap().calls, 22);
    service
        .store
        .save_scraping_settings(settings(&base, Mode::Llm, Some("fixture-secret")))
        .await
        .unwrap();
    remote.lock().unwrap().status = 401;
    assert!(service
        .preview(input(&base))
        .await
        .unwrap()
        .iter()
        .all(|r| r.decision == Decision::NeedsReview));
    assert_eq!(
        remote.lock().unwrap().calls,
        23,
        "stop on the first provider failure"
    );
    server.stop(false).await;
}

#[actix_web::test]
async fn settings_are_persistent_redacted_and_endpoint_change_requires_new_key() {
    let (base, _, server) = mock().await;
    let directory = std::env::temp_dir().join(format!("metadata-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let url = format!("sqlite://{}", directory.join("db").display());
    let store = Store::open(&url).await.unwrap();
    store
        .save_scraping_settings(settings(&base, Mode::Llm, Some("fixture-secret")))
        .await
        .unwrap();
    store.pool.close().await;
    let store = Store::open(&url).await.unwrap();
    let service = Arc::new(Service::new(store, &config(&base)).await.unwrap());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(service.clone()))
            .configure(http::configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/scraping/settings")
            .to_request(),
    )
    .await;
    assert_eq!(response.headers().get("Cache-Control").unwrap(), "no-store");
    let view: Value = test::read_body_json(response).await;
    assert_eq!(view["api_key_configured"], true);
    assert!(!view.to_string().contains("fixture-secret"));
    assert!(service
        .store
        .save_scraping_settings(settings("https://different.invalid", Mode::Llm, None))
        .await
        .is_err());
    assert!(service
        .store
        .save_scraping_settings(settings(&base, Mode::Llm, Some("")))
        .await
        .is_err());
    assert_eq!(
        service.preview(input(&base)).await.unwrap()[0].episode,
        Some(7),
        "saved key survives restart and rejected changes"
    );
    service
        .store
        .save_scraping_settings(settings(&base, Mode::Regex, Some("")))
        .await
        .unwrap();
    assert_eq!(
        service.store.scraping_settings().await.unwrap().view()["api_key_configured"],
        false
    );
    service.store.pool.close().await;
    server.stop(false).await;
    std::fs::remove_dir_all(directory).unwrap();
}
