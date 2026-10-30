# HTTP API（M1）

服务端和网页客户端共同遵守本文件定义的接口。所有接口都在 `/api` 下，请求和响应都是 JSON（图片接口除外）。出错时返回合适的状态码和 `{"error": "说明"}`。M1 没有认证。

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
  "tags": [
    { "namespace": "", "values": ["独立标签"] },
    { "namespace": "artist", "values": ["某某"] }
  ]
}
```

列表项带上全部标签，网页端在列表里直接显示，不用逐本请求详情。`tags` 的格式与归档详情相同。

`cover_url` 和页面 URL 都由服务端生成，客户端原样使用，不自己拼接。URL 里的 `v` 是归档的快速指纹，文件内容变化后 URL 随之变化，所以图片响应可以永久缓存。

## 搜索

`GET /api/search?q=&sort=&after=&limit=`

- `q`：LRR 风格的搜索语句，语法见 `docs/plan/m1-backend.md` 的 T3。为空时返回全部。
- `sort`：`title`（默认，标题自然序）或 `date`（最新加入在前）。
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

`next_after` 为 `null` 表示没有下一页。`degraded` 为 `true` 表示语句里有 M1 还不支持的通配符位置，已经降级为分段子串匹配，结果可能比预期多，客户端可以提示用户。语句有错误或用到还不支持的写法（例如 `pages:>20` 这类数值条件，或排除条件中间的通配符）时返回 400。`after` 指向的归档已不存在时返回 400，客户端应从第一页重新开始。

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
  "progress": null,
  "cover_url": "/api/archives/42/thumbnail?v=<quick_hash>",
  "tags": [
    { "namespace": "", "values": ["独立标签"] },
    { "namespace": "artist", "values": ["某某"] }
  ]
}
```

`tags` 按命名空间分组，组按命名空间名排序，没有命名空间的标签放在 `namespace` 为空字符串的组里。

## 页面清单

`GET /api/archives/{id}/pages`

```json
{
  "pages": [
    { "width": 1200, "height": 1700, "url": "/api/archives/42/pages/0?v=<quick_hash>" }
  ]
}
```

## 图片

`GET /api/archives/{id}/pages/{n}?v=<quick_hash>[&w=<宽度>]`

返回第 `n` 页（从 0 开始）的图片。不带 `w` 时返回原始字节，`Content-Type` 按图片格式设置。带 `w` 时返回缩到该宽度的 JPEG，原图不比 `w` 宽时返回原图；缩放结果缓存在数据目录。`w` 只接受 720、1080、1440、2160 这几档，其他值返回 400，避免同一页缓存过多变体。超过 6400 万像素的图片不缩放，请求 `w` 或缩略图时返回 422。

`GET /api/archives/{id}/thumbnail?v=<quick_hash>`

返回封面（第 0 页）缩到 500 宽的 JPEG。

两个图片接口都带 `Cache-Control: public, max-age=31536000, immutable` 和 `ETag`，请求带匹配的 `If-None-Match` 时返回 304。

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

## 静态文件

`/api` 以外的路径由服务端从网页端的构建目录（`web/build`）提供。找不到对应文件时返回 `index.html`，交给客户端路由处理。
