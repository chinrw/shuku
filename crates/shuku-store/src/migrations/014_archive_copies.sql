-- 旧记录可能已有不同的人工元数据，只有升级后新扫描到的 ZIP 可以自动并入另一条记录。
ALTER TABLE archive ADD COLUMN auto_copy INTEGER NOT NULL DEFAULT 0;
ALTER TABLE archive ADD COLUMN copy_stamp INTEGER;
CREATE INDEX archive_by_content ON archive(content_hash, id) WHERE content_hash IS NOT NULL;

CREATE TABLE archive_copy (
    path TEXT PRIMARY KEY,
    archive_id INTEGER NOT NULL REFERENCES archive(id) ON DELETE CASCADE,
    mtime INTEGER NOT NULL,
    stamp INTEGER
) WITHOUT ROWID;
CREATE UNIQUE INDEX archive_copy_by_book ON archive_copy(archive_id, path);

CREATE TABLE archive_redirect (
    old_id INTEGER PRIMARY KEY,
    archive_id INTEGER NOT NULL REFERENCES archive(id) ON DELETE CASCADE
);

-- 独立的 scan 命令也能连接同一个库。无论哪个写连接先提交，路径都不能同时属于两本书。
CREATE TRIGGER archive_copy_path_insert BEFORE INSERT ON archive_copy
WHEN EXISTS (SELECT 1 FROM archive WHERE path = NEW.path)
BEGIN SELECT RAISE(ABORT, 'copy path is already a primary path'); END;

CREATE TRIGGER archive_primary_path_insert BEFORE INSERT ON archive
WHEN EXISTS (SELECT 1 FROM archive_copy WHERE path = NEW.path)
BEGIN SELECT RAISE(ABORT, 'primary path is already a copy'); END;

CREATE TRIGGER archive_primary_path_update BEFORE UPDATE OF path ON archive
WHEN EXISTS (SELECT 1 FROM archive_copy WHERE path = NEW.path)
BEGIN SELECT RAISE(ABORT, 'primary path is already a copy'); END;
