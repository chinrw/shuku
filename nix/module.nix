# NixOS 模块：每个 services.shuku.instances.<名字> 是一个 shuku-<名字>.service，
# 各自的内容目录、数据目录和端口。用法见 docs/nixos.md。
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  inherit (lib) mkOption types;
  cfg = config.services.shuku;
  enabled = lib.filterAttrs (_: instance: instance.enable) cfg.instances;

  # null 表示不管这一项，控制台里改的值保持不变，所以不写进设置文件。
  nullOr =
    type: description:
    mkOption {
      type = types.nullOr type;
      default = null;
      inherit description;
    };

  instanceModule =
    { name, ... }:
    {
      options = {
        enable = mkOption {
          type = types.bool;
          default = true;
          description = "运行这个实例。";
        };
        package = mkOption {
          type = types.package;
          default = cfg.package;
          defaultText = lib.literalExpression "config.services.shuku.package";
          description = "这个实例用的 Shuku 包。";
        };
        # 用 str 而不是 path：写成路径字面量时，插进命令行会把整个书库复制进 Nix store。
        libraryDir = mkOption {
          type = types.str;
          example = "/srv/manga";
          description = ''
            书库内容目录。服务以只读方式看到它，除非打开 allowFileDeletion。
            运行服务的用户要能读它。
          '';
        };
        dataDir = mkOption {
          type = types.str;
          default = "/var/lib/shuku/${name}";
          defaultText = lib.literalExpression ''"/var/lib/shuku/''${name}"'';
          description = "SQLite 和缓存目录，不能和内容目录互相包含。";
        };
        listenAddress = mkOption {
          type = types.str;
          default = "0.0.0.0";
          example = "127.0.0.1";
          description = "监听的地址。只给 tailscale serve 等本机反向代理用时设成 127.0.0.1。";
        };
        port = mkOption {
          type = types.port;
          example = 3002;
          description = "监听的端口，各实例不能相同。";
        };
        openFirewall = mkOption {
          type = types.bool;
          default = false;
          description = "在防火墙上打开这个端口。";
        };
        allowFileDeletion = mkOption {
          type = types.bool;
          default = false;
          description = ''
            让服务可以写内容目录，这样网页端的详情页可以连原文件一起删除。
            关着时内容目录在服务看来是只读的，网页端只提供删除记录。
          '';
        };
        user = mkOption {
          type = types.str;
          default = cfg.user;
          defaultText = lib.literalExpression "config.services.shuku.user";
          description = "运行这个实例的用户。不是模块建的 shuku 用户时要自己建。";
        };
        group = mkOption {
          type = types.str;
          default = cfg.group;
          defaultText = lib.literalExpression "config.services.shuku.group";
          description = "运行这个实例的用户组。";
        };
        environment = mkOption {
          type = types.attrsOf types.str;
          default = { };
          example = {
            HTTPS_PROXY = "http://127.0.0.1:8080";
          };
          description = "额外的环境变量。会进 Nix store，不要放密码或令牌，用 environmentFile。";
        };
        environmentFile = mkOption {
          type = types.nullOr types.path;
          default = null;
          example = "/run/secrets/shuku-env";
          description = "systemd 的 EnvironmentFile，放不能进 Nix store 的变量。";
        };
        extraArgs = mkOption {
          type = types.listOf types.str;
          default = [ ];
          description = "追加在 shuku 命令行后面的参数。";
        };
        settings = {
          scan = {
            watch = nullOr types.bool "监视内容目录，有变化时自动扫描（控制台的“自动扫描新文件”）。";
            scanHidden = nullOr types.bool "也扫描以 . 开头的目录和文件。";
            scanIntervalHours = nullOr (types.ints.between 0 720) "每隔多少小时定时扫描，0 表示不定时。";
            extraSkip = nullOr (types.listOf types.str) "额外跳过的目录或文件名，可以用 *，例如 @eaDir、*.part。";
            seriesFromTopDir = nullOr types.bool "内容目录下每个第一级文件夹是一部作品（按卷出版的普通漫画书库用）。";
          };
          server = {
            name = nullOr types.str "书库名，显示在书库页左上角和客户端的服务器列表里。";
            detectWebtoons = nullOr types.bool "按页面宽高比自动判断条漫。";
            allowedHosts = nullOr (types.listOf types.str) "除 IP、localhost 和不带点的主机名以外允许的 Host，例如 Tailscale 的完整域名。";
          };
        };
      };
    };

  settingsFile =
    name: instance:
    let
      inherit (instance.settings) scan server;
      drop = lib.filterAttrs (_: value: value != null);
      groups = lib.filterAttrs (_: group: group != { }) {
        scan = drop {
          inherit (scan) watch;
          scan_hidden = scan.scanHidden;
          scan_interval_hours = scan.scanIntervalHours;
          extra_skip = scan.extraSkip;
          series_from_top_dir = scan.seriesFromTopDir;
        };
        server = drop {
          inherit (server) name;
          detect_webtoons = server.detectWebtoons;
          allowed_hosts = server.allowedHosts;
        };
      };
    in
    pkgs.writeText "shuku-${name}-settings.json" (builtins.toJSON groups);

  # IPv6 地址要加方括号才能和端口拼在一起。
  listen =
    instance:
    if lib.hasInfix ":" instance.listenAddress then
      "[${instance.listenAddress}]:${toString instance.port}"
    else
      "${instance.listenAddress}:${toString instance.port}";

  service =
    name: instance:
    let
      stateDirectory = lib.removePrefix "/var/lib/" instance.dataDir;
      managedState = lib.hasPrefix "/var/lib/" instance.dataDir;
      privilegedPort = instance.port < 1024;
    in
    {
      description = "Shuku manga library (${name})";
      after = [ "network.target" ];
      wantedBy = [ "multi-user.target" ];
      # 内容目录在网络盘上时等它挂载好。
      unitConfig.RequiresMountsFor = [
        instance.libraryDir
        instance.dataDir
      ];
      inherit (instance) environment;
      serviceConfig = {
        ExecStart = lib.escapeShellArgs (
          [
            (lib.getExe instance.package)
            "--content-dir"
            instance.libraryDir
            "--data-dir"
            instance.dataDir
            "--listen"
            (listen instance)
            "--settings"
            (settingsFile name instance)
          ]
          ++ instance.extraArgs
        );
        User = instance.user;
        Group = instance.group;
        EnvironmentFile = lib.mkIf (instance.environmentFile != null) instance.environmentFile;
        Restart = "on-failure";
        # shuku 收到 Ctrl-C（SIGINT）时才走正常退出，停掉写队列和后台任务。
        KillSignal = "SIGINT";
        StateDirectory = lib.mkIf managedState stateDirectory;
        StateDirectoryMode = "0750";
        ReadWritePaths =
          lib.optional (!managedState) instance.dataDir
          ++ lib.optional instance.allowFileDeletion instance.libraryDir;
        UMask = "0027";

        # 除了数据目录（和打开 allowFileDeletion 时的内容目录）全部只读。内容目录或数据目录在
        # /home 下时也能用，所以家目录是只读而不是隐藏。
        ProtectSystem = "strict";
        ProtectHome = "read-only";
        PrivateTmp = true;
        PrivateDevices = true;
        NoNewPrivileges = true;
        CapabilityBoundingSet = lib.optional privilegedPort "CAP_NET_BIND_SERVICE";
        AmbientCapabilities = lib.optional privilegedPort "CAP_NET_BIND_SERVICE";
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectKernelLogs = true;
        ProtectControlGroups = true;
        ProtectClock = true;
        ProtectHostname = true;
        ProtectProc = "invisible";
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [
          "@system-service"
          "~@privileged"
        ];
      };
    };
