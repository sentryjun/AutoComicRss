// SPDX-License-Identifier: GPL-2.0-only
use actix_web::{dev::ServerHandle, test, web, App, HttpRequest, HttpResponse, HttpServer};
use autocomic_rss::{config::Config, http, service::Service, storage::Store};
use serde_json::Value;
use std::{net::TcpListener, sync::Arc, time::Duration};

const SEARCH: &str = r#"<form action="/Home/Search"><input name="searchstr"></form>
<div class="central-container"><ul class="list-inline an-ul">
<li><a href="/Home/Bangumi/101"><div class="an-text" title="测试番剧">测试番剧</div></a></li>
<li><a href="/Home/Bangumi/202"><div class="an-text" title="测试番剧 第二季">测试番剧 第二季</div></a></li>
</ul></div>"#;

// Independently authored fixtures; publish-group ID deliberately differs from RSS group ID.
const DETAIL: &str = r#"<p class="bangumi-title">测试番剧
<a class="mikan-rss" href="/RSS/Bangumi?bangumiId=101"></a></p>
<p class="bangumi-info">放送日期：星期五</p>
<p class="bangumi-info">Bangumi番组计划链接：<a href="https://bgm.tv/subject/400602">详情</a></p>
<div class="subgroup-text" id="12"><a href="/Home/PublishGroup/99">字幕组 &amp; A</a>
<a class="mikan-rss" href="/RSS/Bangumi?bangumiId=101&amp;subgroupid=12"></a></div>"#;

async fn mock() -> (String, ServerHandle) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = HttpServer::new(|| {
        App::new().default_service(web::to(|req: HttpRequest| async move {
            if req.path() == "/Home/Bangumi/202" {
                return HttpResponse::Ok().content_type("text/html").body(format!(
                    "{}<div class=\"bangumi-poster\" style=\"background-image: url('/images/Bangumi/cover.png');\"></div>",
                    DETAIL.replace("101", "202")));
            }
            if req.path() == "/images/Bangumi/cover.png" {
                return HttpResponse::Ok().content_type("image/png").body(b"\x89PNG\r\n\x1a\nfixture".as_slice());
            }
            if req.path() == "/Home/Bangumi/101" {
                return HttpResponse::Ok().content_type("text/html").body(DETAIL);
            }
            if req.path() == "/RSS/Bangumi" {
                assert_eq!(req.query_string(), "bangumiId=101&subgroupid=12");
                return HttpResponse::Ok()
                    .content_type("application/rss+xml")
                    .body(include_str!("fixtures/releases.xml"));
            }
            assert_eq!(req.path(), "/Home/Search");
            let query = web::Query::<std::collections::HashMap<String, String>>::from_query(
                req.query_string(),
            )
            .unwrap();
            match query.get("searchstr").map(String::as_str) {
                Some("empty") => return HttpResponse::Ok().content_type("text/html")
                    .body(r#"<form action="/Home/Search"><input name="searchstr"></form><div class="central-container"><ul class="an-ul"></ul></div>"#),
                Some("challenge") => return HttpResponse::Ok().content_type("text/html").body("<h1>Verify your browser</h1>"),
                Some("changed") => return HttpResponse::Ok().content_type("text/html")
                    .body(r#"<form action="/Home/Search"><input name="searchstr"></form><ul class="an-ul"><a href="/Home/Bangumi/101"><span class="new-title">番剧</span></a></ul>"#),
                _ => {}
            }
            assert_eq!(query.get("searchstr").unwrap(), "测试 & S2");
            HttpResponse::Ok()
                .content_type("text/html; charset=utf-8")
                .body(SEARCH)
        }))
    })
    .workers(1)
    .listen(listener)
    .unwrap()
    .run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    (url, handle)
}

#[actix_web::test]
async fn covers_use_an_application_url_and_details_remain_available_in_bounded_cache() {
    let (base, server) = mock().await;
    let service = Service::new(
        Store::open("sqlite::memory:").await.unwrap(),
        &config(&base),
    )
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(Arc::new(service)))
            .configure(http::configure),
    )
    .await;
    let detail: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/api/sources/mikan/series/202")
            .to_request(),
    )
    .await;
    assert_eq!(detail["cover_url"], "/api/sources/mikan/series/202/cover");
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri(detail["cover_url"].as_str().unwrap())
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers().get("Content-Type").unwrap(), "image/png");
    assert_eq!(
        test::read_body(response).await.as_ref(),
        b"\x89PNG\r\n\x1a\nfixture"
    );
    server.stop(false).await;
    let cached: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/api/sources/mikan/series/202")
            .to_request(),
    )
    .await;
    assert_eq!(cached, detail);
}

