//! 用固定种子生成随机书库和查询，与逐本检查的暴力实现对比计数、分页和标签统计。

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeSet, HashMap};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use shuku_index::{ArchiveId, ArchiveRecord, Comparison, Index, Query, Sort, TagCount, Term};

const SEED: u64 = 0x5348_554b_5500;
/// 2000 本时，匹配数达到 32 本就走沿排列遍历的分支，随机查询两个分支都会碰到。
const ARCHIVES: i64 = 2_000;
const QUERIES: usize = 400;
/// 每个查询从第一页起最多跟着游标翻这么多页。
const PAGES_PER_QUERY: usize = 5;

const NAMESPACES: &[&str] = &["artist", "female", "language", "parody", ""];
const TAG_VALUES: &[&str] = &[
    "alpha",
    "alpine",
    "beta",
    "glasses",
    "sunglasses",
    "ponytail",
    "japanese",
    "chinese",
    "full color",
    "ab",
    "ab2",
    // 字节序在 ab2 前面、自然序在后面，按命名空间排序要取自然序最小的值。
    "ab10",
];
/// 标题由这些片段拼成，相邻的数字片段会连成更长的数字。
const TITLE_PARTS: &[&str] = &[
    "魔法", "少女", "妹妹", "的", "秘密", "Summer", "diary", "Vol.", " ", "第", "话", "2", "10",
    "007", "a", "B",
];

#[test]
fn index_matches_a_brute_force_scan() {
    let mut rng = StdRng::seed_from_u64(SEED);
    let tags = tag_universe();
    let mut records = random_library(&mut rng, &tags);
    let naive = Naive::new(&records);
    // 打乱构建顺序，确认结果不依赖内部编号与归档 ID 的对应关系。
    records.shuffle(&mut rng);
    let index = Index::build(records).unwrap();

    for _ in 0..QUERIES {
        let query = random_query(&mut rng, &tags, &naive);
        let expected = naive.search(&query);

        check_pages(&index, &query, &expected);
        check_cursor_before_match(&mut rng, &index, &naive, &query, &expected);
        // 偶尔取比标签总数还大的 limit，前 limit 名永远填不满。
        let limit = match rng.random_range(0..10) {
            0 => usize::MAX,
            1 => 60,
            _ => rng.random_range(0..=12),
        };
        assert_eq!(
            index.facets(&query, limit),
            naive.facets(&query, limit),
            "{query:?}"
        );
    }
}

/// 从第一页起跟着游标翻页，每页的计数、内容和游标都要与暴力结果一致。
fn check_pages(index: &Index, query: &Query, expected: &[ArchiveId]) {
    let mut start = 0;
    let mut after = None;
    for _ in 0..PAGES_PER_QUERY {
        let page = index
            .search(&Query {
                after,
                ..query.clone()
            })
            .unwrap();
        assert_eq!(page.total, expected.len() as u64, "{query:?}");
        let end = (start + query.limit).min(expected.len());
        assert_eq!(page.ids, expected[start..end], "{query:?} after {after:?}");
        if end == expected.len() {
            assert_eq!(page.next_after, None, "{query:?}");
            return;
        }
        assert_eq!(page.next_after, page.ids.last().copied(), "{query:?}");
        after = page.next_after;
        start = end;
    }
}

/// 游标可以是不在本次结果里的归档，这时从它在排序里的位置之后接着取。
/// 选排序里紧挨在某个匹配之前的归档作游标，检查这个匹配不会被跳过。
fn check_cursor_before_match(
    rng: &mut StdRng,
    index: &Index,
    naive: &Naive,
    query: &Query,
    expected: &[ArchiveId],
) {
    let (order, rank) = naive.order(&query.sort, query.reverse);
    let (order, rank) = (&order, &rank);
    let Some(&target) = expected.get(rng.random_range(0..expected.len().max(1))) else {
        return;
    };
    if rank[&target] == 0 {
        return;
    }
    let cursor = order[rank[&target] - 1];
    let page: Vec<ArchiveId> = expected
        .iter()
        .copied()
        .filter(|id| rank[id] > rank[&cursor])
        .take(query.limit)
        .collect();

    let result = index
        .search(&Query {
            after: Some(cursor),
            ..query.clone()
        })
        .unwrap();

    assert_eq!(result.ids, page, "{query:?} after {cursor}");
    assert_eq!(result.total, expected.len() as u64, "{query:?}");
}

fn tag_universe() -> Vec<String> {
    NAMESPACES
        .iter()
        .flat_map(|ns| {
            TAG_VALUES.iter().map(move |value| match *ns {
                "" => value.to_string(),
                ns => format!("{ns}:{value}"),
            })
        })
        .collect()
}

