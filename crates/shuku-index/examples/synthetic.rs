//! 在 30 万本合成书库上测量索引的构建耗时、内存和查询耗时。
//!
//! 运行：`nix develop -c cargo run --release -p shuku-index --example synthetic`
//!
//! 每本有一个语言标签（日语、中文、英语三选一）和 9 个按 Zipf 分布抽取的标签，
//! Zipf 词表 7.8 万个标签；标题混合中文、日文和英文。随机数种子固定，每次生成
//! 的书库相同。查询耗时指取第一页 50 本加总数，每个查询重复多次取中位数。

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use shuku_index::{ArchiveRecord, Index, Query, Sort, Term};

const SEED: u64 = 0x5348_554b_5501;
const ARCHIVES: usize = 300_000;
/// Zipf 词表的大小，不含语言标签。
const ZIPF_TAGS: usize = 78_000;
const ZIPF_TAGS_PER_ARCHIVE: usize = 9;
/// 指数为 1 时，排名第一的标签出现在约 55% 的归档里。
const ZIPF_EXPONENT: f64 = 1.0;
const PAGE: usize = 50;
const FACETS: usize = 10;
const WARMUP_RUNS: usize = 3;
const TIMED_RUNS: usize = 51;

/// 语言标签互相排斥，百分比之和为 100。
const LANGUAGES: [(&str, u32); 3] = [
    ("language:japanese", 56),
    ("language:chinese", 28),
    ("language:english", 16),
];

/// Zipf 排名最靠前的标签用常见的真实标签名，其余按排名生成。
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
    "other:story arc",
    "female:swimsuit",
    "male:glasses",
    "female:maid",
    "female:kimono",
    "other:anthology",
    "female:dark skin",
    "female:short hair",
    "female:pantyhose",
    "female:sunglasses",
];
/// 长尾标签的命名空间按排名轮换，作者出现两次，占长尾的 40%。
const TAIL_NAMESPACES: [&str; 5] = ["artist", "artist", "group", "character", "parody"];

const JA_WORDS: &[&str] = &[
    "魔法少女",
    "放課後",
    "秘密",
    "先生",
    "お姉ちゃん",
    "夏休み",
    "恋人",
    "日記",
    "幼なじみ",
    "彼女",
    "温泉",
    "旅行",
    "同級生",
    "後輩",
    "先輩",
    "図書館",
    "雨の日",
    "約束",
    "花火",
    "海辺",
    "教室",
    "少女",
    "天使",
    "魔王",
    "勇者",
    "異世界",
    "メイド",
    "お嬢様",
    "生徒会",
    "ひみつ",
];
const ZH_WORDS: &[&str] = &[
    "魔法少女",
    "妹妹",
    "秘密",
    "学姐",
    "夏日",
    "恋爱",
    "日记",
    "青梅竹马",
    "女友",
    "温泉",
    "旅行",
    "同学",
    "学妹",
    "前辈",
    "图书馆",
    "雨天",
    "约定",
    "烟花",
    "海边",
    "教室",
    "少女",
    "天使",
    "魔王",
    "勇者",
    "异世界",
    "女仆",
    "大小姐",
    "学生会",
    "姐姐",
    "老师",
];
const EN_WORDS: &[&str] = &[
    "Summer", "Diary", "Secret", "Magical", "Girl", "After", "School", "Love", "Story", "Days",
    "Holiday", "Rainy", "Memories", "Sister", "Teacher", "Princess", "Angel", "Night", "Island",
    "Promise",
];

/// 统计当前仍在使用的堆内存，用来估算索引的大小。
struct CountingAllocator;

