# 在 NixOS 上运行

仓库的 flake 提供 Shuku 的包和一个 NixOS 模块。模块为每个实例建一个 systemd 服务，包里带着构建好的网页端，不用另外构建 `web/`。需要的只是一台用 flake 管理配置的 NixOS 机器和已有的内容目录。

## 加进配置

在系统的 flake 里加上输入，并导入模块：

```nix
{
  inputs.shuku.url = "github:chinrw/shuku";

  outputs = { nixpkgs, shuku, ... }: {
    nixosConfigurations.nas = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        shuku.nixosModules.default
        ./configuration.nix
      ];
    };
  };
}
```

然后在 `configuration.nix` 里开启。下面的例子跑两个实例，一个放同人志，一个放按卷出版的普通漫画：

```nix
{
  programs.shuku = {
    enable = true;
    instances.doujinshi = {
      libraryDir = "/srv/doujinshi";
      port = 3002;
      openFirewall = true;
      settings.server.name = "同人志";
    };
    instances.manga = {
      libraryDir = "/srv/manga";
      port = 3003;
      openFirewall = true;
      settings.scan.seriesFromTopDir = true;
      settings.server.name = "漫画";
    };
  };
}
```

`nixos-rebuild switch` 之后，每个实例是一个 `shuku-<名字>.service`，例如 `shuku-manga.service`，启动时先扫描一次内容目录。数据默认在 `/var/lib/shuku/<名字>`。

## 选项

`programs.shuku` 下面：

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `enable` | `false` | 开启模块。 |
| `package` | flake 里的包 | 所有实例默认用的包。 |
| `user`、`group` | `shuku` | 运行服务的用户和组。保持默认时模块会建这个系统用户。 |
| `instances.<名字>` | `{}` | 各个实例，见下表。 |

每个实例：

| 选项 | 默认值 | 说明 |
| --- | --- | --- |
| `enable` | `true` | 写了实例就运行，设成 `false` 暂时停掉。 |
| `libraryDir` | 必填 | 内容目录，一个实例一个。 |
| `dataDir` | `/var/lib/shuku/<名字>` | 数据库和缓存目录。不在 `/var/lib` 下时由 tmpfiles 建好并交给服务的用户。 |
| `listenAddress` | `0.0.0.0` | 监听地址，只给本机的 `tailscale serve` 转发时设成 `127.0.0.1`。 |
| `port` | 必填 | 端口，各实例不能相同。 |
| `openFirewall` | `false` | 在防火墙上打开这个端口。 |
| `allowFileDeletion` | `false` | 让服务可以写内容目录，详情页才能连原文件一起删除。 |
| `user`、`group` | 上一级的值 | 单独给这个实例换用户。换成别的用户时要自己建。 |
| `environment` | `{}` | 额外的环境变量，例如访问 E-Hentai 用的 `HTTPS_PROXY`。会写进 Nix store。 |
| `environmentFile` | `null` | systemd 的 `EnvironmentFile`，放不能进 Nix store 的变量。 |
| `extraArgs` | `[]` | 追加在 `shuku` 命令行后面的参数。 |
| `package` | 上一级的值 | 单独给这个实例换包。 |

服务的用户要能读内容目录。服务看到的整个文件系统除数据目录外都是只读的（`ProtectSystem=strict`，家目录也是只读），所以内容目录放在 `/home` 下也能用；打开 `allowFileDeletion` 后内容目录才可写。内容目录在网络盘上时，服务等它挂载好再启动。

## 声明控制台里的设置

`settings` 下的选项对应控制台设置页里的开关。默认都是 `null`，表示不管这一项，控制台里改的值照常保留。写了值的项在每次服务启动时套用，并且写进数据库：服务运行时仍可以在控制台改，但下次启动会被改回配置里的值。

| 选项 | 控制台里的名字 |
| --- | --- |
| `settings.scan.watch` | 扫描：自动扫描新文件 |
| `settings.scan.scanHidden` | 扫描：扫描隐藏的目录和文件 |
| `settings.scan.scanIntervalHours` | 扫描：定时扫描的间隔（小时，0 到 720，0 表示不定时） |
| `settings.scan.extraSkip` | 扫描：额外跳过的名字，例如 `[ "@eaDir" "*.part" ]` |
| `settings.scan.seriesFromTopDir` | 扫描：第一级目录为一部作品 |
| `settings.server.name` | 书库名 |
| `settings.server.detectWebtoons` | 自动判断条漫 |
| `settings.server.allowedHosts` | 允许的域名，例如 `[ "nas.example.ts.net" ]` |

元数据功能（E-Hentai 的 cookie、Bangumi 的令牌等）和“网页端也需要令牌”不在这里，仍在控制台里设置：前者带着密码，写进 Nix 配置就会进 Nix store。

模块把这些值写成一个 JSON 文件，用 `--settings` 交给 `shuku`。不用 NixOS 时也可以手写这个文件，格式见 [server.md](server.md#启动服务)。

## 日常操作

看日志用 `journalctl -u shuku-manga`。模块把 `shuku` 装进了系统路径；新建设备令牌要以服务的用户运行它，并指向同一个数据目录：

```bash
sudo -u shuku shuku --content-dir /srv/manga --data-dir /var/lib/shuku/manga token create 手机
```

服务停止时收到 SIGINT，和在终端里按 Ctrl-C 一样正常退出。

## 测试模块

`checks.x86_64-linux.nixos-module` 是一个 NixOS 虚拟机测试：起两个实例，各读一个构建时生成的小书库，确认都扫描入库、设置生效、加固的限制没有挡住文件监视。它需要 KVM：

```bash
nix build .#checks.x86_64-linux.nixos-module -L
```
