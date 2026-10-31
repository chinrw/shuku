# HTTP API（M1）

服务端和网页客户端共同遵守本文件定义的接口。所有接口都在 `/api` 下，请求和响应都是 JSON（图片接口除外）。出错时返回合适的状态码和 `{"error": "说明"}`。M1 没有认证；Host 不是 IP、`localhost`、不带点的主机名或 `--allowed-host` 里的名字时，所有路径都返回 421（防 DNS rebinding）。

ID 都是 SQLite 的归档主键，JSON 里用数字表示。时间戳都是 Unix 毫秒，只有 `date_added` 是 Unix 秒（沿用 LRR 的数据）。

## 共用对象

`Progress`：

```json
{ "page": 12, "client_time": 1761800000000 }
```

`page` 从 0 开始。没有进度时字段为 `null`。

`ArchiveItem`（列表项）：

```json
{
  "id": 42,
  "title": "作品名 第1卷",
  "page_count": 180,
  "is_new": false,
  "progress": { "page": 12, "client_time": 1761800000000 },
  "cover_url": "/api/archives/42/thumbnail?v=<quick_hash>",
  "date_added": 1700000000,
  "source_url": null,
  "tags": [
    { "namespace": "", "values": ["独立标签"] },
    { "namespace": "artist", "values": ["某某"] }
  ]
}
```

列表项带上全部标签和加入时间、来源，网页端在列表和标签弹窗里直接显示，不用逐本请求详情。LRR 把加入时间和来源存成 `date_added`、`source` 标签，Shuku 存在归档表的列里，显示时由网页端还原成这两行。`tags` 的格式与归档详情相同。

`cover_url` 和页面 URL 都由服务端生成，客户端原样使用，不自己拼接。URL 里的 `v` 是归档的快速指纹，文件内容变化后 URL 随之变化，所以图片响应可以永久缓存。

## 搜索

`GET /api/search?q=&category=&filter=&sort=&order=&after=&limit=`

- `q`：LRR 风格的搜索语句，语法见 `docs/plan/m1-backend.md` 的 T3。为空时返回全部。和 LRR 一样支持 `pages:` 和 `read:` 数值条件：后面跟 `>`、`<`、`>=`、`<=`、`=` 或不带运算符的非负整数，例如 `pages:>20`、`read:0`，前面加 `-` 表示排除。`read:` 比较的是读到的页码，从 1 开始，没读过是 0，所以 `read:0` 是没读过的书。
- `category`：分类 ID（见下面的分类接口），只返回这个分类里的归档，和 `q` 同时生效。静态分类按成员筛选；动态分类按它的搜索语句筛选，语句 Shuku 还不能解析时这个分类为空。分类不存在时返回 400。
- `filter`：书库页的快捷筛选，逗号分隔，同时生效。`new` 是新入库还没读过的书（`is_new` 为真且没有进度），`unread` 是没读过的，`reading` 是有进度但还没读到最后一页的，`finished` 是读到了最后一页的，`untagged` 是一个标签都没有的。其他值返回 400。
- `sort`：`title`（默认，标题自然序）、`date`（加入时间）、`pages`（页数）或 `ns:<命名空间>`（按这个命名空间下的标签值自然序，例如 `ns:artist` 按作者）。按命名空间排序时，一本有几个值取自然序最小的那个，没有这个命名空间的在升序时排在最后，同值按标题；`order` 反向时整个排列倒过来，没有这个命名空间的排到最前，同值的标题次序也倒过来。某个命名空间第一次用来排序时要先算一遍排列（30 万本的合成数据上约 60 ms），之后缓存到快照重建。其他值返回 400。
- `order`：`asc` 或 `desc`。不给时加入时间是新的在前，其余从小到大。
- `after`：上一页响应里的 `next_after`，第一页不传。
- `limit`：默认 50，上限 200。

响应：

```json
{
  "total": 1234,
  "next_after": 99,
  "degraded": false,
  "items": [ArchiveItem]
}
```

`next_after` 为 `null` 表示没有下一页。`degraded` 为 `true` 表示语句里有 M1 还不支持的通配符位置，已经降级为分段子串匹配，结果可能比预期多，客户端可以提示用户。语句有错误或用到还不支持的写法（例如 `pages:!=20`、`rating:>3` 这类比较，或排除条件中间的通配符）时返回 400。`after` 指向的归档已不存在时返回 400，客户端应从第一页重新开始。

