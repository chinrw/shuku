# Shuku 项目说明

Shuku（拼音“书库”）是一个自用的漫画归档服务，用 Rust 重写，用来取代 LANraragi（LRR）。本文件记录已经定下的设计决策和开发约定，是本仓库唯一的权威来源。设计讨论的完整记录在作者的 claude.ai 文档里，代码代理无法访问，所以凡是实现需要的决定都必须写在这里或 `docs/` 下。

与用户交流、写文档和注释时使用中文；标识符、提交信息用英文。

## 开发环境

所有构建、测试和工具命令都在仓库的 Nix flake 开发环境里运行，不使用本机的 rustup 或系统 rustc：

```bash
nix develop -c cargo test
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo fmt --all
```

需要新的系统库（例如 libvips）时，加进 `flake.nix` 的 `packages`。提交前必须通过 fmt、clippy（`-D warnings`）和全部测试。

## 当前进度

- M1 后端的 T1 到 T5（`docs/plan/m1-backend.md`）都已实现并有测试：存储、zip 读取、查询解析、服务端 `shuku`、LRR 迁移工具 `shuku-migrate-lrr`。运行方式见 `docs/server.md`，接口见 `docs/api.md`。
- 网页端（`web/`）有书库、搜索、详情和阅读器（翻页和滚动两种模式），已在本地生成的测试书库上跑通。
- 还没做的：
  - 用真实书库扫描，并在真实 LRR 的 Redis 上运行迁移。
  - 文件被删除后，记录仍留在库里，仍会出现在搜索结果中，删除处理放到 M2。
  - 完整 BLAKE3 指纹的后台计算，目前 `content_hash` 都为空。

## 前提

- 只给一个人自用。不兼容 LRR 的 Perl 插件和 HTTP API，不保留 LRR 的归档 ID，不考虑 Mihon 等现有客户端；客户端自己做。
- LRR 的元数据（标签、标题、分类、Tankoubon、阅读进度）要迁移过来，迁移完成后下线 LRR。
- 书库约 30 万本，每本约 10 个标签。归档格式绝大部分是 zip（含 cbz），固实的 7z 和 RAR 很少。
- 部署在局域网内，用 HTTP 直连，不配置 HTTPS。

## 架构

服务端是单个 Rust 进程（axum + tokio）。SQLite（WAL）是唯一数据源；搜索走常驻内存的 roaring bitmap 索引（`shuku-index`），启动时从 SQLite 构建，不落盘。慢操作放进 `spawn_blocking` 或 rayon 线程池。网页客户端的静态文件由服务端托管，与 API 同源。后台任务用 tokio task 加一张 SQLite 任务表，文件监视用 notify。

Cargo workspace 按层拆分，crate 都放在 `crates/` 下：

| crate | 职责 |
| --- | --- |
| `shuku-index` | 内存位图索引与搜索（已实现） |
| `shuku-store` | SQLite 表结构、迁移、读写接口 |
| `shuku-archive` | 归档读取、页面清单、图片尺寸、内容指纹 |
| `shuku-server` | HTTP API、扫描、后台任务；可执行文件 `shuku` |
| `shuku-migrate-lrr` | 从 LRR 的 Redis 导入元数据的一次性工具 |
| `shuku-types` | 服务端与客户端共享的 API 类型（有客户端需要时再建） |
| `shuku-client` | 客户端核心，供 Android 和 Tauri 使用（M4 再建） |

## 数据库与搜索

选择 SQLite 加内存位图索引的依据是在 30 万本合成数据上对 SQLite、PostgreSQL、Redis、DuckDB、Tantivy 和位图索引的实测：位图索引 16 个典型查询的第一页最慢 2.7 ms，中位数约 0.01 ms，计数几乎为零；PostgreSQL（int[] + GIN）中位数 1.1 ms 但最慢 301 ms，SQLite 最慢 441 ms。数据库的最慢情况都来自“两个大标签交集为空”（例如日语 ∩ 中文）：规划器假设标签互相独立，标签相关时会选错执行策略。位图索引先算出确切交集再决定排序方式，所以不受影响。

SQLite 的约定：

- 归档主键用 `INTEGER PRIMARY KEY AUTOINCREMENT`（64 位），删除后编号不复用；URL 和客户端缓存都用它。`shuku-index` 内部另有一套稠密 u32 编号，不对外暴露。
- 标签单独成表，命名空间和值分开存，另存小写的完整名用于唯一约束。归档与标签的关系表用 `WITHOUT ROWID`，主键 `(tag_id, archive_id)`，另建 `(archive_id, tag_id)` 索引。
- `date_added`、来源 URL 等每本书唯一的值不存成标签，作为归档表的列。LRR 把它们存成标签，30 万本会因此多出约 60 万个标签。
- SQLite 侧只为单本读写和构建索引建常规索引，不为复杂搜索做优化；搜索全部走 `shuku-index`。
- 用 rusqlite 并开启 `bundled` 特性。写入先在 SQLite 事务里提交，再更新内存索引。

