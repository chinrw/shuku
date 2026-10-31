//! In-memory search index for Shuku.
//!
//! Every archive gets a dense internal number and every tag a roaring bitmap
//! of those numbers, so tag filters are bitmap set operations. Sort orders are
//! precomputed once at build time. A built [`Index`] is immutable; the server
//! rebuilds it (or swaps in a new one) after the database changes.

mod natural;
mod parser;

pub use parser::{ParseError, ParsedQuery, parse_query};

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use memchr::memmem;
use roaring::RoaringBitmap;

/// The archive's primary key in the database. It is stable across rebuilds,
/// unlike the dense internal numbers the bitmaps store.
pub type ArchiveId = i64;

/// Page size used when a query does not set one.
pub const DEFAULT_LIMIT: usize = 50;

/// A page is filled by walking the sort order when at least 1/64 of all
/// archives match; below that it is cheaper to sort the matches by rank.
/// Measured on 300k archives, both paths stay under 3 ms at this split.
const DENSE_DIVISOR: u64 = 64;

/// 结果不到全部归档的 1/16 时，统计标签改为逐本累加每本的标签，否则按标签
/// 从大到小求交集。在 30 万本上取前 10 个标签时，结果占 5% 时逐本累加更快，
/// 占 11% 时求交集更快。
const FACET_DIVISOR: u64 = 16;

/// One archive as the index needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveRecord {
    pub id: ArchiveId,
    pub title: String,
    /// Unix seconds.
    pub date_added: i64,
    /// Full tag names such as `artist:alpha`.
    pub tags: Vec<String>,
}

/// A single search condition. Tag names and titles match case-insensitively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// The full tag name, for example `language:japanese`.
    Tag(String),
    /// Every tag whose name starts with this string, for example `artist:al`.
    TagPrefix(String),
    /// Every tag whose name contains this string in any namespace.
    TagContains(String),
    /// Archives whose title contains this string.
    TitleContains(String),
    /// 子条件取并集；空列表不匹配任何归档。
    Any(Vec<Term>),
    /// 这些归档，例如一个静态分类的成员。索引里没有的 ID 忽略。搜索语句不会产生它。
    Archives(Vec<ArchiveId>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    /// Natural title order, then archive id.
    #[default]
    Title,
    /// Newest `date_added` first, then the larger archive id first.
    DateAddedNewest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// Every term must match. An empty list matches every archive.
    pub include: Vec<Term>,
    /// No term may match.
    pub exclude: Vec<Term>,
    pub sort: Sort,
    /// Continue after this archive, taken from [`SearchResult::next_after`].
    pub after: Option<ArchiveId>,
    pub limit: usize,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
            sort: Sort::default(),
            after: None,
            limit: DEFAULT_LIMIT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// One page of matches in sort order.
    pub ids: Vec<ArchiveId>,
    /// Number of matches across all pages.
    pub total: u64,
    /// Cursor for the next page, or `None` when this page is the last.
    pub next_after: Option<ArchiveId>,
}

/// 一个标签，以及搜索结果里有多少本归档带这个标签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagCount {
    /// 小写的完整标签名，例如 `artist:alpha`。
    pub tag: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    #[error("archive id {0} appears more than once")]
    DuplicateId(ArchiveId),
    #[error("the index holds at most u32::MAX archives")]
    TooManyArchives,
    #[error("the index holds at most u32::MAX distinct tags")]
    TooManyTags,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    /// The cursor archive is not in this index, usually because it was deleted
    /// after the previous page was served.
    #[error("cursor archive {0} is not in the index")]
    UnknownCursor(ArchiveId),
}

struct SortOrder {
    /// Internal numbers in sort order.
    order: Vec<u32>,
    /// Position of each internal number in `order`.
    rank: Vec<u32>,
}

impl SortOrder {
    fn new(order: Vec<u32>) -> Self {
        let mut rank = vec![0; order.len()];
        for (position, &doc) in order.iter().enumerate() {
            rank[doc as usize] = position as u32;
        }
        Self { order, rank }
    }
}

pub struct Index {
    /// Internal number to archive id.
    ids: Vec<ArchiveId>,
    doc_of: HashMap<ArchiveId, u32>,
    /// Normalized tag names, sorted so exact and prefix lookups can bisect.
    tag_names: Vec<String>,
    /// Bitmap for the tag at the same position in `tag_names`.
    tag_docs: Vec<RoaringBitmap>,
    /// 标签位置按归档数从多到少、同数时按名字排列，统计和补全标签时可以提前停止。
    tags_by_size: Vec<u32>,
    /// 第 n 本归档的标签位置是 `doc_tags[doc_tag_start[n]..doc_tag_start[n + 1]]`。
    doc_tag_start: Vec<usize>,
    doc_tags: Vec<u32>,
    /// Lowercased titles, each followed by `\n`, scanned with memmem.
    titles: Vec<u8>,
    /// Byte offset where each title starts in `titles`.
    title_start: Vec<usize>,
    by_title: SortOrder,
    by_date: SortOrder,
}

