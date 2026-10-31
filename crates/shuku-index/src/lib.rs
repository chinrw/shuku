//! In-memory search index for Shuku.
//!
//! Every archive gets a dense internal number and every tag a roaring bitmap
//! of those numbers, so tag filters are bitmap set operations. Sort orders are
//! precomputed once at build time. A built [`Index`] is immutable; the server
//! rebuilds it (or swaps in a new one) after the database changes.

mod natural;
mod parser;
mod similar;

pub use parser::{ParseError, ParsedQuery, ReadCondition, parse_query};

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveRecord {
    pub id: ArchiveId,
    pub title: String,
    /// Unix seconds.
    pub date_added: i64,
    /// Full tag names such as `artist:alpha`.
    pub tags: Vec<String>,
    pub page_count: u32,
    /// 入库后还没打开过。扫描新增的归档为真，LRR 导入的沿用 LRR 的 isnew。
    pub is_new: bool,
    /// 所属系列的 ID 和卷号，见 [`Query::collapse_series`]。
    pub series: Option<(i64, u32)>,
}

/// `pages:` 和 `read:` 条件里的比较，例如 `pages:>=20` 是 `AtLeast(20)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    Less(u32),
    AtMost(u32),
    Equal(u32),
    AtLeast(u32),
    Greater(u32),
}

