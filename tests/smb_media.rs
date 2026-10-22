// SPDX-License-Identifier: GPL-2.0-only
use actix_web::{test, web, App};
use autocomic_rss::{
    config::Config, http, media::DestinationInput, service::Service, storage::Store,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Arc,
    time::Duration,
};

fn config(database_url: String) -> Config {
    Config {
        database_url,
        host: "127.0.0.1".parse().unwrap(),
        port: 8080,
        poll_interval: Duration::from_secs(300),
        qbit: None,
        mikan_base_url: "https://mikanani.me".into(),
        website: Default::default(),
    }
}
fn destination(port: u16) -> Value {
    json!({"name":"NAS", "kind":"smb", "root_path":"library", "smb":{
        "server":"127.0.0.1", "port":port, "share":"MEDIA", "username":"reader",
        "password":"fixture-password", "timeout_seconds":3
    }})
}

#[actix_web::test]
async fn smb_configuration_is_validated_and_credentials_are_never_returned() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let service = Arc::new(
        Service::new(store.clone(), &config("sqlite::memory:".into()))
            .await
            .unwrap(),
    );
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(service))
            .configure(http::configure),
    )
    .await;
    for (key, value) in [
        ("server", json!("smb://user:password@host")),
        ("server", json!("host:445")),
        ("port", json!(0)),
        ("share", json!("../MEDIA")),
        ("share", json!("a/b")),
        ("username", json!("Guest")),
        ("password", json!("")),
        ("timeout_seconds", json!(61)),
    ] {
        let mut body = destination(445);
        body["smb"][key] = value;
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/destinations")
                .set_json(body)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400, "{key}");
    }
    for root in [
        "/absolute",
        "../up",
        "C:/drive",
        "a\\b",
        "a/./b",
        "a?b",
        "a.",
        "a ",
    ] {
        let mut body = destination(445);
        body["root_path"] = json!(root);
        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/api/destinations")
                .set_json(body)
                .to_request(),
        )
        .await;
        assert_eq!(response.status(), 400, "{root}");
    }
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/api/destinations")
            .set_json(destination(445))
            .to_request(),
    )
    .await;
    assert_eq!(response.status(), 201);
    let saved: Value = test::read_body_json(response).await;
    assert_eq!(saved["kind"], "smb");
    assert_eq!(saved["smb"]["password_configured"], true);
    assert!(saved["smb"].get("password").is_none());
    assert!(!saved.to_string().contains("fixture-password"));
    let listed: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri("/api/destinations")
            .to_request(),
    )
    .await;
    assert!(!listed.to_string().contains("fixture-password"));
    for path in ["../x", "a%5Cb", "a%3Fb", "/absolute", "name%3Astream"] {
        let uri = format!(
            "/api/destinations/{}/entries?path={path}",
            saved["id"].as_str().unwrap()
        );
        assert_eq!(
            test::call_service(&app, test::TestRequest::get().uri(&uri).to_request())
                .await
                .status(),
            400
        );
    }
}

#[actix_web::test]
async fn database_upgrade_preserves_destinations_mappings_jobs_and_file_history() {
    for version in [3, 4] {
        upgrade_contract(version).await;
    }
}

