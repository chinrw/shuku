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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LrrArchive {
    pub lrr_id: String,
    pub title: String,
    pub summary: String,
    /// 为 `None` 时保留扫描时记录的值。
    pub date_added: Option<i64>,
    pub source_url: Option<String>,
    pub is_new: bool,
    pub tags: Vec<Tag>,
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
    /// 按阅读顺序排列。
    pub archives: Vec<i64>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// 页面在清单中的下标就是从 0 开始的 page_no。
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