搜索结果和列表项来自服务端的内存快照。上报阅读进度后，列表项里的 `progress` 和 `read:`、阅读状态筛选的结果约 10 秒后才更新；归档详情直接读数据库，立即反映新进度。

## 分类

`GET /api/categories`

书库页搜索框下面的一排按钮，和 LRR 首页一样是用户自己的分类（从 LRR 导入），置顶的在前，再按名字排。`count` 是分类里的本数，和搜索结果一样来自内存快照。

```json
{ "categories": [{ "id": 2, "name": "收藏", "pinned": true, "dynamic": false, "count": 812 }] }
```

下面几个接口修改分类。M1 没有认证，它们和进度上报一样只在局域网里用；和控制台一样，`Sec-Fetch-Site` 不是 `same-origin` 或 `none`、或者 `Origin` 与 `Host` 不一致的请求返回 403。写入提交后服务端立即刷新快照里的分类，紧接着的分类列表和按分类搜索已经是新的。

`POST /api/categories`：新建分类，请求体 `{"name": "收藏", "search": "", "pinned": false}`，`search` 和 `pinned` 可省略。`name` 去掉首尾空白后不能为空，最多 100 个字。`search` 非空是动态分类，语句要能被搜索接口解析，否则返回 400。成功返回 201 和 `{"id": 7}`。名字可以重复。

`PATCH /api/categories/{id}`：改 `name`、`search`、`pinned` 中给出的字段，校验同上，成功返回 204。静态分类和动态分类不能互相转换：给静态分类设非空的 `search`、或者把动态分类的 `search` 设为空，返回 400。

`DELETE /api/categories/{id}`：删除分类和它的成员关系，归档本身不动，返回 204。

`PUT /api/categories/{id}/archives/{archive}`、`DELETE /api/categories/{id}/archives/{archive}`：把归档加进静态分类或移出，返回 204。已经在（或已经不在）分类里时同样返回 204。动态分类返回 400。

以上接口在分类或归档不存在时返回 404。迁移工具重新导入 LRR 的分类时，会按 LRR 的现状整体替换那些从 LRR 来的分类的成员，在 Shuku 里对它们做的增删会被覆盖；在 Shuku 里新建的分类不受影响。

## 标签统计和补全

`GET /api/facets?q=&category=&filter=&limit=`

返回搜索语句 `q` 的结果里出现最多的标签。`q`、`category`、`filter` 的语法和错误处理与搜索相同。`limit` 默认 20，范围 1 到 100。网页端的书库页现在显示分类，不再用这个接口。

`GET /api/tags?prefix=&limit=`

搜索框补全用，返回完整名或冒号后的值以 `prefix` 开头的标签（按小写匹配），按全库的本数排序。`prefix` 必填，为空时返回空列表。`limit` 默认 10，范围 1 到 100。

两个接口的响应格式相同，按本数从多到少、同数时按标签名排列。`tag` 是小写的完整标签名，没有命名空间的标签不带冒号：

```json
{ "tags": [{ "tag": "artist:某某", "count": 12 }] }
```

## 归档详情

`GET /api/archives/{id}`

```json
{
  "id": 42,
  "title": "作品名 第1卷",
  "summary": "",
  "page_count": 180,
  "is_new": false,
  "date_added": 1700000000,
  "source_url": null,
  "file_size": 123456789,
  "path": "作品/作品名 第1卷.cbz",
  "missing": false,
  "progress": null,
  "cover_url": "/api/archives/42/thumbnail?v=<quick_hash>",
  "categories": [2],
  "tags": [
    { "namespace": "", "values": ["独立标签"] },
    { "namespace": "artist", "values": ["某某"] }
  ]
}
```

`categories` 是这本书所在的静态分类的 ID，按 ID 升序；动态分类不列出。

`tags` 按命名空间分组，组按命名空间名排序，没有命名空间的标签放在 `namespace` 为空字符串的组里。

`path` 是相对内容目录的路径。`missing` 为 `true` 表示文件已经不在内容目录里，这条记录可以删除；无法确认时（例如所在目录没有读权限）按还在处理。

## 编辑和删除

下面几个接口修改数据。和分类接口一样还没有认证，只在局域网里用，跨站请求返回 403。

`PATCH /api/archives/{id}`

改一本书的标题、简介、来源或标签，只改给出的字段，成功时返回改完的详情（格式同上）。

```json
{ "title": "新标题", "summary": "简介", "source_url": "exhentai.org/g/1/abc",
  "tags": ["artist:某某", "language:chinese", "独立标签"] }
```