async fn upgrade_contract(version: usize) {
    let store = Store::open("sqlite::memory:").await.unwrap();
    // A fresh schema must retain referential integrity after the DROP COLUMN migration.
    let violations: Vec<(String, Option<i64>, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(store.pool.sqlite().unwrap())
            .await
            .unwrap();
    assert!(violations.is_empty());
    // Construct a real pre-P3 SQLite database and its migration ledger.
    let folder = std::env::temp_dir().join(format!("smb-upgrade-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&folder).unwrap();
    let url = format!("sqlite://{}?mode=rwc", folder.join("old.db").display());
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    let old_migrations = folder.join("migrations");
    std::fs::create_dir(&old_migrations).unwrap();
    for name in [
        "0001_initial.sql",
        "0002_downloader_instances.sql",
        "0003_media_destinations.sql",
        "0004_smb_destinations.sql",
    ]
    .into_iter()
    .take(version)
    {
        std::fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("migrations")
                .join(name),
            old_migrations.join(name),
        )
        .unwrap();
    }
    sqlx::migrate::Migrator::new(old_migrations.as_path())
        .await
        .unwrap()
        .run(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO media_destinations(id,name,kind,root_path) VALUES ('destination','Local','local','/media')")
        .execute(&pool)
        .await
        .unwrap();
    if version == 4 {
        sqlx::query("UPDATE media_destinations SET kind = 'smb', root_path = 'Series', smb_config = ?")
            .bind(serde_json::json!({"server":"nas.invalid","share":"media","username":"reader","password":"retained-secret"}).to_string())
            .execute(&pool).await.unwrap();
    }
    sqlx::query("UPDATE destination_selection SET destination_id = 'destination'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO downloader_instances(id,name,kind,endpoint,config) VALUES ('downloader','Qbit','qbittorrent','http://localhost','{}')").execute(&pool).await.unwrap();
    sqlx::query("UPDATE downloader_instances SET config = ?")
        .bind(serde_json::json!({"url":"http://localhost","username":"test","password":"retained-qbit-secret","save_path":"/downloads"}).to_string())
        .execute(&pool).await.unwrap();
    sqlx::query("UPDATE downloader_selection SET downloader_id = 'downloader'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO destination_mappings VALUES ('mapping','destination','downloader',1,'/downloads')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO subscriptions(id,spec) VALUES ('subscription','{}')")
        .execute(&pool)
        .await
        .unwrap();
    seed_job(&pool, "destination").await;
    pool.close().await;
    let upgraded = Store::open(&url).await.unwrap();
    assert_eq!(
        upgraded.destination("destination").await.unwrap().kind,
        if version == 4 { "smb" } else { "local" }
    );
    assert_eq!(
        upgraded.active_destination_id().await.unwrap().as_deref(),
        Some("destination")
    );
    assert_eq!(
        upgraded.active_downloader_id().await.unwrap().as_deref(),
        Some("downloader")
    );
    let instances = upgraded.downloaders().await.unwrap();
    assert_eq!(
        serde_json::to_value(&instances[0]).unwrap()["kind"],
        "qbittorrent"
    );
    assert!(instances[0].password_configured);
    let transmission = upgraded.create_downloader(serde_json::from_value(serde_json::json!({
        "name":"new", "kind":"transmission", "connection":{
            "url":"http://localhost:9091/transmission/rpc", "username":"test", "password":"test", "save_path":"/downloads"
        }
    })).unwrap()).await.unwrap();
    assert_eq!(
        serde_json::to_value(transmission).unwrap()["kind"],
        "transmission"
    );
    let job = upgraded.job("job").await.unwrap();
    assert_eq!(job.mapping_id.as_deref(), Some("mapping"));
    let report = upgraded.file_report("job").await.unwrap();
    assert_eq!(report.files[0].relative_path, "Series/ep01.mkv");
    let violations: Vec<(String, Option<i64>, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(upgraded.pool.sqlite().unwrap())
            .await
            .unwrap();
    assert!(violations.is_empty());
    upgraded.pool.close().await;
    std::fs::remove_dir_all(folder).unwrap();
}

async fn seed_job(pool: &sqlx::SqlitePool, destination: &str) {
    sqlx::query("INSERT INTO download_jobs(id,subscription_id,resource_key,title,season,episode,download_url,state,destination_id,mapping_id,download_save_path,downloader_id) VALUES ('job','subscription','resource','Example',1,1,'magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','completed',?,'mapping','/downloads','downloader')")
        .bind(destination).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO media_files(job_id,relative_path,expected_size) VALUES ('job','Series/ep01.mkv',3)").execute(pool).await.unwrap();
}

struct Fixture {
    child: Child,
    folder: PathBuf,
    port: u16,
}
impl Fixture {
    async fn start() -> Self {
        let python = std::env::var("AUTOCOMIC_SMB_TEST_PYTHON")
            .expect("set AUTOCOMIC_SMB_TEST_PYTHON to a Python with impacket==0.13.0");
        let folder = std::env::temp_dir().join(format!("smb-wire-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(folder.join("share/library/Series")).unwrap();
        std::fs::write(folder.join("share/library/Series/ep01.mkv"), b"abc").unwrap();
        std::fs::create_dir_all(folder.join("share/library/denied")).unwrap();
        let ready = folder.join("ready");
        let child = Command::new(python)
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/smb_server.py"))
            .arg(folder.join("share"))
            .arg(&ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut fixture = Self {
            child,
            folder,
            port: 0,
        };
        for _ in 0..100 {
            if let Ok(port) = std::fs::read_to_string(&ready) {
                fixture.port = port.parse().unwrap();
                return fixture;
            }
            assert!(
                fixture.child.try_wait().unwrap().is_none(),
                "SMB fixture exited"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("SMB fixture startup timed out");
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

#[actix_web::test]
#[ignore = "requires AUTOCOMIC_SMB_TEST_PYTHON with impacket==0.13.0; uses a loopback-only SMB server"]
async fn smb_wire_listing_file_checks_restart_and_failure_states() {
    let mut fixture = Fixture::start().await;
    let url = format!("sqlite://{}", fixture.folder.join("state.db").display());
    let store = Store::open(&url).await.unwrap();
    let saved = store
        .create_destination(
            serde_json::from_value::<DestinationInput>(destination(fixture.port)).unwrap(),
        )
        .await
        .unwrap();
    let id = saved.id;
    sqlx::query("INSERT INTO downloader_instances(id,name,kind,endpoint,config) VALUES ('downloader','Qbit','qbittorrent','http://localhost','{}')").execute(store.pool.sqlite().unwrap()).await.unwrap();
    sqlx::query(
        "INSERT INTO destination_mappings VALUES ('mapping',?,'downloader',1,'/downloads')",
    )
    .bind(&id)
    .execute(store.pool.sqlite().unwrap())
    .await
    .unwrap();
    sqlx::query("INSERT INTO subscriptions(id,spec) VALUES ('subscription','{}')")
        .execute(store.pool.sqlite().unwrap())
        .await
        .unwrap();
    seed_job(store.pool.sqlite().unwrap(), &id).await;
    store.pool.close().await;
    // Reopen: the persisted SMB connection and job location work without a live downloader.
    let store = Store::open(&url).await.unwrap();
    let mut settings = config(url);
    settings.website.proxy_url = Some("http://127.0.0.1:1".into());
    let service = Arc::new(Service::new(store.clone(), &settings).await.unwrap());
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(service.clone()))
            .configure(http::configure),
    )
    .await;
    let listing_uri = format!("/api/destinations/{id}/entries?path=Series");
    let listing: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get().uri(&listing_uri).to_request(),
    )
    .await;
    assert_eq!(listing["status"], "available", "{listing}");
    assert_eq!(listing["entries"][0]["name"], "ep01.mkv");
    assert_eq!(listing["entries"][0]["size"], 3);
    std::fs::create_dir(fixture.folder.join("share/library/many")).unwrap();
    for n in 0..1001 {
        std::fs::write(
            fixture.folder.join(format!("share/library/many/{n:04}")),
            b"",
        )
        .unwrap();
    }
    let many: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/destinations/{id}/entries?path=many"))
            .to_request(),
    )
    .await;
    assert_eq!(many["status"], "available", "{many}");
    assert_eq!(many["entries"].as_array().unwrap().len(), 1000);
    assert_eq!(many["truncated"], true);
    let check_uri = "/api/jobs/job/files/check";
    let present: Value =
        test::call_and_read_body_json(&app, test::TestRequest::post().uri(check_uri).to_request())
            .await;
    assert_eq!(present["status"], "present", "{present}");
    let local = store.create_destination(serde_json::from_value(json!({"name":"New default", "kind":"local", "root_path":fixture.folder.to_str().unwrap()})).unwrap()).await.unwrap();
    sqlx::query("UPDATE downloader_selection SET downloader_id = 'downloader'")
        .execute(store.pool.sqlite().unwrap())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO destination_mappings VALUES ('local-mapping',?,'downloader',1,'/new')",
    )
    .bind(&local.id)
    .execute(store.pool.sqlite().unwrap())
    .await
    .unwrap();
    store.activate_destination(&local.id).await.unwrap();
    assert_eq!(
        store.job("job").await.unwrap().destination_id.as_deref(),
        Some(id.as_str())
    );
    assert_eq!(
        service.check_job_files("job").await.unwrap().status,
        "present"
    );
    std::fs::remove_file(fixture.folder.join("share/library/Series/ep01.mkv")).unwrap();
    let absent = service.check_job_files("job").await.unwrap();
    assert_eq!(absent.status, "absent");
    let last = absent.last_success_at;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    std::fs::rename(
        fixture.folder.join("share/library"),
        fixture.folder.join("share/offline"),
    )
    .unwrap();
    let offline = service.check_job_files("job").await.unwrap();
    assert_eq!(offline.status, "unavailable");
    assert_eq!(offline.error.as_deref(), Some("smb_root_unavailable"));
    assert_eq!(offline.last_success_at, last);
    std::fs::rename(
        fixture.folder.join("share/offline"),
        fixture.folder.join("share/library"),
    )
    .unwrap();
    // Authentication, permissions and missing shares are distinct, sanitized errors over the public API.
    for (field, value, expected) in [
        (
            "password",
            json!("wrong-secret"),
            "smb_authentication_failed",
        ),
        ("share", json!("MISSING"), "smb_root_unavailable"),
    ] {
        let mut body = destination(fixture.port);
        body["smb"][field] = value;
        let dest: Value = test::call_and_read_body_json(
            &app,
            test::TestRequest::post()
                .uri("/api/destinations")
                .set_json(body)
                .to_request(),
        )
        .await;
        let uri = format!("/api/destinations/{}/entries", dest["id"].as_str().unwrap());
        let response: Value =
            test::call_and_read_body_json(&app, test::TestRequest::get().uri(&uri).to_request())
                .await;
        assert_eq!(response["status"], "unavailable");
        assert_eq!(response["error"], expected, "{response}");
        assert!(!response.to_string().contains("wrong-secret"));
    }
    let denied: Value = test::call_and_read_body_json(
        &app,
        test::TestRequest::get()
            .uri(&format!("/api/destinations/{id}/entries?path=denied"))
            .to_request(),
    )
    .await;
    assert_eq!(denied["error"], "smb_permission_denied", "{denied}");
    fixture.child.kill().unwrap();
    fixture.child.wait().unwrap();
    let disconnected = service.check_job_files("job").await.unwrap();
    assert_eq!(disconnected.status, "unavailable");
    assert_eq!(disconnected.error.as_deref(), Some("smb_connection_failed"));
    assert_eq!(disconnected.last_success_at, last);
    assert_eq!(store.job("job").await.unwrap().state, "completed");
    store.pool.close().await;
}

#[actix_web::test]
async fn silent_smb_peer_times_out_instead_of_reporting_missing() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let store = Store::open("sqlite::memory:").await.unwrap();
    let mut body = destination(port);
    body["smb"]["timeout_seconds"] = json!(1);
    let dest = store
        .create_destination(serde_json::from_value(body).unwrap())
        .await
        .unwrap();
    let service = Arc::new(
        Service::new(store, &config("sqlite::memory:".into()))
            .await
            .unwrap(),
    );
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(service))
            .configure(http::configure),
    )
    .await;
    let uri = format!("/api/destinations/{}/entries", dest.id);
    let response: Value = tokio::time::timeout(
        Duration::from_secs(3),
        test::call_and_read_body_json(&app, test::TestRequest::get().uri(&uri).to_request()),
    )
    .await
    .unwrap();
    assert_eq!(response["status"], "unavailable");
    assert_eq!(response["error"], "smb_timeout");
    server.abort();
}
