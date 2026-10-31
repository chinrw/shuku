//! 拒绝 Host 不像是局域网访问的请求，防 DNS rebinding。
//!
//! 没有认证时，任意网页都可以把自己的域名解析到局域网里的服务端地址，浏览器把这当成
//! 同源请求，于是那个网页能读书库、触发控制台的任务。这类请求的 Host 是攻击者的域名，
//! 所以只接受 IP 地址、localhost、不带点的局域网主机名，以及 `--allowed-host` 列出的名字。

use crate::{App, Error};
use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::IpAddr;

pub(crate) async fn check_host(State(app): State<App>, request: Request, next: Next) -> Response {
    // 浏览器总会带 Host；没有 Host 的只能是直接发请求的工具，不存在 rebinding。
    let allowed = match request.headers().get(header::HOST) {
        None => true,
        Some(value) => value
            .to_str()
            .is_ok_and(|host| host_allowed(host, &app.0.config.allowed_hosts)),
    };
    if !allowed {
        return Error::Http {
            status: StatusCode::MISDIRECTED_REQUEST,
            message: "这个主机名不在允许列表里，用 --allowed-host 加上它".into(),
        }
        .into_response();
    }
    next.run(request).await
}

pub(crate) fn host_allowed(host: &str, extra: &[String]) -> bool {
    let name = strip_port(host).trim_end_matches('.');
    if name.is_empty() {
        return false;
    }
    if name.parse::<IpAddr>().is_ok() {
        return true;
    }
    let name = name.to_ascii_lowercase();
    // 公网域名至少有一个点；不带点的名字只能靠局域网的 DNS 或 hosts 文件解析，攻击者控制不了。
    !name.contains('.')
        || name.ends_with(".localhost")
        || extra
            .iter()
            .any(|allowed| allowed.trim_end_matches('.').eq_ignore_ascii_case(&name))
}

/// 去掉端口；IPv6 地址写在方括号里。
fn strip_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split_once(']').map_or(rest, |(address, _)| address);
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => name,
        _ => host,
    }
}

#[cfg(test)]
mod tests {
    use super::host_allowed;

    #[test]
    fn allows_addresses_local_names_and_configured_hosts() {
        let extra = vec!["books.example.ts.net".to_string()];
        for host in [
            "192.168.1.20:3002",
            "100.64.0.1",
            "[::1]:3002",
            "[fe80::1]",
            "localhost:3002",
            "media-lxc:3002",
            "MEDIA-LXC",
            "app.localhost",
            "Books.Example.ts.net:3002",
            "books.example.ts.net.",
        ] {
            assert!(host_allowed(host, &extra), "{host}");
        }
    }

    #[test]
    fn rejects_other_domain_names() {
        let extra = vec!["books.example.ts.net".to_string()];
        for host in [
            "evil.example",
            "evil.example:3002",
            "books.example.ts.net.evil.example",
            "media-lxc.local",
            "",
            ":3002",
        ] {
            assert!(!host_allowed(host, &extra), "{host}");
        }
    }
}
