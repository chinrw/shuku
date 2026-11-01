# 服务端（shuku、shuku-migrate-lrr）加网页端。网页端装在 share/shuku/web，包装后的 shuku
# 默认从那里读，--web-dir 或 SHUKU_WEB_DIR 仍然优先。
{
  lib,
  rustPlatform,
  stdenvNoCC,
  fetchPnpmDeps,
  pnpmConfigHook,
  pnpm,
  nodejs,
  nasm,
  makeWrapper,
}:
let
  version = (lib.importTOML ../crates/shuku-server/Cargo.toml).package.version;
  fs = lib.fileset;

  web = stdenvNoCC.mkDerivation (finalAttrs: {
    pname = "shuku-web";
    inherit version;
    src = fs.toSource {
      root = ../web;
      fileset = fs.difference ../web (
        fs.unions [
          (fs.maybeMissing ../web/node_modules)
          (fs.maybeMissing ../web/build)
          (fs.maybeMissing ../web/.svelte-kit)
        ]
      );
    };
    pnpmDeps = fetchPnpmDeps {
      inherit (finalAttrs) pname version src;
      inherit pnpm;
      fetcherVersion = 5;
      hash = "sha256-EBM0KdsFfv8ByWCJiRZucdmjSL7d1Acg81PxcZQ+L58=";
    };
    nativeBuildInputs = [
      nodejs
      pnpm
      pnpmConfigHook
    ];
    buildPhase = ''
      runHook preBuild
      pnpm build
      runHook postBuild
    '';
    installPhase = ''
      runHook preInstall
      cp -r build $out
      runHook postInstall
    '';
  });
in
rustPlatform.buildRustPackage {
  pname = "shuku";
  inherit version;
  src = fs.toSource {
    root = ../.;
    fileset = fs.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [
    "-p"
    "shuku-server"
    "-p"
    "shuku-migrate-lrr"
  ];
  # 测试由 CI 在开发环境里跑，这里只构建。
  doCheck = false;
  # AVIF 解码器的 x86 SIMD 汇编需要 NASM。
  nativeBuildInputs = [
    nasm
    makeWrapper
  ];
  postInstall = ''
    mkdir -p $out/share/shuku
    ln -s ${web} $out/share/shuku/web
    wrapProgram $out/bin/shuku --set-default SHUKU_WEB_DIR $out/share/shuku/web
  '';
  passthru = { inherit web; };
  meta = {
    description = "Self-hosted manga archive server";
    license = lib.licenses.mit;
    mainProgram = "shuku";
    platforms = lib.platforms.linux;
  };
}
