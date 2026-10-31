use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use shuku_server::{App, Config};
use std::{net::SocketAddr, path::PathBuf};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
    #[arg(
        long,
        env = "SHUKU_NO_WEBTOON_DETECTION",
        global = true,
        help = "不按页面宽高比自动判断条漫（默认判断）"
    )]
    no_webtoon_detection: bool,
    #[arg(
        long = "allowed-host",
        env = "SHUKU_ALLOWED_HOSTS",
        value_delimiter = ',',
        global = true,
        help = "额外允许的主机名，可重复或用逗号分隔；IP、localhost 和不带点的主机名总是允许"
    )]
    allowed_hosts: Vec<String>,
    #[arg(
        long,
        env = "SHUKU_NO_WATCH",
        global = true,
        help = "不监视内容目录的变化（默认监视，有变化时自动扫描）"
    )]
    no_watch: bool,
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
    // 终端照常输出，同时把最近的日志留给控制台的日志页。
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_target(false),
        )
        .with(shuku_server::log_layer())
        .with(tracing_subscriber::filter::LevelFilter::INFO)
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
        detect_webtoons: !cli.no_webtoon_detection,
        allowed_hosts: cli.allowed_hosts,
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
    // 监视器建不起来时照常运行，只是新文件要等手动或下次启动扫描才入库。
    let _watcher = if cli.no_watch {
        None
    } else {
        app.watch_content(shuku_server::WATCH_QUIET, shuku_server::WATCH_MAX_DELAY)
            .inspect_err(|error| tracing::warn!(%error, "不监视内容目录"))
            .ok()
    };
    let listener = tokio::net::TcpListener::bind(cli.listen).await?;
    tracing::info!(listen = %listener.local_addr()?, "HTTP 服务已启动");
    app.spawn_scheduler();
    let background = app.clone();
    tokio::spawn(async move {
        let scanned = background.scan().await;
        background.spawn_metadata_worker();
        if let Err(error) = scanned {
            tracing::error!(%error, "启动扫描失败");
            return;
        }
        // 扫描先把新文件和变化的文件入库，再补算它们的完整指纹。
        if let Err(error) = background.hash_contents().await {
            tracing::error!(%error, "计算完整指纹失败");
        }
    });
    let stopper = app.clone();
    axum::serve(listener, app.router())
        .with_graceful_shutdown(async move {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(%error, "等待退出信号失败");
            }
            stopper.stop();
        })
        .await?;
    Ok(())
}
