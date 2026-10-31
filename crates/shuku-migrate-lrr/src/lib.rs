//! 把 LANraragi 存在 Redis 里的元数据导入 Shuku 的 SQLite。
//!
//! 导入按文件路径把 LRR 记录对应到归档。已经扫描入库的归档直接写入；
//! 还没扫描到的暂存起来，扫描新增那个路径时再套用，所以导入和扫描谁先谁后都可以。
//! 可以反复运行，每次都以 LRR 的数据为准覆盖已导入的部分。

mod redis_source;

use std::collections::HashMap;

pub use redis_source::RedisSource;
use shuku_store::{LrrArchive, LrrCategory, LrrPending, LrrTankoubon, Store, Tag, Writer};

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
    /// 直接写进已入库归档的记录数。
    pub archives: usize,
    /// 还没扫描到、暂存起来等扫描时套用的记录数。
    pub pending: usize,
    /// 路径不在 `lrr_content_dir` 下的 LRR 记录的 `file` 字段。
    pub unmatched: Vec<String>,
    /// 写入或暂存的记录里没有 date_added 标签的数量，这些归档用文件的修改时间作为加入时间。
    pub undated: usize,
    pub categories: usize,
    pub tankoubons: usize,
    /// 分类和 Tankoubon 里指向不存在的 LRR 记录的成员数。
    pub missing_members: usize,
}

/// 每个事务写这么多本。服务端可能同时在运行，事务短一些，它的写入等得也短。
const BATCH: usize = 512;

struct Record {
    path: String,
    archive: LrrArchive,
    progress: Option<(u32, i64)>,
}

/// 一本归档所属的分类和 Tankoubon，暂存时随元数据一起保存。
#[derive(Clone, Default)]
struct Memberships {
    categories: Vec<String>,
    tankoubons: Vec<(String, u32)>,
}

struct Group<T> {
    value: T,
    /// 按顺序排列的 LRR 归档 ID。
    members: Vec<String>,
}

/// `lrr_content_dir` 是 LRR 看到的内容目录。LRR 跑在 Docker 里时它和本机路径不同。
pub fn migrate(
    source: &mut impl Source,
    store: &mut Store,
    lrr_content_dir: &str,
) -> Result<Report> {
    let prefix = lrr_content_dir.trim_end_matches('/');
    let mut report = Report::default();
    // 先把 LRR 的数据全部读进内存：暂存还没扫描到的归档时要带上它所属的分类和 Tankoubon。
    let records = read_archives(source, prefix, &mut report)?;
    let categories = read_categories(source)?;
    let tankoubons = read_tankoubons(source)?;

    let mut memberships: HashMap<&str, Memberships> = HashMap::new();
    for category in &categories {
        for member in &category.members {
            let entry = memberships.entry(member).or_default();
            entry.categories.push(category.value.lrr_id.clone());
        }
    }
    for tankoubon in &tankoubons {
        for (position, member) in positions(&tankoubon.members) {
            let entry = memberships.entry(member).or_default();
            entry
                .tankoubons
                .push((tankoubon.value.lrr_id.clone(), position));
        }
    }

    // 没有对应 LRR 记录的成员算缺失；暂存的成员不算，扫描到时会补进分类和 Tankoubon。
    let paths: HashMap<&str, &str> = records
        .iter()
        .map(|record| (record.archive.lrr_id.as_str(), record.path.as_str()))
        .collect();
    for members in categories
        .iter()
        .map(|category| &category.members)
        .chain(tankoubons.iter().map(|tankoubon| &tankoubon.members))
    {
        report.missing_members += members
            .iter()
            .filter(|member| !paths.contains_key(member.as_str()))
            .count();
    }
    report.categories = categories.len();
    report.tankoubons = tankoubons.len();

    // 分类和 Tankoubon 先于归档写入，成员按本次 LRR 记录的路径找已入库的归档。服务端可能
    // 同时在扫描，后面每一步都只往分类里加成员，不会被这里的整体替换覆盖：
    // - 这个事务之后、所在批次之前被扫描入库的归档，批次里直接写入时补上成员；
    // - 在批次里暂存、之后被扫描到的归档，扫描套用暂存时分类已经存在，由扫描插入。
    // 旧的暂存在同一个事务里清掉，扫描不会再按上次的成员关系插回已经移除的成员。
    store.write(|writer| -> Result<()> {
        let resolve = |member: &String| -> Result<Option<i64>> {
            match paths.get(member.as_str()) {
                Some(path) => Ok(writer.find_by_path(path)?),
                None => Ok(None),
            }
        };
        let mut resolved = Vec::new();
        for category in &categories {
            let mut archives = Vec::new();
            for member in &category.members {
                archives.extend(resolve(member)?);
            }
            resolved.push(archives);
        }
        let mut ordered = Vec::new();
        for tankoubon in &tankoubons {
            let mut archives = Vec::new();
            for (position, member) in positions(&tankoubon.members) {
                archives.extend(resolve(member)?.map(|id| (id, position)));
            }
            ordered.push(archives);
        }
        for (category, archives) in categories.iter().zip(resolved) {
            writer.import_lrr_category(&LrrCategory {
                archives,
                ..category.value.clone()
            })?;
        }
        for (tankoubon, archives) in tankoubons.iter().zip(ordered) {
            writer.import_lrr_tankoubon(&LrrTankoubon {
                archives,
                ..tankoubon.value.clone()
            })?;
        }
        writer.clear_lrr_pending()?;
        Ok(())
    })?;

    for batch in records.chunks(BATCH) {
        store.write(|writer| -> Result<()> {
            for record in batch {
                let lrr_id = record.archive.lrr_id.as_str();
                report.undated += usize::from(record.archive.date_added.is_none());
                let Memberships {
                    categories,
                    tankoubons,
                } = memberships.get(lrr_id).cloned().unwrap_or_default();
                if let Some(id) = writer.find_by_path(&record.path)? {
                    write_archive(writer, id, record)?;
                    writer.add_lrr_memberships(id, &categories, &tankoubons)?;
                    report.archives += 1;
                } else {
                    writer.stage_lrr_pending(
                        &record.path,
                        &LrrPending {
                            archive: record.archive.clone(),
                            progress: record.progress,
                            categories,
                            tankoubons,
                        },
                    )?;
                    report.pending += 1;
                }
            }
            Ok(())
        })?;
    }
    Ok(report)
}

