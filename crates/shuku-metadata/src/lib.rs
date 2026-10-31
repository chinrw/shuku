//! Shuku 的内置元数据功能：取代 LRR 插件的文件名解析、ComicInfo.xml、E-Hentai/ExHentai 查找
//! 和 EhTagTranslation 中文翻译，以及给按卷出版的普通漫画用的 Bangumi 和 AniList。
//! 这里只解析和联网，不碰数据库；结果由服务端写入。

pub mod anilist;
pub mod bangumi;
pub mod comicinfo;
pub mod ehentai;
pub mod filename;
pub mod http;
pub mod series;
pub mod translate;

use serde::{Deserialize, Serialize};
use shuku_store::Tag;

/// 一次元数据查找的结果。和 LRR 的插件一样，标签只会加到已有标签上，不删旧标签。
/// 服务端把待确认的结果按这个结构存成 JSON，改字段要照顾已经存着的候选。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Metadata {
    /// `None` 表示不改标题。
    pub title: Option<String>,
    /// 归档还没有简介时才写入。
    pub summary: Option<String>,
    pub tags: Vec<Tag>,
    /// 归档还没有来源地址时才写入。
    pub source_url: Option<String>,
    /// 这一本属于哪个系列、是第几本，扫描用它把各卷归成一个系列。不算要写入的元数据，
    /// [`Metadata::is_empty`] 不看它，也不存进待确认的候选。
    #[serde(skip)]
    pub volume: Option<Volume>,
}

/// ComicInfo 的 `Series` 和 `Number`（没有时用 `Volume`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    pub series: String,
    pub number: u32,
}

impl Metadata {
    /// 后来的结果追加在后面，标题和来源以后来的为准。
    pub fn merge(&mut self, other: Metadata) {
        if other.title.is_some() {
            self.title = other.title;
        }
        if other.summary.is_some() {
            self.summary = other.summary;
        }
        if other.source_url.is_some() {
            self.source_url = other.source_url;
        }
        if other.volume.is_some() {
            self.volume = other.volume;
        }
        self.tags.extend(other.tags);
    }

    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.summary.is_none()
            && self.tags.is_empty()
            && self.source_url.is_none()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("正则表达式无效：{0}")]
    Regex(#[from] regex::Error),
    #[error("ComicInfo.xml 无法解析：{0}")]
    Xml(#[from] roxmltree::Error),
    #[error("ComicInfo.xml 不是 UTF-8")]
    XmlEncoding,
    #[error("翻译数据库无法解析：{0}")]
    Translation(#[from] serde_json::Error),
    #[error("联网请求失败：{0}")]
    Http(#[from] reqwest::Error),
    #[error("对方返回 HTTP {0}")]
    Status(u16),
    #[error("对方返回错误：{0}")]
    Api(String),
    #[error("ExHentai 返回了空页面，登录 cookie 可能没设置或已经过期")]
    EmptyPage,
    #[error("访问 E-Hentai 太频繁，IP 被暂时封禁")]
    Banned,
    #[error("对方提示请求太快，过一会儿再查")]
    Throttled,
    #[error("没有找到匹配的结果")]
    NotFound,
    #[error("地址无效：{0}")]
    GalleryUrl(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// 把 `命名空间:值` 拆成标签，没有冒号的是不带命名空间的标签。两边去掉空白，
/// 值为空的返回 `None`。和 LRR 一样只在第一个冒号处拆开。
pub fn parse_tag(text: &str) -> Option<Tag> {
    let (namespace, value) = match text.split_once(':') {
        Some((namespace, value)) => (namespace.trim(), value.trim()),
        None => ("", text.trim()),
    };
    if value.is_empty() {
        return None;
    }
    Some(Tag {
        namespace: namespace.into(),
        value: value.into(),
    })
}

/// 按逗号拆开的标签列表，LRR 插件返回标签的格式。
pub(crate) fn parse_tag_list(text: &str) -> impl Iterator<Item = Tag> + '_ {
    text.split(',').filter_map(parse_tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_splits_at_the_first_colon_and_drops_empty_values() {
        assert_eq!(
            parse_tag(" female : big breasts "),
            Some(Tag {
                namespace: "female".into(),
                value: "big breasts".into()
            })
        );
        assert_eq!(
            parse_tag("source:https://example.org/g/1/a").unwrap().value,
            "https://example.org/g/1/a"
        );
        assert_eq!(parse_tag("full color").unwrap().namespace, "");
        assert_eq!(parse_tag("artist: "), None);
        assert_eq!(parse_tag("  "), None);
    }
}
