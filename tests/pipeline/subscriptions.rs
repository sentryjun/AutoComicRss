// SPDX-License-Identifier: GPL-2.0-only
use super::*;

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
