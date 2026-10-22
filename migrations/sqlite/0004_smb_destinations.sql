-- SQLite DROP COLUMN preserves the table identity and all referencing job/mapping rows.
-- Existing destinations are local; widen the discriminator without rewriting old migrations.
ALTER TABLE media_destinations DROP COLUMN kind;
ALTER TABLE media_destinations ADD COLUMN kind TEXT NOT NULL DEFAULT 'local'
    CHECK (kind IN ('local', 'smb'));
ALTER TABLE media_destinations ADD COLUMN smb_config TEXT
    CHECK ((kind = 'local' AND smb_config IS NULL) OR (kind = 'smb' AND smb_config IS NOT NULL));
