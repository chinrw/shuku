//! 在控制台里改的扫描设置，存在 SQLite 的 setting 表里，键是 `scan`。表由各功能共用，
//! 每个功能一个键、值是那组设置的 JSON。目录、监听地址和允许的主机名仍是启动参数：
//! 改它们要重启，主机名列表在页面里改错了还会把自己挡在外面。

use crate::{App, Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// 定时扫描最长隔 30 天。
pub(crate) const MAX_SCAN_INTERVAL_HOURS: u32 = 24 * 30;
/// 定时扫描到点时如果上一次还没扫完，隔这么久再看。
const BUSY_RECHECK: Duration = Duration::from_secs(60);

/// 在 setting 表里的键。
pub(crate) const KEY: &str = "scan";

/// 缺的字段用默认值，加字段不用迁移。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    /// 也扫描以 `.` 开头的目录和文件。默认跳过：它们通常是下载工具的临时目录，
    /// 往往只有那个程序自己能读。
    pub(crate) scan_hidden: bool,
    /// 每隔多少小时自动扫描一次，0 表示只在启动时和手动扫描。
    pub(crate) scan_interval_hours: u32,
}

/// PUT 的请求体，没给的项保持不变。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsPatch {
    scan_hidden: Option<bool>,
    scan_interval_hours: Option<u32>,
}

impl Settings {
    /// 库里的值读不懂时（例如以后改了格式又回退版本）用默认值并记日志，不让服务起不来。
    pub(crate) fn load(stored: shuku_store::Result<Option<Self>>) -> Self {
        let mut settings = match stored {
            Ok(stored) => stored.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(%error, "扫描设置无法解析，使用默认值");
                Self::default()
            }
        };
        settings.scan_interval_hours = settings.scan_interval_hours.min(MAX_SCAN_INTERVAL_HOURS);
        settings
    }

    pub(crate) fn apply(mut self, patch: &SettingsPatch) -> Result<Self> {
        if let Some(hours) = patch.scan_interval_hours {
            if hours > MAX_SCAN_INTERVAL_HOURS {
                return Err(Error::bad_request(format!(
                    "scan_interval_hours 必须在 0 到 {MAX_SCAN_INTERVAL_HOURS} 之间"
                )));
            }
            self.scan_interval_hours = hours;
        }
        if let Some(hidden) = patch.scan_hidden {
            self.scan_hidden = hidden;
        }
        Ok(self)
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
        *self
            .0
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// 写进库并换上新设置，通知定时扫描重新计时。
    pub(crate) fn update_settings(&self, patch: &SettingsPatch) -> Result<Settings> {
        let mut current = self
            .0
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let updated = current.apply(patch)?;
        self.write(|writer| Ok(writer.set_setting(KEY, &updated)?))?;
        *current = updated;
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
        let hours = self.settings().scan_interval_hours;
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
    use super::{Settings, SettingsPatch, next_scan_delay};
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
        let partial: Settings = serde_json::from_str(r#"{"scan_interval_hours": 100000}"#).unwrap();
        let settings = Settings::load(Ok(Some(partial)));
        assert!(!settings.scan_hidden);
        assert_eq!(settings.scan_interval_hours, super::MAX_SCAN_INTERVAL_HOURS);
        assert_eq!(Settings::load(Ok(None)), Settings::default());
    }

    #[test]
    fn patches_change_only_the_given_fields_and_are_validated() {
        let settings = Settings {
            scan_hidden: true,
            scan_interval_hours: 12,
        };
        let patch: SettingsPatch = serde_json::from_str(r#"{"scan_interval_hours": 24}"#).unwrap();
        assert_eq!(
            settings.apply(&patch).unwrap(),
            Settings {
                scan_hidden: true,
                scan_interval_hours: 24
            }
        );
        let patch: SettingsPatch = serde_json::from_str(r#"{"scan_interval_hours": 721}"#).unwrap();
        assert!(settings.apply(&patch).is_err());
        assert!(serde_json::from_str::<SettingsPatch>(r#"{"content_dir": "/"}"#).is_err());
    }
}
