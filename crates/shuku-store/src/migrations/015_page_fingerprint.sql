-- 每条归档只存整本的页面签名。NULL 表示还不能用这条记录排除重复。
ALTER TABLE archive ADD COLUMN page_fingerprint BLOB;
CREATE INDEX archive_by_pages ON archive(page_fingerprint) WHERE missing = 0;
CREATE TRIGGER invalidate_page_fingerprint AFTER UPDATE OF content_hash, page_count ON archive
WHEN OLD.page_fingerprint IS NOT NULL AND
    (OLD.content_hash IS NOT NEW.content_hash OR OLD.page_count != NEW.page_count)
BEGIN
    UPDATE archive SET page_fingerprint = NULL WHERE id = NEW.id;
END;
