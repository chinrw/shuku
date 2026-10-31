use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use shuku_index::ArchiveRecord;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

/// 扫描只需文件身份，不加载标签、进度和页面清单。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEntry {
    pub id: i64,
    pub path: String,
    pub file_size: u64,
    pub mtime: i64,
    pub quick_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveMetadata {
    /// 相对内容目录的路径。
    pub path: String,
    pub title: String,
    pub summary: String,
    pub file_size: u64,
    /// Unix 秒。
    pub mtime: i64,
    pub quick_hash: String,
    pub content_hash: Option<String>,
    /// Unix 秒，与 shuku-index 的时间单位相同。
    pub date_added: i64,
    pub source_url: Option<String>,
    pub is_new: bool,
    pub lrr_id: Option<String>,
}

/// LRR 导入时覆盖的元数据。文件信息和页面清单由扫描维护，导入不改动它们。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LrrArchive {
    pub lrr_id: String,
    pub title: String,
    pub summary: String,
    /// 为 `None` 时用文件的修改时间。扫描时间对从 LRR 迁移来的整个书库都是同一天，
    /// 按最新加入排序时这些书会全挤在最前面。
    pub date_added: Option<i64>,
    pub source_url: Option<String>,
    pub is_new: bool,
    pub tags: Vec<Tag>,
}

/// 导入时还没扫描到的归档，扫描新增它时由 [`crate::Writer::apply_lrr_pending`] 套用。
/// 用 postcard 按字段顺序编码存进数据库，改字段要加迁移重写或清空 `lrr_pending` 表。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LrrPending {
    pub archive: LrrArchive,
    /// 从 0 开始的页码和 Unix 毫秒的阅读时间。
    pub progress: Option<(u32, i64)>,
    /// 所属静态分类的 LRR ID。
    pub categories: Vec<String>,
    /// 所属 Tankoubon 的 LRR ID 和在其中的位置。
    pub tankoubons: Vec<(String, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LrrCategory {
    pub lrr_id: String,
    pub name: String,
    /// 非空表示动态分类，此时 `archives` 为空。
    pub search: String,
    pub pinned: bool,
    pub archives: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LrrTankoubon {
    pub lrr_id: String,
    pub name: String,
    pub summary: String,
    pub tags: Vec<Tag>,
    /// 归档 ID 和它在 LRR 里的位置（从 1 开始）。还没扫描到的成员不在这里，
    /// 扫描到时按同一个位置补进来，所以位置可以不连续。
    pub archives: Vec<(i64, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub id: i64,
    pub metadata: ArchiveMetadata,
    pub page_count: u32,
    pub tags: Vec<Tag>,
    pub progress: Option<Progress>,
}

/// 搜索快照的原料，见 [`crate::Reader::library`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    /// 按小写完整名排序，[`LibraryArchive::tags`] 是这里的下标。
    pub tags: Vec<LibraryTag>,
    /// 按 ID 升序。
    pub archives: Vec<LibraryArchive>,
    pub progress: HashMap<i64, Progress>,
    /// 按置顶在前、再按名字排序。
    pub categories: Vec<Category>,
    /// 至少有两卷的系列，按 ID 升序。
    pub series: Vec<Series>,
}

/// 一部按卷出版的作品。只有一卷的系列不当作系列显示：扫描按文件名归组，
/// 单独一本名字里带 “Vol.2” 的同人志不该显示成系列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series {
    pub id: i64,
    pub name: String,
    /// 各卷的归档 ID，按卷号、再按 ID 排列。
    pub volumes: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: i64,
    pub name: String,
    /// 非空表示动态分类：成员是这条搜索语句的结果，`archives` 为空。
    pub search: String,
    pub pinned: bool,
    /// 静态分类的成员，按 ID 升序。
    pub archives: Vec<i64>,
}

impl Library {
    /// 构建 [`shuku_index::Index`] 用的记录，标签用小写完整名。
    pub fn index_records(&self) -> impl Iterator<Item = ArchiveRecord> + '_ {
        self.archives.iter().map(|archive| ArchiveRecord {
            id: archive.id,
            title: archive.title.clone(),
            date_added: archive.date_added,
            tags: archive
                .tags
                .iter()
                .map(|&position| self.tags[position as usize].full.clone())
                .collect(),
            page_count: archive.page_count,
            is_new: archive.is_new,
            series: archive.series,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryTag {
    pub tag: Tag,
    /// 小写完整名，`namespace:value` 或没有命名空间时的 `value`。
    pub full: String,
}

/// 列表页需要的元数据。内容指纹由服务端用来生成封面 URL。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryArchive {
    pub id: i64,
    pub title: String,
    pub page_count: u32,
    pub quick_hash: String,
    pub is_new: bool,
    /// Unix 秒。
    pub date_added: i64,
    pub source_url: Option<String>,
    /// [`Library::tags`] 的下标，升序，也就是按小写完整名排序，与 [`Archive::tags`] 一致。
    pub tags: Vec<u32>,
    /// 所属系列的 ID 和卷号，只有一卷的系列不算。
    pub series: Option<(i64, u32)>,
}

/// 见 [`crate::Reader::stats`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryStats {
    pub archives: u64,
    pub pages: u64,
    pub bytes: u64,
    /// 已经算出完整指纹的归档数。
    pub hashed: u64,
    /// 从 LRR 导入过元数据的归档数。
    pub lrr_imported: u64,
    pub with_progress: u64,
    /// 暂存着、等扫描到时套用的 LRR 记录数。
    pub lrr_pending: u64,
    /// 有坏页的归档数。
    pub with_broken_pages: u64,
    pub categories: u64,
    pub tankoubons: u64,
}

/// 有坏页的归档，见 [`crate::Reader::broken_archives`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenArchive {
    pub id: i64,
    pub path: String,
    pub title: String,
    /// 坏页的条目名，按页序。
    pub pages: Vec<String>,
}

/// 见 [`crate::Reader::duplicates`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateFile {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub file_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub namespace: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub page: u32,
    /// 客户端翻页时间，单位为 Unix 毫秒。
    pub client_time: i64,
    /// 服务端接受上报的时间，单位为 Unix 毫秒。
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionMethod {
    Stored,
    Deflate,
}

impl ToSql for CompressionMethod {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            Self::Stored => "stored",
            Self::Deflate => "deflate",
        }
        .into())
    }
}

impl FromSql for CompressionMethod {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "stored" => Ok(Self::Stored),
            "deflate" => Ok(Self::Deflate),
            method => Err(FromSqlError::Other(
                format!("不支持的压缩方式：{method}").into(),
            )),
        }
    }
}

/// 页面在清单中的下标就是从 0 开始的 page_no。宽和高为 0 表示读不出尺寸的坏页。
/// 清单用 postcard 按字段顺序编码存进数据库，增删或调换字段都要加一项迁移重写旧数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub entry_name: String,
    pub data_offset: u64,
    pub compressed_size: u64,
    pub size: u64,
    pub method: CompressionMethod,
    pub width: u32,
    pub height: u32,
}

/// 在线来源查到、等人或 agent 确认的结果，见 `metadata_candidate` 表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataCandidate {
    pub archive_id: i64,
    pub provider: String,
    /// 服务端写入的 JSON，存储层不解析。
    pub found: String,
    /// Unix 毫秒。
    pub fetched_at: i64,
}
