//! 在线来源的后台队列。每个来源一条队列、一个后台任务、各自限速。
//! 队列里的书不管自动运行开没开都会处理：自动运行只决定扫描到新书时放不放进来，
//! 用接口手动放进来的书照样要查。

use super::Provider;
use crate::{App, Error};
use std::time::Duration;

/// 后台队列读库出错时，隔这么久再试。
const RETRY_DELAY: Duration = Duration::from_secs(10);

impl App {
    /// 有书放进了队列（扫描或接口）后调用。
    pub(crate) fn metadata_queued(&self) {
        for provider in Provider::NETWORK {
            self.wake_provider(provider);
        }
    }

    fn wake_provider(&self, provider: Provider) {
        if let Some(wake) = self.0.metadata.network.wake.get(&provider) {
            wake.notify_one();
        }
    }

    /// 让暂停的来源（`None` 是全部）继续。
    pub(crate) fn resume_metadata(&self, provider: Option<Provider>) {
        for each in Provider::NETWORK {
            if provider.is_none_or(|provider| provider == each) {
                self.queue_status(each, |status| status.paused = None);
                self.wake_provider(each);
            }
        }
    }

    /// 每个在线来源一个后台任务，各自限速，一个来源在等冷却或暂停时不挡别的来源。
    /// 服务运行期间一直在。只在 HTTP 服务里、启动扫描之后启动：命令行的 `shuku scan`
    /// 扫完就退出，不该开始联网；停机期间移动或改过的文件要等扫描更新了记录再查，
    /// 否则按旧路径读不到封面。
    pub fn spawn_metadata_worker(&self) {
        for provider in Provider::NETWORK {
            let app = self.clone();
            tokio::spawn(async move { app.metadata_worker(provider).await });
        }
    }

    async fn metadata_worker(&self, provider: Provider) {
        let Some(wake) = self.0.metadata.network.wake.get(&provider) else {
            return;
        };
        let name = provider.as_str();
        loop {
            if self.queue_status(provider, |status| status.paused.is_some()) {
                wake.notified().await;
                continue;
            }
            let next = self
                .blocking(move |app| Ok(app.reader()?.next_queued_metadata(name)?))
                .await;
            match next {
                Ok(Some(id)) => self.process_queued(provider, id).await,
                Ok(None) => wake.notified().await,
                Err(error) => {
                    tracing::error!(provider = name, %error, "读元数据队列失败");
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
    }

    async fn process_queued(&self, provider: Provider, id: i64) {
        self.queue_status(provider, |status| status.current = Some(id));
        let review = self.metadata_settings().review(provider);
        let result = match self.run_network(provider, id, None).await {
            Ok(found) => {
                self.blocking(move |app| {
                    if review {
                        app.store_candidate(id, provider, &found)
                    } else {
                        app.apply_found(id, found).map(|_| ())
                    }
                })
                .await
            }
            Err(error) => Err(error),
        };
        let throttled = matches!(
            &result,
            Err(Error::Metadata(shuku_metadata::Error::Throttled))
        );
        let paused = result.as_ref().is_err_and(stops_queue);
        let message = result.as_ref().err().map(ToString::to_string);
        let name = provider.as_str();
        match &message {
            Some(error) if paused => tracing::error!(id, provider = name, %error, "元数据队列暂停"),
            Some(error) => tracing::warn!(id, provider = name, %error, "元数据查找失败"),
            None => {}
        }
        self.queue_status(provider, |status| {
            status.current = None;
            match &result {
                Ok(()) => status.done += 1,
                // 下一次查找之前已经要等一阵，这本留在队列里重查，不算失败。
                Err(_) if throttled => {}
                Err(_) if paused => status.paused = message.clone(),
                Err(_) => {
                    status.failed += 1;
                    status.last_error = message.clone();
                }
            }
        });
        // 停下或被限速时那本书留在队列里，之后重新查它。
        if !paused
            && !throttled
            && let Err(error) = self
                .blocking(move |app| app.write(|writer| Ok(writer.dequeue_metadata(id, name)?)))
                .await
        {
            tracing::error!(id, provider = name, %error, "从元数据队列去掉归档失败");
            tokio::time::sleep(RETRY_DELAY).await;
        }
    }
}

/// 这些错误说明后面的书也会一样失败，这个来源停下等用户处理，不把整个队列都标成失败。
fn stops_queue(error: &Error) -> bool {
    use shuku_metadata::Error as Metadata;
    match error {
        Error::Metadata(Metadata::Banned | Metadata::EmptyPage | Metadata::Http(_)) => true,
        // 令牌或 cookie 无效、对方出错。
        Error::Metadata(Metadata::Status(status)) => matches!(status, 401 | 403) || *status >= 500,
        Error::Config(_) | Error::Lock => true,
        Error::Store(error) => !matches!(error, shuku_store::Error::ArchiveNotFound(_)),
        _ => false,
    }
}