impl Comparison {
    pub fn matches(self, value: u32) -> bool {
        match self {
            Self::Less(bound) => value < bound,
            Self::AtMost(bound) => value <= bound,
            Self::Equal(bound) => value == bound,
            Self::AtLeast(bound) => value >= bound,
            Self::Greater(bound) => value > bound,
        }
    }
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
    /// 页数满足比较的归档，搜索语句里的 `pages:`。
    Pages(Comparison),
    /// [`ArchiveRecord::is_new`] 为真的归档。搜索语句不会产生它。
    New,
    /// 一个标签都没有的归档。搜索语句不会产生它。
    Untagged,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Sort {
    /// Natural title order, then archive id.
    #[default]
    Title,
    /// Newest `date_added` first, then the larger archive id first.
    DateAddedNewest,
    /// 页数从少到多，同页数按标题自然序。
    Pages,
    /// 按这个命名空间下的标签值自然序，例如 `artist` 按作者名排。一本有几个值时取自然序
    /// 最小的那个；没有这个命名空间的归档排在最后。同值按标题自然序。
    Namespace(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// Every term must match. An empty list matches every archive.
    pub include: Vec<Term>,
    /// No term may match.
    pub exclude: Vec<Term>,
    pub sort: Sort,
    /// 倒过来遍历排序，例如按标题从 Z 到 A、按加入时间从旧到新。
    pub reverse: bool,
    /// Continue after this archive, taken from [`SearchResult::next_after`].
    pub after: Option<ArchiveId>,
    pub limit: usize,
    /// 同一系列的结果只留卷号最小的一卷，书库页每个系列只显示一张卡。总数也按留下的算。
    pub collapse_series: bool,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
            sort: Sort::default(),
            reverse: false,
            after: None,
            limit: DEFAULT_LIMIT,
            collapse_series: false,
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

    /// `reverse` 时从后往前数的名次，分页和游标都按它算。
    fn rank(&self, doc: u32, reverse: bool) -> usize {
        let rank = self.rank[doc as usize] as usize;
        if reverse {
            self.order.len() - 1 - rank
        } else {
            rank
        }
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
    by_pages: SortOrder,
    /// 按命名空间排序的排列第一次用到时才算，30 万本的合成数据上约 60 ms，算好后留到索引被换掉。
    /// 只缓存索引里有标签的命名空间，个数有限。
    by_namespace: Mutex<HashMap<String, Arc<SortOrder>>>,
    page_counts: Vec<u32>,
    new_docs: RoaringBitmap,
    untagged_docs: RoaringBitmap,
    /// 属于某个系列的归档。
    series_docs: RoaringBitmap,
    /// `series_docs` 里每本的系列 ID 和卷号。
    doc_series: HashMap<u32, (i64, u32)>,
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
        let mut page_counts = Vec::new();
        let mut new_docs = RoaringBitmap::new();
        let mut series_docs = RoaringBitmap::new();
        let mut doc_series = HashMap::new();

        for record in records {
            let doc = u32::try_from(ids.len()).map_err(|_| BuildError::TooManyArchives)?;
            if doc_of.insert(record.id, doc).is_some() {
                return Err(BuildError::DuplicateId(record.id));
            }
            ids.push(record.id);
            dates.push(record.date_added);
            page_counts.push(record.page_count);
            if record.is_new {
                new_docs.insert(doc);
            }
            if let Some(series) = record.series {
                series_docs.insert(doc);
                doc_series.insert(doc, series);
            }
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
        let untagged_docs = (0..ids.len() as u32)
            .filter(|&doc| doc_tag_start[doc as usize] == doc_tag_start[doc as usize + 1])
            .collect();

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
        let by_title = SortOrder::new(by_title);
        let mut by_pages: Vec<u32> = (0..count).collect();
        by_pages
            .sort_unstable_by_key(|&doc| (page_counts[doc as usize], by_title.rank[doc as usize]));

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
            by_title,
            by_date: SortOrder::new(by_date),
            by_pages: SortOrder::new(by_pages),
            by_namespace: Mutex::new(HashMap::new()),
            page_counts,
            new_docs,
            untagged_docs,
            series_docs,
            doc_series,
        })
    }

    pub fn search(&self, query: &Query) -> Result<SearchResult, SearchError> {
        let namespace_order;
        let order = match &query.sort {
            Sort::Title => &self.by_title,
            Sort::DateAddedNewest => &self.by_date,
            Sort::Pages => &self.by_pages,
            Sort::Namespace(namespace) => {
                namespace_order = self.namespace_order(namespace);
                namespace_order.as_deref().unwrap_or(&self.by_title)
            }
        };
        let start = match query.after {
            None => 0,
            Some(id) => {
                let doc = self.doc_of.get(&id).ok_or(SearchError::UnknownCursor(id))?;
                order.rank(*doc, query.reverse) + 1
            }
        };

        let mut matches = self.evaluate(query);
        if query.collapse_series {
            self.collapse_series(&mut matches);
        }
        // Fetch one extra match to learn whether another page exists.
        let mut page = self.page(
            &matches,
            order,
            query.reverse,
            start,
            query.limit.saturating_add(1),
        );
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

    /// 每个系列只留结果里卷号最小的一卷，同卷号按标题顺序。只遍历结果里属于系列的归档，
    /// 书库里没有系列时没有额外开销。
    fn collapse_series(&self, matches: &mut RoaringBitmap) {
        let members = &*matches & &self.series_docs;
        if members.is_empty() {
            return;
        }
        let mut first: HashMap<i64, (u32, u32, u32)> = HashMap::new();
        for doc in &members {
            let (series, volume) = self.doc_series[&doc];
            let candidate = (volume, self.by_title.rank[doc as usize], doc);
            first
                .entry(series)
                .and_modify(|best| *best = (*best).min(candidate))
                .or_insert(candidate);
        }
        *matches -= &members;
        matches.extend(first.into_values().map(|(_, _, doc)| doc));
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
            // 30 万本逐个比较不到 1 ms，不值得为页数另建排好序的结构。
            Term::Pages(comparison) => Cow::Owned(
                (0u32..)
                    .zip(&self.page_counts)
                    .filter(|&(_, &pages)| comparison.matches(pages))
                    .map(|(doc, _)| doc)
                    .collect(),
            ),
            Term::New => Cow::Borrowed(&self.new_docs),
            Term::Untagged => Cow::Borrowed(&self.untagged_docs),
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

    /// 结果里随机的一本，`seed` 由调用方给出。没有结果时返回 `None`。
    pub fn random(&self, query: &Query, seed: u64) -> Option<ArchiveId> {
        let matches = self.evaluate(query);
        let count = matches.len();
        if count == 0 {
            return None;
        }
        let doc = matches.select((seed % count) as u32)?;
        Some(self.ids[doc as usize])
    }

    /// 按命名空间排序的排列。命名空间在索引里没有标签时返回 `None`，这时每本都没有值，
    /// 排出来就是标题自然序。
    fn namespace_order(&self, namespace: &str) -> Option<Arc<SortOrder>> {
        let prefix = format!("{}:", normalize_tag(namespace));
        let lo = self
            .tag_names
            .partition_point(|name| name.as_str() < prefix.as_str());
        let hi = lo + self.tag_names[lo..].partition_point(|name| name.starts_with(&prefix));
        if lo == hi {
            return None;
        }
        let mut cache = self
            .by_namespace
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(order) = cache.get(&prefix) {
            return Some(order.clone());
        }
        // 标签名按字节序排列，和自然序不同（unit10 在 unit2 前面），所以每本要在这个
        // 命名空间的几个值里取自然序最小的，不能直接取第一个。每个值的键只算一次。
        let value_keys: Vec<String> = self.tag_names[lo..hi]
            .iter()
            .map(|name| natural::natural_key(&name[prefix.len()..]))
            .collect();
        let keys: Vec<Option<&str>> = (0..self.ids.len())
            .map(|doc| {
                self.doc_tags[self.doc_tag_start[doc]..self.doc_tag_start[doc + 1]]
                    .iter()
                    .map(|&tag| tag as usize)
                    .filter(|tag| (lo..hi).contains(tag))
                    .map(|tag| value_keys[tag - lo].as_str())
                    .min()
            })
            .collect();
        let mut order: Vec<u32> = (0..self.ids.len() as u32).collect();
        order.sort_unstable_by(|&a, &b| {
            let (key_a, key_b) = (&keys[a as usize], &keys[b as usize]);
            // `None` 排在最后。
            key_a
                .is_none()
                .cmp(&key_b.is_none())
                .then_with(|| key_a.cmp(key_b))
                .then(self.by_title.rank[a as usize].cmp(&self.by_title.rank[b as usize]))
        });
        let order = Arc::new(SortOrder::new(order));
        cache.insert(prefix, order.clone());
        Some(order)
    }

    /// Returns up to `take` matches whose rank is at least `start`, in sort order.
    fn page(
        &self,
        matches: &RoaringBitmap,
        order: &SortOrder,
        reverse: bool,
        start: usize,
        take: usize,
    ) -> Vec<u32> {
        if matches.len() * DENSE_DIVISOR >= self.ids.len() as u64 {
            let docs = order.order.iter().copied();
            let docs: Box<dyn Iterator<Item = u32>> = if reverse {
                Box::new(docs.rev())
            } else {
                Box::new(docs)
            };
            return docs
                .skip(start)
                .filter(|&doc| matches.contains(doc))
                .take(take)
                .collect();
        }
        let rank = |doc: &u32| order.rank(*doc, reverse);
        let mut docs: Vec<u32> = matches.iter().filter(|doc| rank(doc) >= start).collect();
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
