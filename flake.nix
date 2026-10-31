{
  description = "Shuku: a self-hosted manga archive server";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
            # AVIF 解码器的 x86 SIMD 汇编需要 NASM，不链接系统图像库。
            nasm
            sqlite
            nodejs
            pnpm
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
        # 桌面客户端（app/）在 Linux 上编译和运行要的 WebKitGTK 等系统库，用
        # `nix develop .#app` 进入。Android 和其他平台只在 CI 上构建。
        app = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            cargo-tauri
            nodejs
            pkg-config
            webkitgtk_4_1
            libsoup_3
            gtk3
            librsvg
            openssl
            glib-networking
          ];
        };
      });
    };
}
