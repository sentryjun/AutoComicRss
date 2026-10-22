// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[actix_web::test]
async fn bangumi_metadata_covers_association_cache_and_failure_isolation() {
    let (base, remote, server) = mock().await;
    remote.lock().unwrap().images = true;
    let service = Arc::new(
        Service::new(
            Store::open("sqlite::memory:").await.unwrap(),
            &config(&base),
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
    let candidates: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/api/sources/bangumi/search?q=Example")
            .to_request(),
    )
    .await;
    assert_eq!(candidates[0]["id"], 123);
    assert_eq!(
        candidates[0]["cover_url"],
        "/api/sources/bangumi/subjects/123/cover"
    );
    assert_eq!(candidates[0]["rating"], 8.2);
    assert_eq!(candidates[0]["episode_count"], 12);
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/sources/bangumi/subjects/123/cover")
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get("Content-Type").unwrap(), "image/png");
    let mut sub = input(&base);
    sub.bangumi_id = Some(123);
    let saved = service.store.create(sub).await.unwrap();
    assert_eq!(
        service
            .store
            .subscription(&saved.id)
            .await
            .unwrap()
            .input
            .bangumi_id,
        Some(123)
    );
    assert!(service.bangumi.subject(999).await.is_err());
    assert_eq!(
        service.preview_subscription(&saved.id).await.unwrap()[0].episode,
        Some(1)
    );
    assert!(service.store.jobs().await.unwrap().is_empty());
    server.stop(false).await;
    assert_eq!(
        service.bangumi.subject(123).await.unwrap().title,
        "星空列车"
    );
    assert_eq!(service.bangumi.search("Example").await.unwrap().len(), 1);
}

#[actix_web::test]
async fn website_proxy_is_used_by_bangumi_and_never_by_llm() {
    let (base, _, server) = mock().await;
    let mut cfg = config(&base);
    cfg.website.proxy_url = Some(base.clone());
    cfg.bangumi_base_url = "http://bangumi.invalid".into();
    let service = Service::new(Store::open("sqlite::memory:").await.unwrap(), &cfg)
        .await
        .unwrap();
    let subject = service.bangumi.subject(123).await.unwrap();
    assert!(
        subject.cover_url.is_none(),
        "arbitrary cover origins are rejected"
    );
    assert!(service.bangumi.cover(123).await.is_err());
    cfg.website.proxy_url = Some("http://127.0.0.1:1".into());
    let service = Service::new(Store::open("sqlite::memory:").await.unwrap(), &cfg)
        .await
        .unwrap();
    let settings = service
        .store
        .save_scraping_settings(settings(&base, Mode::Llm, Some("fixture-secret")))
        .await
        .unwrap();
    assert_eq!(
        service
            .scraper
            .parse(&settings, "Example [03]", 1)
            .await
            .unwrap()
            .metadata
            .episode,
        Some(7)
    );
    server.stop(false).await;
}
