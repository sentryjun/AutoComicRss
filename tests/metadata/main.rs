// SPDX-License-Identifier: GPL-2.0-only
use actix_web::{dev::ServerHandle, test, web, App, HttpRequest, HttpResponse, HttpServer};
use autocomic_rss::{
    config::Config,
    domain::{Decision, SubscriptionInput},
    http,
    scraping::{Mode, SettingsInput},
    service::Service,
    storage::Store,
};
use serde_json::{json, Value};
use std::{
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Default)]
struct Remote {
    calls: usize,
    response: Option<Value>,
    status: u16,
    count: usize,
    images: bool,
}
async fn mock() -> (String, Arc<Mutex<Remote>>, ServerHandle) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let origin = base.clone();
    let remote = Arc::new(Mutex::new(Remote::default()));
    let state = remote.clone();
    let server=HttpServer::new(move || { let origin=origin.clone(); App::new().app_data(web::Data::new(state.clone())).default_service(web::to(move |req:HttpRequest,body:web::Bytes,state:web::Data<Arc<Mutex<Remote>>>| {
        let origin=origin.clone(); async move {
            let mut remote=state.lock().unwrap();
            if req.path()=="/v1/chat/completions" {
                remote.calls+=1;
                assert_eq!(req.headers().get("Authorization").unwrap(),"Bearer fixture-secret");
                let request:Value=serde_json::from_slice(&body).unwrap();
                assert_eq!(request["response_format"]["type"],"json_object");
                assert_eq!(request["model"],"fixture-model");
                assert!(request["messages"][1]["content"].as_str().unwrap().contains("resource_title"));
                if remote.status==302 { return HttpResponse::Found().insert_header(("Location",format!("{origin}/leak"))).finish(); }
                if remote.status!=0 { return HttpResponse::build(actix_web::http::StatusCode::from_u16(remote.status).unwrap()).body("fixture-secret private-provider-body"); }
                return HttpResponse::Ok().json(remote.response.clone().unwrap_or_else(||completion(json!({"title":"星空列车","season":null,"episode":7,"group":"Group","resolution":"1080p","kind":"single"}))));
            }
            assert!(req.headers().get("Authorization").is_none(),"credentials must never reach websites");
            if req.path()=="/feed" {
                let count=if remote.count==0 {1} else {remote.count};
                let items=(0..count).map(|i| format!(r#"<item><guid>{i}</guid><title>[Group] Example - {:02} [1080p]</title><enclosure url="magnet:?xt=urn:btih:{:040x}" type="application/x-bittorrent" length="1"/></item>"#,i+1,i+1)).collect::<String>();
                return HttpResponse::Ok().body(format!("<rss version=\"2.0\"><channel><title>fixture</title><link>{origin}</link><description>test</description>{items}</channel></rss>"));
            }
            assert!(req.headers().get("User-Agent").unwrap().to_str().unwrap().contains("sentryjun/AutoComicRss"));
            let subject=json!({"id":123,"type":2,"name":"Hoshizora","name_cn":"星空列车","summary":"<script>untrusted</script>","date":"2026-01-01","total_episodes":12,"rating":{"score":8.2},"images":{"common":if remote.images {format!("{origin}/image")}else{"https://evil.invalid/secret".into()}}});
            match req.path() {
                "/v0/search/subjects" => { let body:Value=serde_json::from_slice(&body).unwrap(); assert_eq!(body["filter"]["type"],json!([2])); HttpResponse::Ok().json(json!({"data":[subject]})) },
                "/v0/subjects/123" => HttpResponse::Ok().json(subject),
                "/image" => HttpResponse::Ok().content_type("text/html").body(b"\x89PNG\r\n\x1a\nfixture".as_slice()),
                _ => HttpResponse::ServiceUnavailable().finish(),
            }
        }
    })) }).workers(1).listen(listener).unwrap().run();
    let handle = server.handle();
    actix_web::rt::spawn(server);
    (base, remote, handle)
}
fn completion(metadata: Value) -> Value {
    json!({"choices":[{"finish_reason":"stop","message":{"content":metadata.to_string()}}]})
}
fn config(base: &str) -> Config {
    Config {
        database_url: "sqlite::memory:".into(),
        host: "127.0.0.1".parse().unwrap(),
        port: 8080,
        poll_interval: Duration::from_secs(300),
        qbit: None,
        mikan_base_url: base.into(),
        bangumi_base_url: base.into(),
        website: Default::default(),
    }
}
fn input(base: &str) -> SubscriptionInput {
    serde_json::from_value(
        json!({"title":"Example","rss_url":format!("{base}/feed"),"enabled":true}),
    )
    .unwrap()
}
fn settings(base: &str, mode: Mode, key: Option<&str>) -> SettingsInput {
    SettingsInput {
        mode,
        base_url: format!("{base}/v1"),
        model: "fixture-model".into(),
        api_key: key.map(str::to_owned),
    }
}

mod bangumi;
mod scraping;
