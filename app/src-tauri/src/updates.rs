//! 更新只使用安装包自带的渠道地址和桌面公钥，不信任漫画服务器提供的更新信息。
use reqwest::{StatusCode, Url};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tauri::AppHandle;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Channel {
    Stable,
    Nightly,
}

impl Channel {
    fn endpoint(self) -> &'static str {
        match self {
            Self::Stable => {
                "https://github.com/chinrw/shuku/releases/latest/download/client-update.json"
            }
            Self::Nightly => {
                "https://github.com/chinrw/shuku/releases/download/nightly/client-update.json"
            }
        }
    }
    fn asset(self, url: &Url, tag: &str) -> bool {
        let valid_tag = match self {
            Self::Nightly => tag == "nightly",
            Self::Stable => tag
                .strip_prefix('v')
                .and_then(|v| Version::parse(v).ok())
                .is_some_and(|v| v.pre.is_empty() && v.build.is_empty()),
        };
        valid_tag
            && url.origin().ascii_serialization() == "https://github.com"
            && url
                .path()
                .strip_prefix(&format!("/chinrw/shuku/releases/download/{tag}/"))
                .is_some_and(|file| file.starts_with("shuku-") && !file.contains('/'))
    }
}

#[derive(Deserialize)]
struct DesktopPackage {
    url: Url,
    signature: String,
}

#[derive(Deserialize)]
struct Manifest {
    channel: Channel,
    tag: String,
    version: Version,
    build: u32,
    android: Url,
    platforms: std::collections::HashMap<String, DesktopPackage>,
}

fn number(version: &Version) -> Result<u32, String> {
    version
        .build
        .as_str()
        .parse::<u32>()
        .ok()
        .filter(|n| (1..=2_100_000_000).contains(n))
        .ok_or_else(|| "客户端版本缺少有效构建号".into())
}

fn is_newer(installed: &Version, candidate: &Version) -> Result<bool, String> {
    Ok(number(candidate)? > number(installed)? && !candidate.cmp_precedence(installed).is_lt())
}

fn platform() -> Option<String> {
    #[cfg(target_os = "android")]
    {
        Some("android-aarch64".into())
    }
    #[cfg(desktop)]
    {
        #[cfg(target_os = "linux")]
        if !matches!(
            tauri::utils::platform::bundle_type(),
            Some(tauri::utils::config::BundleType::AppImage)
        ) {
            return None;
        }
        tauri_plugin_updater::target()
    }
}

#[derive(Serialize)]
pub(crate) struct ClientInfo {
    channel: Channel,
    version: String,
    managed: bool,
}

fn installation_channel() -> Channel {
    match option_env!("SHUKU_CLIENT_CHANNEL") {
        Some("nightly") => Channel::Nightly,
        _ => Channel::Stable,
    }
}

