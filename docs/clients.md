# 桌面和 Android 客户端

客户端把 Shuku 的网页端装成一个独立的应用，在一个地方管理几台 Shuku 服务器（例如同人志和普通漫画各一个实例），各自的设备令牌存在这台设备上，不用每次在浏览器里输地址和登录。支持 Linux、Windows、macOS 和 Android（arm64）。

使用前需要：

- 一台已经在运行的 Shuku 服务器，版本要有 `/api/info` 接口。
- 如果服务器在控制台打开了“网页端也需要令牌”，在控制台“设置 › 网络与安全 › 设备令牌”里给这台设备新建一个令牌，或者在服务器上运行 `shuku token create 设备名`。

## 安装

在仓库的 Releases 页面下载对应平台的文件：

| 平台 | 文件 |
| --- | --- |
| Linux | `…-linux-x86_64.AppImage`（加执行权限后直接运行）或 `…-linux-x86_64.deb` |
| Windows | `…-windows-x86_64.setup.exe` |
| macOS | `…-macos-universal.dmg`，Intel 和 Apple Silicon 通用 |
| Android | `…-android-arm64.apk` |

安装包没有经过 Apple 和微软的代码签名。macOS 第一次打开时在 Finder 里右键选“打开”，或者运行 `xattr -cr /Applications/Shuku.app`；Windows 的 SmartScreen 提示里选“仍要运行”。

`SHA256SUMS` 里是这些文件的校验值。

## 添加和切换服务器

第一次打开时是“添加服务器”页：

1. 填浏览器里打开 Shuku 用的那个地址，例如 `http://192.168.1.10:3002` 或 Tailscale 的 `https://nas.example.ts.net`。不写协议时按 `http`。离开输入框后客户端会读服务器信息，显示版本、本数和要不要令牌。
2. 服务器要求令牌时填上设备令牌。
3. 名字默认取服务器在控制台设置的书库名，只在这台设备上用。

保存后直接打开那个服务器的网页端。在书库页点左上角的书库名回到服务器列表，可以切换到别的服务器、添加新的，或者点“编辑”修改地址、令牌、名字和删除。切回一个服务器时回到上次离开时的页面。

下次启动时，如果上次用的服务器在线，客户端直接打开它；不在线时停在服务器列表上，列表里显示每台服务器在不在线。

令牌被吊销后，打开服务器时会停在网页端的登录页。在控制台新建一个令牌，回到列表里“编辑”这台服务器换上新令牌。

## 限制

- 客户端不带网页端的副本，书库、阅读器和控制台都是服务器提供的页面，所以没有离线阅读。离线阅读留给以后的原生 Android 客户端。
- 在网页端用新窗口打开的链接（来源网址等）交给系统浏览器。
- 还没有扫描控制台二维码添加服务器的功能。
- Android 版在 CI 上构建，还没有在真机上完整测过。

## 开发和构建

客户端的代码在 `app/`：`app/ui` 是服务器列表页（不需要构建的 HTML、CSS 和 JavaScript），`app/src-tauri` 是 Tauri 2 的外壳，`app/src-tauri/gen/android` 是 Android 工程。外壳是独立的 Cargo workspace，服务端的 `cargo test` 和 clippy 不编译它。

在 Linux 上用 flake 里的 `app` 开发环境编译和运行桌面版：

```bash
cd app/src-tauri
nix develop ../..#app -c cargo tauri dev
nix develop ../..#app -c cargo clippy --all-targets -- -D warnings
node --test '../test/*.test.js'
```

外壳的工作方式：

- 窗口先打开 `app/ui/index.html`。服务器和令牌存在这个页面的 localStorage 里，服务器的页面是别的源，读不到它们。
- 打开服务器时整个窗口跳到服务器的地址。有令牌时先到 `/login?next=<上次的路径>#token=<令牌>`，网页端的登录页从片段里取出令牌登录，片段不会发给服务器，也不进服务器日志。
- 外壳在每个页面加载前放一个 `window.shukuApp = { home }`。网页端看到它，就把书库页左上角的书库名改成指向 `home?from=<当前地址>` 的链接，列表页用 `from` 记下每台服务器上次的页面（见 `web/src/lib/nativeApp.ts`）。
- Android 的发布版允许明文 HTTP，局域网里的服务器用 http 直连。

### CI 和发布

`.github/workflows/clients.yml` 在 `app/` 有变化并推到 main 时构建四个平台，产物在那次运行的 Artifacts 里，保留 14 天。推送 `v*` 标签时 `release.yml` 带着版本号调用它，把安装包和 `SHA256SUMS` 附加到 Release 上。

Android 的发布签名从仓库的 Actions secrets 读取。没有配置时 APK 用构建机临时的调试密钥签名，每次构建的签名都不同，装新版本前要先卸载旧的，服务器列表也会随之丢失。配置一次正式密钥：

```bash
keytool -genkeypair -v -keystore shuku-release.jks -alias shuku \
  -keyalg RSA -keysize 4096 -validity 10000
base64 -w0 shuku-release.jks   # 结果填进 ANDROID_KEYSTORE_BASE64
```

在仓库的 Settings › Secrets and variables › Actions 里添加 `ANDROID_KEYSTORE_BASE64`、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS`（上面是 `shuku`）和 `ANDROID_KEY_PASSWORD`。密钥文件自己备份好，丢了以后新版本就不能覆盖安装。
