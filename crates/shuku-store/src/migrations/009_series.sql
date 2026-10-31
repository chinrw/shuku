-- 按卷出版的作品：一部作品是一个系列，每卷是一本归档。扫描按文件夹和文件名里的卷号自动归组，
-- `key` 是归组依据（卷所在的文件夹，文件名带作品名时再加上作品名），再扫到同一组的卷时复用。
-- 一本归档最多属于一个系列；LRR 的 Tankoubon 一本可以属于几个，仍留在原来的表里。
CREATE TABLE series (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    key TEXT NOT NULL UNIQUE
);

ALTER TABLE archive ADD COLUMN series_id INTEGER REFERENCES series(id) ON DELETE SET NULL;
-- 卷号，系列里按它排序。
ALTER TABLE archive ADD COLUMN volume INTEGER;
CREATE INDEX archive_by_series ON archive (series_id, volume);