- `title` 去掉首尾空白后不能为空，最多 1000 个字。`summary` 最多 2 万字，原样保存。
- `source_url` 去掉首尾空白后为空字符串时清除来源；不要求带协议，LRR 的来源常常写成 `exhentai.org/g/…`。中间不能有空白，最多 2000 个字。
- 标签写成 `命名空间:值`，在第一个冒号处分开，没有冒号的是不带命名空间的标签，冒号两边和首尾的空白去掉。值不能为空，标签里不能有英文逗号（LRR 和网页端的编辑框都用它分隔）和换行等控制字符，每个最多 500 个字。按小写去重，已有标签保留第一次写入时的大小写；只改大小写要用下面的改名接口。
- `tags` 整体替换全部标签。`add_tags`、`remove_tags` 只加或只删，其余标签不变，适合插件补标签；它们不能和 `tags` 同时给出。每个数组最多 1000 项。
- 不认识的字段、校验失败返回 400，归档不存在返回 404。

详情立即是新的；搜索结果和列表项来自内存快照，约 10 秒后更新。被换掉的标签不再有归档用到时从标签表里删除，不再出现在补全里。

`DELETE /api/archives/{id}`

删除文件已经不在内容目录里的记录，连同阅读进度、分类和 Tankoubon 里的成员关系，原始文件不动，编号以后也不会再分给别的书。成功返回 204，记录不存在返回 404。下列情况返回 409，什么都不删：

- 文件还在（删了记录，下次扫描会把它当作新书重新入库，元数据就丢了）。
- 扫描正在进行：扫描开始时读出的失踪记录可能正要被改了名的同一个文件认领。
- 内容目录是空的：网络盘没挂载时每本书都像是丢了。

`POST /api/admin/missing/delete`

一次删除多条，请求体 `{"ids": [42, 43]}`，最多 1000 个，在一个事务里删除。条件同上，文件还在的和已经没有的不删，分别列出。三个列表按请求里的顺序，重复给出的 ID 只删一次，但照样列出：

```json
{ "deleted": [42], "present": [43], "not_found": [] }
```

删掉的记录同时从问题页的失踪列表里拿掉。扫描进行中或内容目录为空时整批返回 409。

`POST /api/admin/tags/rename`

请求体 `{"from": "artist:alfa", "to": "artist:alpha"}`，写法同上。`to` 已经存在时两个标签合并，原来带 `from` 的归档和 Tankoubon 都改带 `to`；两个名字只差大小写时只改显示形式。返回 `{"archives": 12}`，即原来用到 `from` 的本数。`from` 不存在时返回 404。改名后立即重建内存快照，搜索和标签统计马上是新的。

## 页面清单

`GET /api/archives/{id}/pages`

```json
{
  "pages": [
    {
      "width": 1200,
      "height": 1700,
      "broken": false,
      "url": "/api/archives/42/pages/0?v=<quick_hash>",
      "thumbnail_url": "/api/archives/42/pages/0/thumbnail?v=<quick_hash>"
    }
  ]
}
```

`thumbnail_url` 是这一页的缩略图，详情页的页面总览用。

响应另有 `"webtoon": true/false`：页面高宽比的中位数达到 2 时为 `true`，网页端据此把这本当条漫，用上下连续滚动阅读。服务端用 `--no-webtoon-detection` 启动时总是 `false`。阅读器里的手动开关存在浏览器里，优先于这个值。

`broken` 为 `true` 的是读不出尺寸的坏页（全是 0 的图、0 字节的文件、改了扩展名的别的文件），这时 `width` 和 `height` 是 1000×1414 的占位值，请求它的图片通常返回 422。和 LRR 一样，有坏页的书照样入库。

## 图片

`GET /api/archives/{id}/pages/{n}?v=<quick_hash>[&w=<宽度>]`

返回第 `n` 页（从 0 开始）的图片。不带 `w` 时返回原始字节，`Content-Type` 按图片格式设置。带 `w` 时返回缩到该宽度的 JPEG，原图不比 `w` 宽时返回原图；缩放结果缓存在数据目录。`w` 只接受 720、1080、1440、2160 这几档，其他值返回 400，避免同一页缓存过多变体。超过 6400 万像素的图片不缩放，请求 `w` 或缩略图时返回 422。认不出格式的坏页也返回 422。

`GET /api/archives/{id}/thumbnail?v=<quick_hash>`

