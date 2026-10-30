mod common;

use axum::http::StatusCode;
use common::{Fixture, json, request};
use image::GenericImageView;

#[tokio::test]
async fn every_archive_image_format_has_original_and_jpeg_variants() {
    use image::{DynamicImage, ImageFormat, RgbImage};
    use std::{
        fs::File,
        io::{Cursor, Write},
    };
    use zip::{ZipWriter, write::SimpleFileOptions};
    use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace, options::EncoderOptions};

    let source = DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 48, image::Rgb([120, 60, 30])));
    for (extension, format, mime) in [
        ("jpg", Some(ImageFormat::Jpeg), "image/jpeg"),
        ("jpeg", Some(ImageFormat::Jpeg), "image/jpeg"),
        ("png", Some(ImageFormat::Png), "image/png"),
        ("gif", Some(ImageFormat::Gif), "image/gif"),
        ("webp", Some(ImageFormat::WebP), "image/webp"),
        ("avif", Some(ImageFormat::Avif), "image/avif"),
        ("jxl", None, "image/jxl"),
        ("bmp", Some(ImageFormat::Bmp), "image/bmp"),
    ] {
        let mut bytes = Cursor::new(Vec::new());
        if let Some(format) = format {
            source.write_to(&mut bytes, format).unwrap();
        } else {
            zune_jpegxl::JxlSimpleEncoder::new(
                source.as_bytes(),
                EncoderOptions::new(32, 48, ColorSpace::RGB, BitDepth::Eight),
            )
            .encode(bytes.get_mut())
            .unwrap();
        }
        let bytes = bytes.into_inner();
        let fixture = Fixture::new();
        let mut zip =
            ZipWriter::new(File::create(fixture.config.content_dir.join("book.cbz")).unwrap());
        zip.start_file(format!("1.{extension}"), SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&bytes).unwrap();
        zip.finish().unwrap();
        let app = fixture.app().await;
        assert_eq!(app.scan().await.unwrap().added, 1, "{extension}");
        let router = app.router();
        let item = &json(&router, "/api/search").await["items"][0];
        let pages = json(&router, &format!("/api/archives/{}/pages", item["id"])).await;
        let url = pages["pages"][0]["url"].as_str().unwrap();
        let (status, headers, original) = request(&router, "GET", url, None, &[]).await;
        assert_eq!(status, StatusCode::OK, "{extension}");
        assert_eq!(headers["content-type"], mime);
        assert_eq!(original, bytes);
        for (uri, dimensions) in [
            (format!("{url}&w=16"), (16, 24)),
            (item["cover_url"].as_str().unwrap().into(), (32, 48)),
        ] {
            let (status, headers, jpeg) = request(&router, "GET", &uri, None, &[]).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "{extension}: {}",
                String::from_utf8_lossy(&jpeg)
            );
            assert_eq!(headers["content-type"], "image/jpeg");
            assert_eq!(
                image::load_from_memory(&jpeg).unwrap().dimensions(),
                dimensions
            );
        }
    }
}

