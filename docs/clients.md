# 桌面和 Android 客户端

客户端把 Shuku 的网页端装成一个独立的应用，在一个地方管理几台 Shuku 服务器（例如同人志和普通漫画各一个实例），各自的设备令牌存在这台设备上，不用每次在浏览器里输地址和登录。支持 Linux、Windows、macOS 和 Android（arm64）。

使用前需要：

- 一台已经在运行的 Shuku 服务器，版本要有 `/api/info` 接口。
- 如果服务器在控制台打开了“网页端也需要令牌”，在控制台“设置 › 网络与安全 › 设备令牌”里给这台设备新建一个令牌，或者在服务器上运行 `shuku token create 设备名`。

## 安装

[Nightly 开发版](https://github.com/chinrw/shuku/releases/tag/nightly)随 main 上的客户端成功构建更新，下载链接固定。需要正式版时，到 [Releases](https://github.com/chinrw/shuku/releases) 选择带客户端附件的 `vX.Y.Z`；`v0.1.0` 只有服务器。

| 平台 | 文件 |
| --- | --- |
| Linux | 用浏览器访问服务器，或按下文从源码构建桌面端。Release 不提供 Linux 桌面安装包。 |
| Windows | [下载 Windows x64 安装包](https://github.com/chinrw/shuku/releases/download/nightly/shuku-windows-x86_64.setup.exe) |
| macOS | [下载通用 DMG](https://github.com/chinrw/shuku/releases/download/nightly/shuku-macos-universal.dmg)，Intel 和 Apple Silicon 通用 |
| Android | [下载 arm64 APK](https://github.com/chinrw/shuku/releases/download/nightly/shuku-android-arm64.apk) |

安装包没有经过 Apple 和微软的代码签名。macOS 第一次打开时在 Finder 里右键选“打开”，或者运行 `xattr -cr /Applications/Shuku.app`；Windows 的 SmartScreen 提示里选“仍要运行”。

`SHA256SUMS` 里是这些文件的校验值。

## 更新客户端

第一次使用应用内更新，需要手动安装一次带更新入口的客户端。安装后可在自带的服务器列表页选择 **Stable（稳定版）** 或 **Nightly（开发版）**，也可以点“检查更新”。默认使用安装包的来源；改选后记在这台设备。启动时只在后台检查，不弹窗，不从阅读器跳走。

更新提示比较的是客户端版本和构建号（例如 `0.1.1+1001` 的构建号是 1001），不看服务端版本、Release 更新时间或 Nightly 标签的提交。切回 Stable 时，如果它比已装的 Nightly 旧，会保持当前安装，等稳定渠道提供更高构建号再提示。

点“下载并安装更新”后，Windows 和 macOS 通过 Tauri Updater 校验版本绑定的签名，成功后才替换客户端。Android 打开所选渠道的 APK 下载地址，由用户在系统安装器中确认覆盖安装。两个渠道使用同一个 Android 包名、固定证书和递增的 versionCode，不必为了切换渠道先卸载。与固定证书不同的旧调试包不支持直接覆盖。

Linux 官方 Release 仍然只提供服务端。自行分发的 AppImage 只有在所选渠道清单提供匹配架构的签名包时才能应用内更新；`.deb` 由包管理器更新。

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

`.github/workflows/clients.yml` 在 `app/` 或客户端工作流有变化时检查 Linux 桌面端能编译和链接，为 Windows、macOS 和 Android 构建安装包。PR 检查调用同一工作流，不传发布签名密钥。推到 main 后也会构建，产物在那次运行的 Artifacts 里，保留 14 天。main 上成功的推送构建和手动运行的 Clients 还会把同一批安装包与该提交的 Linux 服务端一起发布到 Nightly，不需要等到晚上或重新编译。推送正式版 `v*` 标签时 `release.yml` 带着下载标签调用它，把安装包和 `SHA256SUMS` 附加到 Release 上。

客户端版本由 `app/src-tauri/tauri.conf.json` 的 `version` 单独管理，格式为 `主.次.补丁+构建号`。修改客户端并发布时提高构建号，不能回退版本。Android 的 Gradle 构建直接从同一字段读取 versionName 和 versionCode。两个渠道发布同一版客户端时使用同一构建号；单独更新服务器或重新打包不构成客户端升级。

两个渠道各随 Release 附带 `client-update.json`。Stable 使用最新正式 Release 的附件，Nightly 使用 `nightly` 的固定下载地址。清单列出客户端版本、构建号、渠道、APK 地址及各桌面架构的签名和更新包地址。macOS 用户首次手动安装用 DMG，应用内更新用签名的 `.app.tar.gz`。

发布 Android 客户端时必须沿用仓库已有的 `ANDROID_KEYSTORE_BASE64`、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS` 和 `ANDROID_KEY_PASSWORD`。CI 用 `apksigner` 校验证书，用 `aapt` 校验包名和版本后才发布。无发布凭据的 PR 只验证构建，不发布调试签名 APK。

桌面两个渠道共用一把 **Tauri updater 签名密钥**，不使用 Android keystore。CI 从仓库变量 `TAURI_UPDATER_PUBLIC_KEY` 读取 Tauri 生成的 `.pub` 文件内容，构建时内置到 `plugins.updater.pubkey`，不是安装时从远端取公钥；私钥和密码分别存在仓库 Secrets `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，另行离线备份。不要为新版本重新生成密钥，否则已安装的客户端无法验签。这个签名不代替 Apple 公证或 Windows Authenticode。

在持有现有桌面密钥的机器上配置一次即可。以下两条命令从文件读取，不需要把密钥粘贴到命令行或聊天中：

```sh
updater_key='/path/to/shuku-updater.key'
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo chinrw/shuku < "$updater_key"
gh variable set TAURI_UPDATER_PUBLIC_KEY --repo chinrw/shuku < "$updater_key.pub"
```

如果私钥有密码，用 `gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo chinrw/shuku` 在隐藏提示中输入密码。CI 发布清单前会用同一公钥校验每个桌面更新包。没有密钥的提交检查不发布。
