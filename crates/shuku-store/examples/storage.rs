//! 在 30 万本合成书库上测量存储层：导入、启动时读出快照、单本读取，以及并发读写。
//!
//! 运行：`nix develop -c cargo run --release -p shuku-store --example storage -- <空目录>`
//!
//! 标签分布与 shuku-index 的 synthetic 基准相同：每本一个语言标签加 9 个按 Zipf 分布抽取的标签，
//! 另外每本 10 到 70 页，共约 1200 万页。随机数种子固定，每次生成的书库相同。
//! 环境变量 `ARCHIVES` 可以改书库大小。冷启动用 `posix_fadvise(DONTNEED)` 丢掉数据库文件的
//! 页缓存来模拟，不重启机器。结果每行一项，制表符分隔。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use shuku_index::Index;
use shuku_store::{ArchiveMetadata, CompressionMethod, Page, Reader, ReaderPool, Store, Tag};

const SEED: u64 = 0x5348_554b_5501;
const ZIPF_TAGS: usize = 78_000;
const ZIPF_TAGS_PER_ARCHIVE: usize = 9;
const LANGUAGES: [(&str, u32); 3] = [
    ("language:japanese", 56),
    ("language:chinese", 28),
    ("language:english", 16),
];
const HOT_TAGS: &[&str] = &[
    "other:full color",
    "female:sole female",
    "male:sole male",
    "other:uncensored",
    "female:glasses",
    "female:stockings",
    "female:schoolgirl uniform",
    "female:ponytail",
    "female:twintails",
    "other:multi-work series",
];
const TAIL_NAMESPACES: [&str; 5] = ["artist", "artist", "group", "character", "parody"];
const WORDS: &[&str] = &[
    "魔法少女",
    "放課後",
    "秘密",
    "先生",
    "夏休み",
    "恋人",
    "日記",
    "幼なじみ",
    "温泉",
    "旅行",
    "妹妹",
    "学姐",
    "青梅竹马",
    "图书馆",
    "Summer",
    "Diary",
    "Secret",
    "Love",
    "Story",
    "Days",
];
/// 导入时每个事务写这么多本，与扫描和迁移的批量写入相当。
const IMPORT_BATCH: usize = 1000;
const POINT_READS: usize = 20_000;
const READERS: usize = 8;
const CONCURRENT: Duration = Duration::from_secs(10);

struct Generator {
    cdf: Vec<f64>,
    names: Vec<String>,
}

struct Book {
    metadata: ArchiveMetadata,
    tags: Vec<Tag>,
    pages: Vec<Page>,
}

impl Generator {
    fn new() -> Self {
        let weights: Vec<f64> = (1..=ZIPF_TAGS).map(|rank| 1.0 / rank as f64).collect();
        let total: f64 = weights.iter().sum();
        let cdf = weights
            .iter()
            .scan(0.0, |sum, weight| {
                *sum += weight / total;
                Some(*sum)
            })
            .collect();
        let names = (0..ZIPF_TAGS)
            .map(|rank| match HOT_TAGS.get(rank) {
                Some(name) => name.to_string(),
                None => {
                    let namespace = TAIL_NAMESPACES[rank % TAIL_NAMESPACES.len()];
                    format!("{namespace}:{namespace}{rank}")
                }
            })
            .collect();
        Self { cdf, names }
    }