impl Index {
    pub fn build(records: impl IntoIterator<Item = ArchiveRecord>) -> Result<Self, BuildError> {
        let mut ids = Vec::new();
        let mut doc_of = HashMap::new();
        let mut tag_lists: HashMap<String, Vec<u32>> = HashMap::new();
        let mut titles = Vec::new();
        let mut title_start = Vec::new();
        let mut title_keys = Vec::new();
        let mut dates = Vec::new();

        for record in records {
            let doc = u32::try_from(ids.len()).map_err(|_| BuildError::TooManyArchives)?;
            if doc_of.insert(record.id, doc).is_some() {
                return Err(BuildError::DuplicateId(record.id));
            }
            ids.push(record.id);
            dates.push(record.date_added);
            title_keys.push(natural::natural_key(&record.title));
            title_start.push(titles.len());
            titles.extend_from_slice(scan_text(&record.title).as_bytes());
            titles.push(b'\n');
            for tag in &record.tags {
                tag_lists.entry(normalize_tag(tag)).or_default().push(doc);
            }
        }

        let mut tags: Vec<(String, Vec<u32>)> = tag_lists.into_iter().collect();
        tags.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let (tag_names, tag_docs): (Vec<String>, Vec<RoaringBitmap>) = tags
            .into_iter()
            .map(|(name, docs)| (name, docs.into_iter().collect::<RoaringBitmap>()))
            .unzip();
        let tag_count = u32::try_from(tag_docs.len()).map_err(|_| BuildError::TooManyTags)?;
        let mut tags_by_size: Vec<u32> = (0..tag_count).collect();
        tags_by_size.sort_by_key(|&tag| Reverse(tag_docs[tag as usize].len()));
        let (doc_tag_start, doc_tags) = doc_tag_table(ids.len(), &tag_docs);

        let count = ids.len() as u32;
        let mut by_title: Vec<u32> = (0..count).collect();
        by_title.sort_unstable_by(|&a, &b| {
            let (a, b) = (a as usize, b as usize);
            title_keys[a].cmp(&title_keys[b]).then(ids[a].cmp(&ids[b]))
        });
        let mut by_date: Vec<u32> = (0..count).collect();
        by_date.sort_unstable_by(|&a, &b| {
            let (a, b) = (a as usize, b as usize);
            dates[b].cmp(&dates[a]).then(ids[b].cmp(&ids[a]))
        });

        Ok(Self {
            ids,
            doc_of,
            tag_names,
            tag_docs,
            tags_by_size,
            doc_tag_start,
            doc_tags,
            titles,
            title_start,
            by_title: SortOrder::new(by_title),
            by_date: SortOrder::new(by_date),
        })
    }

    pub fn search(&self, query: &Query) -> Result<SearchResult, SearchError> {
        let order = match query.sort {
            Sort::Title => &self.by_title,
            Sort::DateAddedNewest => &self.by_date,
        };
        let start = match query.after {
            None => 0,
            Some(id) => {
                let doc = self.doc_of.get(&id).ok_or(SearchError::UnknownCursor(id))?;
                order.rank[*doc as usize] as usize + 1
            }
        };

        let matches = self.evaluate(query);
        // Fetch one extra match to learn whether another page exists.
        let mut page = self.page(&matches, order, start, query.limit.saturating_add(1));
        let next_after = if page.len() > query.limit {
            page.truncate(query.limit);
            page.last().map(|&doc| self.ids[doc as usize])
        } else {
            None
        };

        Ok(SearchResult {
            ids: page.into_iter().map(|doc| self.ids[doc as usize]).collect(),
            total: matches.len(),
            next_after,
        })
    }

    /// 返回 `query` 的结果里出现最多的 `limit` 个标签，按归档数从多到少、
    /// 同数时按标签名排列，不含数量为零的标签。忽略 `sort`、`after` 和
    /// `limit`；包含条件里的标签照常统计。
    pub fn facets(&self, query: &Query, limit: usize) -> Vec<TagCount> {
        let matches = self.evaluate(query);
        if limit == 0 || matches.is_empty() {
            return Vec::new();
        }
        // 返回的标签不会比索引里的标签多，限制后也不会按调用方的 limit 预分配过多内存。
        let limit = limit.min(self.tag_names.len());
        let top = if matches.len() * FACET_DIVISOR < self.ids.len() as u64 {
            self.top_tags_per_doc(&matches, limit)
        } else {
            self.top_tags_by_size(&matches, limit)
        };
        top.into_sorted()
            .into_iter()
            .map(|(count, tag)| TagCount {
                tag: self.tag_names[tag as usize].clone(),
                count,
            })
            .collect()
    }

