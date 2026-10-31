//! 从文件名取标题和标签，行为照 LRR 的 Filename Parsing 插件（RegexParse 1.2）：
//! 默认按同人志的命名习惯 `(活动) [作者] 标题 (原作) [语言]` 拆开。

use crate::{Metadata, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use shuku_store::Tag;

/// LRR 的默认正则改写成 Rust 的语法：Rust 的字符类里 `[` 开始一个嵌套类，要转义。
pub const DEFAULT_REGEX: &str = r"(\((?<event>[^(\[]+)\))?\s*(\[(?<artist>[^\]]+)\])?\s*(?<title>[^(\[]+)\s*(\((?<series>[^(\[)]+)\))?\s*(\[(?<language>[^\]]+)\])?(?<tail>.*)?";

/// 不属于标准格式的方括号和圆括号内容放在这个命名空间下，打开 `keep_all_captures` 才保留。
const EXTRA_NAMESPACE: &str = "parsed";

/// 放在有命名空间的位置上时多半不是名字，改归到 `parsed:`。
const COMMON_EXTRANEOUS_VALUES: &[&str] = &[
    "uncensored",
    "decensored",
    "ongoing",
    "pixiv",
    "twitter",
    "fanbox",
    "cosplay",
    "digital",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilenameOptions {
    /// 文件名以 `{a, b}` 结尾时，把花括号里的内容当作不带命名空间的标签。
    pub trailing_tags: bool,
    /// 标准格式之外的括号内容也留下，放在 `parsed:` 命名空间。
    pub keep_all_captures: bool,
    /// 用命名分组决定标签的命名空间，见 [`DEFAULT_REGEX`]。为空时用默认正则。
    pub regex: String,
}

/// 编译好的解析器，扫描整个书库时只编译一次正则。
pub struct FilenameParser {
    regex: Regex,
    options: FilenameOptions,
}

impl FilenameParser {
    pub fn new(options: &FilenameOptions) -> Result<Self> {
        let pattern = if options.regex.trim().is_empty() {
            DEFAULT_REGEX
        } else {
            &options.regex
        };
        Ok(Self {
            regex: Regex::new(pattern)?,
            options: options.clone(),
        })
    }

    /// `stem` 是去掉扩展名的文件名。正则匹配不上时返回 `None`，什么都不改。
    pub fn parse(&self, stem: &str) -> Option<Metadata> {
        let mut filename = stem.replace('_', " ");
        // 末尾的花括号先从整个文件名上拿掉：LRR 只在 tail 里找，没有语言和原作时
        // 默认正则的标题会把花括号吞进去，标签就成了标题的一部分。
        let mut trailing = None;
        if self.options.trailing_tags
            && let Some((head, tags)) = trailing_braces(filename.trim_end())
        {
            trailing = Some(tags.to_owned());
            filename = head.to_owned();
        }
        let captures = self.regex.captures(&filename)?;
        let tail = captures
            .name("tail")
            .map_or("", |tail| tail.as_str().trim());
        let extra = if self.options.keep_all_captures {
            bracketed(tail)
        } else {
            Vec::new()
        };
        let mut tags = Vec::new();
        for name in self.regex.capture_names().flatten() {
            if matches!(name, "title" | "tail") {
                continue;
            }
            let Some(value) = captures.name(name).map(|value| value.as_str().trim()) else {
                continue;
            };
            if value.is_empty() {
                continue;
            }
            // artist2 和 artist 是同一个命名空间，让正则能写好几个同类分组。
            let namespace = name.trim_end_matches(|c: char| c.is_ascii_digit());
            match namespace {
                "tag" => tags.extend(value.split(',').map(|value| tag("", value))),
                "artist" => tags.extend(artist_tags(value)),
                "event" => tags.push(tag("event", value)),
                _ => tags.extend(classified(value, namespace)),
            }
        }
        for value in &extra {
            tags.extend(classified(value, EXTRA_NAMESPACE));
        }
        if let Some(trailing) = &trailing {
            tags.extend(classified(trailing, ""));
        }
        tags.retain(|tag| !tag.value.is_empty());
        if !self.options.keep_all_captures {
            tags.retain(|tag| tag.namespace != EXTRA_NAMESPACE);
        }
        tags.sort_by(|a, b| (&a.namespace, &a.value).cmp(&(&b.namespace, &b.value)));
        let title = captures
            .name("title")
            .map(|title| title.as_str().trim().to_owned())
            .filter(|title| !title.is_empty());
        Some(Metadata {
            title,
            summary: None,
            tags,
            source_url: None,
        })
    }
}

fn tag(namespace: &str, value: &str) -> Tag {
    Tag {
        namespace: namespace.into(),
        value: value.trim().into(),
    }
}

/// `社团 (作者)` 拆成 `group:社团` 和 `artist:作者`。
fn artist_tags(value: &str) -> Vec<Tag> {
    let mut tags = Vec::new();
    let mut artist = value;
    if let Some(open) = value.rfind(" (")
        && let Some(inner) = value[open + 2..].strip_suffix(')')
    {
        tags.push(tag("group", &value[..open]));
        artist = inner;
    }
    tags.extend(classified(artist, "artist"));
    tags
}

/// 逗号分开的几个值放进同一个命名空间。纯数字和常见的无关词改放 `parsed:`。
fn classified(value: &str, namespace: &str) -> Vec<Tag> {
    value
        .split(',')
        .map(str::trim)
        .map(|item| {
            let extraneous = !namespace.is_empty()
                && COMMON_EXTRANEOUS_VALUES.contains(&item.to_lowercase().as_str());
            if extraneous || looks_like_number(item) {
                tag(EXTRA_NAMESPACE, item)
            } else {
                tag(namespace, item)
            }
        })
        .collect()
}

/// 近似 Perl 的 `looks_like_number`：整数、小数和科学计数法。
fn looks_like_number(item: &str) -> bool {
    !item.is_empty() && item.parse::<f64>().is_ok_and(f64::is_finite)
}

/// 末尾的 `{...}` 和它前面的部分。
fn trailing_braces(tail: &str) -> Option<(&str, &str)> {
    let inner = tail.strip_suffix('}')?;
    let open = inner.rfind('{')?;
    let tags = &inner[open + 1..];
    (!tags.contains('}') && !tags.is_empty()).then(|| (&tail[..open], tags))
}

/// 圆括号、花括号和方括号里的内容，按出现顺序。
fn bracketed(tail: &str) -> Vec<String> {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN
        .get_or_init(|| Regex::new(r"\(([^)]+)\)|\{([^}]+)\}|\[([^\]]+)\]").expect("固定的正则"));
    pattern
        .captures_iter(tail)
        .filter_map(|captures| {
            captures
                .iter()
                .skip(1)
                .flatten()
                .next()
                .map(|value| value.as_str().trim().to_owned())
        })
        .filter(|value| !value.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(stem: &str, options: FilenameOptions) -> Metadata {
        FilenameParser::new(&options)
            .unwrap()
            .parse(stem)
            .expect("应当匹配")
    }

    fn names(metadata: &Metadata) -> Vec<String> {
        metadata
            .tags
            .iter()
            .map(|tag| {
                if tag.namespace.is_empty() {
                    tag.value.clone()
                } else {
                    format!("{}:{}", tag.namespace, tag.value)
                }
            })
            .collect()
    }

    #[test]
    fn the_default_regex_reads_the_doujinshi_naming_convention() {
        let metadata = parse(
            "(C99) [Circle (Some Artist)] Great Title (Original Series) [Chinese] [Digital]",
            FilenameOptions::default(),
        );
        assert_eq!(metadata.title.as_deref(), Some("Great Title"));
        assert_eq!(
            names(&metadata),
            [
                "artist:Some Artist",
                "event:C99",
                "group:Circle",
                "language:Chinese",
                "series:Original Series"
            ]
        );
    }

    #[test]
    fn a_plain_filename_only_sets_the_title() {
        let metadata = parse("just_a_title", FilenameOptions::default());
        assert_eq!(metadata.title.as_deref(), Some("just a title"));
        assert!(metadata.tags.is_empty());
    }

    #[test]
    fn extra_captures_and_trailing_tags_follow_the_options() {
        let stem = "[Artist] Title [Chinese] (Uncensored) [2023] {full color, 3}";
        let off = parse(stem, FilenameOptions::default());
        assert_eq!(names(&off), ["artist:Artist", "language:Chinese"]);

        let on = parse(
            stem,
            FilenameOptions {
                trailing_tags: true,
                keep_all_captures: true,
                regex: String::new(),
            },
        );
        assert_eq!(
            names(&on),
            [
                "full color",
                "artist:Artist",
                "language:Chinese",
                "parsed:2023",
                "parsed:3",
                "parsed:Uncensored"
            ]
        );
    }

    #[test]
    fn trailing_tags_are_found_without_a_language_or_series() {
        let metadata = parse(
            "[Artist] Book {full color, ongoing}",
            FilenameOptions {
                trailing_tags: true,
                ..FilenameOptions::default()
            },
        );
        assert_eq!(metadata.title.as_deref(), Some("Book"));
        assert_eq!(names(&metadata), ["full color", "ongoing", "artist:Artist"]);
    }

    #[test]
    fn named_groups_in_a_custom_regex_become_namespaces() {
        let metadata = parse(
            "Artist One - Artist Two - Title - tag a, tag b",
            FilenameOptions {
                regex: r"(?<artist>[^-]+) - (?<artist2>[^-]+) - (?<title>[^-]+) - (?<tag>.+)"
                    .into(),
                ..FilenameOptions::default()
            },
        );
        assert_eq!(metadata.title.as_deref(), Some("Title"));
        assert_eq!(
            names(&metadata),
            ["tag a", "tag b", "artist:Artist One", "artist:Artist Two"]
        );
    }

    #[test]
    fn a_numeric_or_common_word_in_a_namespace_moves_to_parsed() {
        let metadata = parse("(2023) [pixiv] Title", FilenameOptions::default());
        // event 不经过分类，原样保留。
        assert_eq!(names(&metadata), ["event:2023"]);
    }

    #[test]
    fn a_filename_the_regex_does_not_match_changes_nothing() {
        let parser = FilenameParser::new(&FilenameOptions {
            regex: r"^\[(?<artist>[^\]]+)\] (?<title>.+)$".into(),
            ..FilenameOptions::default()
        })
        .unwrap();
        assert_eq!(parser.parse("no brackets here"), None);
    }

    #[test]
    fn an_invalid_regex_is_rejected() {
        assert!(
            FilenameParser::new(&FilenameOptions {
                regex: "(unclosed".into(),
                ..FilenameOptions::default()
            })
            .is_err()
        );
    }
}
