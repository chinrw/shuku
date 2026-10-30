# M1 后端实现计划

M1 后端的目标是：服务能扫描内容目录里的 zip 归档、从 LRR 导入元数据，并通过只读 API 提供搜索、详情、页面清单、页面图片、缩略图和进度上报，供网页客户端在 3002 端口上与 LRR（3001 端口）并行使用。设计决策见仓库根目录的 `AGENTS.md`，本文件只规定任务顺序、范围和验收标准。

按下面的顺序逐个完成任务。每个任务结束时都要满足通用验收标准；任务之间的依赖已经体现在顺序里。

## 通用验收标准

- 所有命令在 flake 环境里运行：`nix develop -c cargo fmt --all`、`nix develop -c cargo clippy --all-targets -- -D warnings`、`nix develop -c cargo test`，三者都通过。
- 新行为先写测试。测试用临时目录和内存或临时 SQLite，不依赖网络和真实书库。
- 库 crate 的错误类型用 `thiserror`，可执行文件用 `anyhow`。生产代码不用 `unwrap`。
- 不修改 `shuku-index` 已有测试的期望；需要扩展它时，新增测试。
- 不提交（不运行 `git commit`）。任务完成后在最终回复里列出改动的文件、新增的依赖和尚未解决的问题。

## T1：`shuku-store`

SQLite 存储层，供服务端和迁移工具使用。

- 用 rusqlite（`bundled`）。连接时设置 `journal_mode=WAL`、`synchronous=NORMAL`、`foreign_keys=ON`。
- 迁移用 `PRAGMA user_version` 记录版本，迁移脚本按顺序嵌在代码里。不引入迁移框架。
- 表：
  - `archive`：`id INTEGER PRIMARY KEY AUTOINCREMENT`、`path`（相对内容目录，唯一）、`title`、`summary`、`file_size`、`mtime`、`quick_hash`、`content_hash`（可空）、`page_count`、`date_added`、`source_url`（可空）、`is_new`、`lrr_id`（可空，迁移时记录 LRR 的旧 ID）。
  - `tag`：`id`、`namespace`、`value`、`full`（小写的 `namespace:value`，唯一；没有命名空间时就是值本身）。
  - `archive_tag`：`(tag_id, archive_id)` 主键，`WITHOUT ROWID`，另建 `(archive_id, tag_id)` 索引；两个外键都 `ON DELETE CASCADE`。
  - `page`：`archive_id`、`page_no`（从 0 开始）、`entry_name`、`data_offset`、`compressed_size`、`size`、`method`（stored 或 deflate）、`width`、`height`；主键 `(archive_id, page_no)`。
  - `progress`：`archive_id` 主键、`page`、`client_time`、`updated_at`。
  - `category`：`id`、`name`、`search`（动态分类的搜索语句，静态分类为空）、`pinned`；`category_archive(category_id, archive_id)`。
  - `tankoubon`：`id`、`name`、`summary`；`tankoubon_archive(tankoubon_id, archive_id, position)`；`tankoubon_tag(tankoubon_id, tag_id)`。
- 接口至少包括：插入或更新一本归档及其页面（同一事务）、替换一本归档的标签、按 ID 读取归档详情和页面清单、按路径查找、写进度、按 ID 列表批量读取列表页元数据，以及流式读出构建索引所需的 `ArchiveRecord`。
- 写进度时只接受 `client_time` 比已有记录新的上报，返回是否被接受。
- 验收：上述每个接口都有测试；另有一个测试生成 1 万本、每本 10 个标签的数据，验证读出的 `ArchiveRecord` 能构建出与直接构建一致的索引。

## T2：`shuku-archive`（zip）

