# 运行 M1 后端

Shuku 扫描本地 ZIP 漫画归档，提供搜索、图片和阅读进度 API。运行前需要 Nix、一个已有的内容目录，以及一个独立的数据目录。网页端构建产物默认从 `web/build` 读取；未构建网页端时仍可使用 API。

## 首次扫描

在仓库根目录运行：

```bash
nix develop -c cargo run -p shuku-server --bin shuku -- scan \
  --content-dir /path/to/comics --data-dir /path/to/shuku-data
```

内容目录只读。数据目录不存在时会自动创建，不能与内容目录相同或互相包含。数据库保存在 `shuku.sqlite`，图片变体保存在 `cache/images/`。

扫描递归遍历普通文件，不跟随符号链接，按魔数识别 ZIP，不要求 `.zip` 或 `.cbz` 扩展名。不支持的归档格式会跳过并记日志；损坏文件会记为失败，但不妨碍其他文件入库。只要有扫描失败的文件，`scan` 就以非零状态退出。

## 启动服务

```bash
nix develop -c cargo run -p shuku-server --bin shuku -- \
  --content-dir /path/to/comics --data-dir /path/to/shuku-data
```

默认监听 `0.0.0.0:3002`，LRR 占用 3001。启动先从 SQLite 构建索引，再开始监听并在后台扫描。日志中的 `count` 和 `elapsed_ms` 分别是索引本数和从 SQLite 读取、构建索引的总耗时。后台扫描结束后换入新索引，正在使用旧快照的请求可以继续完成。

四项配置都支持环境变量，命令行参数优先：

| 参数 | 环境变量 | 默认值 |
| --- | --- | --- |
| `--content-dir` | `SHUKU_CONTENT_DIR` | 必填 |
| `--data-dir` | `SHUKU_DATA_DIR` | 必填 |
| `--web-dir` | `SHUKU_WEB_DIR` | `web/build` |
| `--listen` | `SHUKU_LISTEN` | `0.0.0.0:3002` |

相对路径以进程的工作目录为准。参数可以放在 `scan` 前后。按 Ctrl-C 停止 HTTP 服务。

## 内容变化与缓存

扫描按路径、修改时间和大小识别变化。新文件的初始标题取文件名（去掉扩展名），加入时间取扫描时间，并标记为新书；已有归档的人工元数据和进度会保留。扫描只计算快速指纹（文件大小加首尾各 64 KiB），不读整个文件；完整 BLAKE3 指纹以后由后台任务计算。

路径消失的记录暂不删除，后续发现相同快速指纹的新文件时可以保留原 ID 并更新路径。若多个失踪记录具有相同快速指纹，按 ID 从小到大关联；原路径仍存在时，同内容的副本作为新归档入库。

M1 不监视文件变化。服务运行期间单独执行 `scan` 只会更新数据库和该命令自身的索引，不会通知已有服务进程；服务重启后会重新读取数据库并扫描。

客户端应原样使用 [API 文档](api.md) 返回的图片 URL。图片请求必须带 `v`，缺失返回 400，版本不符返回 404。文件已变化但尚未重新扫描时返回 409，不会把新内容缓存到旧 URL。`w` 只能是 720、1080、1440 或 2160；原图不比请求宽度大时直接返回原图。超过 6400 万像素的图片不缩放，返回 422。缩略图始终为 JPEG，小封面不放大。JPEG 质量固定为 85。

JPEG、PNG、GIF、WebP、AVIF、JXL 和 BMP 都可以读取原图并生成 JPEG 变体。AVIF 和 JXL 使用额外的 Rust 解码器；Nix 环境中的 NASM 只用于编译 AVIF 解码器的 x86 汇编，不需要系统图像库。

搜索的 `limit` 范围为 1 到 200，超出范围返回 400。未知 API 路径返回 JSON 404；只有 `/api` 以外的路径才使用网页端的 `index.html` 回退。

## 从 LRR 导入元数据

先用 `shuku scan` 让归档入库，再运行迁移工具。它按文件路径把 LRR 的记录对应到已入库的归档，导入标题、标签、简介、分类、Tankoubon 和阅读进度，可以反复运行，每次以 LRR 为准。

```bash
nix develop -c cargo run -p shuku-migrate-lrr -- \
  --redis redis://127.0.0.1:6381/0 \
  --lrr-content-dir /LRR/看到的/内容目录 \
  --data-dir /path/to/shuku-data
```

media-lxc 上 LRR 的 Redis（valkey）监听 6381，不是默认的 6379。`--lrr-content-dir` 填 LRR 自己记录的内容目录，工具用它把 LRR 里的绝对路径换成相对内容目录的路径；它和 Shuku 的 `--content-dir` 可以不同。找不到对应文件的记录会列在输出里。

## 使用发布包

推送 `vX.Y.Z` 或 `vX.Y.Z-后缀` 形式的标签（例如 `git tag v0.1.0 && git push origin v0.1.0`）后，GitHub Actions 构建并发布 `shuku-<标签>-x86_64-linux.tar.gz` 和它的 SHA-256 校验文件，在仓库的 Releases 页面下载。包里有静态链接（musl）的 `shuku` 和 `shuku-migrate-lrr`，不依赖系统库，NixOS 上也能直接运行；网页端构建产物在 `web/build`。

在解压出的目录里运行 `./shuku`，默认的 `--web-dir web/build` 就能找到网页端；从别的目录运行时用 `--web-dir` 指向它。其余参数与上文相同。
