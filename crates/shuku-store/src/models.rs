use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub id: i64,
    pub metadata: ArchiveMetadata,
    pub page_count: u32,
    pub tags: Vec<Tag>,
    pub progress: Option<Progress>,
}

/// 列表项携带内容指纹，由服务端据此生成封面 URL。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveListItem {
    pub id: i64,
    pub title: String,
    pub page_count: u32,
    pub progress: Option<Progress>,
    pub quick_hash: String,
    pub content_hash: Option<String>,
    pub is_new: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub entry_name: String,
    pub data_offset: u64,
    pub compressed_size: u64,
    pub size: u64,
    pub method: CompressionMethod,
    pub width: u32,
    pub height: u32,
}
