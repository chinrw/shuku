//! 联网的那一层。各个在线来源只经过 [`Http`]，测试里换成假的。

use crate::{Error, Result};
use serde_json::Value;
use std::time::Duration;

/// 对方慢的时候一页要好几秒，超过这么久算失败。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Bangumi 要求带能联系到作者的 User-Agent。
const USER_AGENT: &str = concat!(
    "shuku/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/chinrw/shuku)"
);

pub trait Http {
    fn get(&self, url: &str) -> impl Future<Output = Result<String>> + Send;
    fn post_json(&self, url: &str, body: &Value) -> impl Future<Output = Result<String>> + Send;
}

/// 每个请求都带上同一个请求头（E-Hentai 的 cookie、Bangumi 的令牌）的客户端。
/// 一个客户端只给一个来源用，请求头不会发到别的站。
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    header: Option<(reqwest::header::HeaderName, String)>,
}

impl Client {
    pub fn new(header: Option<(reqwest::header::HeaderName, String)>) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .user_agent(USER_AGENT)
                .build()?,
            header,
        })
    }

    async fn send(&self, builder: reqwest::RequestBuilder) -> Result<String> {
        let builder = match &self.header {
            Some((name, value)) => builder.header(name, value),
            None => builder,
        };
        let response = builder.send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::Throttled);
        }
        if !status.is_success() {
            return Err(Error::Status(status.as_u16()));
        }
        Ok(response.text().await?)
    }
}

impl Http for Client {
    async fn get(&self, url: &str) -> Result<String> {
        self.send(self.http.get(url)).await
    }

    async fn post_json(&self, url: &str, body: &Value) -> Result<String> {
        self.send(self.http.post(url).json(body)).await
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    /// 按顺序回放预设的回复，GET 和 POST 共用一个队列，并记下每个请求。
    pub(crate) struct FakeHttp {
        replies: Mutex<Vec<Result<String>>>,
        requests: Mutex<Vec<String>>,
    }

    impl FakeHttp {
        pub(crate) fn new(replies: Vec<Result<String>>) -> Self {
            Self {
                replies: Mutex::new(replies.into_iter().rev().collect()),
                requests: Mutex::new(Vec::new()),
            }
        }

        pub(crate) fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }

        fn reply(&self, request: String) -> Result<String> {
            self.requests.lock().unwrap().push(request);
            self.replies.lock().unwrap().pop().expect("多出来的请求")
        }
    }

    impl Http for FakeHttp {
        async fn get(&self, url: &str) -> Result<String> {
            self.reply(format!("GET {url}"))
        }

        async fn post_json(&self, url: &str, body: &Value) -> Result<String> {
            self.reply(format!("POST {url} {body}"))
        }
    }
}