返回封面缩到 500 宽的 JPEG。封面是第一张不是坏页的页，通常就是第 0 页。

`GET /api/archives/{id}/pages/{n}/thumbnail?v=<quick_hash>`

返回第 `n` 页缩到 500 宽的 JPEG，规则和封面相同，第 0 页与封面共用同一份缓存。

两个图片接口都带 `Cache-Control: public, max-age=31536000, immutable` 和 `ETag`，请求带匹配的 `If-None-Match` 时返回 304。

## 下载原文件

`GET /api/archives/{id}/file`

返回原始归档文件，带 `Content-Disposition: attachment` 和原文件名（`filename*` 是 UTF-8 原名）。`Content-Type` 固定为 `application/octet-stream`，免得浏览器按类型改扩展名。支持 `Range` 断点续传（`If-Range` 只认响应里的 `Last-Modified`，不一致时返回整个文件）和 `If-Modified-Since`；Range 越界时返回 416，带 `Content-Range: bytes */<长度>`；不带内容指纹，所以不加永久缓存头。归档不存在或文件已经不在内容目录里时返回 404。

## 阅读进度

`PUT /api/archives/{id}/progress`

请求体：

```json
{ "page": 13, "client_time": 1761800005000 }
```

`client_time` 是客户端翻到这一页的时间。服务端只接受比已有记录新的上报。响应带回服务端当前的进度，客户端据此判断自己是否落后于别的设备：

```json
{ "accepted": true, "progress": { "page": 13, "client_time": 1761800005000 } }
```

`page` 超出页数时返回 400，归档不存在时返回 404。

服务端把一秒内到达的上报合并成一次提交，响应在写入提交后才返回，所以最多要等约 1 秒。收到响应的上报已经写进数据库。

`GET /api/random?q=&category=&filter=`

在和搜索相同的条件下随机挑一本，返回 `{"id": 42}`；没有符合条件的书时返回 404。参数的语法和错误处理与搜索相同。

`GET /api/recent?limit=`

最近读过的归档，按 `progress.client_time` 从新到旧，网页端书库首页的“最近阅读”一栏用它。`limit` 默认 20，范围 1 到 100。从 LRR 导入的进度用 LRR 的 `lastreadtime` 作为翻页时间，所以导入后就有内容。和搜索一样读内存快照，新上报的进度约 10 秒后出现。

```json
{ "items": [ArchiveItem] }
```

## 控制台

网页端的 `/admin` 页用下面这些接口。M1 没有认证，它们和其他接口一样只在局域网里用；删除失踪记录和标签改名见上面的“编辑和删除”。为了不让别的网页借浏览器触发任务，POST 和 DELETE 请求的 `Sec-Fetch-Site` 不是 `same-origin` 或 `none`、或者 `Origin` 与 `Host` 不一致时返回 403；两个头都没有的请求（curl 等）照常处理。

`GET /api/admin/status`

控制台每 5 秒请求一次。SQLite 的计数和内存快照各读一次，不逐本查询。

```json
{
  "server": { "version": "0.1.0", "started_at": 1761800000000, "content_dir": "/srv/comics",
              "data_dir": "/var/lib/shuku", "web_dir": "web/build", "listen": "0.0.0.0:3002" },
  "library": { "archives": 58756, "pages": 1912340, "bytes": 2001000000000, "hashed": 31020,
               "lrr_imported": 58548, "lrr_pending": 76403, "with_progress": 767,
               "read_last_week": 12, "categories": 3, "tankoubons": 0,
               "tags": 41207, "namespaces": 23 },
  "storage": { "database_bytes": 432013312, "wal_bytes": 3250000,
               "cache_bytes": 2576980378, "cache_files": 18302 },
  "snapshot": { "built_at": 1761800000000, "archives": 58756, "read_ms": 31, "build_ms": 58 },
  "scan": { "running": true, "started_at": 1761790000000, "finished_at": null,
            "total": 137012, "processed": 61880,
            "report": { "added": 3124, "updated": 0, "moved": 0, "unchanged": 58704, "skipped": 12, "failed": 40 },
            "error": null,
            "issues": { "failed": 37, "skipped": 12, "unreadable": 3, "missing": 0 } },
  "hash": { "running": false, "started_at": null, "finished_at": null, "total": 0,
            "report": { "hashed": 0, "skipped": 0, "failed": 0 }, "error": null }
}
```

