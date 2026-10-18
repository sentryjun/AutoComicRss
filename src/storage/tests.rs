// SPDX-License-Identifier: GPL-2.0-only
use super::{backend::db_query, *};
use crate::{domain::*, media::*};
use serde_json::json;

async fn contract(store: &Store) {
    let sub = store
        .create(SubscriptionInput {
            title: "动画 😺".into(),
            rss_url: "https://example.invalid/feed".into(),
            season: 1,
            episode_offset: 0,
            include: vec![],
            exclude: vec![],
            enabled: true,
        })
        .await
        .unwrap();
    // Updating an unchanged record is successful on either engine.
    store.update(&sub.id, sub.input.clone()).await.unwrap();
    store
        .bootstrap_downloader(Some(&downloader("A", "qbittorrent").connection))
        .await
        .unwrap();
    let a = store.downloaders().await.unwrap().remove(0);
    store
        .bootstrap_downloader(Some(&downloader("B", "qbittorrent").connection))
        .await
        .unwrap();
    assert_eq!(
        store.active_downloader_id().await.unwrap().as_deref(),
        Some(a.id.as_str())
    );
    assert!(store
        .create_downloader(downloader("A", "qbittorrent"))
        .await
        .is_err());
    store.activate_downloader(&a.id).await.unwrap();
    let destination = store.create_destination(serde_json::from_value(json!({"name":"NAS","kind":"smb","root_path":"媒体", "smb":{"server":"nas.invalid","share":"media","username":"reader","password":"smb-secret"}})).unwrap()).await.unwrap();
    let map = store
        .create_mapping(
            &destination.id,
            MappingInput {
                downloader_id: a.id.clone(),
                download_root: "/downloads/动画".into(),
            },
        )
        .await
        .unwrap();
    store.activate_destination(&destination.id).await.unwrap();
    let release = Release {
        key: format!("Case-Sensitive-{}", "x".repeat(9000)),
        title: "Example - 01 😺".into(),
        download_url: Some("magnet:?xt=urn:btih:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
        episode: Some(1),
        decision: Decision::Ready,
        reason: "ready".into(),
    };
    let one = vec![release.clone()];
    let (first, second) =
        tokio::join!(store.record_scan(&sub, &one), store.record_scan(&sub, &one));
    assert_eq!(first.unwrap() + second.unwrap(), 1);
    let mut other = release.clone();
    other.key = other.key.to_lowercase();
    other.episode = Some(2);
    assert_eq!(store.record_scan(&sub, &[other.clone()]).await.unwrap(), 1);
    other.key.push(' ');
    other.episode = Some(3);
    assert_eq!(store.record_scan(&sub, &[other]).await.unwrap(), 1);
    assert_eq!(store.releases(&sub.id).await.unwrap().len(), 3);
    // Unicode-equivalent spellings remain distinct resource IDs, just as in SQLite.
    for (episode, key) in [(5, "café"), (6, "cafe\u{301}")] {
        let mut candidate = release.clone();
        candidate.key = key.into();
        candidate.episode = Some(episode);
        assert!(!store.already_queued(&sub.id, 1, &candidate).await.unwrap());
        assert_eq!(store.record_scan(&sub, &[candidate]).await.unwrap(), 1);
    }

    let job = store
        .jobs()
        .await
        .unwrap()
        .into_iter()
        .find(|j| j.episode == 1)
        .unwrap();
    assert_eq!(job.downloader_id.as_deref(), Some(a.id.as_str()));
    assert_eq!(job.mapping_id.as_deref(), Some(map.id.as_str()));
    assert!(!job.legacy_unassigned);
    store
        .prepare(
            &job.id,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some(&[0, 1, 255, 0, 128]),
        )
        .await
        .unwrap();
    store.state(&job.id, "submitting").await.unwrap();
    store.failed(&job, "uncertain submission").await.unwrap();
    assert_eq!(store.job(&job.id).await.unwrap().attempts, 1);
    assert!(store.job(&job.id).await.unwrap().next_attempt_at > 0);
    assert!(store
        .active_jobs()
        .await
        .unwrap()
        .iter()
        .all(|j| j.id != job.id));
    let b = store
        .create_downloader(downloader("B", "transmission"))
        .await
        .unwrap();
    assert!(store.activate_downloader(&b.id).await.is_err());
    store
        .create_mapping(
            &destination.id,
            MappingInput {
                downloader_id: b.id.clone(),
                download_root: "/new/media".into(),
            },
        )
        .await
        .unwrap();
    store.activate_downloader(&b.id).await.unwrap();
    let next = store
        .create_mapping(
            &destination.id,
            MappingInput {
                downloader_id: a.id.clone(),
                download_root: "/changed".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(next.version, 2);
    let mut new_release = release;
    new_release.key = "next".into();
    new_release.episode = Some(4);
    store.record_scan(&sub, &[new_release]).await.unwrap();
    let newest = store.jobs().await.unwrap().remove(0);
    assert_eq!(newest.downloader_id.as_deref(), Some(b.id.as_str()));
    store.state(&job.id, "failed").await.unwrap();
    store.retry(&job.id).await.unwrap();
    let original = store.job(&job.id).await.unwrap();
    assert_eq!(original.downloader_id.as_deref(), Some(a.id.as_str()));
    assert_eq!(
        original.download_save_path.as_deref(),
        Some("/downloads/动画")
    );
    assert_eq!(
        store.torrent_bytes(&job.id).await.unwrap(),
        Some(vec![0, 1, 255, 0, 128])
    );
    let mut files = vec![MediaFile {
        relative_path: "动画/第01集.mkv".into(),
        expected_size: 3,
        observed_size: None,
        status: "unchecked".into(),
        error: None,
    }];
    let mut unsafe_file = files[0].clone();
    unsafe_file.relative_path = "../outside".into();
    assert!(store
        .save_manifest(&job.id, &[files[0].clone(), unsafe_file])
        .await
        .is_err());
    assert!(store.file_report(&job.id).await.unwrap().files.is_empty()); // Whole transaction rolled back.
    store.save_manifest(&job.id, &files).await.unwrap();
    store.save_manifest(&job.id, &files).await.unwrap();
    files[0].observed_size = Some(3);
    files[0].status = "present".into();
    store
        .save_file_check(&job.id, "present", None, &files)
        .await
        .unwrap();
    let checked = store.file_report(&job.id).await.unwrap();
    files[0].status = "unavailable".into();
    files[0].error = Some("timeout".into());
    store
        .save_file_check(&job.id, "unavailable", Some("timeout"), &files)
        .await
        .unwrap();
    assert_eq!(
        store.file_report(&job.id).await.unwrap().last_success_at,
        checked.last_success_at
    );
    assert_eq!(store.job(&job.id).await.unwrap().state, "queued");
    assert!(!serde_json::to_string(&store.downloaders().await.unwrap())
        .unwrap()
        .contains("download-secret"));
    assert!(!serde_json::to_string(&store.destinations().await.unwrap())
        .unwrap()
        .contains("smb-secret"));
    store.delete(&sub.id).await.unwrap();
    assert!(store.subscriptions().await.unwrap().is_empty());
    assert_eq!(store.jobs().await.unwrap().len(), 6);
}
fn downloader(name: &str, kind: &str) -> DownloaderInput {
    serde_json::from_value(json!({"name":name,"kind":kind,"connection":{"url":format!("http://{}.invalid",name.to_lowercase()),"username":"user","password":"download-secret","save_path":"/downloads"}})).unwrap()
}
struct MysqlFixture {
    admin: sqlx::MySqlPool,
    name: String,
    url: String,
}
impl MysqlFixture {
    async fn new() -> Self {
        let base=std::env::var("AUTOCOMIC_TEST_MYSQL_URL").expect("set AUTOCOMIC_TEST_MYSQL_URL to a dedicated test server with CREATE DATABASE permission");
        let admin = sqlx::MySqlPool::connect(&base).await.unwrap();
        let name = format!("autocomic_test_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!(
            "CREATE DATABASE {name} CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_as_cs"
        ))
        .execute(&admin)
        .await
        .unwrap();
        let mut url = reqwest::Url::parse(&base).unwrap();
        url.set_path(&format!("/{name}"));
        Self {
            admin,
            name,
            url: url.into(),
        }
    }
    async fn close(self) {
        sqlx::query(&format!("DROP DATABASE {}", self.name))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}
#[tokio::test]
async fn sqlite_storage_contract() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    contract(&store).await;
    store.pool.close().await;
}
#[tokio::test]
#[ignore = "requires a dedicated MySQL 8.4 server; see tests/README.md"]
async fn mysql_storage_contract() {
    let fixture = MysqlFixture::new().await;
    let store = Store::open(&fixture.url).await.unwrap();
    contract(&store).await;
    store.pool.close().await;
    fixture.close().await;
}
#[tokio::test]
#[ignore = "requires a dedicated MySQL 8.4 server; see tests/README.md"]
async fn offline_roundtrip_and_atomic_failure() {
    let folder = std::env::temp_dir().join(format!("autocomic-transfer-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&folder).unwrap();
    let source_url = format!("sqlite://{}", folder.join("source.db").display());
    let destination_url = format!("sqlite://{}", folder.join("roundtrip.db").display());
    let source = Store::open(&source_url).await.unwrap();
    contract(&source).await;
    let prepared_job = source
        .jobs()
        .await
        .unwrap()
        .into_iter()
        .find(|j| j.info_hash.is_some())
        .unwrap();
    source.state(&prepared_job.id, "submitting").await.unwrap();
    source
        .failed(&prepared_job, "pending reconciliation")
        .await
        .unwrap();
    source.pool.close().await;
    let fixture = MysqlFixture::new().await;
    let report = migrate_offline(&source_url, &fixture.url).await.unwrap();
    assert!(report.rows > 10);
    let mysql = Store::open(&fixture.url).await.unwrap();
    assert_eq!(mysql.jobs().await.unwrap().len(), 6);
    mysql.pool.close().await;
    assert!(migrate_offline(&source_url, &fixture.url).await.is_err()); // No overwrite.
    migrate_offline(&fixture.url, &destination_url)
        .await
        .unwrap();
    let roundtrip = Store::open(&destination_url).await.unwrap();
    let source = Store::open(&source_url).await.unwrap();
    assert_eq!(
        serde_json::to_value(source.jobs().await.unwrap()).unwrap(),
        serde_json::to_value(roundtrip.jobs().await.unwrap()).unwrap()
    );
    let configs_source:Vec<String>=db_query!(source.pool,query_scalar,"SELECT config FROM downloader_instances ORDER BY rowid";[];fetch_all).unwrap();
    let configs_target:Vec<String>=db_query!(roundtrip.pool,query_scalar,"SELECT config FROM downloader_instances ORDER BY rowid";[];fetch_all).unwrap();
    assert_eq!(configs_source, configs_target);
    roundtrip.pool.close().await;
    // An unrelated target schema must be rejected before application DDL is run.
    let unrelated = MysqlFixture::new().await;
    let unrelated_pool = sqlx::MySqlPool::connect(&unrelated.url).await.unwrap();
    sqlx::query("CREATE TABLE unrelated_data(id INT)")
        .execute(&unrelated_pool)
        .await
        .unwrap();
    assert!(migrate_offline(&source_url, &unrelated.url)
        .await
        .unwrap_err()
        .to_string()
        .contains("unrelated tables"));
    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema=DATABASE()",
    )
    .fetch_one(&unrelated_pool)
    .await
    .unwrap();
    assert_eq!(tables, 1);
    unrelated_pool.close().await;
    unrelated.close().await;
    // Corrupt a source row deliberately: failure occurs after several target tables were inserted.
    sqlx::query("PRAGMA ignore_check_constraints=ON")
        .execute(source.pool.sqlite().unwrap())
        .await
        .unwrap();
    sqlx::query("UPDATE download_jobs SET state='invalid-state'")
        .execute(source.pool.sqlite().unwrap())
        .await
        .unwrap();
    source.pool.close().await;
    let empty = MysqlFixture::new().await;
    assert!(migrate_offline(&source_url, &empty.url).await.is_err());
    let target = Store::open(&empty.url).await.unwrap();
    assert!(target.downloaders().await.unwrap().is_empty());
    assert!(target.jobs().await.unwrap().is_empty());
    let count: i64 =
        db_query!(target.pool,query_scalar,"SELECT COUNT(*) FROM subscriptions";[];fetch_one)
            .unwrap();
    assert_eq!(count, 0);
    target.pool.close().await;
    empty.close().await;
    fixture.close().await;
    std::fs::remove_dir_all(folder).unwrap();
}
#[tokio::test]
async fn unsupported_database_and_missing_migration_source_fail_without_creating_source() {
    assert!(Store::open("postgres://invalid").await.is_err());
    let missing = std::env::temp_dir().join(format!("missing-{}.db", uuid::Uuid::new_v4()));
    let result = migrate_offline(
        &format!("sqlite://{}", missing.display()),
        "mysql://invalid/target",
    )
    .await;
    assert!(result.is_err());
    assert!(!missing.exists());
    // Connection configuration remains a deployment concern; no database HTTP configuration exists.
}
