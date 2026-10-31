-- 在线来源不止 E-Hentai 一个，还有 Bangumi 和 AniList，队列改成按（归档，来源）排。
-- 原来队列里的都是等着查 E-Hentai 的书。SQLite 不能改主键，只能重建。
CREATE TABLE metadata_queue_new (
    archive_id INTEGER NOT NULL REFERENCES archive(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    PRIMARY KEY (archive_id, provider)
) WITHOUT ROWID;
INSERT INTO metadata_queue_new (archive_id, provider)
    SELECT archive_id, 'ehentai' FROM metadata_queue;
DROP TABLE metadata_queue;
ALTER TABLE metadata_queue_new RENAME TO metadata_queue;
-- 后台按来源取下一本。
CREATE INDEX metadata_queue_by_provider ON metadata_queue (provider, archive_id);

-- 打开“先放候选、确认后写入”时，在线来源查到的结果放在这里，等人或 agent 改好后批准写入，
-- 或者拒绝。found 是服务端写的 JSON，每本书每个来源最多一条。
CREATE TABLE metadata_candidate (
    archive_id INTEGER NOT NULL REFERENCES archive(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    found TEXT NOT NULL,
    fetched_at INTEGER NOT NULL,
    PRIMARY KEY (archive_id, provider)
) WITHOUT ROWID;
