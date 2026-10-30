#![allow(dead_code)]

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use image::{DynamicImage, ImageFormat, RgbImage};
use serde_json::Value;
use shuku_server::{App, Config};
use std::{
    fs::{self, File},
    io::{Cursor, Write},
    path::PathBuf,
};
use tempfile::TempDir;
use tower::ServiceExt;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

pub async fn request(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    headers: &[(&str, &str)],
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let body = match body {
        Some(value) => {
            request = request.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    (
        parts.status,
        parts.headers,
        to_bytes(body, usize::MAX).await.unwrap().to_vec(),
    )
}

pub async fn json(app: &Router, uri: &str) -> Value {
    let (status, _, bytes) = request(app, "GET", uri, None, &[]).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{uri}: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).unwrap()
}

pub struct Fixture {
    pub root: TempDir,
    pub config: Config,
}

impl Fixture {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            content_dir: root.path().join("content"),
            data_dir: root.path().join("data"),
            web_dir: root.path().join("web"),
            listen: "127.0.0.1:0".parse().unwrap(),
        };
        fs::create_dir(&config.content_dir).unwrap();
        fs::create_dir(&config.web_dir).unwrap();
        fs::write(config.web_dir.join("index.html"), "<html>书库</html>").unwrap();
        fs::write(config.web_dir.join("app.js"), "const shuku = true;").unwrap();
        Self { root, config }
    }

    pub fn archive(&self, name: &str, width: u32, height: u32) -> Vec<u8> {
        let path = self.config.content_dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(RgbImage::from_pixel(
            width,
            height,
            image::Rgb([120, 60, 30]),
        ))
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
        let bytes = bytes.into_inner();
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for (name, method) in [
            ("10.png", CompressionMethod::Deflated),
            ("2.png", CompressionMethod::Stored),
        ] {
            zip.start_file(
                name,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
            zip.write_all(&bytes).unwrap();
        }
        zip.finish().unwrap();
        bytes
    }

    pub async fn app(&self) -> App {
        App::open(self.config.clone()).await.unwrap()
    }

    pub fn database(&self) -> PathBuf {
        self.config.data_dir.join("shuku.sqlite")
    }
}
