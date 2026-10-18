// SPDX-License-Identifier: GPL-2.0-only
use actix_web::{http::StatusCode, test, App};
use autocomic_rss::http;

#[actix_web::test]
async fn embedded_ui_is_served_without_masking_api_or_unknown_routes() {
    let app = test::init_service(App::new().configure(http::configure)).await;
    for (path, mime, marker) in [
        ("/", "text/html", "lang=\"zh-CN\""),
        ("/assets/app.js", "text/javascript", "subscriptionEditor"),
        ("/assets/app.css", "text/css", "@media"),
    ] {
        let response =
            test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(mime));
        assert_eq!(
            response.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert_eq!(response.headers().get("cache-control").unwrap(), "no-cache");
        let policy = response
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(policy.contains("frame-ancestors 'none'"));
        assert!(!policy.contains("unsafe-inline"));
        let body = test::read_body(response).await;
        assert!(std::str::from_utf8(&body).unwrap().contains(marker));
    }
    for path in ["/api/unknown", "/assets/unknown.js", "/unknown"] {
        let response =
            test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response =
        test::call_service(&app, test::TestRequest::get().uri("/health").to_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("application/json"));
}
