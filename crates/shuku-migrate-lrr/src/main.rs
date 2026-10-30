use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use shuku_migrate_lrr::{RedisSource, migrate};
use shuku_store::Store;

/// 把 LANraragi 的标题、标签、分类、Tankoubon 和阅读进度导入 Shuku。
/// 先运行 `shuku scan` 让归档入库；可以反复运行，每次以 LRR 为准覆盖。
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
    // 文件名与 shuku-server 打开数据库时一致。
    let database = args.data_dir.join("shuku.sqlite");
    anyhow::ensure!(
        database.is_file(),
        "{} 不存在，请先运行 shuku scan",
        database.display()
    );
    let mut store = Store::open(&database)
        .with_context(|| format!("打开数据库 {} 失败", database.display()))?;
    let report = migrate(&mut source, &mut store, &args.lrr_content_dir)?;

    println!("导入归档 {} 本", report.archives);
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
            "{} 条 LRR 记录找不到对应文件，未导入：",
            report.unmatched.len()
        );
        for path in report.unmatched.iter().take(SHOWN_UNMATCHED) {
            println!("  {path}");
        }
    }
    Ok(())
}