fn random_library(rng: &mut StdRng, tags: &[String]) -> Vec<ArchiveRecord> {
    (0..ARCHIVES)
        .map(|i| {
            let parts = rng.random_range(1..=5);
            let title = (0..parts)
                .map(|_| TITLE_PARTS[rng.random_range(..TITLE_PARTS.len())])
                .collect();
            let tag_count = rng.random_range(0..=6);
            ArchiveRecord {
                // ID 不连续，也不从 0 开始。
                id: i * 3 + 7,
                title,
                // 日期取值很少，大量并列，用来检查按 ID 打破并列。
                date_added: rng.random_range(0..50),
                tags: (0..tag_count).map(|_| random_tag(rng, tags)).collect(),
                page_count: rng.random_range(0..30),
                is_new: rng.random_bool(0.3),
            }
        })
        .collect()
}

fn random_query(rng: &mut StdRng, tags: &[String], naive: &Naive) -> Query {
    let include = rng.random_range(0..=3);
    let exclude = rng.random_range(0..=2);
    Query {
        include: (0..include)
            .map(|_| random_term(rng, tags, naive, 2))
            .collect(),
        exclude: (0..exclude)
            .map(|_| random_term(rng, tags, naive, 2))
            .collect(),
        sort: match rng.random_range(0..5) {
            0 => Sort::Title,
            1 => Sort::DateAddedNewest,
            2 => Sort::Pages,
            3 => Sort::Namespace("missing".into()),
            _ => {
                let namespace = NAMESPACES[rng.random_range(..NAMESPACES.len())];
                Sort::Namespace(random_case(rng, namespace))
            }
        },
        reverse: rng.random_bool(0.3),
        after: None,
        // 页面大多很小，匹配不多的结果也要翻好几页。
        limit: if rng.random_bool(0.8) {
            rng.random_range(1..=8)
        } else {
            rng.random_range(1..=60)
        },
    }
}

fn random_term(rng: &mut StdRng, tags: &[String], naive: &Naive, depth: u32) -> Term {
    let kinds = if depth == 0 { 8 } else { 9 };
    match rng.random_range(0..kinds) {
        0 if rng.random_bool(0.1) => Term::Tag("missing:tag".into()),
        0 => Term::Tag(random_tag(rng, tags)),
        1 => {
            let tag = &tags[rng.random_range(..tags.len())];
            let end = rng.random_range(0..=tag.len());
            Term::TagPrefix(random_case(rng, &tag[..end]))
        }
        2 => {
            let tag = &tags[rng.random_range(..tags.len())];
            let start = rng.random_range(0..tag.len());
            let end = rng.random_range(start + 1..=tag.len());
            Term::TagContains(random_case(rng, &tag[start..end]))
        }
        3 => {
            let title: Vec<char> = naive.records[rng.random_range(..naive.records.len())]
                .title
                .chars()
                .collect();
            let start = rng.random_range(0..title.len());
            let end = rng.random_range(start + 1..=title.len().min(start + 4));
            Term::TitleContains(random_case(
                rng,
                &title[start..end].iter().collect::<String>(),
            ))
        }
        4 => {
            // 一部分归档，外加一个索引里没有的 ID。
            let mut ids: Vec<ArchiveId> = naive
                .records
                .iter()
                .filter(|_| rng.random_bool(0.2))
                .map(|record| record.id)
                .collect();
            ids.push(ArchiveId::MAX);
            Term::Archives(ids)
        }
        5 => {
            let bound = rng.random_range(0..32);
            Term::Pages(match rng.random_range(0..5) {
                0 => Comparison::Less(bound),
                1 => Comparison::AtMost(bound),
                2 => Comparison::Equal(bound),
                3 => Comparison::AtLeast(bound),
                _ => Comparison::Greater(bound),
            })
        }
        6 => Term::New,
        7 => Term::Untagged,
        _ => {
            let children = rng.random_range(0..=3);
            Term::Any(
                (0..children)
                    .map(|_| random_term(rng, tags, naive, depth - 1))
                    .collect(),
            )
        }
    }
}

/// 偏向列表前面的标签，让少数标签很常见、其余的较少见。
fn random_tag(rng: &mut StdRng, tags: &[String]) -> String {
    let x: f64 = rng.random();
    let i = ((x * x * tags.len() as f64) as usize).min(tags.len() - 1);
    random_case(rng, &tags[i])
}