- 解析 zip 中央目录（可以用 `zip` crate，或只读中央目录的轻量实现），产出页面清单：只保留图片扩展名（jpg、jpeg、png、gif、webp、avif、jxl、bmp），忽略 `__MACOSX/`、`Thumbs.db`、`.DS_Store` 和 `._` 开头的文件，按自然序排序（数字按数值比较，不区分大小写）。
- 每页记录本地文件头之后的数据偏移、压缩大小、原始大小和压缩方式；读一页时直接定位读取，stored 原样返回，deflate 解压。其他压缩方式返回明确的“不支持”错误。
- 用 `imagesize` 读取每页的宽高，只读图片头部所需的字节。
- 快速指纹：文件大小加首尾各 64 KiB 的 BLAKE3；完整指纹：整文件 BLAKE3（流式）。
- 非 zip 格式（按魔数判断，不看扩展名）返回“不支持的格式”，扫描时跳过并记日志。
- 验收：测试用代码现场生成 zip（含 stored 和 deflate 条目、垃圾文件、`2.jpg` 与 `10.jpg` 的排序、损坏的条目），验证清单、读页结果和图片尺寸。

## T3：查询语法解析

把 LRR 风格的搜索语句解析成 `shuku-index::Query`，放在 `shuku-index` 里，因为它定义的是搜索语义。

- 逗号分隔词条，首尾空白忽略。
- 开头的 `-` 表示排除。
- 双引号包起来的词条，或以 `$` 结尾的词条，是精确匹配。
- `*` 和 `%` 是任意多个字符，`?` 和 `_` 是单个字符。M1 只需要支持词条末尾的 `*`/`%`（转成前缀匹配）。其他位置出现通配符时，在通配符处把词条切成几段，要求每段都按子串出现，并在解析结果里标记“已降级”，方便以后补全。这样得到的是真实匹配的超集，所以只用于包含条件；排除条件里出现这种通配符时返回“不支持”的错误。
- 反斜杠让下一个字符失去语法含义：`\"`、`\,`、`\-`、`\$` 和 `\*`、`\%`、`\?`、`\_` 都只当普通字符，`\\` 是一个反斜杠，词条末尾孤立的反斜杠按字面保留。这样客户端点击标签或补全时，值里含通配符字符的标签也能精确匹配。`:` 无论是否转义都是命名空间分隔符。
- 带命名空间（含 `:`）的精确词条解析成 `Term::Tag`，带命名空间的非精确词条解析成 `Term::TagPrefix`。不带命名空间的词条匹配标签名子串或标题子串，满足其一即可。
- 为了表达“其一”，给 `shuku-index` 增加 `Term::Any(Vec<Term>)`，语义是各子条件的并集，并补测试。
- `pages:>20`、`read:>0` 这类数值条件 M1 先解析成“不支持”的错误，不静默忽略。
- 验收：表驱动测试覆盖以上每条规则和空语句。

## T4：`shuku-server`（可执行文件 `shuku`）

- 配置来自命令行参数和环境变量：内容目录、数据目录（放 SQLite 和缓存）、网页端构建目录（默认 `web/build`）、监听地址（默认 `0.0.0.0:3002`）。
- 启动时打开数据库，从 `shuku-store` 读出记录构建索引，放进 `ArcSwap<Index>`。日志里打印构建耗时和本数，这个数字是待测指标之一。
- 扫描：`shuku scan` 子命令和启动时的后台扫描。遍历内容目录（rayon 并行），按路径和 `mtime`/大小判断新增或变化；新文件先算快速指纹，若与数据库中某条路径已不存在的记录相同，视为移动并更新路径。扫描结束后重建索引并换入。
- API 的路径、参数和 JSON 格式以 `docs/api.md` 为准，网页端按同一份文档实现。下面只列要点：
  - `GET /api/search?q=&sort=title|date&after=&limit=`：返回 `total`、`next_after` 和列表项（ID、标题、页数、进度、封面 URL、是否新）。`limit` 上限 200。
  - `GET /api/archives/{id}`：详情，含标签（按命名空间分组）和摘要。
  - `GET /api/archives/{id}/pages`：页面清单，每页含宽、高和图片 URL。
  - `GET /api/archives/{id}/pages/{n}`：页面原图。URL 里带内容指纹（例如 `?v=<quick_hash>`），响应带 `Cache-Control: public, max-age=31536000, immutable`、`ETag`，并处理 `If-None-Match`。`?w=` 时返回缩小到该宽度的 JPEG（只缩不放），结果缓存到数据目录。
  - `GET /api/archives/{id}/thumbnail`：封面缩略图，宽 500，JPEG，缓存到数据目录，缓存头同上。
  - `PUT /api/archives/{id}/progress`：请求体 `{"page": n, "client_time": unix_ms}`，按 T1 的规则写入，返回是否被接受以及服务端当前的进度。
  - `/api` 以外的路径从网页端构建目录提供静态文件，找不到时返回 `index.html`。
