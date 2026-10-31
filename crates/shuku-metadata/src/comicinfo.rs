//! 读归档里的 ComicInfo.xml，字段对应照 LRR 的 ComicInfo 插件（1.2）。

use crate::{Error, Metadata, Result, parse_tag_list};
use shuku_store::Tag;

/// 归档里要找的文件名，不分大小写，可以在子目录里。
pub const FILE_NAME: &str = "ComicInfo.xml";

/// 元素和标签命名空间的对应。值可以用逗号分开写好几个。
const NAMESPACED: &[(&str, &str)] = &[
    ("Writer", "group"),
    ("Penciller", "artist"),
    ("Series", "series"),
    ("Characters", "character"),
    ("Publisher", "publisher"),
];

pub fn parse(bytes: &[u8]) -> Result<Metadata> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::XmlEncoding)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    // 不接受 DTD：实体展开能让几 KiB 的文件膨胀成几 GiB。ComicInfo.xml 本来就不带 DOCTYPE。
    let document = roxmltree::Document::parse(text)?;
    let field = |name: &str| -> Option<String> {
        let node = document
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == name)?;
        let text: String = node
            .children()
            .filter_map(|child| child.text())
            .collect::<String>()
            .trim()
            .to_owned();
        (!text.is_empty()).then_some(text)
    };
    let mut tags = Vec::new();
    for (element, namespace) in NAMESPACED {
        if let Some(value) = field(element) {
            tags.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| Tag {
                        namespace: (*namespace).into(),
                        value: value.into(),
                    }),
            );
        }
    }
    if let Some(language) = field("LanguageISO") {
        tags.push(Tag {
            namespace: "language".into(),
            value: language,
        });
    }
    // Genre 和 Tags 里的值原样当作标签，写成 `命名空间:值` 的保留命名空间。
    for element in ["Genre", "Tags"] {
        if let Some(value) = field(element) {
            tags.extend(parse_tag_list(&value));
        }
    }
    // LRR 把 Web 存成 source: 标签，Shuku 的来源地址是归档表的一列。
    let source_url = field("Web").and_then(|web| {
        web.split(',')
            .map(str::trim)
            .find(|url| !url.is_empty())
            .map(str::to_owned)
    });
    Ok(Metadata {
        title: field("Title"),
        tags,
        source_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_map_to_namespaces_and_genres_become_plain_tags() {
        let xml = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>
<ComicInfo xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">
  <Title>The Title</Title>
  <Series>Some Series</Series>
  <Writer>Circle A, Circle B</Writer>
  <Penciller>Artist</Penciller>
  <Characters>Alice</Characters>
  <Publisher>Pub</Publisher>
  <Genre>Romance, female:glasses</Genre>
  <Tags>full color</Tags>
  <Web>https://example.org/g/1/abc/</Web>
  <LanguageISO>zh</LanguageISO>
  <Summary>ignored</Summary>
</ComicInfo>";
        let metadata = parse(xml.as_bytes()).unwrap();
        assert_eq!(metadata.title.as_deref(), Some("The Title"));
        assert_eq!(
            metadata.source_url.as_deref(),
            Some("https://example.org/g/1/abc/")
        );
        let names: Vec<_> = metadata
            .tags
            .iter()
            .map(|tag| format!("{}:{}", tag.namespace, tag.value))
            .collect();
        assert_eq!(
            names,
            [
                "group:Circle A",
                "group:Circle B",
                "artist:Artist",
                "series:Some Series",
                "character:Alice",
                "publisher:Pub",
                "language:zh",
                ":Romance",
                "female:glasses",
                ":full color"
            ]
        );
    }

    #[test]
    fn empty_and_missing_fields_add_nothing() {
        let metadata = parse(b"<ComicInfo><Title> </Title><Genre></Genre></ComicInfo>").unwrap();
        assert_eq!(metadata, Metadata::default());
    }

    #[test]
    fn a_broken_file_is_an_error() {
        assert!(parse(b"<ComicInfo><Title>").is_err());
        assert!(parse(&[0xff, 0xfe, 0x00]).is_err());
    }
}