in
{
  options.services.shuku = {
    enable = lib.mkEnableOption "Shuku 漫画书库服务";
    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "shuku.packages.\${system}.default";
      description = "Shuku 包，含网页端。";
    };
    user = mkOption {
      type = types.str;
      default = "shuku";
      description = "运行服务的用户。保持默认时由模块创建。";
    };
    group = mkOption {
      type = types.str;
      default = "shuku";
      description = "运行服务的用户组。保持默认时由模块创建。";
    };
    instances = mkOption {
      type = types.attrsOf (types.submodule instanceModule);
      default = { };
      description = "各个实例，每个是一个 shuku-<名字>.service。";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions =
      let
        duplicates = values: lib.unique (lib.filter (v: lib.count (x: x == v) values > 1) values);
        ports = duplicates (lib.mapAttrsToList (_: i: i.port) enabled);
        dataDirs = duplicates (lib.mapAttrsToList (_: i: toString i.dataDir) enabled);
      in
      [
        {
          assertion = ports == [ ];
          message = "services.shuku.instances 的端口重复：${lib.concatMapStringsSep ", " toString ports}";
        }
        {
          assertion = dataDirs == [ ];
          message = "services.shuku.instances 的数据目录重复：${lib.concatStringsSep ", " dataDirs}";
        }
      ];

    users.users = lib.mkIf (cfg.user == "shuku") {
      shuku = {
        isSystemUser = true;
        inherit (cfg) group;
      };
    };
    users.groups = lib.mkIf (cfg.group == "shuku") { shuku = { }; };

    # 新建设备令牌等命令要在命令行运行 shuku。
    environment.systemPackages = [ cfg.package ];

    # 数据目录不在 /var/lib 下时 StateDirectory 管不了，由 tmpfiles 建好并交给服务的用户。
    systemd.tmpfiles.rules = lib.concatLists (
      lib.mapAttrsToList (
        _: instance:
        lib.optional (
          !lib.hasPrefix "/var/lib/" instance.dataDir
        ) "d ${instance.dataDir} 0750 ${instance.user} ${instance.group} - -"
      ) enabled
    );

    systemd.services = lib.mapAttrs' (
      name: instance: lib.nameValuePair "shuku-${name}" (service name instance)
    ) enabled;

    networking.firewall.allowedTCPPorts = lib.mapAttrsToList (_: i: i.port) (
      lib.filterAttrs (_: i: i.openFirewall) enabled
    );
  };
}
