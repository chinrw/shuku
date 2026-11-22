# Shuku 开发约定

Shuku 是面向个人书库的漫画归档服务。界面和文档同时面向按卷出版的漫画和同人志，不假设用户使用某个元数据站点或标签命名空间。与用户交流、写文档和代码注释用中文；标识符和提交信息用英文。

SQLite（WAL）是唯一数据源，写入必须按事务完整提交。搜索可以读稍有延迟的内存快照。扫描和阅读都不改原始文件，派生数据写到独立的数据目录；只有用户明确选择连文件一起删除时才删原文件。

## 开发与验证

本地构建、测试和工具命令都在仓库的 Nix flake 环境运行。系统依赖加到 `flake.nix`，不用宿主机的 rustup 或 rustc。提交前在仓库根目录运行：

```sh
nix develop -c cargo fmt --all --check
nix develop -c cargo clippy --workspace --all-targets --locked -- -D warnings
nix develop -c cargo test --workspace --locked
```

网页端在 `web/` 下用 `nix develop -c pnpm` 运行 `check`、`test` 和 `build`。改了 `web/pnpm-lock.yaml`，同步更新 `nix/package.nix` 的 `fetchPnpmDeps` 哈希，用 `nix build .#default` 验证。改启动参数或控制台设置时同步检查 NixOS 模块选项。`app/` 是独立 Cargo workspace；Linux 桌面版用 `nix develop .#app`，不在根目录的测试范围内。

## 按任务查阅

| 修改或排查的内容 | 对应文档 |
| --- | --- |
| 扫描、归档读取、文件指纹、LRR 迁移、元数据来源或后续范围 | [服务端设计决策](docs/design.md)；运行参数和目录示例见[服务端手册](docs/server.md)。 |
| 数据库迁移、写队列、快照、搜索排序或性能 | [存储与搜索](docs/storage.md)，含选型依据和基准命令。 |
| HTTP 接口、认证、图片缓存或 MCP | [HTTP API](docs/api.md)。 |
| 书库布局、阅读器、主题或离线行为 | [网页端交互约定](docs/web.md)。 |
| 桌面和 Android 外壳、打包或发布 | [客户端文档](docs/clients.md)。 |
| NixOS 部署、多实例或模块测试 | [NixOS 模块](docs/nixos.md)。 |

新的设计决定记在对应主题文档里；这里保留跨任务约定和入口，不记功能进度。