- `library.tags` 和 `namespaces` 只数至少被一本用到的标签；`read_last_week` 是翻页时间在最近 7 天内的本数。
- `storage.cache_*` 是 `数据目录/cache/images` 里的文件，每分钟最多统计一次。
- `scan` 是正在进行或服务启动以来最近一次的扫描。`total` 在遍历完内容目录后才有值；`issues` 是各类问题的数量，明细见下面的接口。`hash` 同理，`total` 是这轮开始时还没有完整指纹的本数。服务重启后两者都从空开始。

`GET /api/admin/issues`

当前或最近一次扫描遇到的问题，每类最多列 1000 条，`total` 是实际数量：

```json
{
  "scanning": false, "finished_at": 1761800000000,
  "failed": { "total": 1, "items": [{ "path": "a/broken.zip", "error": "zip 结构损坏" }] },
  "skipped": { "total": 1, "items": [{ "path": "b/book.rar" }] },
  "unreadable": { "total": 0, "items": [] },
  "missing": { "total": 1, "items": [{ "id": 42, "path": "c/gone.zip", "title": "已删除的书" }] },
  "broken": { "total": 1, "items": [{ "id": 7, "path": "d/book.zip", "title": "标题", "pages": ["01.jpg"] }] }
}
```

路径都相对内容目录。`unreadable` 是没有读权限的子目录；`missing` 是库里有记录、文件已经不在的归档，`title` 取自快照，快照里没有时为 `null`。`broken` 是有坏页的书和坏页的条目名，存在库里（`archive.broken_pages`），不随扫描清空；状态接口的 `scan.issues.broken` 是它的本数。

`GET /api/admin/duplicates?limit=`

完整指纹相同的归档，每组按路径排列。`limit` 是最多返回的组数，默认 100，范围 1 到 500。还没算出完整指纹的归档不参与。

```json
{ "total": 214, "groups": [{ "files": [{ "id": 7, "path": "a.zip", "title": "标题", "file_size": 43200000 }] }] }
```

`GET /api/admin/namespaces`

每个命名空间的标签数和使用次数，按使用次数从多到少。命名空间不分大小写，空字符串是没有命名空间的标签。某个命名空间下最常用的标签用 `GET /api/tags?prefix=<命名空间>:` 取。

```json
{ "namespaces": [{ "namespace": "artist", "tags": 18240, "uses": 61377 }] }
```

`GET /api/admin/logs?level=&after=&limit=`

进程内最近 1000 条日志，按时间顺序。`level` 是 `info`（默认，全部）、`warn`（警告和错误）或 `error`；`after` 是上次取到的最后一条的 `seq`，只返回更新的；`limit` 默认 200，范围 1 到 1000，超出时返回最新的那些。

```json
{ "entries": [{ "seq": 812, "time": 1761800000000, "level": "warn", "message": "扫描文件失败 path=… error=…" }],
  "started_at": 1761790000000 }
```

`seq` 每次启动从 1 开始；`started_at` 标识服务端进程，和上次不同时说明服务重启过，客户端要丢掉旧的 `after` 从头取。

`GET /api/admin/settings/scan`、`PUT /api/admin/settings/scan`

控制台设置页改的扫描设置，存在数据库的 `setting` 表里（键 `scan`），所有设备共用：

```json
{ "scan_hidden": false, "scan_interval_hours": 24 }
```

`scan_hidden` 为 `true` 时也扫描以 `.` 开头的目录和文件。`scan_interval_hours` 是定时扫描的间隔，从上一次扫描结束算起，0 表示不定时，最大 720。PUT 只需带要改的项，未知字段或超出范围返回 400，响应是改后的全部设置。

`POST /api/admin/scan`、`POST /api/admin/hash`

在后台开始扫描（结束后接着补算指纹，和服务启动时一样）或补算指纹，立即返回 202 和 `{"started": true}`。同一种任务已经在运行时返回 409。进度从状态接口看。

`POST /api/admin/refresh`

立即按数据库重建内存快照，返回 `{"snapshot": {...}}`，格式同状态接口的 `snapshot`。服务运行时用迁移工具导入后可以用它让搜索结果马上更新。

`DELETE /api/admin/cache`

删除图片缓存里的文件，目录本身保留，返回 `{"removed_files": 18302, "removed_bytes": 2576980378}`。图片之后按需重新生成。

## 静态文件

`/api` 以外的路径由服务端从网页端的构建目录（`web/build`）提供。找不到对应文件时返回 `index.html`，交给客户端路由处理。