- 图片缩放用 `image` 加 `fast_image_resize`，不依赖系统库。解码和缩放放在 `spawn_blocking` 里。
- 读页、缩放、扫描都不能阻塞 tokio 的 worker 线程。
- 验收：用临时内容目录和现场生成的 zip 做集成测试，覆盖扫描、搜索分页、页面清单、读页（含 304）、缩略图和进度规则。

## T5：`shuku-migrate-lrr`

从 LRR 的 Redis 把元数据导入 Shuku 的 SQLite。必须幂等：可以反复运行，每次以 LRR 为准覆盖已导入的数据。

LRR 的 Redis 结构（以 LRR master 源码为准，`lib/LANraragi/Utils/Database.pm`、`Model/Category.pm`、`Model/Tankoubon.pm`；拿不准时去读源码）：

- 主库（默认 db 0）里，每本书是一个以 40 位十六进制 ID 为键的哈希，字段有 `title`、`tags`（逗号分隔）、`summary`、`file`（绝对路径）、`isnew`（`"true"`/`"false"`）、`progress`、`pagecount`、`lastreadtime`、`arcsize`。字符串是 UTF-8，部分数据可能被重复编码过一次，LRR 自己用“先尝试 UTF-8 解码，再解码一次”的方式读取，导入时照做。
- `tags` 里的 `date_added:<unix 秒>` 写入 `archive.date_added`，`source:<url>` 写入 `archive.source_url`，二者都不进标签表。
- 分类是 `SET_<10 位数字>` 哈希：`name`、`search`（非空表示动态分类）、`pinned`、`archives`（ID 的 JSON 数组）。
- Tankoubon 是 `TANK_<10 位数字>` 有序集合：分值 0 的成员是 `name_<名字>`，-1 是 `summary_<摘要>`，-2 是 `tags_<逗号分隔标签>`，-3 是 `progress_<页>`；分值 ≥ 1 的成员是归档 ID，分值就是顺序。
- 归档按文件路径对应：LRR 的 `file` 去掉内容目录前缀后，与 `archive.path` 匹配。命令行参数提供 Redis 地址、db 编号、LRR 的内容目录前缀（LRR 跑在 Docker 里时路径前缀与本机不同）。
- 找不到对应文件的 LRR 记录不导入，最后汇总报告数量和前若干个路径。
- 迁移进度：LRR 的 `progress` 从 1 开始计，0 或空表示没读过，写入时减 1；`client_time` 用 `lastreadtime`（秒转毫秒）。
- 分类和 Tankoubon 在 Shuku 里记录 LRR 的 ID（`lrr_id` 列），重复导入时更新同一行；在 Shuku 里新建的不受影响。Tankoubon 的整套阅读进度（分值 -3）不导入。
- 读取 Redis 时用 `SCAN`，不用 `KEYS`。
- 验收：Redis 交互放在一个 trait 后面，测试用内存实现覆盖上述每条映射规则和幂等性；真实 Redis 的端到端运行留给用户手动执行。

## 不在 M1 后端范围内

固实 7z、RAR、tar、PDF；页级指纹；认证；OPDS；标签编辑等写操作；文件监视；插件。
