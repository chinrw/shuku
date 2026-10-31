-- 读到过最后一页就记下，之后重读到一半也不清掉。“读完”和书库的“隐藏读完的”按它算，
-- 不按当前页码：读完的书重看几页就不算读完，筛选就没用了。
ALTER TABLE progress ADD COLUMN finished INTEGER NOT NULL DEFAULT 0;

-- 已有的进度里停在最后一页的补记上。
UPDATE progress SET finished = 1
WHERE page + 1 >= (SELECT page_count FROM archive WHERE archive.id = progress.archive_id);
