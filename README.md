# Shuku

Shuku（拼音“书库”）是一个自托管的漫画书库服务：把一个目录里的漫画归档扫描入库，在浏览器里浏览、搜索和阅读，并记住每本书读到哪一页。它面向几十万本规模的个人书库，按卷出版的漫画和同人志都可以放在同一个库里。

使用前需要准备：

- 一台 Linux 机器，用 Nix 从源码运行，或者下载 Releases 里静态链接的 x86_64 二进制。
- 一个存放漫画的内容目录。目前支持 ZIP 和 CBZ 归档，Shuku 只读取它，从不修改或重新打包。
- 一个独立的数据目录，用来存数据库和图片缓存。

## 能做什么

- 扫描内容目录，记录每本书的页面清单和每页的宽高。读一页只做一次定位读取，大图可以按屏幕宽度缩小后再发给浏览器。
- 网页端在手机、平板和电脑上都能用：书库、搜索、详情页和阅读器。阅读器支持翻页和纵向滚动两种模式，翻页方向可以切换成从右到左或从左到右，横屏时可以双页并排。
- 搜索按标签、标题、页数和阅读状态组合筛选。30 万本的书库里，搜索结果的第一页一般在几毫秒内返回。
- 分类：把书加进自己建的分类。从 LRR 导入的动态分类按它的搜索语句筛选。
- 阅读进度保存在服务端，换一台设备可以从上次的页继续读。离线设备补传的旧进度不会覆盖更新的进度。
- 控制台（`/admin`）显示书库状态、扫描问题和日志，可以手动触发重新扫描等任务。

## 快速开始

```bash
# 首次扫描
nix develop -c cargo run -p shuku-server --bin shuku -- scan \
  --content-dir /path/to/comics --data-dir /path/to/shuku-data

# 启动服务，默认监听 0.0.0.0:3002
nix develop -c cargo run -p shuku-server --bin shuku -- \
  --content-dir /path/to/comics --data-dir /path/to/shuku-data
```

网页端需要先构建：在 `web/` 下运行 `nix develop -c pnpm install` 和 `nix develop -c pnpm build`，服务端默认从 `web/build` 读取。完整的参数、发布包的用法和扫描规则见 [docs/server.md](docs/server.md)，HTTP 接口见 [docs/api.md](docs/api.md)。

## 元数据

新入库的书以文件名作为标题，标签、简介和分类可以从 LANraragi 导入（见下一节）。以后计划从多种来源补全元数据，每种来源都是可选的：归档里自带的 ComicInfo.xml、文件名和目录结构，以及在线站点。不用的来源不需要配置。

## 从 LANraragi 迁移

Shuku 最初是为了取代 LANraragi（LRR）而写的。迁移工具 `shuku-migrate-lrr` 从 LRR 的 Redis 导入标题、标签、简介、加入时间、来源、分类、Tankoubon 和阅读进度，可以在扫描前后反复运行。两个服务可以读同一个内容目录、并行运行，确认没有问题后再下线 LRR。用法见 [docs/server.md](docs/server.md#从-lrr-导入元数据)。

Shuku 不兼容 LRR 的 HTTP API 和 Perl 插件，也不保留 LRR 的归档 ID。

## 限制

- 只支持 ZIP 和 CBZ。7z、RAR、tar 和 PDF 还没有实现。
- 没有认证，只适合在局域网内用 HTTP 访问。
- 不监视文件变化，新加的书要等下次扫描（重启服务或在控制台触发）。
- 文件删除后记录仍留在库里，控制台的问题页会列出它们。

开发约定和已经定下的设计决策见 [AGENTS.md](AGENTS.md)。

## 许可

[MIT](LICENSE)
