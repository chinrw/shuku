use crate::{App, Error, Result};
use axum::{
    body::Body,
    extract::{
        Path as UrlPath, Query, State,
        rejection::{PathRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use fast_image_resize::{PixelType, Resizer, images::Image};
use serde::Deserialize;
use shuku_store::CompressionMethod;
use std::{fs, io::Write, path::Path};

/// 每个宽度都会在数据目录里缓存一份 JPEG，任意宽度会让缓存无限增长。网页端的阅读器用原图，
/// 这几档留给需要省流量的客户端。
const WIDTHS: [u32; 4] = [720, 1080, 1440, 2160];

/// 缩放要把整张图解码进内存，超过这个像素数的图片不缩放。
/// 约 800×60000 的长条漫画仍在范围内。AVIF 解码器没有 image 那样的内存上限，靠这里限制。
const MAX_PIXELS: u64 = 64_000_000;

#[derive(Deserialize)]
pub(crate) struct ImageParams {
    v: String,
    w: Option<u32>,
}

pub(crate) async fn page(
    State(app): State<App>,
    path: std::result::Result<UrlPath<(i64, usize)>, PathRejection>,
    params: std::result::Result<Query<ImageParams>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Response> {
    let UrlPath((id, n)) = path.map_err(Error::bad_request)?;
    let Query(params) = params.map_err(Error::bad_request)?;
    if params.w.is_some_and(|width| !WIDTHS.contains(&width)) {
        return Err(Error::bad_request("w 只能是 720、1080、1440 或 2160"));
    }
    app.blocking(move |app| respond(&app, id, Which::Page(n), params, headers))
        .await
}

pub(crate) async fn thumbnail(
    State(app): State<App>,
    path: std::result::Result<UrlPath<i64>, PathRejection>,
    params: std::result::Result<Query<ImageParams>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Response> {
    let UrlPath(id) = path.map_err(Error::bad_request)?;
    let Query(params) = params.map_err(Error::bad_request)?;
    app.blocking(move |app| respond(&app, id, Which::Cover, params, headers))
        .await
}

/// 详情页页面总览里的一格。和封面同样缩到 500 宽，第 0 页与封面共用缓存。
pub(crate) async fn page_thumbnail(
    State(app): State<App>,
    path: std::result::Result<UrlPath<(i64, usize)>, PathRejection>,
    params: std::result::Result<Query<ImageParams>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Response> {
    let UrlPath((id, n)) = path.map_err(Error::bad_request)?;
    let Query(params) = params.map_err(Error::bad_request)?;
    app.blocking(move |app| respond(&app, id, Which::Thumbnail(n), params, headers))
        .await
}

/// 要哪一页、要不要缩成缩略图。
#[derive(Clone, Copy)]
enum Which {
    Page(usize),
    Thumbnail(usize),
    /// 第一张读得出尺寸的页的缩略图，封面坏了也有图；全是坏页时仍用第一页，解码失败返回 422。
    Cover,
}

fn respond(
    app: &App,
    id: i64,
    which: Which,
    params: ImageParams,
    headers: HeaderMap,
) -> Result<Response> {
    let thumbnail = !matches!(which, Which::Page(_));
    let (archive, n, page) = {
        let (archive, pages) = app
            .reader()?
            .archive_with_pages(id)?
            .ok_or_else(Error::not_found)?;
        if params.v != archive.metadata.quick_hash {
            return Err(Error::not_found());
        }
        let n = match which {
            Which::Page(n) | Which::Thumbnail(n) => n,
            Which::Cover => pages.iter().position(|page| page.width > 0).unwrap_or(0),
        };
        let page = pages.into_iter().nth(n).ok_or_else(Error::not_found)?;
        (archive, n, page)
    };
    // 路径由扫描写入，是内容目录里的普通文件或散放图片的文件夹。内容被替换但还没重新扫描时，
    // 记录的偏移已经失效，按旧 URL 返回新内容会被浏览器永久缓存，所以先比对大小和修改时间。
    let path = app.0.config.content_dir.join(&archive.metadata.path);
    let identity = shuku_archive::identity(&path).map_err(|error| match error {
        shuku_archive::Error::Io(error) => file_error(error),
        error => error.into(),
    })?;
    if identity.size != archive.metadata.file_size || identity.modified != archive.metadata.mtime {
        return Err(changed_file());
    }
    if thumbnail && page.width == 0 {
        return Err(broken_page());
    }
    let width = if thumbnail {
        Some(500.min(page.width))
    } else {
        params.w.filter(|width| *width < page.width)
    };
    if width.is_some() && u64::from(page.width) * u64::from(page.height) > MAX_PIXELS {
        return Err(Error::Http {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "图片像素太多，不能缩放".into(),
        });
    }
    // 以前的版本对动图也缓存过静帧 JPEG，页面变体换个名字（`w1440`），数据目录里的旧缓存
    // 就不再使用。缩略图仍是 `jpeg-500`。
    let variant = match width {
        Some(width) if thumbnail => format!("jpeg-{width}"),
        Some(width) => format!("w{width}"),
        None => "original".into(),
    };
    let key = format!("{id}-{}-{n}-{variant}", archive.metadata.quick_hash);
    let etag = format!("\"{key}\"");
    let mut response = Response::new(Body::empty());
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).map_err(|error| Error::File(error.to_string()))?,
    );
    for condition in headers.get_all(header::IF_NONE_MATCH) {
        let condition = condition.to_str().map_err(Error::bad_request)?;
        if condition.split(',').any(|tag| {
            let tag = tag.trim();
            tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == etag
        }) {
            *response.status_mut() = StatusCode::NOT_MODIFIED;
            return Ok(response);
        }
    }
    let read_original = || -> Result<Vec<u8>> {
        let bytes = shuku_archive::read_page(
            &path,
            &shuku_archive::Page {
                entry_name: page.entry_name.clone(),
                data_offset: page.data_offset,
                compressed_size: page.compressed_size,
                size: page.size,
                method: match page.method {
                    CompressionMethod::Stored => shuku_archive::CompressionMethod::Stored,
                    CompressionMethod::Deflate => shuku_archive::CompressionMethod::Deflate,
                },
                width: page.width,
                height: page.height,
            },
        )?;
        Ok(bytes)
    };
    let (bytes, content_type) = match width {
        Some(width) => {
            let directory = app.0.config.data_dir.join("cache/images");
            let cache_path = directory.join(format!("{key}.jpg"));
            match fs::read(&cache_path) {
                Ok(bytes) => (bytes, "image/jpeg"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let original = read_original()?;
                    // 转成 JPEG 会丢掉动画，手机上的阅读器对每页都带 `w`，所以动图原样返回。
                    if !thumbnail && animated(&original) {
                        let content_type = content_type(&original)?;
                        (original, content_type)
                    } else {
                        let bytes = resize(&original, width)?;
                        fs::create_dir_all(&directory)?;
                        write_cache(&cache_path, &bytes)?;
                        (bytes, "image/jpeg")
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        None => {
            let bytes = read_original()?;
            let content_type = content_type(&bytes)?;
            (bytes, content_type)
        }
    };
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    *response.body_mut() = Body::from(bytes);
    Ok(response)
}

fn resize(bytes: &[u8], width: u32) -> Result<Vec<u8>> {
    let source = crate::decode::decode(bytes)?.to_rgb8();
    let width = width.min(source.width());
    let height =
        (u64::from(source.height()) * u64::from(width) / u64::from(source.width())).max(1) as u32;
    let mut target = Image::new(width, height, PixelType::U8x3);
    Resizer::new().resize(&source, &mut target, None)?;
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 85).encode(
        target.buffer(),
        width,
        height,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(bytes)
}

fn write_cache(path: &Path, bytes: &[u8]) -> Result<()> {
    // 同一目录里的 rename 原子发布，重复生成也不会暴露未写完的 JPEG。
    // 临时文件名没有 .jpg 扩展名，控制台清空缓存时靠这一点跳过生成中的文件。
    let parent = path
        .parent()
        .ok_or_else(|| Error::File(path.display().to_string()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// GIF 一律算动图：静态的大 GIF 很少，调色板图转 JPEG 也不省多少。PNG 看 IDAT 之前有没有
/// APNG 的 acTL 块，WebP 看 VP8X 头的动画标志。
fn animated(bytes: &[u8]) -> bool {
    if bytes.starts_with(b"GIF8") {
        return true;
    }
    if let Some(mut chunks) = bytes.strip_prefix(b"\x89PNG\r\n\x1a\n") {
        while let [a, b, c, d, kind @ ..] = chunks
            && kind.len() >= 4
        {
            let length = u32::from_be_bytes([*a, *b, *c, *d]) as usize;
            match &kind[..4] {
                b"acTL" => return true,
                b"IDAT" => return false,
                _ => {}
            }
            // 类型 4 字节、数据、CRC 4 字节。
            let Some(rest) = kind.get(length.saturating_add(8)..) else {
                return false;
            };
            chunks = rest;
        }
        return false;
    }
    bytes.len() > 20
        && &bytes[..4] == b"RIFF"
        && &bytes[8..16] == b"WEBPVP8X"
        && bytes[20] & 0x02 != 0
}

/// 认不出格式的是坏页（全是 0、0 字节、改了扩展名的别的文件），返回 422。
fn content_type(bytes: &[u8]) -> Result<&'static str> {
    if crate::decode::is_jxl(bytes) {
        return Ok("image/jxl");
    }
    image::guess_format(bytes)
        .map(|format| format.to_mime_type())
        .map_err(|_| broken_page())
}

pub(crate) fn broken_page() -> Error {
    Error::Http {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        message: "页面图片已损坏".into(),
    }
}

fn changed_file() -> Error {
    Error::Http {
        status: StatusCode::CONFLICT,
        message: "归档文件已变化，需要重新扫描".into(),
    }
}

fn file_error(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::NotFound {
        Error::not_found()
    } else {
        error.into()
    }
}

#[cfg(test)]
mod tests {
    use super::animated;

    fn png(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in chunks {
            bytes.extend((data.len() as u32).to_be_bytes());
            bytes.extend(*kind);
            bytes.extend(*data);
            bytes.extend([0; 4]);
        }
        bytes
    }

    #[test]
    fn detects_apng_only_when_actl_precedes_idat() {
        let header: &[u8] = &[0; 13];
        assert!(animated(&png(&[
            (b"IHDR", header),
            (b"acTL", &[0; 8]),
            (b"IDAT", &[1])
        ])));
        assert!(!animated(&png(&[
            (b"IHDR", header),
            (b"IDAT", &[1]),
            (b"acTL", &[0; 8])
        ])));
        assert!(!animated(&png(&[(b"IHDR", header), (b"IDAT", &[1])])));
        // 块长度超出文件时不越界。
        let mut truncated = png(&[(b"IHDR", header)]);
        truncated[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(!animated(&truncated));
    }

    #[test]
    fn detects_animated_webp_from_the_vp8x_flags() {
        let webp = |flags: u8| {
            let mut bytes = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0".to_vec();
            bytes.push(flags);
            bytes.extend([0; 9]);
            bytes
        };
        assert!(animated(&webp(0x02)));
        assert!(!animated(&webp(0x10)));
        assert!(!animated(b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0\0\0\0\0\0\0"));
    }
}
