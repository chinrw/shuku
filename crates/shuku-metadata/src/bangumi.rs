//! 在 Bangumi（bgm.tv）上按系列找书籍条目，取作者、出版社、连载杂志、简介和用户标签。
//! 中文名多，日本漫画的单行本信息比较全。接口文档：<https://bangumi.github.io/api/>。

use crate::{
    Error, Metadata, Result,
    http::{Client, Http},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shuku_store::Tag;

const API: &str = "https://api.bgm.tv/v0";
/// 条目类型 1 是书籍，漫画、小说和画集都在里面。
const BOOK: u8 = 1;
/// 这些信息框字段里的人名记成 `artist:`。
const ARTIST_KEYS: &[&str] = &["作者", "作画", "原作", "脚本", "插图"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BangumiOptions {
    /// 系列名用中文名（有的话），否则用原名。
    pub prefer_chinese: bool,
    /// 最多加这么多个用户标签，按标注人数从多到少。
    pub tag_limit: usize,
}

impl Default for BangumiOptions {
    fn default() -> Self {
        Self {
            prefer_chinese: true,
            tag_limit: 10,
        }
    }
}

/// 带访问令牌的客户端。没有令牌也能查，但搜不到被标成 NSFW 的条目。
pub fn client(token: &str) -> Result<Client> {
    let token = token.trim();
    Client::new(
        (!token.is_empty()).then(|| (reqwest::header::AUTHORIZATION, format!("Bearer {token}"))),
    )
}

#[derive(Debug, Default)]
pub struct Lookup<'a> {
    /// 搜索词，见 [`crate::series::series_query`]。
    pub query: Option<&'a str>,
    /// 归档已有的来源地址，是 Bangumi 条目时直接用它。
    pub source_url: Option<&'a str>,
    /// 用户指定的条目地址，优先于其他方式。
    pub subject_url: Option<&'a str>,
}

#[derive(Deserialize)]
struct SearchResult {
    #[serde(default)]
    data: Vec<Subject>,
}

#[derive(Deserialize)]
struct Subject {
    id: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    name_cn: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    tags: Vec<SubjectTag>,
    #[serde(default)]
    infobox: Vec<InfoboxItem>,
}

#[derive(Deserialize)]
struct SubjectTag {
    name: String,
    #[serde(default)]
    count: u64,
}

#[derive(Deserialize)]
struct InfoboxItem {
    key: String,
    /// 字符串，或者 `[{"k": ..., "v": ...}]` 这样的列表（别名、各地区版本）。
    value: Value,
}

pub async fn lookup(
    http: &impl Http,
    options: &BangumiOptions,
    lookup: &Lookup<'_>,
) -> Result<Metadata> {
    let known = match lookup.subject_url {
        Some(url) => Some(subject_id(url).ok_or_else(|| Error::GalleryUrl(url.into()))?),
        None => lookup.source_url.and_then(subject_id),
    };
    let subject = match known {
        Some(id) => {
            let body = http
                .get(&format!("{API}/subjects/{id}"))
                .await
                .map_err(not_found_on_404)?;
            serde_json::from_str::<Subject>(&body).map_err(api_error)?
        }
        None => {
            let query = lookup.query.ok_or(Error::NotFound)?;
            let body = http
                .post_json(
                    &format!("{API}/search/subjects?limit=1"),
                    &json!({"keyword": query, "filter": {"type": [BOOK]}}),
                )
                .await?;
            serde_json::from_str::<SearchResult>(&body)
                .map_err(api_error)?
                .data
                .into_iter()
                .next()
                .ok_or(Error::NotFound)?
        }
    };
    let mut metadata = subject_metadata(subject, options);
    // 来源已经是这个条目时不再记一遍。
    if known.is_some() && lookup.subject_url.is_none() {
        metadata.source_url = None;
    }
    Ok(metadata)
}

/// `https://bgm.tv/subject/12345` 这样的条目地址里的 ID。
pub fn subject_id(url: &str) -> Option<u64> {
    let url = url.trim();
    let url = if url.contains("://") {
        url::Url::parse(url)
    } else {
        url::Url::parse(&format!("https://{url}"))
    }
    .ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(
            url.host_str()?,
            "bgm.tv" | "bangumi.tv" | "chii.in" | "www.bgm.tv" | "www.bangumi.tv"
        )
    {
        return None;
    }
    let mut segments = url.path_segments()?;
    if segments.next()? != "subject" {
        return None;
    }
    segments.next()?.parse().ok()
}

fn not_found_on_404(error: Error) -> Error {
    match error {
        Error::Status(404) => Error::NotFound,
        error => error,
    }
}

fn api_error(error: serde_json::Error) -> Error {
    Error::Api(format!("Bangumi 的回复无法解析：{error}"))
}

fn subject_metadata(subject: Subject, options: &BangumiOptions) -> Metadata {
    let mut tags = Vec::new();
    let series = if options.prefer_chinese && !subject.name_cn.trim().is_empty() {
        subject.name_cn.trim()
    } else {
        subject.name.trim()
    };
    push(&mut tags, "series", series);
    for item in &subject.infobox {
        let namespace = match item.key.as_str() {
            key if ARTIST_KEYS.contains(&key) => "artist",
            "出版社" => "publisher",
            "连载杂志" => "magazine",
            _ => continue,
        };
        // 值是一个字符串，或者几个人分开写成 `[{"v": "甲"}, {"v": "乙"}]`。
        let texts: Vec<&str> = match &item.value {
            Value::String(text) => vec![text.as_str()],
            Value::Array(items) => items
                .iter()
                .filter_map(|item| item.get("v").and_then(Value::as_str))
                .collect(),
            _ => Vec::new(),
        };
        for name in texts
            .into_iter()
            .flat_map(|text| text.split(['、', ',', '，', '/', '／', '&']))
        {
            push(&mut tags, namespace, name);
        }
    }
    // 用户标签里常有作者名、出版社和年份，这些已经在上面了，不再当作普通标签加一遍。
    let known: Vec<String> = tags.iter().map(|tag| tag.value.to_lowercase()).collect();
    let mut user_tags = subject.tags;
    user_tags.sort_by_key(|tag| std::cmp::Reverse(tag.count));
    let extra: Vec<String> = user_tags
        .into_iter()
        .map(|tag| tag.name.trim().to_owned())
        .filter(|name| {
            !name.is_empty()
                && name != "漫画"
                && !name.chars().all(|c| c.is_ascii_digit())
                && !known.contains(&name.to_lowercase())
        })
        .take(options.tag_limit)
        .collect();
    for name in &extra {
        push(&mut tags, "", name);
    }
    let summary = subject
        .summary
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned();
    Metadata {
        title: None,
        summary: (!summary.is_empty()).then_some(summary),
        tags,
        source_url: Some(format!("https://bgm.tv/subject/{}", subject.id)),
        volume: None,
    }
}

fn push(tags: &mut Vec<Tag>, namespace: &str, value: &str) {
    let value = value.trim();
    if value.is_empty()
        || tags
            .iter()
            .any(|tag| tag.namespace == namespace && tag.value == value)
    {
        return;
    }
    tags.push(Tag {
        namespace: namespace.into(),
        value: value.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::FakeHttp;

    const SUBJECT: &str = r#"{"id": 305429, "name": "葬送のフリーレン", "name_cn": "葬送的芙莉莲",
        "summary": "故事从“冒险的结束”开始。\r\n后日谈奇幻作品！",
        "tags": [{"name": "漫画", "count": 1076}, {"name": "旅行", "count": 689},
                 {"name": "奇幻", "count": 650}, {"name": "山田鐘人", "count": 384},
                 {"name": "2020", "count": 249}, {"name": "治愈", "count": 591}],
        "infobox": [{"key": "中文名", "value": "葬送的芙莉莲"},
                    {"key": "别名", "value": [{"v": "FRIEREN"}]},
                    {"key": "作画", "value": "アベツカサ"}, {"key": "原作", "value": "山田鐘人"},
                    {"key": "出版社", "value": "小学館"}, {"key": "连载杂志", "value": "週刊少年サンデー"},
                    {"key": "版本:东立版", "value": [{"k": "出版社", "v": "東立出版社"}]}]}"#;

    fn names(metadata: &Metadata) -> Vec<String> {
        metadata
            .tags
            .iter()
            .map(|tag| format!("{}:{}", tag.namespace, tag.value))
            .collect()
    }

    #[tokio::test]
    async fn a_search_takes_the_first_book_and_maps_the_infobox() {
        let http = FakeHttp::new(vec![Ok(format!(r#"{{"data": [{SUBJECT}], "total": 1}}"#))]);
        let metadata = lookup(
            &http,
            &BangumiOptions {
                tag_limit: 2,
                ..BangumiOptions::default()
            },
            &Lookup {
                query: Some("葬送的芙莉莲"),
                ..Lookup::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            http.requests(),
            [
                r#"POST https://api.bgm.tv/v0/search/subjects?limit=1 {"filter":{"type":[1]},"keyword":"葬送的芙莉莲"}"#
            ]
        );
        assert_eq!(metadata.title, None);
        assert_eq!(
            metadata.summary.as_deref(),
            Some("故事从“冒险的结束”开始。\n后日谈奇幻作品！")
        );
        // 用户标签去掉了“漫画”、年份和已经记成作者的名字，按人数取前两个。
        assert_eq!(
            names(&metadata),
            [
                "series:葬送的芙莉莲",
                "artist:アベツカサ",
                "artist:山田鐘人",
                "publisher:小学館",
                "magazine:週刊少年サンデー",
                ":旅行",
                ":奇幻"
            ]
        );
        assert_eq!(
            metadata.source_url.as_deref(),
            Some("https://bgm.tv/subject/305429")
        );
    }

    #[tokio::test]
    async fn a_known_subject_is_fetched_directly_and_keeps_the_source() {
        let http = FakeHttp::new(vec![Ok(SUBJECT.into())]);
        let metadata = lookup(
            &http,
            &BangumiOptions {
                prefer_chinese: false,
                ..BangumiOptions::default()
            },
            &Lookup {
                query: Some("ignored"),
                source_url: Some("https://bgm.tv/subject/305429"),
                ..Lookup::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            http.requests(),
            ["GET https://api.bgm.tv/v0/subjects/305429"]
        );
        assert_eq!(names(&metadata)[0], "series:葬送のフリーレン");
        assert_eq!(metadata.source_url, None);
    }

    #[tokio::test]
    async fn misses_and_bad_urls_are_errors() {
        let http = FakeHttp::new(vec![Ok(r#"{"data": [], "total": 0}"#.into())]);
        let error = lookup(
            &http,
            &BangumiOptions::default(),
            &Lookup {
                query: Some("nothing"),
                ..Lookup::default()
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::NotFound));

        let http = FakeHttp::new(vec![Err(Error::Status(404))]);
        let error = lookup(
            &http,
            &BangumiOptions::default(),
            &Lookup {
                subject_url: Some("https://bgm.tv/subject/1"),
                ..Lookup::default()
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::NotFound));

        let http = FakeHttp::new(Vec::new());
        let error = lookup(
            &http,
            &BangumiOptions::default(),
            &Lookup {
                subject_url: Some("https://example.org/subject/1"),
                ..Lookup::default()
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::GalleryUrl(_)));
        let error = lookup(&http, &BangumiOptions::default(), &Lookup::default())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::NotFound));
    }

    #[tokio::test]
    async fn infobox_values_written_as_lists_are_read_too() {
        let http = FakeHttp::new(vec![Ok(r#"{"id": 1, "name": "作品",
            "infobox": [{"key": "作者", "value": [{"v": "作者甲"}, {"v": "作者乙、作者丙"}]},
                        {"key": "出版社", "value": [{"k": "日本", "v": "社甲"}]}]}"#
            .into())]);
        let metadata = lookup(
            &http,
            &BangumiOptions::default(),
            &Lookup {
                subject_url: Some("https://bgm.tv/subject/1"),
                ..Lookup::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            names(&metadata),
            [
                "series:作品",
                "artist:作者甲",
                "artist:作者乙",
                "artist:作者丙",
                "publisher:社甲"
            ]
        );
    }

    #[test]
    fn subject_urls_are_recognized_only_on_bangumi_hosts() {
        assert_eq!(subject_id("https://bgm.tv/subject/305429"), Some(305429));
        assert_eq!(subject_id("bangumi.tv/subject/305429/ep"), Some(305429));
        assert_eq!(subject_id("https://example.org/bgm.tv/subject/1"), None);
        assert_eq!(subject_id("https://bgm.tv/person/1"), None);
    }
}
