-- 等着在后台查 E-Hentai 的归档。查找要联网、两次之间要隔几秒，扫描只把新入库的书放进来，
-- 后台按 ID 顺序逐本处理，处理完删掉。放在库里，重启后接着处理，记录删除时一并删除。
CREATE TABLE metadata_queue (
    archive_id INTEGER PRIMARY KEY REFERENCES archive(id) ON DELETE CASCADE
);
