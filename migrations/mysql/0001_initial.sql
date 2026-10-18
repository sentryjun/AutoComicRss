-- MySQL 8.4 LTS. Case/accent-sensitive NO PAD text; exact resource/path identity uses SHA-256 digests.
CREATE TABLE storage_lock (singleton INT PRIMARY KEY CHECK(singleton = 1)) ENGINE=InnoDB;
INSERT INTO storage_lock VALUES (1);
CREATE TABLE subscriptions (
    rowid BIGINT NOT NULL AUTO_INCREMENT UNIQUE,
    id VARCHAR(128) PRIMARY KEY,
    spec LONGTEXT NOT NULL,
    deleted INT NOT NULL DEFAULT 0,
    last_checked_at BIGINT,
    last_error LONGTEXT
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_as_cs;
CREATE TABLE downloader_instances (
    rowid BIGINT NOT NULL AUTO_INCREMENT UNIQUE,
    id VARCHAR(128) PRIMARY KEY,
    name LONGTEXT NOT NULL,
    kind VARCHAR(32) NOT NULL CHECK(kind IN ('qbittorrent','transmission')),
    endpoint LONGTEXT NOT NULL,
    endpoint_digest BINARY(32) GENERATED ALWAYS AS (UNHEX(SHA2(endpoint,256))) STORED UNIQUE,
    config LONGTEXT NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_as_cs;
CREATE TABLE media_destinations (
    rowid BIGINT NOT NULL AUTO_INCREMENT UNIQUE,
    id VARCHAR(128) PRIMARY KEY,
    name LONGTEXT NOT NULL,
    kind VARCHAR(16) NOT NULL CHECK(kind IN ('local','smb')),
    root_path LONGTEXT NOT NULL,
    smb_config LONGTEXT,
    CHECK((kind = 'local' AND smb_config IS NULL) OR (kind = 'smb' AND smb_config IS NOT NULL))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_as_cs;
CREATE TABLE downloader_selection (
    singleton INT PRIMARY KEY CHECK(singleton = 1),
    downloader_id VARCHAR(128),
    FOREIGN KEY(downloader_id) REFERENCES downloader_instances(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_as_cs;
INSERT INTO downloader_selection(singleton) VALUES(1);
CREATE TABLE destination_selection (
    singleton INT PRIMARY KEY CHECK(singleton = 1),
    destination_id VARCHAR(128),
    FOREIGN KEY(destination_id) REFERENCES media_destinations(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_as_cs;
INSERT INTO destination_selection(singleton) VALUES(1);
CREATE TABLE destination_mappings (
    rowid BIGINT NOT NULL AUTO_INCREMENT UNIQUE,
    id VARCHAR(128) PRIMARY KEY,
    destination_id VARCHAR(128) NOT NULL,
    downloader_id VARCHAR(128) NOT NULL,
    version BIGINT NOT NULL,
    download_root LONGTEXT NOT NULL,
    UNIQUE(destination_id,downloader_id,version),
    FOREIGN KEY(destination_id) REFERENCES media_destinations(id),
    FOREIGN KEY(downloader_id) REFERENCES downloader_instances(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_as_cs;
CREATE TABLE releases (
    rowid BIGINT PRIMARY KEY AUTO_INCREMENT,
    subscription_id VARCHAR(128) NOT NULL,
    resource_key LONGTEXT NOT NULL,
    resource_digest BINARY(32) GENERATED ALWAYS AS (UNHEX(SHA2(resource_key,256))) STORED,
    payload LONGTEXT NOT NULL,
    UNIQUE(subscription_id,resource_digest),
    FOREIGN KEY(subscription_id) REFERENCES subscriptions(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_as_cs;
CREATE TABLE download_jobs (
    rowid BIGINT NOT NULL AUTO_INCREMENT UNIQUE,
    id VARCHAR(128) PRIMARY KEY,
    subscription_id VARCHAR(128) NOT NULL,
    resource_key LONGTEXT NOT NULL,
    resource_digest BINARY(32) GENERATED ALWAYS AS (UNHEX(SHA2(resource_key,256))) STORED,
    title LONGTEXT NOT NULL,
    season INT NOT NULL,
    episode INT NOT NULL,
    download_url LONGTEXT NOT NULL,
    torrent_bytes LONGBLOB,
    info_hash VARCHAR(40),
    state VARCHAR(16) NOT NULL DEFAULT 'queued' CHECK(state IN ('queued','submitting','submitted','downloading','completed','missing','failed')),
    attempts INT NOT NULL DEFAULT 0,
    last_error LONGTEXT,
    next_attempt_at BIGINT NOT NULL DEFAULT 0,
    downloader_id VARCHAR(128),
    legacy_unassigned INT NOT NULL DEFAULT 0,
    destination_id VARCHAR(128),
    mapping_id VARCHAR(128),
    download_save_path LONGTEXT,
    file_status VARCHAR(16) NOT NULL DEFAULT 'unchecked' CHECK(file_status IN ('unchecked','present','absent','unavailable')),
    file_checked_at BIGINT,
    file_last_success_at BIGINT,
    file_error LONGTEXT,
    UNIQUE(subscription_id,resource_digest),
    UNIQUE(subscription_id,season,episode),
    INDEX jobs_state(state,next_attempt_at),
    INDEX jobs_downloader(downloader_id,state),
    FOREIGN KEY(subscription_id) REFERENCES subscriptions(id),
    FOREIGN KEY(downloader_id) REFERENCES downloader_instances(id),
    FOREIGN KEY(destination_id) REFERENCES media_destinations(id),
    FOREIGN KEY(mapping_id) REFERENCES destination_mappings(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_as_cs;
CREATE TABLE media_files (
    rowid BIGINT PRIMARY KEY AUTO_INCREMENT,
    job_id VARCHAR(128) NOT NULL,
    relative_path LONGTEXT NOT NULL,
    path_digest BINARY(32) GENERATED ALWAYS AS (UNHEX(SHA2(relative_path,256))) STORED,
    expected_size BIGINT NOT NULL,
    observed_size BIGINT,
    status VARCHAR(16) NOT NULL DEFAULT 'unchecked' CHECK(status IN ('unchecked','present','absent','unavailable')),
    error LONGTEXT,
    UNIQUE(job_id,path_digest),
    FOREIGN KEY(job_id) REFERENCES download_jobs(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_as_cs;
