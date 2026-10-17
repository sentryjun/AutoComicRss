CREATE TABLE downloader_instances (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind = 'qbittorrent'),
    endpoint TEXT NOT NULL UNIQUE,
    config TEXT NOT NULL
);
CREATE TABLE downloader_selection (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    downloader_id TEXT REFERENCES downloader_instances(id)
);
INSERT INTO downloader_selection(singleton) VALUES (1);

ALTER TABLE download_jobs ADD COLUMN downloader_id TEXT REFERENCES downloader_instances(id);
ALTER TABLE download_jobs ADD COLUMN legacy_unassigned INTEGER NOT NULL DEFAULT 0;
-- Existing jobs have no recorded owner. Do not assign them to an arbitrary new instance.
UPDATE download_jobs SET legacy_unassigned = 1;
CREATE INDEX jobs_downloader ON download_jobs(downloader_id, state);