    /// 搜索框补全用：返回完整名或冒号后的值以 `prefix` 开头的标签，按全库
    /// 归档数从多到少、同数时按标签名排列，最多 `limit` 个。前缀按小写匹配，
    /// 为空时不返回任何标签。
    pub fn suggest_tags(&self, prefix: &str, limit: usize) -> Vec<TagCount> {
        let prefix = normalize_tag(prefix);
        if prefix.is_empty() {
            return Vec::new();
        }
        // tags_by_size 已按归档数从多到少、同数按名字排好，前 limit 个命中的就是答案。
        // 前缀很常见时很快凑满；很少见时要比较全部标签名，在 30 万本的合成数据上约 1.5 ms。
        self.tags_by_size
            .iter()
            .map(|&tag| tag as usize)
            .filter(|&tag| {
                let name = &self.tag_names[tag];
                let value = name
                    .split_once(':')
                    .map_or(name.as_str(), |(_, value)| value);
                name.starts_with(&prefix) || value.starts_with(&prefix)
            })
            .take(limit)
            .map(|tag| TagCount {
                tag: self.tag_names[tag].clone(),
                count: self.tag_docs[tag].len(),
            })
            .collect()
    }

    /// 逐本累加结果里每本归档的标签。除了清零一张按标签数分配的计数表，
    /// 耗时与结果大小成正比。
    fn top_tags_per_doc(&self, matches: &RoaringBitmap, limit: usize) -> TopTags {
        let mut counts = vec![0u32; self.tag_names.len()];
        let mut seen = Vec::new();
        for doc in matches {
            let doc = doc as usize;
            for &tag in &self.doc_tags[self.doc_tag_start[doc]..self.doc_tag_start[doc + 1]] {
                if counts[tag as usize] == 0 {
                    seen.push(tag);
                }
                counts[tag as usize] += 1;
            }
        }
        let mut top = TopTags::new(limit);
        for tag in seen {
            top.offer(u64::from(counts[tag as usize]), tag);
        }
        top
    }

    /// 按标签从大到小求交集。结果很大时，最大的几个标签就能填满前 `limit` 名。
    fn top_tags_by_size(&self, matches: &RoaringBitmap, limit: usize) -> TopTags {
        let mut top = TopTags::new(limit);
        for &tag in &self.tags_by_size {
            let docs = &self.tag_docs[tag as usize];
            // 交集不会超过标签本身的归档数，标签又按归档数从多到少遍历，
            // 所以一旦比最弱候选还少，后面的标签都进不了前 `limit` 名。
            if top.weakest().is_some_and(|weakest| docs.len() < weakest) {
                break;
            }
            let count = docs.intersection_len(matches);
            if count > 0 {
                top.offer(count, tag);
            }
        }
        top
    }

    fn evaluate(&self, query: &Query) -> RoaringBitmap {
        let mut included: Vec<Cow<'_, RoaringBitmap>> = query
            .include
            .iter()
            .map(|term| self.term_docs(term))
            .collect();
        // Intersecting the smallest set first keeps every intermediate small.
        included.sort_unstable_by_key(|docs| docs.len());
        let mut result = match included.split_first() {
            None => self.all_docs(),
            Some((first, rest)) => rest
                .iter()
                .fold(first.clone().into_owned(), |acc, docs| acc & docs.as_ref()),
        };
        for term in &query.exclude {
            result -= self.term_docs(term).as_ref();
        }
        result
    }

