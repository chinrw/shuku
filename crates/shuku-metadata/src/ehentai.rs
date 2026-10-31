//! 在 E-Hentai 或 ExHentai 上找到归档对应的画廊，取标题和标签。查找顺序和选项照
//! LRR 的 E-Hentai 元数据插件（2.6）和用户在用的 ETagCN（2.5.3，同样的查找，
//! 结果再译成中文）：已知画廊地址、封面图的 SHA-1、标题里的画廊 ID、标题文字。

use crate::{
    Error, Metadata, Result,
    http::{Client, Http},
    parse_tag,
    translate::Translations,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use shuku_store::Tag;
use std::sync::OnceLock;

const API_URL: &str = "https://api.e-hentai.org/api.php";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EhOptions {
    /// 在 ExHentai 上搜索。要有能访问 ExHentai 的登录 cookie。
    pub exhentai: bool,
    /// 搜索时限定的语言，例如 `chinese`。E-Hentai 的限制使日语不能用。
    pub language: String,
    /// 先用封面图的 SHA-1 搜索，找不到再按标题。
    pub use_thumbnail: bool,
    /// 标题里有 `[数字]` 时先按画廊 ID 搜索。
    pub search_gid: bool,
    /// 有日文标题时用日文标题。
    pub japanese_title: bool,
    /// 另加上传者和发布时间两个标签。
    pub extra_tags: bool,
    /// 也搜索已经被删除的画廊。
    pub expunged: bool,
}

impl Default for EhOptions {
    fn default() -> Self {
        Self {
            exhentai: false,
            language: String::new(),
            use_thumbnail: true,
            search_gid: false,
            japanese_title: false,
            extra_tags: false,
            expunged: false,
        }
    }
}

/// E-Hentai 的登录 cookie，和 LRR 的 E-Hentai 登录插件的四个参数一样。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cookies {
    pub ipb_member_id: String,
    pub ipb_pass_hash: String,
    /// 可选，有它不用 ExHentai 也能看被隐藏的画廊。
    pub star: String,
    /// 可选，有它在欧美以外的 IP 上也能访问 ExHentai。
    pub igneous: String,
}

impl Cookies {
    pub fn is_set(&self) -> bool {
        !self.ipb_member_id.is_empty() && !self.ipb_pass_hash.is_empty()
    }

    /// `Cookie` 请求头的值。没有登录信息时为 `None`，和 LRR 一样连 `nw` 也不带。
    fn header(&self) -> Option<String> {
        if !self.is_set() {
            return None;
        }
        let mut pairs = vec![
            format!("ipb_member_id={}", self.ipb_member_id),
            format!("ipb_pass_hash={}", self.ipb_pass_hash),
        ];
        for (name, value) in [("star", &self.star), ("igneous", &self.igneous)] {
            if !value.is_empty() {
                pairs.push(format!("{name}={value}"));
            }
        }
        // 跳过“内容警告”页，直接拿到画廊链接。
        pairs.push("nw=1".into());
        Some(pairs.join("; "))
    }
}

/// 只给 E-Hentai 用的客户端，每个请求都带登录 cookie。
pub fn client(cookies: &Cookies) -> Result<Client> {
    Client::new(
        cookies
            .header()
            .map(|cookie| (reqwest::header::COOKIE, cookie)),
    )
}

/// 查找要用的归档信息。
#[derive(Debug, Default)]
pub struct Lookup<'a> {
    pub title: &'a str,
    pub tags: &'a [Tag],
    /// 归档已有的来源地址，是 E-Hentai 画廊时直接用它。
    pub source_url: Option<&'a str>,
    /// 用户手动指定的画廊地址，优先于其他方式。
    pub gallery_url: Option<&'a str>,
    /// 封面图原始字节的 SHA-1，见 [`image_hash`]。
    pub cover_hash: Option<&'a str>,
}

#[derive(Debug)]
pub struct Found {
    pub metadata: Metadata,
    /// E-Hentai 提示请求太频繁了，下一次之前应该多等一会儿。
    pub slow_down: bool,
}

