use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use shuku_server::{App, Config};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(name = "shuku", about = "扫描和阅读本地漫画归档")]
struct Cli {
    #[arg(
        long,
        env = "SHUKU_CONTENT_DIR",
        global = true,
        help = "归档内容目录（只读）"
    )]
    content_dir: Option<PathBuf>,
    #[arg(
        long,
        env = "SHUKU_DATA_DIR",
        global = true,
        help = "SQLite 和缓存目录"
    )]
    data_dir: Option<PathBuf>,
    #[arg(
        long,
        env = "SHUKU_WEB_DIR",
        global = true,
        default_value = "web/build",
        help = "网页端构建目录"
    )]
    web_dir: PathBuf,
    #[arg(
        long,
        env = "SHUKU_LISTEN",
        global = true,
        default_value = "0.0.0.0:3002",
        help = "HTTP 监听地址"
    )]
    listen: SocketAddr,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 扫描内容目录并更新数据库，不启动 HTTP 服务。
    Scan,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .try_init()
        .map_err(|error| anyhow::anyhow!(error))?;
    let config = Config {
        content_dir: cli
            .content_dir
            .context("请设置 --content-dir 或 SHUKU_CONTENT_DIR")?,
        data_dir: cli
            .data_dir
            .context("请设置 --data-dir 或 SHUKU_DATA_DIR")?,
        web_dir: cli.web_dir,
        listen: cli.listen,
    };
    let app = App::open(config).await?;
    if matches!(cli.command, Some(Command::Scan)) {
        let report = app.scan().await?;
        anyhow::ensure!(
            report.failed == 0,
            "{} 个文件扫描失败，详情见日志",
            report.failed
        );
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind(cli.listen).await?;
    tracing::info!(listen = %listener.local_addr()?, "HTTP 服务已启动");
    let scanner = app.clone();
    tokio::spawn(async move {
        if let Err(error) = scanner.scan().await {
            tracing::error!(%error, "启动扫描失败");
        }
    });
    axum::serve(listener, app.router())
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(%error, "等待退出信号失败");
            }
        })
        .await?;
    Ok(())
}
