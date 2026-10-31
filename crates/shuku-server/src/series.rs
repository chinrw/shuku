//! 从路径认出按卷出版的作品。一部作品常见的放法有两种：
//!
//! - 作品一个文件夹，每卷只用卷号命名：`寄宿学校的朱丽叶/VOL_1/`、`作品/第01卷.zip`、`作品/01.zip`。
//!   系列名取文件夹名。
//! - 文件名里带作品名和卷号：`回复术士的重来人生 第1卷.zip`、`Berserk v01.cbz`。系列名取卷号前的部分，
//!   同一个文件夹里作品名相同的归成一组，所以按月份放的文件夹里混着几部作品也能分开。
//!
//! 卷号必须在名字末尾，同人志名字中间的 `(コミックグレープ Vol.67)` 不算；只有一卷的系列不当作系列
//! 显示（见 [`shuku_store::Series`]），单独一本 `… Vol.2` 的同人志不会显示成系列。

use regex::Regex;
use std::{path::Path, sync::LazyLock};

/// 只有卷号的名字：`VOL_1`、`Vol.01`、`v01`、`第1卷`、`第01巻`、`01`。
static BARE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:vol(?:ume)?[\s._-]*|v|第\s*)?0*(\d{1,4})\s*[卷巻册冊]?$").unwrap()
});

/// 作品名加卷号：`作品 Vol.2`、`作品 v01`、`作品 第1卷`。单独的 `v` 要和作品名隔开、
/// 卷号至少两位，`[汉化组]v2` 这种表示第二版的写法不算。
static NAMED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(.*?\S)[\s._-]*(?:vol(?:ume)?[\s._-]*0*(\d{1,4})|[\s._-]v0*(\d{2,4})|第\s*0*(\d{1,4})\s*[卷巻册冊])$",
    )
    .unwrap()
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Volume {
    /// 归组依据，见 `series` 表的 `key` 列。
    pub(crate) key: String,
    pub(crate) series: String,
    pub(crate) volume: u32,
    /// 名字只有卷号时，换成“系列名 第 N 卷”作标题，搜作品名也能搜到各卷。
    pub(crate) title: Option<String>,
}

/// `relative` 是相对内容目录的路径，`name` 是新书的初始标题（去掉扩展名的文件名或文件夹名）。
/// 直接放在内容目录下的书不归组：没有文件夹名可以当系列名。
pub(crate) fn detect(relative: &str, name: &str) -> Option<Volume> {
    let parent = Path::new(relative).parent()?;
    let folder = parent.file_name()?.to_str()?;
    let parent = parent.to_str()?;
    let name = name.trim();
    if let Some(captures) = BARE.captures(name) {
        let volume = captures[1].parse().ok()?;
        return Some(Volume {
            key: parent.to_owned(),
            series: folder.to_owned(),
            volume,
            title: Some(format!("{folder} 第{volume}卷")),
        });
    }
    let captures = NAMED.captures(name)?;
    let series = captures[1].trim_end_matches([' ', '.', '_', '-']);
    let volume = captures
        .iter()
        .skip(2)
        .flatten()
        .next()?
        .as_str()
        .parse()
        .ok()?;
    Some(Volume {
        key: format!("{parent}/{series}"),
        series: series.to_owned(),
        volume,
        title: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{Volume, detect};

    fn bare(key: &str, series: &str, volume: u32) -> Option<Volume> {
        Some(Volume {
            key: key.into(),
            series: series.into(),
            volume,
            title: Some(format!("{series} 第{volume}卷")),
        })
    }

    fn named(key: &str, series: &str, volume: u32) -> Option<Volume> {
        Some(Volume {
            key: key.into(),
            series: series.into(),
            volume,
            title: None,
        })
    }

    #[test]
    fn names_that_are_only_a_volume_number_take_the_folder_as_the_series() {
        let series = "寄宿学校的朱丽叶";
        for (name, volume) in [
            ("VOL_1", 1),
            ("Vol.01", 1),
            ("vol 12", 12),
            ("v03", 3),
            ("第1卷", 1),
            ("第 02 巻", 2),
            ("010", 10),
        ] {
            assert_eq!(
                detect(&format!("漫画/{series}/{name}"), name),
                bare(&format!("漫画/{series}"), series, volume),
                "{name}"
            );
        }
    }

    #[test]
    fn names_with_a_title_group_by_title_within_the_folder() {
        assert_eq!(
            detect(
                "散本/08/回复术士的重来人生 第3卷.zip",
                "回复术士的重来人生 第3卷"
            ),
            named("散本/08/回复术士的重来人生", "回复术士的重来人生", 3)
        );
        assert_eq!(
            detect(
                "散本/07/[作者] ギルティサークル 第08巻.zip",
                "[作者] ギルティサークル 第08巻"
            ),
            named(
                "散本/07/[作者] ギルティサークル",
                "[作者] ギルティサークル",
                8
            )
        );
        assert_eq!(
            detect("en/Berserk v01.cbz", "Berserk v01"),
            named("en/Berserk", "Berserk", 1)
        );
        assert_eq!(
            detect(
                "同人/[作者] Some Story Vol.2.zip",
                "[作者] Some Story Vol.2"
            ),
            named("同人/[作者] Some Story", "[作者] Some Story", 2)
        );
    }

    #[test]
    fn volume_numbers_inside_the_name_or_revision_marks_are_not_volumes() {
        for name in [
            "[日月ネコ] 種付けザ·ワールド (コミックグレープ Vol.67) [無修正]",
            "[作者] 作品 (原作) [中国翻訳]v2",
            "[作者] 作品 v2",
            "作品 2",
        ] {
            assert_eq!(detect(&format!("本子/{name}.zip"), name), None, "{name}");
        }
    }

    #[test]
    fn books_directly_in_the_content_directory_are_not_grouped() {
        assert_eq!(detect("01.zip", "01"), None);
        assert_eq!(detect("作品 第1卷.zip", "作品 第1卷"), None);
    }
}