#[actix_web::test]
async fn empty_results_are_distinct_from_challenges_or_changed_markup() {
    let (base, server) = mock().await;
    let service = Service::new(
        Store::open("sqlite::memory:").await.unwrap(),
        &config(&base),
    )
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(Arc::new(service)))
            .configure(http::configure),
    )
    .await;
    for (query, status) in [
        ("empty", 200),
        ("challenge", 502),
        ("changed", 502),
        ("%20", 400),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/api/sources/mikan/search?q={query}"))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), status, "query {query}");
        if status == 200 {
            let value: Value = test::read_body_json(response).await;
            assert_eq!(value["series"], serde_json::json!([]));
        }
    }
    server.stop(false).await;
}

#[actix_web::test]
async fn group_candidate_can_be_previewed_and_saved_disabled_without_downloading() {
    let (base, server) = mock().await;
    let service = Service::new(
        Store::open("sqlite::memory:").await.unwrap(),
        &config(&base),
    )
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(Arc::new(service)))
            .configure(http::configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/sources/mikan/series/101")
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let detail: Value = test::read_body_json(response).await;
    assert_eq!(detail["title"], "测试番剧");
    assert_eq!(detail["bangumi_id"], "400602");
    assert_eq!(detail["groups"][0]["group_id"], "12");
    assert_eq!(detail["groups"][0]["name"], "字幕组 & A");
    assert!(detail["cover_url"].is_null());
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/sources/mikan/resolve")
            .set_json(serde_json::json!({"url": detail["groups"][0]["rss_url"]}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let candidate: Value = test::read_body_json(response).await;
    assert_eq!(candidate["scope"], "group");
    assert_eq!(candidate["series_id"], "101");
    assert_eq!(candidate["group_id"], "12");
    assert_eq!(candidate["subscription"]["enabled"], false);
    let preview: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::post()
            .uri("/api/preview")
            .set_json(&candidate["subscription"])
            .to_request(),
    )
    .await;
    assert_eq!(preview[0]["episode"], 3);
    let saved = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/subscriptions")
            .set_json(&candidate["subscription"])
            .to_request(),
    )
    .await;
    assert_eq!(saved.status(), 201);
    let jobs: Value =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri("/api/jobs").to_request())
            .await;
    assert_eq!(jobs, serde_json::json!([]));
    server.stop(true).await;
}

fn config(base: &str) -> Config {
    Config {
        database_url: "sqlite::memory:".into(),
        host: "127.0.0.1".parse().unwrap(),
        port: 8080,
        poll_interval: Duration::from_secs(300),
        qbit: None,
        mikan_base_url: base.into(),
        website: Default::default(),
    }
}

#[actix_web::test]
async fn resolver_marks_keyword_scope_and_rejects_foreign_or_ambiguous_urls() {
    let (base, server) = mock().await;
    let service = Service::new(
        Store::open("sqlite::memory:").await.unwrap(),
        &config(&base),
    )
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(Arc::new(service)))
            .configure(http::configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/sources/mikan/resolve")
            .set_json(serde_json::json!({"url": format!("{base}/RSS/Search?searchstr=test")}))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let candidate: Value = test::read_body_json(response).await;
    assert_eq!(candidate["scope"], "keyword");
    assert!(candidate["series_id"].is_null());
    assert!(candidate["subscription"].is_null());
    assert!(!candidate["warnings"].as_array().unwrap().is_empty());
    for url in [
        "https://other.invalid/Home/Bangumi/101".into(),
        format!("{base}/RSS/Bangumi?bangumiId=101&bangumiId=202"),
        format!("{base}/RSS/Bangumi?bangumiId=101&subgroupid=999"),
        format!("{base}/Home/Bangumi/not-an-id"),
    ] {
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/sources/mikan/resolve")
                .set_json(serde_json::json!({"url": url}))
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400);
    }
    server.stop(false).await;
}

#[actix_web::test]
async fn search_discovers_distinct_series_and_encoded_keyword_rss_without_creating_jobs() {
    let (base, server) = mock().await;
    let service = Service::new(
        Store::open("sqlite::memory:").await.unwrap(),
        &config(&base),
    )
    .await
    .unwrap();
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(Arc::new(service)))
            .configure(http::configure),
    )
    .await;
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/api/sources/mikan/search?q=%E6%B5%8B%E8%AF%95%20%26%20S2")
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body: Value = test::read_body_json(response).await;
    assert_eq!(body["series"][0]["series_id"], "101");
    assert_eq!(body["series"][1]["title"], "测试番剧 第二季");
    let rss = reqwest::Url::parse(body["keyword_rss_url"].as_str().unwrap()).unwrap();
    assert_eq!(rss.path(), "/RSS/Search");
    assert_eq!(
        rss.query_pairs().collect::<Vec<_>>(),
        vec![("searchstr".into(), "测试 & S2".into())]
    );
    let jobs: Value =
        test::call_and_read_body_json(&app, test::TestRequest::get().uri("/api/jobs").to_request())
            .await;
    assert_eq!(jobs, serde_json::json!([]));
    server.stop(true).await;
}
