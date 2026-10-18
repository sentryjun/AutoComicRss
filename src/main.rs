// SPDX-License-Identifier: GPL-2.0-only
use autocomic_rss::{app, config::Config};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if !arguments.is_empty() {
        if arguments != ["migrate", "--offline"] {
            return Err(std::io::Error::other("usage: autocomic-rss [migrate --offline]; migration reads MIGRATION_SOURCE_URL and MIGRATION_TARGET_URL and requires both services stopped"));
        }
        let source = std::env::var("MIGRATION_SOURCE_URL")
            .map_err(|_| std::io::Error::other("MIGRATION_SOURCE_URL is required"))?;
        let target = std::env::var("MIGRATION_TARGET_URL")
            .map_err(|_| std::io::Error::other("MIGRATION_TARGET_URL is required"))?;
        let report = autocomic_rss::storage::migrate_offline(&source, &target)
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        println!(
            "Migration verified and committed: {} rows. Update DATABASE_URL before restarting.",
            report.rows
        );
        return Ok(());
    }
    let config = Config::from_env().map_err(|error| std::io::Error::other(error.to_string()))?;
    app::run(config).await
}