fn random_case(rng: &mut StdRng, text: &str) -> String {
    text.chars()
        .map(|c| {
            if rng.random_bool(0.3) {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect()
}

struct NaiveRecord {
    id: ArchiveId,
    title: String,
    tags: BTreeSet<String>,
    page_count: u32,
    is_new: bool,
}

/// 逐本检查每个条件，排序直接比较标题，不使用位图和预先算好的排列。
struct Naive {
    records: Vec<NaiveRecord>,
    dates: HashMap<ArchiveId, i64>,
}

impl Naive {
    fn new(records: &[ArchiveRecord]) -> Self {
        Self {
            records: records
                .iter()
                .map(|r| NaiveRecord {
                    id: r.id,
                    title: r.title.to_lowercase(),
                    tags: r.tags.iter().map(|t| normalize(t)).collect(),
                    page_count: r.page_count,
                    is_new: r.is_new,
                })
                .collect(),
            dates: records.iter().map(|r| (r.id, r.date_added)).collect(),
        }
    }

    /// 返回全部归档的排列，以及每本在排列里的位置。每次现算。
    fn order(&self, sort: &Sort, reverse: bool) -> (Vec<ArchiveId>, HashMap<ArchiveId, usize>) {
        let mut by_title: Vec<&NaiveRecord> = self.records.iter().collect();
        by_title.sort_by(|a, b| natural_cmp(&a.title, &b.title).then(a.id.cmp(&b.id)));
        let title_rank: HashMap<ArchiveId, usize> = by_title
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id, i))
            .collect();
        let mut order = by_title;
        match sort {
            Sort::Title => {}
            Sort::DateAddedNewest => order.sort_by_key(|r| Reverse((self.dates[&r.id], r.id))),
            Sort::Pages => order.sort_by_key(|r| (r.page_count, title_rank[&r.id])),
            Sort::Namespace(namespace) => {
                let prefix = format!("{}:", normalize(namespace));
                let value = |r: &NaiveRecord| {
                    r.tags
                        .iter()
                        .filter(|tag| tag.starts_with(&prefix))
                        .map(|tag| tag[prefix.len()..].to_string())
                        .min_by(|a, b| natural_cmp(a, b))
                };
                order.sort_by(|a, b| {
                    match (value(a), value(b)) {
                        (Some(x), Some(y)) => natural_cmp(&x, &y),
                        (Some(_), None) => Ordering::Less,
                        (None, Some(_)) => Ordering::Greater,
                        (None, None) => Ordering::Equal,
                    }
                    .then(title_rank[&a.id].cmp(&title_rank[&b.id]))
                });
            }
        }
        let mut ids: Vec<ArchiveId> = order.iter().map(|r| r.id).collect();
        if reverse {
            ids.reverse();
        }
        let rank = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        (ids, rank)
    }

    fn matching(&self, query: &Query) -> impl Iterator<Item = &NaiveRecord> {
        self.records.iter().filter(|r| {
            query.include.iter().all(|t| matches(r, t))
                && !query.exclude.iter().any(|t| matches(r, t))
        })
    }

    /// 返回全部匹配，按查询的排序方式排列。
    fn search(&self, query: &Query) -> Vec<ArchiveId> {
        let matched: BTreeSet<ArchiveId> = self.matching(query).map(|r| r.id).collect();
        self.order(&query.sort, query.reverse)
            .0
            .into_iter()
            .filter(|id| matched.contains(id))
            .collect()
    }

    fn facets(&self, query: &Query, limit: usize) -> Vec<TagCount> {
        let mut counts: HashMap<&str, u64> = HashMap::new();
        for record in self.matching(query) {
            for tag in &record.tags {
                *counts.entry(tag).or_default() += 1;
            }
        }
        let mut counts: Vec<(&str, u64)> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        counts
            .into_iter()
            .take(limit)
            .map(|(tag, count)| TagCount {
                tag: tag.to_string(),
                count,
            })
            .collect()
    }
}

fn matches(record: &NaiveRecord, term: &Term) -> bool {
    match term {
        Term::Tag(name) => record.tags.contains(&normalize(name)),
        Term::TagPrefix(prefix) => {
            let prefix = normalize(prefix);
            record.tags.iter().any(|t| t.starts_with(&prefix))
        }
        Term::TagContains(needle) => {
            let needle = normalize(needle);
            record.tags.iter().any(|t| t.contains(&needle))
        }
        Term::TitleContains(needle) => record.title.contains(&needle.to_lowercase()),
        Term::Any(terms) => terms.iter().any(|t| matches(record, t)),
        Term::Archives(ids) => ids.contains(&record.id),
        Term::Pages(comparison) => match *comparison {
            Comparison::Less(bound) => record.page_count < bound,
            Comparison::AtMost(bound) => record.page_count <= bound,
            Comparison::Equal(bound) => record.page_count == bound,
            Comparison::AtLeast(bound) => record.page_count >= bound,
            Comparison::Greater(bound) => record.page_count > bound,
        },
        Term::New => record.is_new,
        Term::Untagged => record.tags.is_empty(),
    }
}

fn normalize(name: &str) -> String {
    name.trim().to_lowercase()
}

/// 标题的自然序：数字串按数值比较，其余字符按码位比较，不区分大小写。
/// 数字串与普通字符相遇时，按数字串补零后的首字符 `0` 比较。
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (natural_tokens(a), natural_tokens(b));
    for (x, y) in a.iter().zip(&b) {
        let ordering = match (x, y) {
            (Token::Number(x), Token::Number(y)) => x.cmp(y),
            (Token::Char(x), Token::Char(y)) => x.cmp(y),
            (Token::Number(_), Token::Char(y)) => '0'.cmp(y),
            (Token::Char(x), Token::Number(_)) => x.cmp(&'0'),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    a.len().cmp(&b.len())
}

enum Token {
    Number(u64),
    Char(char),
}

fn natural_tokens(title: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut digits = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        if !digits.is_empty() {
            tokens.push(Token::Number(digits.parse().unwrap()));
            digits.clear();
        }
        tokens.push(Token::Char(c));
    }
    if !digits.is_empty() {
        tokens.push(Token::Number(digits.parse().unwrap()));
    }
    tokens
}