`shuku-index` 的行为（已实现，修改时保持这些语义）：

- 标签名和标题都按小写匹配。条件有精确标签、标签前缀、标签名子串、标题子串四种；包含条件取交集，排除条件取差集，没有包含条件时匹配全部。
- 标题子串在一块连续的小写标题内存上用 memmem 扫描，两个汉字的检索词同样可用，不使用 FTS5。
- 排序有标题自然序和最新加入两种，构建时预先算好排列。结果稠密时沿排列遍历，稀疏时按名次部分排序。
- 分页用游标：`after` 是上一页最后一本的归档 ID。游标指向已不存在的归档时返回 `UnknownCursor`。
- 索引构建后不可变。数据变化后由服务端重建，并用 `ArcSwap` 换入新快照。30 万本从内存数据构建约 0.37 s。

## 归档读取

- 原始归档文件始终不改动，也不重新打包。所有派生数据（缩略图、解压缓存、图片变体）放在服务自己的数据目录里，与内容目录分开。
- zip 是主路径：入库时解析一次中央目录，记录每个图片条目的偏移、压缩方式和大小，读一页只做一次定位读取。不压缩存储的条目直接按文件区间返回，deflate 条目解压后返回。
- 页面排序用自然序；忽略 `__MACOSX/`、`Thumbs.db`、`.DS_Store` 和以 `._` 开头的 AppleDouble 文件。
- 固实 7z、RAR、tar 系列和 PDF 不在 M1 范围内。以后实现时，固实包在后台从块头流式解压、边解边写进磁盘缓存，非固实的按条目直接读。
- 入库时用 imagesize 只读图片头部，记录每页的宽和高。

## 身份

- 代理键：上面的自增主键。
- 内容指纹：整文件 BLAKE3，后台计算；入库时先用“文件大小 + 首尾各 64 KiB 的 BLAKE3”作为快速指纹。用于去重，以及文件移动或改名后重新关联到原记录。
- 页级指纹（每页内容哈希加感知哈希）用于重新打包后认出原书，不在 M1 范围内。

## API 原则

- 页面清单返回每页的宽和高，让客户端一开始就能排好布局。
- 页面和缩略图的 URL 包含归档的内容指纹，响应带 `Cache-Control: public, max-age=31536000, immutable` 和 ETag。
- 图片可以按宽度请求（`?w=1170`），只缩不放，结果缓存到数据目录。
- 搜索结果直接带列表页需要的元数据（标题、页数、进度、封面 URL），用游标分页。
- 阅读进度：客户端上报页码时附带自己的翻页时间戳，服务端只接受比已有记录更新的上报。LRR 按请求的到达顺序覆盖，离线设备补传旧进度时会盖掉新进度。
- 认证：每台设备一个可单独吊销的令牌。M1 只在局域网使用，认证放到 M2。
- OPDS（含 PSE 扩展）要做，作为通用阅读器的保底入口，放在 M2。

## 客户端（服务端设计时需要知道的部分）

- 网页端是主力，覆盖 iOS 和桌面：TypeScript + Svelte 5，SvelteKit `adapter-static`、关闭 SSR。因为不用 HTTPS，网页端不使用 Service Worker，重复浏览依赖 HTTP 缓存头，所以缓存头必须正确。
- Android 原生客户端（Kotlin + Compose）在 M4，离线阅读由它和桌面 Tauri 负责。

## 插件

不兼容 LRR 的 Perl 插件。以后定义 `MetadataProvider`、`Downloader`、`LoginProvider` 等 trait，每个站点一个 Rust 实现；另提供子进程 JSON 协议，可以包装 gallery-dl。不在 M1 范围内。

## 路线

1. M1：存储、索引、zip 读取、扫描、LRR 迁移工具、只读 API 和进度上报；网页端列表、搜索、详情、阅读器。期间书库管理仍用 LRR。
2. M2：标签编辑、分类、Tankoubon、文件监视、后台任务、插件、认证、OPDS、网页端管理页。
3. M3：切换并下线 LRR。
4. M4：Android 原生客户端；桌面 Tauri 按需。

新服务在另一个端口与 LRR 并行运行，读同一个内容目录，只写自己的数据目录。LRR 下线前用它做差分测试：逐本对比页面列表，对比常用查询的结果和耗时。

## 待定事项

- 书库总大小，用来估算缩略图和缓存空间。
- 首批内置插件清单。
- LRR 下线的时间点。
- 用真实书库的标签分布重跑搜索基准，并测量从 SQLite 冷启动构建索引的耗时。
