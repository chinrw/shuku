//! 从路径认出按卷出版的作品。一部作品常见的放法有两种：
//!
//! - 作品一个文件夹，每卷只用卷号命名：`寄宿学校的朱丽叶/VOL_1/`、`作品/第01卷.zip`、`作品/01.zip`。
//!   系列名取文件夹名。按话分文件夹的也一样：`结界女王/032/`，几话合在一起的 `001-005/` 按头一个
//!   数排。
//! - 文件名里带作品名和卷号：`回复术士的重来人生 第1卷.zip`、`Berserk v01.cbz`。系列名取卷号前的部分，
//!   同一个文件夹里作品名相同的归成一组，所以按月份放的文件夹里混着几部作品也能分开。
//!
//! 卷号必须在名字末尾，同人志名字中间的 `(コミックグレープ Vol.67)` 不算；只有一卷的系列不当作系列
//! 显示（见 [`shuku_store::Series`]），单独一本 `… Vol.2` 的同人志不会显示成系列。
//!
//! 开了 ComicInfo 功能时，新书的 ComicInfo.xml 写明了系列和序号就以它为准，不看路径。

use crate::{App, Result};
use regex::Regex;
use std::{collections::HashMap, path::Path, sync::LazyLock};

/// 一个事务里最多改这么多本，已入库的书第一次认卷时不会长时间占着写连接。
const ASSIGN_BATCH: usize = 512;

/// 写明是卷的名字：`VOL_1`、`Vol.01`、`v01`、`第1卷`、`第01巻`、`1卷`。
static BARE_VOLUME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:(?:vol(?:ume)?[\s._-]*|v|第\s*)0*(\d{1,4})\s*[卷巻册冊]?|0*(\d{1,4})\s*[卷巻册冊])$",
    )
    .unwrap()
});

/// 只有数字的名字，可能是卷也可能是话：`01`、`032`、`001-005`。
static BARE_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^0*(\d{1,4})(?:\s*[-~～]\s*\d{1,4})?$").unwrap());

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
    /// 名字只有卷号时在前面加上系列名作标题，搜作品名也能搜到各卷：写明是卷的换成
    /// “系列名 第N卷”，只有数字的分不清是卷还是话，写成“系列名 032”。
    pub(crate) title: Option<String>,
}

/// ComicInfo 的系列按名字归组，不管放在哪个文件夹；名字不分大小写。路径归组的依据是相对路径，
/// 不会以 `/` 开头，所以这里用 `/comicinfo/` 开头，两种依据不会撞上。
pub(crate) fn from_comicinfo(volume: shuku_metadata::Volume) -> Volume {
    Volume {
        key: format!("/comicinfo/{}", volume.series.to_lowercase()),
        series: volume.series,
        volume: volume.number,
        title: None,
    }
}

impl App {
    /// 给还不属于系列的已入库书按路径认卷，返回认出了几本。扫描结束前调用：认卷是在系列功能
    /// 之前入库的书，以及新加的卷让同一文件夹里原来的书凑成了系列。`walked` 是这次扫描走到的
    /// 条目（相对路径和是不是文件夹），不在里面的记录不动。标题只在还是初始标题、也不是从
    /// LRR 导入的时候才加上系列名。
    pub(crate) fn assign_series(&self, walked: &HashMap<String, bool>) -> Result<usize> {
        let found: Vec<(i64, Volume, String)> = self
            .reader()?
            .archives_without_series()?
            .into_iter()
            .filter_map(|(id, path)| {
                let path_ref = Path::new(&path);
                let title = if *walked.get(&path)? {
                    path_ref.file_name()
                } else {
                    path_ref.file_stem()
                }?
                .to_str()?;
                Some((id, detect(&path, title)?, title.to_owned()))
            })
            .collect();
        for batch in found.chunks(ASSIGN_BATCH) {
            self.write(|writer| {
                for (id, volume, file_title) in batch {
                    // 认卷和这次写入之间记录可能被删了。
                    let Some(archive) = writer.archive(*id)? else {
                        continue;
                    };
                    writer.set_series(*id, &volume.key, &volume.series, volume.volume)?;
                    if let Some(title) = &volume.title
                        && archive.metadata.lrr_id.is_none()
                        && archive.metadata.title == *file_title
                    {
                        writer.edit_archive(*id, Some(title), None)?;
                    }
                }
                Ok(())
            })?;
            self.listing_changed();
        }
        Ok(found.len())
    }
}

/// `relative` 是相对内容目录的路径，`name` 是新书的初始标题（去掉扩展名的文件名或文件夹名）。
/// 直接放在内容目录下的书不归组：没有文件夹名可以当系列名。
pub(crate) fn detect(relative: &str, name: &str) -> Option<Volume> {
    let parent = Path::new(relative).parent()?;
    let folder = parent.file_name()?.to_str()?;
    let parent = parent.to_str()?;
    let name = name.trim();
    let bare = |volume: u32, title: String| Volume {
        key: parent.to_owned(),
        series: folder.to_owned(),
        volume,
        title: Some(title),
    };
    if let Some(captures) = BARE_VOLUME.captures(name) {
        let volume = captures
            .iter()
            .skip(1)
            .flatten()
            .next()?
            .as_str()
            .parse()
            .ok()?;
        return Some(bare(volume, format!("{folder} 第{volume}卷")));
    }
    if let Some(captures) = BARE_NUMBER.captures(name) {
        let volume = captures[1].parse().ok()?;
        return Some(bare(volume, format!("{folder} {name}")));
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
            ("3卷", 3),
        ] {
            assert_eq!(
                detect(&format!("漫画/{series}/{name}"), name),
                bare(&format!("漫画/{series}"), series, volume),
                "{name}"
            );
        }
    }

    #[test]
    fn bare_numbers_and_ranges_keep_the_name_after_the_series() {
        for (name, volume) in [("032", 32), ("1", 1), ("001-005", 1), ("006~011", 6)] {
            assert_eq!(
                detect(&format!("漫画/结界女王/{name}"), name),
                Some(Volume {
                    key: "漫画/结界女王".into(),
                    series: "结界女王".into(),
                    volume,
                    title: Some(format!("结界女王 {name}")),
                }),
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