    /// 同一个 `(id, version)` 总是生成同一本书；version 不同时模拟重新扫描到改动过的文件。
    fn book(&self, id: i64, version: u64) -> Book {
        let mut rng =
            StdRng::seed_from_u64(SEED ^ (id as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ version);
        let mut roll = rng.random_range(0..100);
        let mut language = LANGUAGES[0].0;
        for (name, percent) in LANGUAGES {
            if roll < percent {
                language = name;
                break;
            }
            roll -= percent;
        }
        let mut tags = vec![split(language)];
        let mut ranks = Vec::new();
        while ranks.len() < ZIPF_TAGS_PER_ARCHIVE {
            let draw: f64 = rng.random();
            let rank = self.cdf.partition_point(|&p| p <= draw).min(ZIPF_TAGS - 1);
            if !ranks.contains(&rank) {
                ranks.push(rank);
                tags.push(split(&self.names[rank]));
            }
        }
        let words: Vec<&str> = (0..rng.random_range(1..=3))
            .map(|_| WORDS[rng.random_range(..WORDS.len())])
            .collect();
        let title = format!(
            "[author{}] {}",
            rng.random_range(0..20_000),
            words.join(" ")
        );
        let folder = rng.random_bool(0.3);
        let mut offset = 0;
        let pages = (0..rng.random_range(10..=70))
            .map(|number| {
                let size = rng.random_range(200_000..2_000_000);
                let page = Page {
                    entry_name: if folder {
                        format!("Chapter {}/{number:04}.jpg", number / 20 + 1)
                    } else {
                        format!("{number:04}.webp")
                    },
                    data_offset: offset + 60,
                    compressed_size: size,
                    size,
                    method: CompressionMethod::Stored,
                    width: 1280,
                    height: rng.random_range(1600..2000),
                };
                offset += size + 60;
                page
            })
            .collect();
        let metadata = ArchiveMetadata {
            path: format!("library/author{}/{}-{id}.zip", id % 5000, title.len()),
            summary: String::new(),
            file_size: offset + 4096,
            mtime: 1_600_000_000 + rng.random_range(0..200_000_000) + version as i64,
            quick_hash: format!("{:032x}", rng.random::<u128>()),
            content_hash: rng
                .random_bool(0.8)
                .then(|| format!("{:064x}", rng.random::<u128>())),
            date_added: 1_600_000_000 + rng.random_range(0..200_000_000),
            source_url: rng.random_bool(0.7).then(|| {
                format!(
                    "https://example.org/g/{id}/{:010x}/",
                    rng.random::<u64>() >> 24
                )
            }),
            is_new: rng.random_bool(0.1),
            lrr_id: Some(format!("{:040x}", rng.random::<u128>())),
            title,
        };
        Book {
            metadata,
            tags,
            pages,
        }
    }
}

fn split(full: &str) -> Tag {
    let (namespace, value) = full.split_once(':').unwrap_or(("", full));
    Tag {
        namespace: namespace.into(),
        value: value.into(),
    }
}

fn report(metric: &str, value: impl std::fmt::Display) {
    println!("{metric}\t{value}");
}

fn latencies(metric: &str, mut samples: Vec<Duration>) {
    samples.sort_unstable();
    let at = |quantile: f64| samples[((samples.len() - 1) as f64 * quantile) as usize];
    let micros = |duration: Duration| format!("{:.1}", duration.as_secs_f64() * 1e6);
    report(&format!("{metric}.p50_us"), micros(at(0.5)));
    report(&format!("{metric}.p99_us"), micros(at(0.99)));
    report(&format!("{metric}.max_us"), micros(at(1.0)));
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let value = work();
    (value, start.elapsed())
}

/// glibc 所有 arena 里仍在使用的堆内存，用来估算快照的大小。
fn heap_bytes() -> usize {
    // SAFETY: mallinfo2 没有参数，只读取分配器的统计。
    let info = unsafe { libc::mallinfo2() };
    info.uordblks + info.hblkhd
}

fn mib(bytes: impl Into<f64>) -> String {
    format!("{:.0}", bytes.into() / 1_048_576.0)
}

fn database_bytes(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |metadata| metadata.len())
}

/// 丢掉数据库文件的页缓存。脏页丢不掉，所以先 fdatasync。
fn evict(path: &Path) {
    use std::os::fd::AsRawFd;
    for suffix in ["", "-wal"] {
        let path = path.with_file_name(format!("shuku.sqlite{suffix}"));
        if let Ok(file) = std::fs::File::open(&path) {
            file.sync_data().unwrap();
            // SAFETY: fd 在 file 存活期间有效，fadvise 只影响页缓存。
            unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
        }
    }
}

fn import(store: &mut Store, generator: &Generator, archives: i64) {
    let start = Instant::now();
    let mut pages = 0;
    for first in (1..=archives).step_by(IMPORT_BATCH) {
        let last = (first + IMPORT_BATCH as i64 - 1).min(archives);
        store
            .write(|writer| {
                for id in first..=last {
                    let book = generator.book(id, 0);
                    pages += book.pages.len();
                    let saved = writer.save_archive(None, &book.metadata, &book.pages)?;
                    writer.replace_tags(saved, &book.tags)?;
                }
                Ok::<_, shuku_store::Error>(())
            })
            .unwrap();
    }
    report("import_s", format!("{:.1}", start.elapsed().as_secs_f64()));
    report("import_pages", pages);
}

/// 和服务端启动时一样：从只读连接读出快照的原料，再建索引。
fn load_snapshot(reader: &Reader, label: &str) {
    let heap = heap_bytes();
    let (library, read) = timed(|| reader.library().unwrap());
    let listing = heap_bytes() - heap;
    let (index, build) = timed(|| Index::build(library.index_records()).unwrap());
    report(
        &format!("{label}.read_s"),
        format!("{:.2}", read.as_secs_f64()),
    );
    report(
        &format!("{label}.build_s"),
        format!("{:.2}", build.as_secs_f64()),
    );
    report(&format!("{label}.library_mib"), mib(listing as f64));
    report(
        &format!("{label}.index_mib"),
        mib((heap_bytes() - heap - listing) as f64),
    );
    drop(index);
}

