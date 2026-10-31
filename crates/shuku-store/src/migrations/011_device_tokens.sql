-- 每台设备（浏览器、阅读程序、agent）一个令牌，可以单独吊销，吊销就是删掉这一行。
-- 库里只存令牌的 BLAKE3 摘要，令牌本身只在新建时显示一次。时间都是 Unix 毫秒。
CREATE TABLE device_token (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER
);
