mod common;

use common::Fixture;
use shuku_store::Store;
use std::{
    fs::{self, File},
    io::{Read, Write},
    net::TcpStream,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_shuku"));
    for name in [
        "SHUKU_CONTENT_DIR",
        "SHUKU_DATA_DIR",
        "SHUKU_WEB_DIR",
        "SHUKU_LISTEN",
    ] {
        command.env_remove(name);
    }
    command
}

#[test]
fn scan_accepts_environment_and_cli_overrides_and_reports_failures() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 40, 60);
    let output = command()
        .arg("scan")
        .env("SHUKU_CONTENT_DIR", fixture.root.path().join("wrong"))
        .env("SHUKU_DATA_DIR", &fixture.config.data_dir)
        .arg("--content-dir")
        .arg(&fixture.config.content_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let logs = String::from_utf8(output.stdout).unwrap();
    assert!(logs.contains("索引构建完成"));
    assert!(logs.contains("elapsed_ms="));
    assert!(logs.contains("count=1"));
    assert!(
        Store::open(fixture.database())
            .unwrap()
            .find_by_path("book.cbz")
            .unwrap()
            .is_some()
    );
    assert!(!command().arg("scan").output().unwrap().status.success());
    fs::write(
        fixture.config.content_dir.join("broken.cbz"),
        b"PK\x03\x04broken",
    )
    .unwrap();
    let output = command()
        .arg("scan")
        .env("SHUKU_CONTENT_DIR", &fixture.config.content_dir)
        .env("SHUKU_DATA_DIR", &fixture.config.data_dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("broken.cbz")
    );
    let help = command().arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("0.0.0.0:3001"));
    assert!(help.contains("web/build"));
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn server_starts_background_scan_and_serves_http_on_configured_listener() {
    let fixture = Fixture::new();
    fixture.archive("book.cbz", 40, 60);
    let log_path = fixture.root.path().join("server.log");
    let log = File::create(&log_path).unwrap();
    let mut child = Running(
        command()
            .env("SHUKU_CONTENT_DIR", &fixture.config.content_dir)
            .env("SHUKU_DATA_DIR", &fixture.config.data_dir)
            .env("SHUKU_WEB_DIR", &fixture.config.web_dir)
            .env("SHUKU_LISTEN", "127.0.0.1:0")
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    let address = loop {
        let log = fs::read_to_string(&log_path).unwrap();
        if let Some(address) = log
            .split("listen=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
        {
            break address.to_string();
        }
        assert!(child.0.try_wait().unwrap().is_none(), "{log}");
        assert!(start.elapsed() < Duration::from_secs(15), "{log}");
        std::thread::sleep(Duration::from_millis(20));
    };
    loop {
        let response = get(&address, "/api/search");
        if response.contains("\"total\":1") {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(15), "{response}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(get(&address, "/reader/1").contains("<html>书库</html>"));
}

fn get(address: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}
