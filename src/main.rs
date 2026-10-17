// SPDX-License-Identifier: GPL-2.0-only
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use rust_project::{
    api,
    config::Config,
    service::{worker, Service},
    storage::Store,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
struct Greeting {
    message: String,
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Health {
    status: String,
    version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CalcRequest {
    a: f64,
    b: f64,
    operation: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CalcResponse {
    result: f64,
    operation: String,
}

async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Hello World! 欢迎使用 Rust Web 服务器")
}

async fn health_check() -> impl Responder {
    let health = Health {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    HttpResponse::Ok().json(health)
}

async fn greet(name: web::Path<String>) -> impl Responder {
    let greeting = Greeting {
        message: format!("你好，{}！", name),
        name: name.into_inner(),
    };
    HttpResponse::Ok().json(greeting)
}

async fn calculate(web::Json(req): web::Json<CalcRequest>) -> impl Responder {
    let result = match req.operation.as_str() {
        "add" => req.a + req.b,
        "subtract" => req.a - req.b,
        "multiply" => req.a * req.b,
        "divide" => {
            if req.b == 0.0 {
                return HttpResponse::BadRequest().json("除数不能为 0");
            }
            req.a / req.b
        }
        _ => return HttpResponse::BadRequest().json("不支持的操作"),
    };

    let response = CalcResponse {
        result,
        operation: req.operation,
    };
    HttpResponse::Ok().json(response)
}

async fn echo(data: web::Json<serde_json::Value>) -> impl Responder {
    HttpResponse::Ok().json(data)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let config = Config::from_env().map_err(std::io::Error::other)?;
    std::fs::create_dir_all("data")?;
    let store = Store::open(&config.database_url)
        .await
        .map_err(std::io::Error::other)?;
    let service = Arc::new(
        Service::new(store, &config)
            .await
            .map_err(std::io::Error::other)?,
    );
    let app_service = service.clone();
    let server = HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(app_service.clone()))
            .app_data(web::JsonConfig::default().limit(32 * 1024))
            .route("/", web::get().to(hello))
            .route("/health", web::get().to(health_check))
            .route("/greet/{name}", web::get().to(greet))
            .route("/api/echo", web::post().to(echo))
            .route("/api/calculate", web::post().to(calculate))
            .configure(api::configure)
            .route(
                "/info",
                web::get().to(|| async {
                    HttpResponse::Ok().json(serde_json::json!({
                        "project": env!("CARGO_PKG_NAME"),
                        "version": env!("CARGO_PKG_VERSION"),
                        "endpoints": [
                            { "method": "GET", "path": "/", "description": "欢迎页面" },
                            { "method": "GET", "path": "/health", "description": "健康检查" },
                            { "method": "GET", "path": "/greet/{name}", "description": "问候接口" },
                            { "method": "POST", "path": "/api/echo", "description": "回显数据" },
                            { "method": "POST", "path": "/api/calculate", "description": "计算接口" }
                        ]
                    }))
                }),
            )
    })
    .bind(("127.0.0.1", config.port))?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let background = tokio::spawn(worker(service, config.poll_interval, shutdown_rx));

    println!("\n==================================================");
    println!("🚀 Rust Web 服务器已启动");
    println!("📡 监听地址: http://127.0.0.1:{}", config.port);
    println!("   GET/POST /api/subscriptions - 订阅管理");
    println!("   POST /api/preview          - RSS 解析预览");
    println!("   POST /api/run              - 执行订阅与下载同步");
    println!("   GET  /api/jobs             - 下载任务");
    println!("📋 可用端点:");
    println!("   GET  /                    - 欢迎页面");
    println!("   GET  /health             - 健康检查");
    println!("   GET  /info              - API 信息");
    println!("   GET  /greet/{{name}}     - 问候接口");
    println!("   POST /api/echo          - 回显数据");
    println!("   POST /api/calculate     - 计算接口");
    println!("==================================================\n");

    let result = server.run().await;
    let _ = shutdown_tx.send(true);
    let _ = background.await;
    result
}
