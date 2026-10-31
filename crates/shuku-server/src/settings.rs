//! 在控制台里改的设置，存在 SQLite 的 setting 表里。表由各功能共用，每个功能一个键、
//! 值是那组设置的 JSON；这里有 `scan`（扫描）和 `server`（条漫判断、允许的主机名）两组。
//! 只有书库目录、数据目录、网页目录和监听地址是启动参数。

use crate::{App, Error, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    sync::{Arc, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// 定时扫描最长隔 30 天。
pub(crate) const MAX_SCAN_INTERVAL_HOURS: u32 = 24 * 30;
/// 定时扫描到点时如果上一次还没扫完，隔这么久再看。
const BUSY_RECHECK: Duration = Duration::from_secs(60);
/// 列表类设置最多这么多项、每项最长这么多字节，免得一次 PUT 塞进巨大的值。
const MAX_LIST_ITEMS: usize = 100;
const MAX_ITEM_LEN: usize = 255;

/// 一组设置：在 setting 表里的键，以及 PUT 时怎样把补丁合进去。
pub(crate) trait Group: Clone + Default + Serialize + DeserializeOwned {
    const KEY: &'static str;
    type Patch: DeserializeOwned;

    fn apply(&self, patch: Self::Patch) -> Result<Self>;

    /// 读进来的值超出范围时（例如手改过库）收回范围内。
    fn normalize(self) -> Self {
        self
    }

    /// 库里的值读不懂时（例如以后改了格式又回退版本）用默认值并记日志，不让服务起不来。
    fn load(stored: shuku_store::Result<Option<Self>>) -> Self {
        match stored {
            Ok(stored) => stored.unwrap_or_default().normalize(),
            Err(error) => {
                tracing::warn!(key = Self::KEY, %error, "设置无法解析，使用默认值");
                Self::default()
            }
        }
    }
}

/// 缺的字段用默认值，加字段不用迁移。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ScanSettings {
    /// 也扫描以 `.` 开头的目录和文件。默认跳过：它们通常是下载工具的临时目录，
    /// 往往只有那个程序自己能读。
    pub(crate) scan_hidden: bool,
    /// 每隔多少小时自动扫描一次，0（默认）表示只在启动时和手动扫描。文件监视的后备。
    pub(crate) scan_interval_hours: u32,
    /// 额外跳过的目录或文件名，可以用 `*` 通配，不分大小写，例如 `@eaDir`、`*.part`。
    /// 不作用于文件夹书里的单张图片，要跳过整本就写文件夹的名字。
    pub(crate) extra_skip: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScanPatch {
    scan_hidden: Option<bool>,
    scan_interval_hours: Option<u32>,
    extra_skip: Option<Vec<String>>,
}

impl Group for ScanSettings {
    const KEY: &'static str = "scan";
    type Patch = ScanPatch;

    fn apply(&self, patch: ScanPatch) -> Result<Self> {
        let mut updated = self.clone();
        if let Some(hours) = patch.scan_interval_hours {
            if hours > MAX_SCAN_INTERVAL_HOURS {
                return Err(Error::bad_request(format!(
                    "scan_interval_hours 必须在 0 到 {MAX_SCAN_INTERVAL_HOURS} 之间"
                )));
            }
            updated.scan_interval_hours = hours;
        }
        if let Some(hidden) = patch.scan_hidden {
            updated.scan_hidden = hidden;
        }
        if let Some(patterns) = patch.extra_skip {
            updated.extra_skip = clean_list("extra_skip", patterns, |pattern| {
                // 只按单个名字匹配，带路径分隔符的写法永远匹配不上，直接拒绝。
                !pattern.contains(['/', '\\'])
            })?;
        }
        Ok(updated)
    }

    fn normalize(mut self) -> Self {
        self.scan_interval_hours = self.scan_interval_hours.min(MAX_SCAN_INTERVAL_HOURS);
        self
    }
}

impl ScanSettings {
    /// 名字是否被“额外跳过”命中。
    pub(crate) fn skips(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        self.extra_skip
            .iter()
            .any(|pattern| wildcard_match(&pattern.to_lowercase(), &name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ServerSettings {
    /// 按页面宽高比判断条漫，让网页端自动改用上下连续滚动。判断错得多时可以关掉，
    /// 关掉后只认阅读器里手动打开的条漫开关。
    pub(crate) detect_webtoons: bool,
    /// 除 IP、localhost 和不带点的主机名以外，还接受这些 Host（例如 Tailscale 的完整域名）。
    /// 在这里删错了也不会把自己挡在外面：用 IP 访问总是允许。
    pub(crate) allowed_hosts: Vec<String>,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            detect_webtoons: true,
            allowed_hosts: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerPatch {
    detect_webtoons: Option<bool>,
    allowed_hosts: Option<Vec<String>>,
}

impl Group for ServerSettings {
    const KEY: &'static str = "server";
    type Patch = ServerPatch;

    fn apply(&self, patch: ServerPatch) -> Result<Self> {
        let mut updated = self.clone();
        if let Some(detect) = patch.detect_webtoons {
            updated.detect_webtoons = detect;
        }
        if let Some(hosts) = patch.allowed_hosts {
            updated.allowed_hosts = clean_list("allowed_hosts", hosts, |host| {
                !host.contains(['/', ':', ' ', '*'])
            })?;
        }
        Ok(updated)
    }
}

/// 去掉首尾空白和空项、去重，逐项检查。
fn clean_list(
    field: &str,
    items: Vec<String>,
    valid: impl Fn(&str) -> bool,
) -> Result<Vec<String>> {
    let mut cleaned: Vec<String> = Vec::new();
    for item in items {
        let item = item.trim();
        if item.is_empty() || cleaned.iter().any(|seen| seen == item) {
            continue;
        }
        if item.len() > MAX_ITEM_LEN || !valid(item) {
            return Err(Error::bad_request(format!("{field} 里的“{item}”无效")));
        }
        cleaned.push(item.to_owned());
    }
    if cleaned.len() > MAX_LIST_ITEMS {
        return Err(Error::bad_request(format!(
            "{field} 最多 {MAX_LIST_ITEMS} 项"
        )));
    }
    Ok(cleaned)
}

/// `*` 匹配任意多个字符，其余字符原样比较。
fn wildcard_match(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    // 贪心匹配，遇到不符时退回最近一个 `*` 多吃一个字符。
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if p < pattern.len() && pattern[p] == name[n] {
            p += 1;
            n += 1;
        } else if let Some((star_p, star_n)) = star {
            p = star_p + 1;
            n = star_n + 1;
            star = Some((star_p, star_n + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&ch| ch == '*')
}

/// 两组设置一起放在 `App` 里，每次读都拿一份副本。
#[derive(Debug, Clone, Default)]
pub(crate) struct Settings {
    pub(crate) scan: ScanSettings,
    pub(crate) server: ServerSettings,
}

impl Settings {
    pub(crate) fn load(reader: &shuku_store::Reader) -> Self {
        Self {
            scan: ScanSettings::load(reader.setting(ScanSettings::KEY)),
            server: ServerSettings::load(reader.setting(ServerSettings::KEY)),
        }
    }
}

/// 下一次定时扫描还要等多久。`last` 是上一次扫描结束的时间，没扫过时是服务启动的时间。
/// 关闭定时扫描时返回 `None`。
pub(crate) fn next_scan_delay(
    interval_hours: u32,
    last: SystemTime,
    now: SystemTime,
) -> Option<Duration> {
    if interval_hours == 0 {
        return None;
    }
    let interval = Duration::from_secs(u64::from(interval_hours) * 3600);
    let elapsed = now.duration_since(last).unwrap_or_default();
    Some(interval.saturating_sub(elapsed))
}

impl App {
    pub(crate) fn settings(&self) -> Settings {
        self.0
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// 合进补丁、写进库、换上新设置，并通知定时扫描重新计时。`select` 取出要改的那一组。
    pub(crate) fn update_settings<G: Group>(
        &self,
        patch: G::Patch,
        select: impl Fn(&mut Settings) -> &mut G,
    ) -> Result<G> {
        let mut current = self
            .0
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let updated = select(&mut current).apply(patch)?;
        self.write(|writer| Ok(writer.set_setting(G::KEY, &updated)?))?;
        // 允许的域名等安全相关的设置也在这里改，留一条日志，控制台的日志页能看到改成了什么。
        tracing::info!(
            key = G::KEY,
            value = %serde_json::to_string(&updated).unwrap_or_default(),
            "设置已更新"
        );
        *select(&mut current) = updated.clone();
        drop(current);
        self.0.settings_changed.notify_one();
        Ok(updated)
    }

    /// 按设置定时在后台扫描（结束后接着补算指纹，和手动触发一样）。只在 HTTP 服务里启动；
    /// 任务只持有弱引用，不会让 `App` 一直活着。
    pub fn spawn_scheduler(&self) {
        let app = Arc::downgrade(&self.0);
        let changed = self.0.settings_changed.clone();
        tokio::spawn(async move {
            loop {
                let Some(inner) = app.upgrade() else { return };
                let delay = App(inner).scheduled_scan_delay();
                let Some(delay) = delay else {
                    changed.notified().await;
                    continue;
                };
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    () = changed.notified() => continue,
                }
                let Some(inner) = app.upgrade() else { return };
                let app = App(inner);
                if app.scheduled_scan_delay() == Some(Duration::ZERO) && app.start_background_scan()
                {
                    tracing::info!("开始定时扫描");
                }
            }
        });
    }

    fn scheduled_scan_delay(&self) -> Option<Duration> {
        let hours = self.settings().scan.scan_interval_hours;
        let (running, finished_at) = self.tasks(|tasks| tasks.scan.timing());
        if hours != 0 && running {
            return Some(BUSY_RECHECK);
        }
        let last = finished_at.map_or(self.0.started_at, |ms| {
            UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).unwrap_or_default())
        });
        next_scan_delay(hours, last, SystemTime::now())
    }
}

#[cfg(test)]
mod tests {
    use super::{Group, ScanSettings, ServerSettings, next_scan_delay, wildcard_match};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn scheduled_scans_wait_for_the_rest_of_the_interval() {
        let last = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let hour = Duration::from_secs(3600);
        assert_eq!(next_scan_delay(0, last, last + hour), None);
        assert_eq!(next_scan_delay(6, last, last + hour), Some(hour * 5));
        assert_eq!(
            next_scan_delay(6, last, last + hour * 7),
            Some(Duration::ZERO)
        );
        // 时钟往回走时等满一个间隔。
        assert_eq!(next_scan_delay(1, last, last - hour), Some(hour));
    }

    #[test]
    fn stored_values_fill_in_defaults_and_are_clamped() {
        let partial: ScanSettings =
            serde_json::from_str(r#"{"scan_interval_hours": 100000}"#).unwrap();
        let settings = ScanSettings::load(Ok(Some(partial)));
        assert!(!settings.scan_hidden);
        assert_eq!(settings.scan_interval_hours, super::MAX_SCAN_INTERVAL_HOURS);
        assert_eq!(ScanSettings::load(Ok(None)), ScanSettings::default());
        // 条漫判断默认开。
        assert!(ServerSettings::load(Ok(None)).detect_webtoons);
        let partial: ServerSettings =
            serde_json::from_str(r#"{"allowed_hosts": ["a.b"]}"#).unwrap();
        assert!(partial.detect_webtoons);
    }

    #[test]
    fn patches_change_only_the_given_fields_and_are_validated() {
        let settings = ScanSettings {
            scan_hidden: true,
            scan_interval_hours: 12,
            extra_skip: Vec::new(),
        };
        let patch = serde_json::from_str(r#"{"scan_interval_hours": 24}"#).unwrap();
        let updated = settings.apply(patch).unwrap();
        assert!(updated.scan_hidden);
        assert_eq!(updated.scan_interval_hours, 24);
        let patch = serde_json::from_str(r#"{"scan_interval_hours": 721}"#).unwrap();
        assert!(settings.apply(patch).is_err());
        assert!(serde_json::from_str::<super::ScanPatch>(r#"{"content_dir": "/"}"#).is_err());

        let patch = serde_json::from_str(r#"{"extra_skip": [" @eaDir ", "", "*.part", "@eaDir"]}"#)
            .unwrap();
        assert_eq!(
            settings.apply(patch).unwrap().extra_skip,
            ["@eaDir", "*.part"]
        );
        let patch = serde_json::from_str(r#"{"extra_skip": ["a/b"]}"#).unwrap();
        assert!(settings.apply(patch).is_err());

        let server = ServerSettings::default();
        let patch = serde_json::from_str(r#"{"allowed_hosts": ["nas.example.ts.net"]}"#).unwrap();
        assert_eq!(
            server.apply(patch).unwrap().allowed_hosts,
            ["nas.example.ts.net"]
        );
        let patch = serde_json::from_str(r#"{"allowed_hosts": ["evil.example:3002"]}"#).unwrap();
        assert!(server.apply(patch).is_err());
    }

    #[test]
    fn extra_skip_patterns_match_whole_names_with_stars() {
        let settings = ScanSettings {
            extra_skip: vec!["@eaDir".into(), "*.PART".into(), "tmp*cache".into()],
            ..ScanSettings::default()
        };
        for name in ["@eadir", "book.part", "TMP-x-cache", "tmpcache"] {
            assert!(settings.skips(name), "{name}");
        }
        for name in ["@eaDir2", "book.part.zip", "tmp-cache-old"] {
            assert!(!settings.skips(name), "{name}");
        }
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("a*b*c", "aXbYc"));
        assert!(!wildcard_match("a*b", "ab c"));
    }
}
