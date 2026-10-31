-- 在控制台里改的设置。每个功能一个键（例如 scan），值是那组设置的 JSON，
-- 由服务端用带默认值的结构读写，加字段不用迁移。目录和监听地址仍是启动参数。
CREATE TABLE setting (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;
