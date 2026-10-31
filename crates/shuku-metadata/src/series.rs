//! 按卷出版的漫画按系列查：一部作品的每一卷在 Bangumi 和 AniList 上是同一个条目，
//! 搜索词要去掉卷号。

use regex::Regex;
use shuku_store::Tag;
use std::sync::OnceLock;

/// 搜索用的系列名，依次用：`series:` 标签（ComicInfo 的 Series、文件名解析的原作）、
/// 扫描按文件夹和卷号认出的系列名 `detected`、去掉括号内容和卷号的标题。
/// 什么都不剩时返回 `None`。
pub fn series_query(title: &str, tags: &[Tag], detected: Option<&str>) -> Option<String> {
    if let Some(series) = tags
        .iter()
        .find(|tag| tag.namespace.eq_ignore_ascii_case("series"))
        .map(|tag| tag.value.trim())
        .into_iter()
        .chain(detected.map(str::trim))
        .find(|series| !series.is_empty())
    {
        return Some(series.to_owned());
    }
    static PATTERNS: OnceLock<[Regex; 3]> = OnceLock::new();
    let [brackets, volume, separators] = PATTERNS.get_or_init(|| {
        [
            // 各种括号里的通常是作者、汉化组、版本，不是作品名。书名号《》里的就是作品名，
            // 不在这里，下面只去掉书名号本身。
            Regex::new(r"[\[【（(〔][^\]】）)〕]*[\]】）)〕]").expect("固定的正则"),
            // Vol.3、v03、第3卷、第三巻、#3、末尾单独的数字。
            Regex::new(
                r"(?i)(\bvol(ume)?\.?\s*\d+(\.\d+)?\b|\bv\d+\b|第\s*[0-9０-９一二三四五六七八九十百]+\s*[卷巻册冊集话話部]|#\s*\d+|\s\d{1,3}$)",
            )
            .expect("固定的正则"),
            Regex::new(r"[\s\-_~～・·:：]+$|^[\s\-_~～・·:：]+").expect("固定的正则"),
        ]
    });
    let without_brackets = brackets.replace_all(title, " ").replace(['《', '》'], " ");
    let without_volume = volume.replace_all(&without_brackets, " ");
    let collapsed = without_volume
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let query = separators.replace_all(&collapsed, "").trim().to_owned();
    (!query.is_empty()).then_some(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(title: &str) -> Option<String> {
        series_query(title, &[], None)
    }

    #[test]
    fn volume_markers_and_brackets_are_removed() {
        assert_eq!(
            query("葬送的芙莉莲 第01卷").as_deref(),
            Some("葬送的芙莉莲")
        );
        assert_eq!(
            query("[山田鐘人] 葬送のフリーレン 第三巻").as_deref(),
            Some("葬送のフリーレン")
        );
        assert_eq!(query("Frieren Vol.3").as_deref(), Some("Frieren"));
        assert_eq!(query("Frieren v03 (Digital)").as_deref(), Some("Frieren"));
        assert_eq!(
            query("Dungeon Meshi - 12").as_deref(),
            Some("Dungeon Meshi")
        );
        assert_eq!(query("Bakuman #4").as_deref(), Some("Bakuman"));
        // 作品名本身带数字的不动。
        assert_eq!(
            query("20th Century Boys").as_deref(),
            Some("20th Century Boys")
        );
        assert_eq!(query("[汉化组]"), None);
        assert_eq!(
            query("《葬送的芙莉莲》第01卷").as_deref(),
            Some("葬送的芙莉莲")
        );
    }

    #[test]
    fn a_series_tag_wins_over_the_title() {
        let tags = [Tag {
            namespace: "Series".into(),
            value: " ダンジョン飯 ".into(),
        }];
        assert_eq!(
            series_query("Delicious in Dungeon 01", &tags, Some("迷宫饭")).as_deref(),
            Some("ダンジョン飯")
        );
        assert_eq!(
            series_query("Delicious in Dungeon 01", &[], Some(" 迷宫饭 ")).as_deref(),
            Some("迷宫饭")
        );
    }
}
