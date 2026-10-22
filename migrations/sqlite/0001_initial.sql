CREATE TABLE subscriptions (
    id TEXT PRIMARY KEY,
    spec TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    last_checked_at INTEGER,
    last_error TEXT
);

CREATE TABLE releases (
    subscription_id TEXT NOT NULL REFERENCES subscriptions(id),
    resource_key TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (subscription_id, resource_key)
);

CREATE TABLE download_jobs (
    id TEXT PRIMARY KEY,
    subscription_id TEXT NOT NULL REFERENCES subscriptions(id),
    resource_key TEXT NOT NULL,
    title TEXT NOT NULL,
    season INTEGER NOT NULL,
    episode INTEGER NOT NULL,
    download_url TEXT NOT NULL,
    torrent_bytes BLOB,
    info_hash TEXT,
    state TEXT NOT NULL DEFAULT 'queued'
        CHECK (state IN ('queued', 'submitting', 'submitted', 'downloading', 'completed', 'missing', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    next_attempt_at INTEGER NOT NULL DEFAULT 0,
    UNIQUE(subscription_id, resource_key),
    UNIQUE(subscription_id, season, episode)
);
CREATE INDEX jobs_state ON download_jobs(state, next_attempt_at);
