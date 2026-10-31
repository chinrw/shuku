-- 迁移工具导入时内容目录里还没扫描到的归档：LRR 的元数据按相对路径暂存，
-- 扫描新增这个路径的归档时套用并删除。data 是 postcard 编码的 LrrPending。
CREATE TABLE lrr_pending (
    path TEXT PRIMARY KEY,
    data BLOB NOT NULL
) WITHOUT ROWID;
