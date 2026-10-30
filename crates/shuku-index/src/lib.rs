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
use std::collections::HashMap;

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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    #[error("archive id {0} appears more than once")]
    DuplicateId(ArchiveId),
    #[error("the index holds at most u32::MAX archives")]
    TooManyArchives,
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
        let (tag_names, tag_docs) = tags
            .into_iter()
            .map(|(name, docs)| (name, docs.into_iter().collect::<RoaringBitmap>()))
            .unzip();

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

fn normalize_tag(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Lowercases text for the title scan. A newline separates titles in the scan
/// buffer, so newlines inside a title or needle become spaces.
fn scan_text(text: &str) -> String {
    text.to_lowercase().replace('\n', " ")
}
