//! 在 AniList 上按系列找漫画条目，取作者、类型、标签和简介。条目以英文和罗马字为主，
//! 搜中文名多半找不到，要用原名或英文名搜。接口文档：<https://docs.anilist.co/>。

use crate::{
    Error, Metadata, Result,
    http::{Client, Http},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use shuku_store::Tag;

const API: &str = "https://graphql.anilist.co";
/// 只取一个条目，字段只要用得上的。
const QUERY: &str = "query ($id: Int, $search: String) {
  Media(id: $id, search: $search, type: MANGA, sort: SEARCH_MATCH) {
    id siteUrl description(asHtml: false) genres
    title { romaji english native }
    tags { name rank isMediaSpoiler isGeneralSpoiler }
    staff(perPage: 25) { edges { role node { name { full native } } } }
  }
}";
/// 这些职位的人记成 `artist:`。职位后面括号里的补充（例如“Art (chs 1-3)”）不看。
const ARTIST_ROLES: &[&str] = &[
    "Story & Art",
    "Story",
    "Art",
    "Original Story",
    "Original Creator",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TitleLanguage {
    Romaji,
    English,
    #[default]
    Native,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnilistOptions {
    /// `series:` 标签用哪种标题，没有时依次退到罗马字、原名、英文。
    pub title: TitleLanguage,
    /// 作者名用原文（例如日文）写法，否则用罗马字。
    pub native_names: bool,
    /// 只要相关度（0 到 100）不低于这个值的标签。
    pub min_tag_rank: u8,
    /// 也要剧透标签。
    pub spoilers: bool,
}

impl Default for AnilistOptions {
    fn default() -> Self {
        Self {
            title: TitleLanguage::Native,
            native_names: true,
            min_tag_rank: 60,
            spoilers: false,
        }
    }
}

pub fn client() -> Result<Client> {
    Client::new(None)
}

#[derive(Debug, Default)]
pub struct Lookup<'a> {
    pub query: Option<&'a str>,
    /// 归档已有的来源地址，是 AniList 条目时直接用它。
    pub source_url: Option<&'a str>,
    /// 用户指定的条目地址，优先于其他方式。
    pub media_url: Option<&'a str>,
}

#[derive(Deserialize)]
struct Response {
    data: Option<Data>,
    #[serde(default)]
    errors: Vec<ApiError>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Data {
    media: Option<Media>,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
    status: Option<u16>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Media {
    id: u64,
    site_url: Option<String>,
    description: Option<String>,
    #[serde(default)]
    genres: Vec<String>,
    title: Titles,
    #[serde(default)]
    tags: Vec<MediaTag>,
    staff: Option<Staff>,
}

#[derive(Deserialize)]
struct Titles {
    romaji: Option<String>,
    english: Option<String>,
    native: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MediaTag {
    name: String,
    #[serde(default)]
    rank: u8,
    #[serde(default)]
    is_media_spoiler: bool,
    #[serde(default)]
    is_general_spoiler: bool,
}

#[derive(Deserialize)]
struct Staff {
    #[serde(default)]
    edges: Vec<StaffEdge>,
}

#[derive(Deserialize)]
struct StaffEdge {
    #[serde(default)]
    role: String,
    node: Person,
}

#[derive(Deserialize)]
struct Person {
    name: PersonName,
}

#[derive(Deserialize)]
struct PersonName {
    full: Option<String>,
    native: Option<String>,
}

pub async fn lookup(
    http: &impl Http,
    options: &AnilistOptions,
    lookup: &Lookup<'_>,
) -> Result<Metadata> {
    let known = match lookup.media_url {
        Some(url) => Some(media_id(url).ok_or_else(|| Error::GalleryUrl(url.into()))?),
        None => lookup.source_url.and_then(media_id),
    };
    let variables = match known {
        Some(id) => json!({ "id": id }),
        None => json!({ "search": lookup.query.ok_or(Error::NotFound)? }),
    };
    // 找不到时 AniList 回 404 和一条 errors，按没找到处理，不当作网络问题。
    let body = match http
        .post_json(API, &json!({"query": QUERY, "variables": variables}))
        .await
    {
        Err(Error::Status(404)) => return Err(Error::NotFound),
        result => result?,
    };
    let response: Response = serde_json::from_str(&body)
        .map_err(|error| Error::Api(format!("AniList 的回复无法解析：{error}")))?;
    if let Some(error) = response.errors.first() {
        // AniList 可能用 HTTP 200 回 GraphQL 错误，状态码在错误里。令牌无效和服务端出错
        // 保留成 `Status`，让服务端暂停这个来源，而不是把书一本本当作没找到。
        return Err(match error.status {
            Some(404) => Error::NotFound,
            Some(429) => Error::Throttled,
            Some(status @ (401 | 403 | 500..)) => Error::Status(status),
            _ => Error::Api(error.message.clone()),
        });
    }
    let media = response
        .data
        .and_then(|data| data.media)
        .ok_or(Error::NotFound)?;
    let mut metadata = media_metadata(media, options);
    if known.is_some() && lookup.media_url.is_none() {
        metadata.source_url = None;
    }
    Ok(metadata)
}

/// `https://anilist.co/manga/118586/...` 这样的条目地址里的 ID。
pub fn media_id(url: &str) -> Option<u64> {
    let url = url.trim();
    let url = if url.contains("://") {
        url::Url::parse(url)
    } else {
        url::Url::parse(&format!("https://{url}"))
    }
    .ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(url.host_str()?, "anilist.co" | "www.anilist.co")
    {
        return None;
    }
    let mut segments = url.path_segments()?;
    if segments.next()? != "manga" {
        return None;
    }
    segments.next()?.parse().ok()
}

fn media_metadata(media: Media, options: &AnilistOptions) -> Metadata {
    let mut tags = Vec::new();
    let titles = &media.title;
    let preferred = match options.title {
        TitleLanguage::Romaji => &titles.romaji,
        TitleLanguage::English => &titles.english,
        TitleLanguage::Native => &titles.native,
    };
    if let Some(series) = [preferred, &titles.romaji, &titles.native, &titles.english]
        .into_iter()
        .flatten()
        .map(|title| title.trim())
        .find(|title| !title.is_empty())
    {
        push(&mut tags, "series", series);
    }
    for edge in media.staff.into_iter().flat_map(|staff| staff.edges) {
        let role = edge.role.split(" (").next().unwrap_or_default().trim();
        if !ARTIST_ROLES.contains(&role) {
            continue;
        }
        let name = &edge.node.name;
        let preferred = if options.native_names {
            name.native.as_ref().or(name.full.as_ref())
        } else {
            name.full.as_ref().or(name.native.as_ref())
        };
        if let Some(name) = preferred {
            push(&mut tags, "artist", name);
        }
    }
    for genre in &media.genres {
        push(&mut tags, "genre", genre);
    }
    for tag in &media.tags {
        let spoiler = tag.is_media_spoiler || tag.is_general_spoiler;
        if tag.rank >= options.min_tag_rank && (options.spoilers || !spoiler) {
            push(&mut tags, "", &tag.name);
        }
    }
    let summary = media
        .description
        .as_deref()
        .map(plain_text)
        .filter(|summary| !summary.is_empty());
    Metadata {
        title: None,
        summary,
        tags,
        source_url: Some(
            media
                .site_url
                .unwrap_or_else(|| format!("https://anilist.co/manga/{}", media.id)),
        ),
    }
}

/// 简介里有 `<br>`、`<i>` 这类标签，转成纯文本。
fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        text.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            text.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_lowercase();
        if tag.starts_with("br") {
            text.push('\n');
        }
        rest = &rest[start + end + 1..];
    }
    text.push_str(rest);
    let text = text
        .replace("&quot;", "\"")
        .replace("&#039;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    // 原文的换行加上 <br> 会变成很多空行，压成最多一个空行。
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim_end) {
        if line.trim().is_empty() && lines.last().is_none_or(|last| last.trim().is_empty()) {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_owned()
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

    const MEDIA: &str = r#"{"data": {"Media": {"id": 118586,
        "siteUrl": "https://anilist.co/manga/118586",
        "description": "The adventure is over.\n<br><br>\nElf mage <i>Frieren</i> &amp; friends.<br><br>\n(Source: VIZ Media)",
        "genres": ["Adventure", "Fantasy"],
        "title": {"romaji": "Sousou no Frieren", "english": "Frieren: Beyond Journey’s End", "native": "葬送のフリーレン"},
        "tags": [{"name": "Magic", "rank": 96, "isMediaSpoiler": false, "isGeneralSpoiler": false},
                 {"name": "Time Skip", "rank": 84, "isMediaSpoiler": false, "isGeneralSpoiler": true},
                 {"name": "Iyashikei", "rank": 40, "isMediaSpoiler": false, "isGeneralSpoiler": false}],
        "staff": {"edges": [
            {"role": "Story", "node": {"name": {"full": "Kanehito Yamada", "native": "山田鐘人"}}},
            {"role": "Art", "node": {"name": {"full": "Tsukasa Abe", "native": "アベツカサ"}}},
            {"role": "Translator (English)", "node": {"name": {"full": "Someone", "native": null}}}]}}}}"#;

    fn names(metadata: &Metadata) -> Vec<String> {
        metadata
            .tags
            .iter()
            .map(|tag| format!("{}:{}", tag.namespace, tag.value))
            .collect()
    }

    #[tokio::test]
    async fn a_search_maps_staff_genres_and_ranked_tags() {
        let http = FakeHttp::new(vec![Ok(MEDIA.into())]);
        let metadata = lookup(
            &http,
            &AnilistOptions::default(),
            &Lookup {
                query: Some("Sousou no Frieren"),
                ..Lookup::default()
            },
        )
        .await
        .unwrap();
        assert!(http.requests()[0].contains(r#""variables":{"search":"Sousou no Frieren"}"#));
        assert_eq!(
            names(&metadata),
            [
                "series:葬送のフリーレン",
                "artist:山田鐘人",
                "artist:アベツカサ",
                "genre:Adventure",
                "genre:Fantasy",
                ":Magic"
            ]
        );
        assert_eq!(
            metadata.summary.as_deref(),
            Some("The adventure is over.\n\nElf mage Frieren & friends.\n\n(Source: VIZ Media)")
        );
        assert_eq!(
            metadata.source_url.as_deref(),
            Some("https://anilist.co/manga/118586")
        );
    }

    #[tokio::test]
    async fn options_pick_titles_names_and_spoilers() {
        let http = FakeHttp::new(vec![Ok(MEDIA.into())]);
        let metadata = lookup(
            &http,
            &AnilistOptions {
                title: TitleLanguage::English,
                native_names: false,
                min_tag_rank: 30,
                spoilers: true,
            },
            &Lookup {
                media_url: Some("https://anilist.co/manga/118586/Sousou-no-Frieren"),
                ..Lookup::default()
            },
        )
        .await
        .unwrap();
        assert!(http.requests()[0].contains(r#""variables":{"id":118586}"#));
        let names = names(&metadata);
        assert_eq!(names[0], "series:Frieren: Beyond Journey’s End");
        assert!(names.contains(&"artist:Kanehito Yamada".into()));
        assert!(names.contains(&":Time Skip".into()));
        assert!(names.contains(&":Iyashikei".into()));
    }

    #[tokio::test]
    async fn not_found_and_rate_limits_are_reported() {
        for (reply, expected) in [
            (Err(Error::Status(404)), "NotFound"),
            (
                Ok(r#"{"data": {"Media": null}, "errors": [{"message": "Not Found.", "status": 404}]}"#.into()),
                "NotFound",
            ),
            (
                Ok(r#"{"data": null, "errors": [{"message": "Too Many Requests.", "status": 429}]}"#.into()),
                "Throttled",
            ),
            (
                Ok(r#"{"data": null, "errors": [{"message": "Service unavailable", "status": 500}]}"#.into()),
                "Status(500)",
            ),
            (
                Ok(r#"{"data": null, "errors": [{"message": "Invalid token", "status": 403}]}"#.into()),
                "Status(403)",
            ),
        ] {
            let http = FakeHttp::new(vec![reply]);
            let error = lookup(
                &http,
                &AnilistOptions::default(),
                &Lookup {
                    query: Some("x"),
                    ..Lookup::default()
                },
            )
            .await
            .unwrap_err();
            assert!(format!("{error:?}").contains(expected), "{error:?}");
        }
    }

    #[test]
    fn media_urls_are_recognized_only_on_anilist() {
        assert_eq!(media_id("https://anilist.co/manga/118586/x"), Some(118586));
        assert_eq!(media_id("https://anilist.co/anime/1"), None);
        assert_eq!(media_id("https://example.org/anilist.co/manga/1"), None);
    }
}