    fn term_docs(&self, term: &Term) -> Cow<'_, RoaringBitmap> {
        match term {
            Term::Tag(name) => match self.tag_names.binary_search(&normalize_tag(name)) {
                Ok(i) => Cow::Borrowed(&self.tag_docs[i]),
                Err(_) => Cow::Owned(RoaringBitmap::new()),
            },
            Term::TagPrefix(prefix) => {
                let prefix = normalize_tag(prefix);
                let lo = self
                    .tag_names
                    .partition_point(|name| name.as_str() < prefix.as_str());
                let hi =
                    lo + self.tag_names[lo..].partition_point(|name| name.starts_with(&prefix));
                Cow::Owned(self.union_of(lo..hi))
            }
            Term::TagContains(needle) => {
                let needle = normalize_tag(needle);
                let finder = memmem::Finder::new(needle.as_bytes());
                let hits = (0..self.tag_names.len())
                    .filter(|&i| finder.find(self.tag_names[i].as_bytes()).is_some());
                Cow::Owned(self.union_of(hits))
            }
            Term::TitleContains(needle) => Cow::Owned(self.title_docs(&scan_text(needle))),
            Term::Any(terms) => {
                Cow::Owned(terms.iter().fold(RoaringBitmap::new(), |mut docs, term| {
                    docs |= self.term_docs(term).as_ref();
                    docs
                }))
            }
            Term::Archives(ids) => Cow::Owned(
                ids.iter()
                    .filter_map(|id| self.doc_of.get(id).copied())
                    .collect(),
            ),
        }
    }

    fn union_of(&self, tag_positions: impl Iterator<Item = usize>) -> RoaringBitmap {
        tag_positions.fold(RoaringBitmap::new(), |acc, i| acc | &self.tag_docs[i])
    }

    fn title_docs(&self, needle: &str) -> RoaringBitmap {
        if needle.is_empty() {
            return self.all_docs();
        }
        memmem::find_iter(&self.titles, needle.as_bytes())
            .map(|pos| (self.title_start.partition_point(|&start| start <= pos) - 1) as u32)
            .collect()
    }

    fn all_docs(&self) -> RoaringBitmap {
        let mut docs = RoaringBitmap::new();
        docs.insert_range(0..self.ids.len() as u32);
        docs
    }

    /// Returns up to `take` matches whose rank is at least `start`, in sort order.
    fn page(
        &self,
        matches: &RoaringBitmap,
        order: &SortOrder,
        start: usize,
        take: usize,
    ) -> Vec<u32> {
        if matches.len() * DENSE_DIVISOR >= self.ids.len() as u64 {
            return order.order[start.min(order.order.len())..]
                .iter()
                .copied()
                .filter(|&doc| matches.contains(doc))
                .take(take)
                .collect();
        }
        let rank = |doc: &u32| order.rank[*doc as usize];
        let mut docs: Vec<u32> = matches
            .iter()
            .filter(|doc| rank(doc) as usize >= start)
            .collect();
        if docs.len() > take {
            docs.select_nth_unstable_by_key(take, rank);
            docs.truncate(take);
        }
        docs.sort_unstable_by_key(rank);
        docs
    }
}

/// 由每个标签的归档位图反推每本归档的标签位置，按归档连续存放。
fn doc_tag_table(doc_count: usize, tag_docs: &[RoaringBitmap]) -> (Vec<usize>, Vec<u32>) {
    let mut start = vec![0; doc_count + 1];
    for docs in tag_docs {
        for doc in docs {
            start[doc as usize + 1] += 1;
        }
    }
    for doc in 0..doc_count {
        start[doc + 1] += start[doc];
    }
    let mut tags = vec![0; start[doc_count]];
    let mut next = start.clone();
    for (tag, docs) in (0u32..).zip(tag_docs) {
        for doc in docs {
            tags[next[doc as usize]] = tag;
            next[doc as usize] += 1;
        }
    }
    (start, tags)
}

/// 保留归档数最多的 `limit` 个标签；同数时位置小、也就是名字靠前的优先。
struct TopTags {
    limit: usize,
    /// 小顶堆，堆顶是目前留下的最弱候选，所以第二个键用 Reverse(位置)。
    heap: BinaryHeap<Reverse<(u64, Reverse<u32>)>>,
}

impl TopTags {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            heap: BinaryHeap::with_capacity(limit),
        }
    }

    /// 已经留满 `limit` 个时，返回最弱候选的归档数。
    fn weakest(&self) -> Option<u64> {
        if self.heap.len() < self.limit {
            return None;
        }
        self.heap.peek().map(|Reverse((count, _))| *count)
    }

    fn offer(&mut self, count: u64, tag: u32) {
        let candidate = Reverse((count, Reverse(tag)));
        if self.heap.len() < self.limit {
            self.heap.push(candidate);
        } else if let Some(mut weakest) = self.heap.peek_mut()
            && candidate < *weakest
        {
            *weakest = candidate;
        }
    }

    /// 返回 `(归档数, 标签位置)`，最强的在前。
    fn into_sorted(self) -> Vec<(u64, u32)> {
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|Reverse((count, Reverse(tag)))| (count, tag))
            .collect()
    }
}

fn normalize_tag(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Lowercases text for the title scan. A newline separates titles in the scan
/// buffer, so newlines inside a title or needle become spaces.
fn scan_text(text: &str) -> String {
    text.to_lowercase().replace('\n', " ")
}
