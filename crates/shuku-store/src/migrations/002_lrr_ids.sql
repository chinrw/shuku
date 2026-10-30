-- 记录 LRR 的分类和 Tankoubon ID。迁移工具重复运行时按它更新同一行，
-- 在 Shuku 里新建的分类和 Tankoubon 这一列为空，不受迁移影响。
ALTER TABLE category ADD COLUMN lrr_id TEXT;
CREATE UNIQUE INDEX category_lrr_id ON category (lrr_id);
ALTER TABLE tankoubon ADD COLUMN lrr_id TEXT;
CREATE UNIQUE INDEX tankoubon_lrr_id ON tankoubon (lrr_id);