/// 文件内容的 SHA-1。E-Hentai 的以图搜图比对的就是原图文件的 SHA-1，
/// LRR 的封面指纹也是第一页原文件的 SHA-1。
pub fn image_hash(bytes: &[u8]) -> String {
    Sha1::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 找画廊、取元数据。`translations` 不为空时把标签译成中文，命名空间的写法跟着 ETagCN：
/// 分类用 `reclass`，上传者和发布时间直接写成中文命名空间。
pub async fn lookup(
    http: &impl Http,
    options: &EhOptions,
    lookup: &Lookup<'_>,
    translations: Option<&Translations>,
) -> Result<Found> {
    let domain = if options.exhentai {
        "exhentai.org"
    } else {
        "e-hentai.org"
    };
    let mut slow_down = false;
    let known = match lookup.gallery_url {
        Some(url) => Some(gallery_id(url).ok_or_else(|| Error::GalleryUrl(url.into()))?),
        None => lookup.source_url.and_then(gallery_id),
    };
    let (gallery, has_source) = match known {
        Some(gallery) => (gallery, lookup.gallery_url.is_none()),
        None => {
            let mut found = None;
            for url in search_urls(domain, options, lookup) {
                let page = search(http, &url).await?;
                slow_down |= page.slow_down;
                if let Some(gallery) = page.gallery {
                    found = Some(gallery);
                    break;
                }
                // 被限速的页面里没有结果不代表真的没有，停下，别接着发下一种搜索。
                if page.slow_down {
                    return Err(Error::Throttled);
                }
            }
            (found.ok_or(Error::NotFound)?, false)
        }
    };
    let response = http
        .post_json(
            API_URL,
            &json!({
                "method": "gdata",
                "gidlist": [[gallery.0, gallery.1]],
                "namespace": 1,
            }),
        )
        .await?;
    let mut metadata = gallery_metadata(&response, options, translations.is_some())?;
    if let Some(translations) = translations {
        metadata.tags = metadata
            .tags
            .iter()
            .map(|tag| translations.translate(tag))
            .collect();
    }
    if !has_source {
        // 手动给的地址按它自己的站点记，搜到的按搜索的站点记。
        let site = match lookup.gallery_url {
            Some(url) if url.contains("exhentai.org") => "exhentai.org",
            Some(_) => "e-hentai.org",
            None => domain,
        };
        metadata.source_url = Some(format!("https://{site}/g/{}/{}/", gallery.0, gallery.1));
    }
    Ok(Found {
        metadata,
        slow_down,
    })
}

/// 画廊地址里的 ID 和 token。
/// 只认 E-Hentai 和 ExHentai 自己的主机，路径以 `/g/ID/token` 开头。LRR 记的来源不带协议
/// （`exhentai.org/g/1/abc`），这时按 https 补上再解析。
pub fn gallery_id(url: &str) -> Option<(u64, String)> {
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
            "e-hentai.org" | "exhentai.org" | "g.e-hentai.org" | "www.e-hentai.org"
        )
    {
        return None;
    }
    let mut segments = url.path_segments()?;
    if segments.next()? != "g" {
        return None;
    }
    let id = segments.next()?.parse().ok()?;
    let token = segments.next()?;
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return None;
    }
    Some((id, token.to_lowercase()))
}

/// 依次尝试的搜索页。
fn search_urls(domain: &str, options: &EhOptions, lookup: &Lookup<'_>) -> Vec<String> {
    let base = format!("https://{domain}/");
    let mut urls = Vec::new();
    if options.use_thumbnail
        && let Some(hash) = lookup.cover_hash
    {
        urls.push(format!("{base}?f_shash={hash}&fs_similar=on&fs_covers=on"));
    }
    if options.search_gid
        && let Some(gid) = title_gid(lookup.title)
    {
        urls.push(format!("{base}?f_search={}", encode(&format!("gid:{gid}"))));
    }
    // 关掉语言、上传者和标签的默认过滤，免得用户的 E-Hentai 设置把结果滤掉。
    let mut url = format!(
        "{base}?advsearch=1&f_sfu=on&f_sft=on&f_sfl=on&f_search={}",
        encode(&format!("\"{}\"", lookup.title))
    );
    // 作者名只有 ASCII 时才加进去，E-Hentai 的作者标签都是罗马字。
    if let Some(artist) = lookup
        .tags
        .iter()
        .find(|tag| matches!(tag.namespace.as_str(), "artist" | "艺术家"))
        .map(|tag| tag.value.as_str())
        .filter(|artist| artist.is_ascii())
    {
        url = format!("{url}+{}", encode(&format!("artist:{artist}")));
    }
    if !options.language.is_empty() {
        url = format!(
            "{url}+{}",
            encode(&format!("language:{}", options.language))
        );
    }
    if options.expunged {
        url.push_str("&f_sh=on");
    }
    urls.push(url);
    urls
}

fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

fn title_gid(title: &str) -> Option<&str> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| Regex::new(r"\[([0-9]+)\]").expect("固定的正则"));
    pattern
        .captures(title)
        .and_then(|captures| captures.get(1))
        .map(|gid| gid.as_str())
}

struct SearchPage {
    gallery: Option<(u64, String)>,
    slow_down: bool,
}

async fn search(http: &impl Http, url: &str) -> Result<SearchPage> {
    let body = http.get(url).await?;
    parse_search(&body)
}

/// 搜索结果页的第一个画廊。各种列表模式里，画廊链接 `<a>` 的第一个子元素都带 `glink` 类。
fn parse_search(body: &str) -> Result<SearchPage> {
    // 没有有效 cookie 的 ExHentai 返回 200 和空白页。
    if body.trim().is_empty() {
        return Err(Error::EmptyPage);
    }
    if body.contains("Your IP address has been") {
        return Err(Error::Banned);
    }
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(
            r#"<a href="https?://e(?:x|-)hentai\.org/g/([0-9]+)/([0-9a-z]+)/?"[^>]*>\s*<div class="[^"]*\bglink\b"#,
        )
        .expect("固定的正则")
    });
    let gallery = pattern
        .captures(body)
        .and_then(|captures| Some((captures[1].parse().ok()?, captures[2].to_owned())));
    Ok(SearchPage {
        gallery,
        slow_down: body.contains("You are opening"),
    })
}

#[derive(Deserialize)]
struct GalleryData {
    gmetadata: Vec<GalleryEntry>,
}

#[derive(Deserialize)]
struct GalleryEntry {
    error: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    title_jpn: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    uploader: String,
    #[serde(default)]
    posted: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// 把 API 的 gdata 回复变成元数据。`chinese` 时用 ETagCN 的命名空间写法。
fn gallery_metadata(response: &str, options: &EhOptions, chinese: bool) -> Result<Metadata> {
    let value: Value =
        serde_json::from_str(response).map_err(|error| Error::Api(error.to_string()))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(Error::Api(error.into()));
    }
    let data: GalleryData =
        serde_json::from_value(value).map_err(|error| Error::Api(error.to_string()))?;
    let entry = data
        .gmetadata
        .into_iter()
        .next()
        .ok_or_else(|| Error::Api("回复里没有画廊".into()))?;
    if let Some(error) = entry.error {
        return Err(Error::Api(error));
    }
    let mut tags: Vec<Tag> = entry.tags.iter().filter_map(|tag| parse_tag(tag)).collect();
    let (category, uploader, posted) = if chinese {
        ("reclass", "上传者", "时间戳")
    } else {
        ("category", "uploader", "timestamp")
    };
    let mut push = |namespace: &str, value: &str| {
        if !value.trim().is_empty() {
            tags.push(Tag {
                namespace: namespace.into(),
                value: value.trim().into(),
            });
        }
    };
    push(category, &entry.category.to_lowercase());
    if options.extra_tags {
        push(uploader, &entry.uploader);
        push(posted, &entry.posted);
    }
    let title = if options.japanese_title && !entry.title_jpn.trim().is_empty() {
        entry.title_jpn
    } else {
        entry.title
    };
    let title = unescape_html(title.trim());
    Ok(Metadata {
        title: (!title.is_empty()).then_some(title),
        summary: None,
        tags,
        source_url: None,
    })
}

