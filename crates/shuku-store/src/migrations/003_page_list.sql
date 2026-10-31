-- 页面清单只会整本读写，每本存成一个 postcard 编码的 BLOB，取代每页一行的 page 表。
-- 在 30 万本的合成库上，读一本的清单从 23 µs 降到 2 µs，库从 948 MiB 降到 544 MiB。
-- 旧表里的数据由 Rust 步骤转换后删除旧表。
CREATE TABLE page_list (
    archive_id INTEGER PRIMARY KEY REFERENCES archive(id) ON DELETE CASCADE,
    pages BLOB NOT NULL
);
