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
        help = "归档内容目录（只在删除书时写入，可以只读挂载）"
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
        env = "SHUKU_SETTINGS",
        global = true,
        help = "启动时套用的设置文件（JSON），写了的字段覆盖控制台里的值"
    )]
    settings: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 扫描内容目录并更新数据库，不启动 HTTP 服务。
    Scan,
    /// 管理设备令牌。服务运行时也可以用，新建和吊销马上生效。
    #[command(subcommand)]
    Token(TokenCommand),
}

#[derive(Subcommand)]
enum TokenCommand {
    /// 新建令牌并打印出来。令牌只显示这一次。
    Create {
        /// 设备名，例如“客厅 iPad”或“Claude”。
        name: String,
    },
    /// 列出全部令牌（不含令牌本身）。
    List,
    /// 吊销令牌。
    Revoke {
        /// `list` 里显示的 ID。
        id: i64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    // 终端照常输出，同时把最近的日志留给控制台的日志页。
    tracing_subscriber::registry()
        .with(
            // 日志写 stderr，stdout 留给命令的输出，例如 `token create` 打印的令牌。
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
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
    };
    let app = App::open(config).await?;
    if let Some(Command::Token(command)) = cli.command {
        return token_command(&app, command);
    }
    if let Some(path) = &cli.settings {
        app.apply_settings_file(path)
            .with_context(|| format!("套用设置文件 {} 失败", path.display()))?;
    }
    if matches!(cli.command, Some(Command::Scan)) {
        let report = app.scan().await?;
        anyhow::ensure!(
            report.failed == 0,
            "{} 个文件扫描失败，详情见日志",
            report.failed
        );
        return Ok(());
    }
    app.sync_watcher();
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

fn token_command(app: &App, command: TokenCommand) -> Result<()> {
    match command {
        TokenCommand::Create { name } => {
            let (record, token) = app.create_device_token(&name)?;
            println!("{token}");
            eprintln!(
                "已为“{}”新建令牌（ID {}），只显示这一次。",
                record.name, record.id
            );
        }
        TokenCommand::List => {
            for token in app.device_tokens()? {
                let used = token.last_used_at.map_or("从未使用".to_owned(), |ms| {
                    let at = std::time::UNIX_EPOCH
                        + std::time::Duration::from_millis(u64::try_from(ms).unwrap_or(0));
                    format!("最近使用 {}", httpdate::fmt_http_date(at))
                });
                println!("{}\t{}\t{used}", token.id, token.name);
            }
        }
        TokenCommand::Revoke { id } => {
            app.revoke_device_token(id)?;
            eprintln!("已吊销令牌 {id}");
        }
    }
    Ok(())
}