static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: 每个方法都把调用原样转给 System，只在分配成功后调整计数，
// 不改变 GlobalAlloc 要求的任何行为。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 调用方保证 layout 的大小不为零，与 System.alloc 的要求相同。
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: 与 alloc 相同。转给 System 才能像正式运行时一样用 calloc，
        // 默认实现会先分配再逐字节清零，计时会偏慢。
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            LIVE_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr 由本分配器（即 System）用同一个 layout 分配。
        unsafe { System.dealloc(ptr, layout) };
        LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: ptr 由 System 用 layout 分配，new_size 的要求由调用方保证。
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            LIVE_BYTES.fetch_add(new_size, Ordering::Relaxed);
            LIVE_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn main() {
    let base_bytes = LIVE_BYTES.load(Ordering::Relaxed);
    let started = Instant::now();
    let (records, distinct_tags) = generate();
    let generate_time = started.elapsed();
    let record_bytes = LIVE_BYTES.load(Ordering::Relaxed) - base_bytes;

    // build 会消耗并释放 records，构建后仍在使用的堆内存就是索引本身。
    let started = Instant::now();
    let index = Index::build(records).expect("generated archive ids are unique");
    let build_time = started.elapsed();
    let index_bytes = LIVE_BYTES.load(Ordering::Relaxed) - base_bytes;

    let top_tag = tag_name(0);
    println!("归档 {ARCHIVES} 本，不同标签 {distinct_tags} 个");
    println!(
        "{} 覆盖 {:.1}%，{top_tag} 覆盖 {:.1}%",
        LANGUAGES[0].0,
        coverage(&index, LANGUAGES[0].0),
        coverage(&index, &top_tag),
    );
    println!(
        "生成数据 {:.2} s，原始记录 {}",
        generate_time.as_secs_f64(),
        mib(record_bytes)
    );
    println!(
        "构建索引 {:.2} s，索引占用 {}",
        build_time.as_secs_f64(),
        mib(index_bytes)
    );
    println!();
    println!(
        "| 查询 | 匹配数 | 标题排序 中位数 ms | 标题排序 最慢 ms | 最新加入 中位数 ms | 标签统计 中位数 ms |"
    );
    println!("| --- | ---: | ---: | ---: | ---: | ---: |");

    let mut medians = Vec::new();
    for (name, query) in cases() {
        let total = index.search(&query).expect("no cursor").total;
        let (title_median, title_max) = time(|| {
            black_box(index.search(&query).expect("no cursor"));
        });
        let date_query = Query {
            sort: Sort::DateAddedNewest,
            ..query.clone()
        };
        let (date_median, _) = time(|| {
            black_box(index.search(&date_query).expect("no cursor"));
        });
        let (facet_median, _) = time(|| {
            black_box(index.facets(&query, FACETS));
        });
        medians.push(title_median);
        println!(
            "| {name} | {total} | {} | {} | {} | {} |",
            ms(title_median),
            ms(title_max),
            ms(date_median),
            ms(facet_median),
        );
    }

    medians.sort_unstable();
    println!();
    println!(
        "标题排序第一页加总数：中位数的中位数 {} ms，最慢的中位数 {} ms",
        ms(medians[medians.len() / 2]),
        ms(medians[medians.len() - 1]),
    );

    // 补全要逐个比较全部标签名，耗时只和标签数有关，取能匹配上万个标签的前缀 a 和几乎不匹配的 zz，后者要比较全部标签名。
    for prefix in ["a", "zz"] {
        let (median, max) = time(|| {
            black_box(index.suggest_tags(prefix, 10));
        });
        println!(
            "标签补全 {prefix:?} 前 10 个：中位数 {} ms，最慢 {} ms",
            ms(median),
            ms(max),
        );
    }

    // 每次换一本，间隔取素数，让抽到的归档分散在整个书库里。
    let mut next_id = 0i64;
    let (median, max) = time(|| {
        next_id = (next_id + 4_999) % ARCHIVES as i64;
        black_box(index.similar(next_id + 1, 12));
    });
    println!(
        "相似作品前 12 本：中位数 {} ms，最慢 {} ms",
        ms(median),
        ms(max),
    );
}

/// 返回生成的归档，以及其中实际出现的不同标签数。
fn generate() -> (Vec<ArchiveRecord>, usize) {
    let mut rng = StdRng::seed_from_u64(SEED);
    let zipf = Zipf::new(ZIPF_TAGS, ZIPF_EXPONENT);
    let names: Vec<String> = (0..ZIPF_TAGS).map(tag_name).collect();
    let mut used = vec![false; ZIPF_TAGS];
    let mut used_languages = [false; LANGUAGES.len()];

    let records = (0..ARCHIVES)
        .map(|i| {
            let language = pick_language(&mut rng);
            used_languages[language] = true;
            let mut ranks = Vec::with_capacity(ZIPF_TAGS_PER_ARCHIVE);
            while ranks.len() < ZIPF_TAGS_PER_ARCHIVE {
                let rank = zipf.sample(&mut rng);
                if !ranks.contains(&rank) {
                    ranks.push(rank);
                }
            }
            let mut tags = vec![LANGUAGES[language].0.to_string()];
            for &rank in &ranks {
                used[rank] = true;
                tags.push(names[rank].clone());
            }
            ArchiveRecord {
                id: i as i64 + 1,
                title: title(&mut rng, language),
                // 约 6 年内随机的加入时间，大致对应真实书库的跨度。
                date_added: 1_600_000_000 + rng.random_range(0..200_000_000),
                tags,
                page_count: rng.random_range(8..400),
                is_new: rng.random_bool(0.3),
                series: None,
            }
        })
        .collect();
    let distinct =
        used.iter().filter(|&&u| u).count() + used_languages.iter().filter(|&&u| u).count();
    (records, distinct)
}

fn pick_language(rng: &mut StdRng) -> usize {
    let mut roll = rng.random_range(0..100);
    for (i, &(_, percent)) in LANGUAGES.iter().enumerate() {
        if roll < percent {
            return i;
        }
        roll -= percent;
    }
    unreachable!("LANGUAGES percentages add up to 100")
}