#[tokio::test]
async fn reads_originals_resizes_without_enlarging_and_caches_with_conditional_requests() {
    let fixture = Fixture::new();
    let original = fixture.archive("book.cbz", 800, 1200);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let item = &json(&router, "/api/search").await["items"][0];
    let id = item["id"].as_i64().unwrap();
    let listing = json(&router, &format!("/api/archives/{id}/pages")).await;
    let url = listing["pages"][0]["url"].as_str().unwrap();
    for page in listing["pages"].as_array().unwrap() {
        let (status, headers, bytes) =
            request(&router, "GET", page["url"].as_str().unwrap(), None, &[]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(bytes, original);
        assert_eq!(headers["content-type"], "image/png");
        assert_eq!(
            headers["cache-control"],
            "public, max-age=31536000, immutable"
        );
        let etag = headers["etag"].to_str().unwrap();
        for condition in [etag.to_string(), format!("\"other\", W/{etag}"), "*".into()] {
            let (status, headers, body) = request(
                &router,
                "GET",
                page["url"].as_str().unwrap(),
                None,
                &[("if-none-match", &condition)],
            )
            .await;
            assert_eq!(status, StatusCode::NOT_MODIFIED);
            assert!(body.is_empty());
            assert_eq!(headers["etag"], etag);
            assert_eq!(
                headers["cache-control"],
                "public, max-age=31536000, immutable"
            );
        }
    }
    let original_etag = request(&router, "GET", url, None, &[]).await.1["etag"].clone();
    let small_url = format!("{url}&w=400");
    let (status, small_headers, small) = request(&router, "GET", &small_url, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(small_headers["content-type"], "image/jpeg");
    assert_ne!(small_headers["etag"], original_etag);
    assert_eq!(
        image::load_from_memory(&small).unwrap().dimensions(),
        (400, 600)
    );
    assert_eq!(
        request(
            &router,
            "GET",
            &small_url,
            None,
            &[("if-none-match", small_headers["etag"].to_str().unwrap())]
        )
        .await
        .0,
        StatusCode::NOT_MODIFIED
    );
    let (_, headers, bytes) = request(&router, "GET", &format!("{url}&w=1000"), None, &[]).await;
    assert_eq!(bytes, original);
    assert_eq!(headers["content-type"], "image/png");
    assert_eq!(headers["etag"], original_etag);
    let cover_url = item["cover_url"].as_str().unwrap();
    let (status, headers, cover) = request(&router, "GET", cover_url, None, &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/jpeg");
    assert_eq!(
        image::load_from_memory(&cover).unwrap().dimensions(),
        (500, 750)
    );
    assert_eq!(
        request(
            &router,
            "GET",
            cover_url,
            None,
            &[("if-none-match", headers["etag"].to_str().unwrap())]
        )
        .await
        .0,
        StatusCode::NOT_MODIFIED
    );
    let cache: Vec<_> = walkdir::WalkDir::new(&fixture.config.data_dir)
        .into_iter()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jpg"))
        .map(|entry| entry.into_path())
        .collect();
    assert_eq!(cache.len(), 2);
    let before: Vec<_> = cache
        .iter()
        .map(|path| std::fs::metadata(path).unwrap().modified().unwrap())
        .collect();
    let (a, b) = tokio::join!(
        request(&router, "GET", &small_url, None, &[]),
        request(&router, "GET", &small_url, None, &[])
    );
    assert_eq!(a.2, small);
    assert_eq!(b.2, small);
    assert_eq!(
        before,
        cache
            .iter()
            .map(|path| std::fs::metadata(path).unwrap().modified().unwrap())
            .collect::<Vec<_>>()
    );
    for (uri, expected) in [
        (format!("{url}&w=0"), StatusCode::BAD_REQUEST),
        (format!("{url}&w=-1"), StatusCode::BAD_REQUEST),
        (
            format!("/api/archives/{id}/pages/0"),
            StatusCode::BAD_REQUEST,
        ),
        (
            format!("/api/archives/{id}/pages/0?v=stale"),
            StatusCode::NOT_FOUND,
        ),
        (url.replace("/pages/0", "/pages/2"), StatusCode::NOT_FOUND),
        (
            url.replace("/pages/0", "/pages/-1"),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, _, body) = request(&router, "GET", &uri, None, &[]).await;
        assert_eq!(status, expected, "{uri}");
        assert!(serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"].is_string());
    }
    fixture.archive("book.cbz", 900, 1300);
    assert_eq!(
        request(&router, "GET", url, None, &[]).await.0,
        StatusCode::CONFLICT
    );
    app.scan().await.unwrap();
    assert_eq!(
        request(&router, "GET", url, None, &[]).await.0,
        StatusCode::NOT_FOUND
    );
    assert_ne!(
        json(&router, &format!("/api/archives/{id}/pages")).await["pages"][0]["url"],
        url
    );
}

#[tokio::test]
async fn thumbnail_of_small_cover_is_jpeg_without_enlargement() {
    let fixture = Fixture::new();
    fixture.archive("small.cbz", 20, 30);
    let app = fixture.app().await;
    app.scan().await.unwrap();
    let router = app.router();
    let item = &json(&router, "/api/search").await["items"][0];
    let (status, headers, bytes) = request(
        &router,
        "GET",
        item["cover_url"].as_str().unwrap(),
        None,
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/jpeg");
    assert_eq!(
        image::load_from_memory(&bytes).unwrap().dimensions(),
        (20, 30)
    );
}
