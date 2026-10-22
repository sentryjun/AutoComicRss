-- Preserve instance IDs and all foreign key references while widening the kind constraint.
ALTER TABLE downloader_instances DROP COLUMN kind;
ALTER TABLE downloader_instances ADD COLUMN kind TEXT NOT NULL DEFAULT 'qbittorrent'
    CHECK (kind IN ('qbittorrent', 'transmission'));
