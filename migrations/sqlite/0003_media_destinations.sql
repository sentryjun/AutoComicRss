CREATE TABLE media_destinations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind = 'local'),
    root_path TEXT NOT NULL
);
CREATE TABLE destination_selection (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    destination_id TEXT REFERENCES media_destinations(id)
);
INSERT INTO destination_selection(singleton) VALUES (1);
CREATE TABLE destination_mappings (
    id TEXT PRIMARY KEY,
    destination_id TEXT NOT NULL REFERENCES media_destinations(id),
    downloader_id TEXT NOT NULL REFERENCES downloader_instances(id),
    version INTEGER NOT NULL,
    download_root TEXT NOT NULL,
    UNIQUE(destination_id, downloader_id, version)
);
ALTER TABLE download_jobs ADD COLUMN destination_id TEXT REFERENCES media_destinations(id);
ALTER TABLE download_jobs ADD COLUMN mapping_id TEXT REFERENCES destination_mappings(id);
ALTER TABLE download_jobs ADD COLUMN download_save_path TEXT;
ALTER TABLE download_jobs ADD COLUMN file_status TEXT NOT NULL DEFAULT 'unchecked'
    CHECK (file_status IN ('unchecked', 'present', 'absent', 'unavailable'));
ALTER TABLE download_jobs ADD COLUMN file_checked_at INTEGER;
ALTER TABLE download_jobs ADD COLUMN file_last_success_at INTEGER;
ALTER TABLE download_jobs ADD COLUMN file_error TEXT;
CREATE TABLE media_files (
    job_id TEXT NOT NULL REFERENCES download_jobs(id),
    relative_path TEXT NOT NULL,
    expected_size INTEGER NOT NULL,
    observed_size INTEGER,
    status TEXT NOT NULL DEFAULT 'unchecked'
        CHECK (status IN ('unchecked', 'present', 'absent', 'unavailable')),
    error TEXT,
    PRIMARY KEY(job_id, relative_path)
);