#[tauri::command]
pub(crate) fn client_info(app: AppHandle) -> ClientInfo {
    ClientInfo {
        channel: installation_channel(),
        version: app.package_info().version.to_string(),
        managed: platform().is_none(),
    }
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Check {
    Available { version: String },
    Current,
    Unavailable,
    Managed,
}

async fn fetch(channel: Channel) -> Result<Option<(Manifest, Value)>, String> {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("Shuku-client-updater")
        .build()
        .map_err(|e| e.to_string())?
        .get(channel.endpoint())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let raw: Value = response
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let manifest: Manifest = serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
    if manifest.channel != channel
        || number(&manifest.version)? != manifest.build
        || !channel.asset(&manifest.android, &manifest.tag)
        || !manifest
            .android
            .path()
            .ends_with("/shuku-android-arm64.apk")
    {
        return Err("更新清单与所选渠道或构建号不符".into());
    }
    if manifest
        .platforms
        .values()
        .any(|package| !channel.asset(&package.url, &manifest.tag) || package.signature.is_empty())
    {
        return Err("桌面更新包缺少签名或不属于所选渠道".into());
    }
    Ok(Some((manifest, raw)))
}

#[tauri::command]
pub(crate) async fn check_client_update(
    app: AppHandle,
    channel: Option<Channel>,
) -> Result<Check, String> {
    let channel = channel.unwrap_or_else(installation_channel);
    let Some(target) = platform() else {
        return Ok(Check::Managed);
    };
    let Some((manifest, _)) = fetch(channel).await? else {
        return Ok(Check::Unavailable);
    };
    if target != "android-aarch64" && !manifest.platforms.contains_key(&target) {
        return Ok(Check::Unavailable);
    }
    if is_newer(&app.package_info().version, &manifest.version)? {
        Ok(Check::Available {
            version: manifest.version.to_string(),
        })
    } else {
        Ok(Check::Current)
    }
}

#[tauri::command]
pub(crate) async fn install_client_update(
    app: AppHandle,
    channel: Channel,
    version: Version,
) -> Result<(), String> {
    let target = platform().ok_or("请通过包管理器更新")?;
    let (manifest, raw) = fetch(channel).await?.ok_or("此渠道没有客户端版本")?;
    if manifest.version != version || !is_newer(&app.package_info().version, &version)? {
        return Err("版本已变化，请重新检查更新".into());
    }
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_opener::OpenerExt;
        let _ = (target, raw);
        app.opener()
            .open_url(manifest.android.as_str(), None::<&str>)
            .map_err(|e| e.to_string())
    }
    #[cfg(desktop)]
    {
        use tauri_plugin_updater::UpdaterExt;
        let update = app
            .updater_builder()
            .target(target)
            .endpoints(vec![
                Url::parse(channel.endpoint()).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| e.to_string())?
            .version_comparator(|local, remote| is_newer(&local, &remote.version).unwrap_or(false))
            .build()
            .map_err(|e| e.to_string())?
            .restore_update(raw)
            .map_err(|e| e.to_string())?
            .ok_or("没有更新版本")?;
        if !channel.asset(&update.download_url, &manifest.tag) {
            return Err("安装包不属于所选渠道".into());
        }
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string())?;
        app.restart()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_requires_new_client_build_without_semver_downgrade() {
        let v = |s: &str| Version::parse(s).unwrap();
        let installed = v("0.1.1+1001");
        for (candidate, expected) in [
            ("0.1.1+1002", true),
            ("0.1.2+1002", true),
            ("0.1.1+1001", false),
            ("0.1.1+1000", false),
            ("0.1.0+1100", false),
            ("0.2.0+1000", false),
        ] {
            assert_eq!(is_newer(&installed, &v(candidate)).unwrap(), expected);
        }
        assert!(number(&v("0.1.0")).is_err());
    }
}

/// 这项安装测试只在指定的临时目录中运行。使用与产品相同的版本规则和 Tauri
/// 签名验证；不改系统安装位置，不把测试公钥或本地源编入产品。
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod install_test {
    use super::*;
    use std::{env, fs, path::PathBuf};
    use tauri_plugin_updater::UpdaterExt;

    #[test]
    #[ignore = "需要临时旧安装和带版本签名的实际更新包"]
    fn installs_verified_upgrade_and_rejects_tampered_payload() {
        let file = |name| fs::read_to_string(env::var(name).unwrap()).unwrap();
        let installed: Version = env::var("SHUKU_TEST_OLD_VERSION").unwrap().parse().unwrap();
        let target = env::var("SHUKU_TEST_TARGET").unwrap();
        let path = PathBuf::from(env::var_os("SHUKU_TEST_OLD_EXECUTABLE").unwrap());
        let payload = fs::read(env::var_os("SHUKU_TEST_PAYLOAD").unwrap()).unwrap();
        let manifest: Value = serde_json::from_str(&file("SHUKU_TEST_MANIFEST")).unwrap();
        let version: Version = manifest["version"].as_str().unwrap().parse().unwrap();
        assert!(is_newer(&installed, &version).unwrap());
        assert!(!is_newer(&version, &installed).unwrap());
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".into(),
            serde_json::json!({"pubkey":file("SHUKU_TEST_PUBLIC_KEY").trim()}),
        );
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap();
        let updater = app
            .updater_builder()
            .target(target)
            .endpoints(vec![Url::parse(Channel::Nightly.endpoint()).unwrap()])
            .unwrap()
            .executable_path(&path)
            .version_comparator(move |_, release| is_newer(&installed, &release.version).unwrap())
            .build()
            .unwrap();
        let update = updater.restore_update(manifest).unwrap().unwrap();
        let mut corrupt = payload.clone();
        corrupt[0] ^= 1;
        let old = fs::read(&path).unwrap();
        assert!(update.install(&corrupt).is_err());
        assert_eq!(fs::read(&path).unwrap(), old);
        update.install(&payload).unwrap();
        assert_ne!(fs::read(&path).unwrap(), old);
    }
}
