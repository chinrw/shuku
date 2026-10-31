//! 桌面和 Android 客户端的外壳。窗口先打开客户端自带的服务器列表页（`app/ui`），选中服务器后
//! 直接跳到那个服务器的网页端，页面、接口和图片都由服务器提供，不经过这里。

use tauri::{WebviewUrl, WebviewWindowBuilder, webview::NewWindowResponse};
use tauri_plugin_opener::OpenerExt;

/// 服务器列表页的地址。Tauri 在 Windows 和 Android 上用 `http://tauri.localhost` 提供自带的
/// 页面，其他平台用 `tauri://localhost`。
#[cfg(any(windows, target_os = "android"))]
const HOME: &str = "http://tauri.localhost/index.html";
#[cfg(not(any(windows, target_os = "android")))]
const HOME: &str = "tauri://localhost/index.html";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 插件默认在页面里拦截 target="_blank" 的点击，改走 IPC；服务器的页面是远程页面，
        // 没有 IPC 权限，点击会被吞掉。新窗口统一由下面的 on_new_window 处理。
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .setup(|app| {
            // 服务器的网页端看到 `window.shukuApp` 就知道自己在客户端里，左上角的书库名改成
            // 回到服务器列表。脚本在每次加载页面前运行，服务器的页面也有它。
            let script = format!("window.shukuApp = Object.freeze({{ home: {HOME:?} }});");
            let handle = app.handle().clone();
            let builder =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .initialization_script(&script)
                    // 网页端用新窗口打开的链接（来源网址等）交给系统浏览器，客户端只有一个窗口。
                    // 只转交网页地址，file: 之类的会让系统打开本机文件或程序。Android 不支持这个
                    // 回调，那里的 WebView 在当前页面打开这类链接，按返回键回来。
                    .on_new_window(move |url, _features| {
                        if matches!(url.scheme(), "http" | "https")
                            && let Err(error) = handle.opener().open_url(url.as_str(), None::<&str>)
                        {
                            eprintln!("打开 {url} 失败：{error}");
                        }
                        NewWindowResponse::Deny
                    });
            #[cfg(desktop)]
            let builder = builder
                .title("Shuku")
                .inner_size(1200.0, 820.0)
                .min_inner_size(360.0, 480.0);
            builder.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动 Shuku 客户端失败");
}
