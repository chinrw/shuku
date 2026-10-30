//! 把 LANraragi 存在 Redis 里的元数据导入 Shuku 的 SQLite。
//!
//! 归档必须先由 `shuku scan` 入库，导入按文件路径把 LRR 记录对应到已有归档。
//! 可以反复运行，每次都以 LRR 的数据为准覆盖已导入的部分。

mod redis_source;

use std::collections::HashMap;

pub use redis_source::RedisSource;
use shuku_store::{LrrArchive, LrrCategory, LrrTankoubon, Store, Tag};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Redis(#[from] redis::RedisError),
    #[error(transparent)]
    Store(#[from] shuku_store::Error),
    #[error("LRR 记录 {key} 的字段 {field} 无效：{value}")]
    InvalidField {
        key: String,
        field: &'static str,
        value: String,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// 导入用到的 Redis 读操作。
pub trait Source {
    /// 用 SCAN 列出匹配 glob 模式的键。
    fn scan(&mut self, pattern: &str) -> Result<Vec<Vec<u8>>>;
    fn hash(&mut self, key: &[u8]) -> Result<HashMap<String, Vec<u8>>>;
    /// 有序集合的全部成员及分值。
    fn sorted_set(&mut self, key: &[u8]) -> Result<Vec<(Vec<u8>, f64)>>;
}

#[derive(Debug, Default)]
pub struct Report {
    pub archives: usize,
    /// 在 Shuku 里找不到对应文件的 LRR 记录的 `file` 字段。
    pub unmatched: Vec<String>,
    pub categories: usize,
    pub tankoubons: usize,
    /// 分类和 Tankoubon 里指向未导入归档的成员数。
    pub missing_members: usize,
}

/// `lrr_content_dir` 是 LRR 看到的内容目录。LRR 跑在 Docker 里时它和本机路径不同。
pub fn migrate(
    source: &mut impl Source,
    store: &mut Store,
    lrr_content_dir: &str,
) -> Result<Report> {
    let prefix = lrr_content_dir.trim_end_matches('/');
    let mut report = Report::default();
    // LRR 归档 ID 到 Shuku 归档 ID，用于对应分类和 Tankoubon 的成员。
    let mut ids = HashMap::new();

    for key in source.scan(&"?".repeat(40))? {
        let Ok(lrr_id) = String::from_utf8(key) else {
            continue;
        };
        if !lrr_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let fields = source.hash(lrr_id.as_bytes())?;
        let file = text(&fields, "file");
        let found = match file
            .strip_prefix(prefix)
            .and_then(|path| path.strip_prefix('/'))
        {
            Some(path) => store.find_by_path(path)?,
            None => None,
        };
        let Some(id) = found else {
            report.unmatched.push(file);
            continue;
        };
        store.import_lrr_archive(id, &archive(&lrr_id, &fields)?)?;
        if let Some((page, client_time)) = progress(&lrr_id, &fields)? {
            store.write_progress(id, page, client_time)?;
        }
        ids.insert(lrr_id, id);
        report.archives += 1;
    }

    for key in source.scan("SET_??????????")? {
        let lrr_id = String::from_utf8_lossy(&key).into_owned();
        let fields = source.hash(&key)?;
        let search = text(&fields, "search");
        // 动态分类没有成员，LRR 读取时也忽略它的 archives 字段。
        let members: Vec<String> = if search.is_empty() {
            let raw = text(&fields, "archives");
            serde_json::from_str(&raw).map_err(|_| invalid(&lrr_id, "archives", raw))?
        } else {
            Vec::new()
        };
        let archives = resolve(&members, &ids, &mut report.missing_members);
        store.import_lrr_category(&LrrCategory {
            name: text(&fields, "name"),
            search,
            pinned: text(&fields, "pinned") == "1",
            archives,
            lrr_id,
        })?;
        report.categories += 1;
    }

    for key in source.scan("TANK_??????????")? {
        let lrr_id = String::from_utf8_lossy(&key).into_owned();
        let mut tankoubon = LrrTankoubon {
            lrr_id,
            name: String::new(),
            summary: String::new(),
            tags: Vec::new(),
            archives: Vec::new(),
        };
        // 分值 0、-1、-2 是名字、摘要和标签，-3 是整套的阅读进度（Shuku 不保存）；
        // 分值从 1 开始的成员是归档 ID，分值就是顺序。
        let mut members = Vec::new();
        for (member, score) in source.sorted_set(&key)? {
            let member = decode(&member);
            if score >= 1.0 {
                members.push((score, member));
            } else if let Some(name) = member.strip_prefix("name_") {
                tankoubon.name = name.into();
            } else if let Some(summary) = member.strip_prefix("summary_") {
                tankoubon.summary = summary.into();
            } else if let Some(tags) = member.strip_prefix("tags_") {
                tankoubon.tags = split_tags(tags).map(parse_tag).collect();
            }
        }
        members.sort_by(|a, b| a.0.total_cmp(&b.0));
        let members: Vec<String> = members.into_iter().map(|(_, member)| member).collect();
        tankoubon.archives = resolve(&members, &ids, &mut report.missing_members);
        store.import_lrr_tankoubon(&tankoubon)?;
        report.tankoubons += 1;
    }

    Ok(report)
}

fn archive(lrr_id: &str, fields: &HashMap<String, Vec<u8>>) -> Result<LrrArchive> {
    let mut archive = LrrArchive {
        lrr_id: lrr_id.into(),
        title: text(fields, "title"),
        summary: text(fields, "summary"),
        date_added: None,
        source_url: None,
        is_new: text(fields, "isnew") == "true",
        tags: Vec::new(),
    };
    // LRR 把加入时间和来源存成标签。Shuku 把它们放进归档表的列，
    // 重复出现的 source 标签保留为普通标签，不丢数据。
    for tag in split_tags(&text(fields, "tags")).map(parse_tag) {
        if tag.namespace.eq_ignore_ascii_case("date_added") && archive.date_added.is_none() {
            let seconds = tag
                .value
                .parse()
                .map_err(|_| invalid(lrr_id, "tags", tag.value))?;
            archive.date_added = Some(seconds);
        } else if tag.namespace.eq_ignore_ascii_case("source") && archive.source_url.is_none() {
            archive.source_url = Some(tag.value);
        } else {
            archive.tags.push(tag);
        }
    }
    Ok(archive)
}

/// LRR 的进度从 1 开始计，0 或空表示没读过；阅读时间是 Unix 秒。
fn progress(lrr_id: &str, fields: &HashMap<String, Vec<u8>>) -> Result<Option<(u32, i64)>> {
    let page = number(lrr_id, fields, "progress")?;
    if page == 0 {
        return Ok(None);
    }
    let page =
        u32::try_from(page - 1).map_err(|_| invalid(lrr_id, "progress", page.to_string()))?;
    let seconds = number(lrr_id, fields, "lastreadtime")?;
    Ok(Some((page, seconds * 1000)))
}

/// 缺失或为空的数值字段按 0 处理，与 LRR 自己读取时一致。
fn number(lrr_id: &str, fields: &HashMap<String, Vec<u8>>, field: &'static str) -> Result<i64> {
    let raw = text(fields, field);
    if raw.is_empty() {
        return Ok(0);
    }
    raw.parse().map_err(|_| invalid(lrr_id, field, raw))
}

fn resolve(members: &[String], ids: &HashMap<String, i64>, missing: &mut usize) -> Vec<i64> {
    members
        .iter()
        .filter_map(|member| {
            let id = ids.get(member).copied();
            *missing += usize::from(id.is_none());
            id
        })
        .collect()
}

fn split_tags(tags: &str) -> impl Iterator<Item = &str> {
    tags.split(',').map(str::trim).filter(|tag| !tag.is_empty())
}

fn parse_tag(tag: &str) -> Tag {
    match tag.split_once(':') {
        Some((namespace, value)) => Tag {
            namespace: namespace.into(),
            value: value.into(),
        },
        None => Tag {
            namespace: String::new(),
            value: tag.into(),
        },
    }
}

fn text(fields: &HashMap<String, Vec<u8>>, field: &str) -> String {
    fields
        .get(field)
        .map(|value| decode(value))
        .unwrap_or_default()
}

fn invalid(key: &str, field: &'static str, value: String) -> Error {
    Error::InvalidField {
        key: key.into(),
        field,
        value,
    }
}

/// 照搬 LRR 的 redis_decode：先按 UTF-8 解码，结果的每个字符都在 Latin-1 范围内时
/// 再当作字节解码一次，用来还原被重复编码过的数据。任何一步失败都保留上一步的结果。
fn decode(bytes: &[u8]) -> String {
    let once = match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        // Perl 解码失败时保留原始字节，每个字节就是一个字符。
        Err(_) => return bytes.iter().copied().map(char::from).collect(),
    };
    let narrow: Option<Vec<u8>> = once.chars().map(|ch| u8::try_from(ch).ok()).collect();
    narrow
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or(once)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_like_lrr() {
        assert_eq!(decode(b"plain"), "plain");
        assert_eq!(decode("漫画".as_bytes()), "漫画");
        let twice: String = "漫画".bytes().map(char::from).collect();
        assert_eq!(decode(twice.as_bytes()), "漫画");
        // 单独的 é 再解码一次不是合法 UTF-8，保留一次解码的结果。
        assert_eq!(decode("é".as_bytes()), "é");
        assert_eq!(decode(b"\xff"), "\u{ff}");
    }
}
