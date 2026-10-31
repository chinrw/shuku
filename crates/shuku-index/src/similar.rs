//! 按标签推荐相似的归档。

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use crate::{ArchiveId, Index};

/// 带这个标签的归档超过全部的 1/10 时不算相似的依据。语言这类标签几乎每本都有，
/// 按它算出来的相似等于随机；跳过它们也限制了要累加的位图大小，30 万本的书库里
/// 每个标签最多累加 3 万本。
const COMMON_DIVISOR: u64 = 10;
/// 不到这么多本的标签不算常见，几十本的小书库里作者标签也会超过 1/10。
const COMMON_MIN: u64 = 100;

impl Index {
    /// 和归档 `id` 最相似的至多 `limit` 本，最相似的在前。
    ///
    /// 两本共有的每个标签加一份权重 ln(全部归档数 / 带这个标签的归档数)，越少见的标签
    /// 权重越高；带这个标签的归档超过全部的 1/10（且多于 100 本）时不计。同分时按标题自然序。
    /// 不含这本自己和同一系列的其他卷；别的系列只留得分最高的一卷，同分取卷号小的，再同取标题靠前的。
    /// `id` 不在索引里时返回空列表。
    pub fn similar(&self, id: ArchiveId, limit: usize) -> Vec<ArchiveId> {
        let Some(&doc) = self.doc_of.get(&id) else {
            return Vec::new();
        };
        if limit == 0 {
            return Vec::new();
        }
        let total = self.ids.len() as u64;
        let mut scores = vec![0f64; self.ids.len()];
        let mut touched = Vec::new();
        for &tag in self.tags_of(doc) {
            let docs = &self.tag_docs[tag as usize];
            let common = docs.len() * COMMON_DIVISOR > total && docs.len() > COMMON_MIN;
            // 每本都有的标签权重为零，也不用累加。
            if common || docs.len() == total {
                continue;
            }
            let weight = (total as f64 / docs.len() as f64).ln();
            for other in docs {
                if scores[other as usize] == 0.0 {
                    touched.push(other);
                }
                scores[other as usize] += weight;
            }
        }

        let own_series = self.doc_series.get(&doc).map(|&(series, _)| series);
        // 每个别的系列得分最高的一卷：(得分, 卷号, 内部编号)。
        let mut best_volume: HashMap<i64, (f64, u32, u32)> = HashMap::new();
        let mut candidates = Vec::new();
        for other in touched {
            if other == doc {
                continue;
            }
            let score = scores[other as usize];
            match self.doc_series.get(&other) {
                None => candidates.push((score, other)),
                Some(&(series, _)) if Some(series) == own_series => {}
                Some(&(series, volume)) => match best_volume.entry(series) {
                    Entry::Vacant(entry) => {
                        entry.insert((score, volume, other));
                    }
                    Entry::Occupied(mut entry) => {
                        let (best_score, best_volume, best) = *entry.get();
                        // 卷号可以重复，同分同卷号时和书库页折叠系列一样取标题靠前的。
                        let better = best_score
                            .total_cmp(&score)
                            .then(volume.cmp(&best_volume))
                            .then(self.title_rank(other).cmp(&self.title_rank(best)))
                            .is_lt();
                        if better {
                            entry.insert((score, volume, other));
                        }
                    }
                },
            }
        }
        candidates.extend(
            best_volume
                .into_values()
                .map(|(score, _, other)| (score, other)),
        );

        let order = |a: &(f64, u32), b: &(f64, u32)| {
            b.0.total_cmp(&a.0)
                .then(self.title_rank(a.1).cmp(&self.title_rank(b.1)))
        };
        if candidates.len() > limit {
            candidates.select_nth_unstable_by(limit, order);
            candidates.truncate(limit);
        }
        candidates.sort_unstable_by(order);
        candidates
            .into_iter()
            .map(|(_, other)| self.ids[other as usize])
            .collect()
    }

    fn title_rank(&self, doc: u32) -> u32 {
        self.by_title.rank[doc as usize]
    }

    fn tags_of(&self, doc: u32) -> &[u32] {
        let doc = doc as usize;
        &self.doc_tags[self.doc_tag_start[doc]..self.doc_tag_start[doc + 1]]
    }
}