fn tag_name(rank: usize) -> String {
    match HOT_TAGS.get(rank) {
        Some(name) => name.to_string(),
        None => {
            let namespace = TAIL_NAMESPACES[rank % TAIL_NAMESPACES.len()];
            format!("{namespace}:{namespace}{rank}")
        }
    }
}

/// 例如 `[author123] 放課後の秘密 第3話` 或 `[author45] Summer Diary Vol.2`。
fn title(rng: &mut StdRng, language: usize) -> String {
    let (words, separator, volume) = match language {
        0 => (JA_WORDS, "の", "第{}話"),
        1 => (ZH_WORDS, "的", "第{}话"),
        _ => (EN_WORDS, " ", "Vol.{}"),
    };
    let count = rng.random_range(1..=3);
    let body: Vec<&str> = (0..count)
        .map(|_| words[rng.random_range(..words.len())])
        .collect();
    let mut title = format!(
        "[author{}] {}",
        rng.random_range(0..20_000),
        body.join(separator)
    );
    if rng.random_bool(0.3) {
        title.push(' ');
        title.push_str(&volume.replace("{}", &rng.random_range(1..=20).to_string()));
    }
    title
}

struct Zipf {
    /// 第 k 项是排名不超过 k 的标签被抽中的累计概率。
    cdf: Vec<f64>,
}

impl Zipf {
    fn new(len: usize, exponent: f64) -> Self {
        let weights: Vec<f64> = (1..=len).map(|k| (k as f64).powf(-exponent)).collect();
        let total: f64 = weights.iter().sum();
        let cdf = weights
            .iter()
            .scan(0.0, |sum, weight| {
                *sum += weight / total;
                Some(*sum)
            })
            .collect();
        Self { cdf }
    }

    fn sample(&self, rng: &mut StdRng) -> usize {
        let u: f64 = rng.random();
        self.cdf
            .partition_point(|&p| p <= u)
            .min(self.cdf.len() - 1)
    }
}

fn cases() -> Vec<(&'static str, Query)> {
    let tag = |rank: usize| Term::Tag(tag_name(rank));
    let japanese = || Term::Tag(LANGUAGES[0].0.into());
    let chinese = || Term::Tag(LANGUAGES[1].0.into());
    // 排名 80 的标签约覆盖 1% 的归档，排名 30000 的约 8 本。
    let medium = 80;
    let rare = 30_000;
    let include = |include: Vec<Term>| Query {
        include,
        limit: PAGE,
        ..Query::default()
    };
    vec![
        ("单个热门标签", include(vec![japanese()])),
        ("单个中等标签", include(vec![tag(medium)])),
        ("单个冷门标签", include(vec![tag(rare)])),
        ("两个热门标签取交集", include(vec![japanese(), tag(0)])),
        ("热门 ∩ 冷门", include(vec![japanese(), tag(rare)])),
        (
            "三个标签减去一个热门标签",
            Query {
                exclude: vec![tag(0)],
                ..include(vec![chinese(), tag(1), tag(3)])
            },
        ),
        (
            "命名空间前缀 ∩ 标签",
            include(vec![Term::TagPrefix("artist:artist12".into()), chinese()]),
        ),
        (
            "标签名子串 ∩ 标签",
            include(vec![Term::TagContains("glasses".into()), japanese()]),
        ),
        (
            "标题子串（4 个汉字）",
            include(vec![Term::TitleContains("魔法少女".into())]),
        ),
        (
            "标题子串（2 个汉字）",
            include(vec![Term::TitleContains("少女".into())]),
        ),
        (
            "四个热门标签",
            include(vec![japanese(), tag(0), tag(1), tag(2)]),
        ),
        (
            "两个互斥的大标签（空结果）",
            include(vec![japanese(), chinese()]),
        ),
    ]
}

fn coverage(index: &Index, tag: &str) -> f64 {
    let query = Query {
        include: vec![Term::Tag(tag.into())],
        limit: 0,
        ..Query::default()
    };
    let total = index.search(&query).expect("no cursor").total;
    total as f64 * 100.0 / ARCHIVES as f64
}

/// 返回多次运行耗时的中位数和最大值。
fn time(mut run: impl FnMut()) -> (Duration, Duration) {
    for _ in 0..WARMUP_RUNS {
        run();
    }
    let mut times: Vec<Duration> = (0..TIMED_RUNS)
        .map(|_| {
            let started = Instant::now();
            run();
            started.elapsed()
        })
        .collect();
    times.sort_unstable();
    (times[TIMED_RUNS / 2], times[TIMED_RUNS - 1])
}

fn ms(duration: Duration) -> String {
    format!("{:.3}", duration.as_secs_f64() * 1e3)
}

fn mib(bytes: usize) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}
