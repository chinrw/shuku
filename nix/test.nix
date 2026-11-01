# 两个实例各读一个生成的小书库，确认服务都起来了、扫描入库、设置文件生效。
{ self, pkgs }:
let
  # 构建时生成两张图打包成 cbz，不用任何真实的书。
  library =
    name: color:
    pkgs.runCommand "shuku-test-library-${name}"
      {
        nativeBuildInputs = [
          pkgs.imagemagick
          pkgs.zip
        ];
      }
      ''
        mkdir -p $out
        magick -size 60x90 xc:${color} 001.png
        magick -size 60x90 xc:white 002.png
        zip -0 $out/book.cbz 001.png 002.png
      '';
in
pkgs.testers.runNixOSTest {
  name = "shuku-nixos-module";
  nodes.machine = {
    imports = [ self.nixosModules.default ];
    services.shuku = {
      enable = true;
      instances.doujinshi = {
        libraryDir = "${library "doujinshi" "red"}";
        port = 3002;
        openFirewall = true;
        settings.server.name = "同人志";
      };
      instances.manga = {
        libraryDir = "${library "manga" "blue"}";
        dataDir = "/srv/shuku-manga";
        port = 3003;
        settings.scan.seriesFromTopDir = true;
        settings.server.allowedHosts = [ "nas.example.ts.net" ];
      };
    };
  };
  testScript = ''
    import json

    def get(port, path):
        return json.loads(machine.succeed(f"curl -sf http://127.0.0.1:{port}{path}"))

    machine.wait_for_unit("shuku-doujinshi.service")
    machine.wait_for_unit("shuku-manga.service")
    for port in (3002, 3003):
        machine.wait_for_open_port(port)
        machine.wait_until_succeeds(
            f"curl -sf http://127.0.0.1:{port}/api/info | grep -q '\"archives\":1'"
        )

    assert get(3002, "/api/info")["name"] == "同人志"
    assert get(3003, "/api/admin/settings/scan")["series_from_top_dir"] is True
    assert get(3003, "/api/admin/settings/server")["allowed_hosts"] == ["nas.example.ts.net"]
    # 没在 Nix 里写的项保持服务端的默认值。
    assert get(3002, "/api/admin/settings/scan")["series_from_top_dir"] is False

    # 加固的限制（系统调用过滤、只读目录）没挡住文件监视和补算指纹。
    machine.wait_until_succeeds("journalctl -u shuku-doujinshi | grep -q 开始监视内容目录")
    machine.fail("journalctl -u 'shuku-*' | grep -E ' (WARN|ERROR) '")

    machine.succeed("test -f /var/lib/shuku/doujinshi/shuku.sqlite")
    machine.succeed("test -f /srv/shuku-manga/shuku.sqlite")
    machine.succeed("stat -c %U /srv/shuku-manga | grep -qx shuku")
    # 网页端由包里的构建产物提供。
    machine.succeed("curl -sf http://127.0.0.1:3002/ | grep -qi '<html'")
    # SIGINT 走正常退出，停服务不算失败。
    machine.systemctl("stop shuku-manga.service")
    machine.succeed("systemctl show -p Result shuku-manga.service | grep -qx Result=success")
  '';
}
