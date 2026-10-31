//! 读归档里的 ComicInfo.xml，字段对应照 LRR 的 ComicInfo 插件（1.2）。

use crate::{Error, Metadata, Result, Volume, parse_tag_list};
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
    // 漫画按章节分文件时 Number 是章号、Volume 是所在的卷，按卷分文件时常常只有其中一个。
    // 一本归档对应一个 Number，所以先用它。`10.5` 这样的番外取整数部分，同号按归档 ID 排。
    let number = |name: &str| {
        let value = field(name)?;
        value.parse::<u32>().ok().or_else(|| {
            value
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && (0.0..=f64::from(u32::MAX)).contains(n))
                .map(|n| n as u32)
        })
    };
    let volume = field("Series").and_then(|series| {
        Some(Volume {
            series,
            number: number("Number").or_else(|| number("Volume"))?,
        })
    });
    Ok(Metadata {
        title: field("Title"),
        summary: field("Summary"),
        tags,
        source_url,
        volume,
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
  <Summary>A summary.</Summary>
</ComicInfo>";
        let metadata = parse(xml.as_bytes()).unwrap();
        assert_eq!(metadata.title.as_deref(), Some("The Title"));
        assert_eq!(metadata.summary.as_deref(), Some("A summary."));
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
    fn series_and_number_give_the_volume() {
        let volume = |xml: &str| parse(xml.as_bytes()).unwrap().volume;
        assert_eq!(
            volume(
                "<ComicInfo><Series>作品</Series><Number>3</Number><Volume>1</Volume></ComicInfo>"
            ),
            Some(Volume {
                series: "作品".into(),
                number: 3
            })
        );
        assert_eq!(
            volume("<ComicInfo><Series>作品</Series><Volume>2</Volume></ComicInfo>")
                .map(|volume| volume.number),
            Some(2)
        );
        assert_eq!(
            volume("<ComicInfo><Series>作品</Series><Number>10.5</Number></ComicInfo>")
                .map(|volume| volume.number),
            Some(10)
        );
        // 不是数、超出范围的序号不算，改用 Volume。
        for number in ["inf", "NaN", "1e999", "4294967296", "-1"] {
            let xml = format!(
                "<ComicInfo><Series>作品</Series><Number>{number}</Number><Volume>2</Volume></ComicInfo>"
            );
            assert_eq!(
                volume(&xml).map(|volume| volume.number),
                Some(2),
                "{number}"
            );
        }
        assert_eq!(volume("<ComicInfo><Number>3</Number></ComicInfo>"), None);
        assert_eq!(
            volume("<ComicInfo><Series>作品</Series><Number>番外</Number></ComicInfo>"),
            None
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
