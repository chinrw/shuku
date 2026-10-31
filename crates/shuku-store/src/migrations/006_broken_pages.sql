-- 页面清单里读不出尺寸的页数（宽和高记为 0）。这样的书照样入库，控制台的问题页按这一列
-- 列出它们，不必解码每本的页面清单。之前入库的书都没有坏页：旧版本遇到坏页整本不入库。
ALTER TABLE archive ADD COLUMN broken_pages INTEGER NOT NULL DEFAULT 0;