fn write_archive(writer: &mut Writer<'_>, id: i64, record: &Record) -> Result<()> {
    writer.import_lrr_archive(id, &record.archive)?;
    if let Some((page, client_time)) = record.progress {
        writer.write_progress(id, page, client_time)?;
    }
    Ok(())
}

/// 位置从 1 开始，与 LRR 里的顺序一致。
fn positions(members: &[String]) -> impl Iterator<Item = (u32, &String)> {
    (1..).zip(members)
}

fn read_archives(
    source: &mut impl Source,
    prefix: &str,
    report: &mut Report,
) -> Result<Vec<Record>> {
    let mut records = Vec::new();
    for key in source.scan(&"?".repeat(40))? {
        let Ok(lrr_id) = String::from_utf8(key) else {
            continue;
        };
        if !lrr_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let fields = source.hash(lrr_id.as_bytes())?;
        let file = text(&fields, "file");
        let Some(path) = file
            .strip_prefix(prefix)
            .and_then(|path| path.strip_prefix('/'))
        else {
            report.unmatched.push(file);
            continue;
        };
        records.push(Record {
            path: path.to_owned(),
            progress: progress(&lrr_id, &fields)?,
            archive: archive(&lrr_id, &fields)?,
        });
    }
    Ok(records)
}

fn read_categories(source: &mut impl Source) -> Result<Vec<Group<LrrCategory>>> {
    let mut categories = Vec::new();
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
        categories.push(Group {
            value: LrrCategory {
                name: text(&fields, "name"),
                search,
                pinned: text(&fields, "pinned") == "1",
                archives: Vec::new(),
                lrr_id,
            },
            members,
        });
    }
    Ok(categories)
}

fn read_tankoubons(source: &mut impl Source) -> Result<Vec<Group<LrrTankoubon>>> {
    let mut tankoubons = Vec::new();
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
        tankoubons.push(Group {
            value: tankoubon,
            members: members.into_iter().map(|(_, member)| member).collect(),
        });
    }
    Ok(tankoubons)
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
