-- 分类可以在 Shuku 里删除和新建以后，ID 不能复用：网页端的地址和打开着的页面
-- 还拿着旧 ID，复用会让它们指向另一个分类。SQLite 不能给已有的表加 AUTOINCREMENT，
-- 只能重建。先建新表、复制数据，再删旧表：category_archive 引用 category 并带
-- ON DELETE CASCADE，先删 category 会把成员关系一起删掉。改名时 SQLite 会把
-- 新成员表里对 category_new 的引用一并改成 category。
CREATE TABLE category_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    search TEXT NOT NULL,
    pinned INTEGER NOT NULL,
    lrr_id TEXT
);
INSERT INTO category_new (id, name, search, pinned, lrr_id)
    SELECT id, name, search, pinned, lrr_id FROM category;

CREATE TABLE category_archive_new (
    category_id INTEGER NOT NULL REFERENCES category_new(id) ON DELETE CASCADE,
    archive_id INTEGER NOT NULL REFERENCES archive(id) ON DELETE CASCADE,
    PRIMARY KEY (category_id, archive_id)
);
INSERT INTO category_archive_new (category_id, archive_id)
    SELECT category_id, archive_id FROM category_archive;

DROP TABLE category_archive;
DROP TABLE category;
ALTER TABLE category_new RENAME TO category;
ALTER TABLE category_archive_new RENAME TO category_archive;
CREATE UNIQUE INDEX category_lrr_id ON category (lrr_id);
-- 详情页要列出一本书所在的分类，按归档查成员。
CREATE INDEX category_archive_archive ON category_archive (archive_id, category_id);
