use crate::{App, Error, Result, api::require_archive};
use axum::{
    body::Body,
    extract::{Path, Request, State, rejection::PathRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use tower::ServiceExt;

/// 下载原始归档文件。ServeFile 处理 Range 和 If-Modified-Since，大文件可以断点续传。
pub(crate) async fn file(
    State(app): State<App>,
    id: std::result::Result<Path<i64>, PathRejection>,
    request: Request,
) -> Result<Response> {
    let Path(id) = id.map_err(Error::bad_request)?;
    let relative = app
        .blocking(move |app| {
            let reader = app.reader()?;
            Ok(require_archive(&reader, id)?.metadata.path)
        })
        .await?;
    // 路径由扫描写入，只会是内容目录里的普通文件。文件已经不在时按不存在的归档处理，
    // 不让 ServeFile 返回没有错误说明的 404。
    let path = app.0.config.content_dir.join(&relative);
    if !tokio::fs::try_exists(&path).await? {
        return Err(Error::not_found());
    }
    // 散放图片的文件夹没有单个原文件。
    if tokio::fs::metadata(&path).await?.is_dir() {
        return Err(Error::Http {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "这本书是散放图片的文件夹，没有原文件可以下载".into(),
        });
    }
    let name = std::path::Path::new(&relative)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (parts, _) = request.into_parts();
    let serve = |headers: HeaderMap| {
        let mut request = Request::new(Body::empty());
        *request.method_mut() = parts.method.clone();
        *request.uri_mut() = parts.uri.clone();
        *request.headers_mut() = headers;
        tower_http::services::ServeFile::new(&path).oneshot(request)
    };
    let mut response = serve(parts.headers.clone())
        .await
        .unwrap_or_else(|never| match never {});
    // ServeFile 不看 If-Range。文件在两次请求之间被替换时，续传会把旧文件的前半段和
    // 新文件的后半段拼在一起。Last-Modified 取自 ServeFile 实际打开的文件，
    // 和条件不一致时去掉 Range 重新取整个文件。新文件比旧的短时 Range 可能越界，416 也一样处理。
    if matches!(
        response.status(),
        StatusCode::PARTIAL_CONTENT | StatusCode::RANGE_NOT_SATISFIABLE
    ) && let Some(condition) = parts.headers.get(header::IF_RANGE)
        && !same_version(condition, response.headers().get(header::LAST_MODIFIED))
    {
        let mut headers = parts.headers.clone();
        headers.remove(header::RANGE);
        response = serve(headers).await.unwrap_or_else(|never| match never {});
    }
    let status = response.status();
    if status.is_client_error() || status.is_server_error() {
        let mut error = Error::Http {
            status,
            message: match status {
                StatusCode::RANGE_NOT_SATISFIABLE => "Range 超出文件大小或格式不支持".into(),
                _ => status.canonical_reason().unwrap_or("下载失败").into(),
            },
        }
        .into_response();
        // 416 的 `Content-Range: bytes */<长度>` 告诉续传的客户端文件现在有多长。
        if let Some(range) = response.headers().get(header::CONTENT_RANGE) {
            error
                .headers_mut()
                .insert(header::CONTENT_RANGE, range.clone());
        }
        return Ok(error);
    }
    let mut response = response.map(Body::new);
    if status == StatusCode::NOT_MODIFIED {
        return Ok(response);
    }
    // ServeFile 按扩展名猜类型，.cbz 会被标成 application/x-cbr。浏览器见到 application/zip
    // 可能把文件名改成 .zip，所以统一用 octet-stream，让文件名保持原样。
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition(&name))
            .map_err(|error| Error::File(error.to_string()))?,
    );
    Ok(response)
}

/// If-Range 只认 ServeFile 发出的 Last-Modified（精确到秒）。响应不带 ETag，
/// 所以 ETag 形式的条件和解析不了的日期都当作不成立。
fn same_version(condition: &HeaderValue, modified: Option<&HeaderValue>) -> bool {
    let date = |value: &HeaderValue| {
        value
            .to_str()
            .ok()
            .and_then(|value| httpdate::parse_http_date(value).ok())
    };
    match (date(condition), modified.and_then(date)) {
        (Some(condition), Some(modified)) => condition == modified,
        _ => false,
    }
}

/// `filename*` 带 UTF-8 的原名；不认识它的客户端用 `filename`，里面的非 ASCII 字符换成下划线。
fn content_disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::content_disposition;

    #[test]
    fn content_disposition_keeps_utf8_name_and_ascii_fallback() {
        assert_eq!(
            content_disposition("作品 \"1\".cbz"),
            "attachment; filename=\"__ _1_.cbz\"; filename*=UTF-8''%E4%BD%9C%E5%93%81%20%221%22.cbz"
        );
    }
}
