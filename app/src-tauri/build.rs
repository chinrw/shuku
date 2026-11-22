fn main() {
    println!("cargo:rerun-if-env-changed=SHUKU_CLIENT_CHANNEL");
    let commands = tauri_build::AppManifest::new().commands(&[
        "client_info",
        "check_client_update",
        "install_client_update",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(commands))
        .expect("生成客户端权限失败");
}