/// API 返回的标题里有 HTML 实体，例如 `&amp;` 和 `&#039;`。
fn unescape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|&end| end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let character = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|n| n.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((character, end))
        });
        match decoded {
            Some((character, end)) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 按顺序回放预设的回复，并记下请求的地址。
    struct FakeHttp {
        pages: Mutex<Vec<String>>,
        api: String,
        requests: Mutex<Vec<String>>,
    }

    impl FakeHttp {
        fn new(pages: &[&str], api: &str) -> Self {
            Self {
                pages: Mutex::new(pages.iter().rev().map(|page| (*page).into()).collect()),
                api: api.into(),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Http for FakeHttp {
        async fn get(&self, url: &str) -> Result<String> {
            self.requests.lock().unwrap().push(url.into());
            Ok(self.pages.lock().unwrap().pop().expect("多出来的请求"))
        }

        async fn post_json(&self, url: &str, body: &Value) -> Result<String> {
            self.requests.lock().unwrap().push(format!("{url} {body}"));
            Ok(self.api.clone())
        }
    }

    const API: &str = r#"{"gmetadata":[{"gid":123,"token":"abcdef0123","title":"Title &amp; More",
        "title_jpn":"タイトル","category":"Doujinshi","uploader":"someone","posted":"1600000000",
        "tags":["language:chinese","female:big breasts","other"]}]}"#;

    const RESULT_PAGE: &str = r#"<table class="itg gltc"><tr><td class="gl3c glname">
        <a href="https://exhentai.org/g/123/abcdef0123/"><div class="glink">Title</div></a>"#;

    const NO_RESULTS: &str = "<p>No hits found</p>";

    fn names(tags: &[Tag]) -> Vec<String> {
        tags.iter()
            .map(|tag| format!("{}:{}", tag.namespace, tag.value))
            .collect()
    }

    #[tokio::test]
    async fn a_known_source_url_skips_the_search() {
        let http = FakeHttp::new(&[], API);
        let found = lookup(
            &http,
            &EhOptions::default(),
            &Lookup {
                title: "whatever",
                source_url: Some("https://e-hentai.org/g/123/abcdef0123/"),
                ..Lookup::default()
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(found.metadata.title.as_deref(), Some("Title & More"));
        assert_eq!(
            names(&found.metadata.tags),
            [
                "language:chinese",
                "female:big breasts",
                ":other",
                "category:doujinshi"
            ]
        );
        // 已经有来源地址，不再改它。
        assert_eq!(found.metadata.source_url, None);
        assert_eq!(http.requests().len(), 1);
        assert!(http.requests()[0].contains(r#""gidlist":[[123,"abcdef0123"]]"#));
    }

    #[tokio::test]
    async fn the_cover_hash_is_tried_before_the_title() {
        let http = FakeHttp::new(&[NO_RESULTS, RESULT_PAGE], API);
        let options = EhOptions {
            exhentai: true,
            extra_tags: true,
            japanese_title: true,
            language: "chinese".into(),
            ..EhOptions::default()
        };
        let tags = [Tag {
            namespace: "artist".into(),
            value: "some artist".into(),
        }];
        let found = lookup(
            &http,
            &options,
            &Lookup {
                title: "A Title",
                tags: &tags,
                cover_hash: Some("0123abcd"),
                ..Lookup::default()
            },
            None,
        )
        .await
        .unwrap();
        let requests = http.requests();
        assert_eq!(
            requests[0],
            "https://exhentai.org/?f_shash=0123abcd&fs_similar=on&fs_covers=on"
        );
        assert_eq!(
            requests[1],
            "https://exhentai.org/?advsearch=1&f_sfu=on&f_sft=on&f_sfl=on\
             &f_search=%22A+Title%22+artist%3Asome+artist+language%3Achinese"
        );
        assert_eq!(found.metadata.title.as_deref(), Some("タイトル"));
        assert_eq!(
            found.metadata.source_url.as_deref(),
            Some("https://exhentai.org/g/123/abcdef0123/")
        );
        assert!(
            names(&found.metadata.tags)
                .ends_with(&["uploader:someone".into(), "timestamp:1600000000".into()])
        );
    }

    #[tokio::test]
    async fn translated_results_use_the_etagcn_namespaces() {
        let translations = Translations::from_json(
            r#"{"data":[
                {"namespace":"female","frontMatters":{"name":"女性"},"data":{"big breasts":{"name":"巨乳"}}},
                {"namespace":"reclass","frontMatters":{"name":"重新分类"},"data":{"doujinshi":{"name":"同人志"}}}
            ]}"#
            .as_bytes(),
        )
        .unwrap();
        let http = FakeHttp::new(&[], API);
        let found = lookup(
            &http,
            &EhOptions {
                extra_tags: true,
                ..EhOptions::default()
            },
            &Lookup {
                gallery_url: Some("https://e-hentai.org/g/123/abcdef0123/"),
                ..Lookup::default()
            },
            Some(&translations),
        )
        .await
        .unwrap();
        assert_eq!(
            names(&found.metadata.tags),
            [
                "language:chinese",
                "女性:巨乳",
                ":other",
                "重新分类:同人志",
                "上传者:someone",
                "时间戳:1600000000"
            ]
        );
        // 手动给的地址也记成来源。
        assert_eq!(
            found.metadata.source_url.as_deref(),
            Some("https://e-hentai.org/g/123/abcdef0123/")
        );
    }

    #[tokio::test]
    async fn searches_report_bans_empty_pages_and_misses() {
        for (page, expected) in [
            ("", "空页面"),
            ("Your IP address has been temporarily banned", "封禁"),
            (NO_RESULTS, "没有找到"),
        ] {
            let http = FakeHttp::new(&[page], API);
            let error = lookup(
                &http,
                &EhOptions {
                    use_thumbnail: false,
                    ..EhOptions::default()
                },
                &Lookup {
                    title: "t",
                    ..Lookup::default()
                },
                None,
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }
    }

    #[tokio::test]
    async fn an_api_error_and_a_bad_gallery_url_are_errors() {
        let http = FakeHttp::new(
            &[],
            r#"{"gmetadata":[{"gid":1,"error":"Key missing, or incorrect key provided."}]}"#,
        );
        let error = lookup(
            &http,
            &EhOptions::default(),
            &Lookup {
                gallery_url: Some("https://e-hentai.org/g/1/aaaa/"),
                ..Lookup::default()
            },
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::Api(_)));
        let error = lookup(
            &http,
            &EhOptions::default(),
            &Lookup {
                gallery_url: Some("https://example.org/"),
                ..Lookup::default()
            },
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::GalleryUrl(_)));
    }

    #[test]
    fn only_gallery_urls_on_the_e_hentai_hosts_are_accepted() {
        for url in [
            "https://e-hentai.org/g/123/abcdef0123/",
            "http://exhentai.org/g/123/abcdef0123",
            "exhentai.org/g/123/abcdef0123/?p=1",
            "https://g.e-hentai.org/g/123/abcdef0123/",
        ] {
            assert_eq!(gallery_id(url), Some((123, "abcdef0123".into())), "{url}");
        }
        for url in [
            "https://example.org/e-hentai.org/g/123/abc/",
            "https://example.org/?u=https://e-hentai.org/g/123/abc/",
            "https://e-hentai.org.example.org/g/123/abc/",
            "https://e-hentai.org/s/123/abc/",
            "https://e-hentai.org/g/abc/def/",
            "ftp://e-hentai.org/g/123/abc/",
        ] {
            assert_eq!(gallery_id(url), None, "{url}");
        }
    }

    #[test]
    fn the_gid_search_uses_the_number_in_brackets() {
        let urls = search_urls(
            "e-hentai.org",
            &EhOptions {
                use_thumbnail: false,
                search_gid: true,
                expunged: true,
                ..EhOptions::default()
            },
            &Lookup {
                title: "Title [1234567]",
                ..Lookup::default()
            },
        );
        assert_eq!(urls[0], "https://e-hentai.org/?f_search=gid%3A1234567");
        assert!(urls[1].ends_with("&f_sh=on"));
    }

    #[tokio::test]
    async fn a_throttled_search_without_results_stops_instead_of_moving_on() {
        let http = FakeHttp::new(&["You are opening pages too fast"], API);
        let error = lookup(
            &http,
            &EhOptions::default(),
            &Lookup {
                title: "t",
                cover_hash: Some("abc"),
                ..Lookup::default()
            },
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::Throttled));
        assert_eq!(http.requests().len(), 1);
    }

    #[test]
    fn the_search_page_reports_a_slow_down_hint() {
        let page = parse_search(&format!("You are opening pages too fast {RESULT_PAGE}")).unwrap();
        assert!(page.slow_down);
        assert_eq!(page.gallery, Some((123, "abcdef0123".into())));
    }

    #[test]
    fn cookies_are_sent_only_with_a_member_id_and_pass_hash() {
        assert_eq!(Cookies::default().header(), None);
        let cookies = Cookies {
            ipb_member_id: "1".into(),
            ipb_pass_hash: "hash".into(),
            star: String::new(),
            igneous: "ig".into(),
        };
        assert_eq!(
            cookies.header().as_deref(),
            Some("ipb_member_id=1; ipb_pass_hash=hash; igneous=ig; nw=1")
        );
    }

    #[test]
    fn html_entities_in_titles_are_decoded() {
        assert_eq!(
            unescape_html("A &amp; B &#039;C&#x27; &unknown; & D"),
            "A & B 'C' &unknown; & D"
        );
    }

    #[test]
    fn the_image_hash_is_the_sha1_hex() {
        assert_eq!(
            image_hash(b"abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }
}