fn point_reads(reader: &Reader, archives: i64) {
    let mut rng = StdRng::seed_from_u64(7);
    let mut details = Vec::with_capacity(POINT_READS);
    let mut pages = Vec::with_capacity(POINT_READS);
    for _ in 0..POINT_READS {
        let id = rng.random_range(1..=archives);
        let (archive, elapsed) = timed(|| reader.archive(id).unwrap());
        assert!(archive.is_some());
        details.push(elapsed);
        let (list, elapsed) = timed(|| reader.pages(id).unwrap());
        assert!(!list.is_empty());
        pages.push(elapsed);
    }
    latencies("archive", details);
    latencies("pages", pages);
}

/// 8 个读线程反复“打开一本”（详情加页面清单）；`writer` 为真时另有一个线程持续交替
/// 重写一本和上报进度，每次一个事务，和改动前的基准相同。
fn concurrent(path: &Path, store: &mut Store, generator: &Generator, archives: i64, writer: bool) {
    let label = if writer { "rw" } else { "ro" };
    let pool = ReaderPool::new(path);
    let stop = AtomicBool::new(false);
    let (opens, writes) = std::thread::scope(|scope| {
        let readers: Vec<_> = (0..READERS)
            .map(|number| {
                let (pool, stop) = (&pool, &stop);
                scope.spawn(move || {
                    let mut rng = StdRng::seed_from_u64(100 + number as u64);
                    let mut samples = Vec::new();
                    while !stop.load(Ordering::Relaxed) {
                        let id = rng.random_range(1..=archives);
                        let (_, elapsed) = timed(|| {
                            let reader = pool.get().unwrap();
                            reader.archive(id).unwrap();
                            reader.pages(id).unwrap()
                        });
                        samples.push(elapsed);
                    }
                    samples
                })
            })
            .collect();
        let writes = writer.then(|| {
            let mut rng = StdRng::seed_from_u64(99);
            let mut samples = Vec::new();
            let mut version = 1;
            let deadline = Instant::now() + CONCURRENT;
            while Instant::now() < deadline {
                let id = rng.random_range(1..=archives);
                let book = generator.book(id, version);
                version += 1;
                let (_, elapsed) = timed(|| {
                    store
                        .write(|writer| {
                            writer.save_archive(Some(id), &book.metadata, &book.pages)?;
                            writer.replace_tags(id, &book.tags)
                        })
                        .unwrap()
                });
                samples.push(elapsed);
                let (_, elapsed) = timed(|| store.write_progress(id, 5, version as i64).unwrap());
                samples.push(elapsed);
            }
            samples
        });
        if !writer {
            std::thread::sleep(CONCURRENT);
        }
        stop.store(true, Ordering::Relaxed);
        let opens: Vec<_> = readers
            .into_iter()
            .flat_map(|reader| reader.join().unwrap())
            .collect();
        (opens, writes)
    });
    let seconds = CONCURRENT.as_secs_f64();
    report(
        &format!("{label}.opens_per_s"),
        format!("{:.0}", opens.len() as f64 / seconds),
    );
    latencies(&format!("{label}.open"), opens);
    if let Some(writes) = writes {
        report(
            &format!("{label}.writes_per_s"),
            format!("{:.0}", writes.len() as f64 / seconds),
        );
        latencies(&format!("{label}.write"), writes);
    }
}

fn main() {
    let directory = std::env::args().nth(1).expect("用法：storage <空目录>");
    let archives: i64 = std::env::var("ARCHIVES")
        .ok()
        .map_or(300_000, |value| value.parse().expect("ARCHIVES 必须是整数"));
    std::fs::create_dir_all(&directory).unwrap();
    let path = Path::new(&directory).join("shuku.sqlite");
    assert!(!path.exists(), "{} 已存在，请换一个空目录", path.display());
    let generator = Generator::new();
    let mut store = Store::open(&path).unwrap();
    import(&mut store, &generator, archives);
    assert!(store.checkpoint().unwrap());
    report("disk_mib", mib(database_bytes(&path) as f64));

    let reader = Reader::open(&path).unwrap();
    load_snapshot(&reader, "warm");
    drop(reader);
    evict(&path);
    let reader = Reader::open(&path).unwrap();
    load_snapshot(&reader, "cold");
    point_reads(&reader, archives);
    drop(reader);

    // 和服务端一样关掉自动 checkpoint；10 秒内一直有写入，不会碰上空闲时的 checkpoint。
    store.disable_auto_checkpoint().unwrap();
    concurrent(&path, &mut store, &generator, archives, false);
    concurrent(&path, &mut store, &generator, archives, true);
}
