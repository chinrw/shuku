use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use shuku_migrate_lrr::{RedisSource, migrate};
use shuku_store::Store;

/// 把 LANraragi 的标题、标签、加入时间、分类、Tankoubon 和阅读进度导入 Shuku。
/// 还没扫描入库的归档先暂存，`shuku` 扫描到时套用；可以反复运行，每次以 LRR 为准覆盖。
#[derive(Parser)]
struct Args {
    /// LRR 的 Redis 地址，库编号写在路径里。
    #[arg(long, default_value = "redis://127.0.0.1:6379/0")]
    redis: String,
    /// LRR 看到的内容目录，用来把 LRR 记录的绝对路径换成相对路径。
    #[arg(long)]
    lrr_content_dir: String,
    /// Shuku 的数据目录，与 `shuku` 服务使用同一个。
    #[arg(long, env = "SHUKU_DATA_DIR")]
    data_dir: PathBuf,
}

/// 报告里最多列出这么多个没找到文件的路径。
const SHOWN_UNMATCHED: usize = 20;

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut source = RedisSource::connect(&args.redis)
        .with_context(|| format!("连接 Redis {} 失败", args.redis))?;
    std::fs::create_dir_all(&args.data_dir)
        .with_context(|| format!("创建数据目录 {} 失败", args.data_dir.display()))?;
    // 文件名与 shuku-server 打开数据库时一致。
    let database = args.data_dir.join("shuku.sqlite");
    let mut store = Store::open(&database)
        .with_context(|| format!("打开数据库 {} 失败", database.display()))?;
    let report = migrate(&mut source, &mut store, &args.lrr_content_dir)?;

    println!("导入已入库的归档 {} 本", report.archives);
    if report.pending > 0 {
        println!(
            "{} 本还没扫描入库，已暂存，扫描到时自动套用",
            report.pending
        );
    }
    if report.undated > 0 {
        println!(
            "{} 本在 LRR 里没有 date_added 标签，用文件的修改时间作为加入时间",
            report.undated
        );
    }
    println!(
        "导入分类 {} 个，Tankoubon {} 个",
        report.categories, report.tankoubons
    );
    if report.missing_members > 0 {
        println!(
            "分类和 Tankoubon 中有 {} 个成员没有对应的归档",
            report.missing_members
        );
    }
    if !report.unmatched.is_empty() {
        println!(
            "{} 条 LRR 记录的路径不在 --lrr-content-dir 下，未导入：",
            report.unmatched.len()
        );
        for path in report.unmatched.iter().take(SHOWN_UNMATCHED) {
            println!("  {path}");
        }
    }
    Ok(())
}
