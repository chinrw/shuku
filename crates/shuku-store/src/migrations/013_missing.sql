ALTER TABLE archive ADD COLUMN missing INTEGER NOT NULL DEFAULT 0;
CREATE INDEX archive_missing ON archive(id) WHERE missing = 1;
