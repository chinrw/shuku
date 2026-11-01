//! 从路径认出按卷出版的作品。一部作品常见的放法有两种：
//!
//! - 作品一个文件夹，每卷只用卷号命名：`寄宿学校的朱丽叶/VOL_1/`、`作品/第01卷.zip`、`作品/01.zip`。
//!   系列名取文件夹名。按话分文件夹的也一样：`结界女王/032/`、`影之实力者/第10.2話/`，几话合在一起的
//!   `001-005/` 按头一个数排。`番外1` 这类附加篇排在正篇后面。
//! - 文件名里带作品名和卷号：`回复术士的重来人生 第1卷.zip`、`Berserk v01.cbz`。系列名取卷号前的部分，
//!   同一个文件夹里作品名相同的归成一组，所以按月份放的文件夹里混着几部作品也能分开。
//!
//! 卷号必须在名字末尾，同人志名字中间的 `(コミックグレープ Vol.67)` 不算；只有一卷的系列不当作系列
//! 显示（见 [`shuku_store::Series`]），单独一本 `… Vol.2` 的同人志不会显示成系列。
//!
//! 扫描设置打开 `series_from_top_dir` 时，内容目录下每个第一级文件夹就是一部作品，里面的书不管叫什么
//! 都归进去，认得出的名字仍按上面的规则排序和改标题。
//!
//! 开了 ComicInfo 功能时，新书的 ComicInfo.xml 写明了系列和序号就以它为准，不看路径。

use crate::{App, Result};
use regex::Regex;
use std::{collections::HashMap, path::Path, sync::LazyLock};

/// 一个事务里最多改这么多本，已入库的书第一次认卷时不会长时间占着写连接。
const ASSIGN_BATCH: usize = 512;

/// 番外等附加篇的序号加上这个数，排在正篇后面。正篇的卷号和话数最多四位，不会超过它。
const EXTRA_OFFSET: f64 = 10_000.0;
/// 第一级文件夹即作品时，名字里认不出序号的书排在正篇和附加篇后面，彼此按入库顺序。
const UNNUMBERED: f64 = 2.0 * EXTRA_OFFSET;

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

/// 写明是话的名字：`第01話`、`第10.2话`、`第3回`、`12話`、`Ch.5`、`Chapter 5.5`。`第N話` 后面可以
/// 跟任何字（`第26話_試看`、`第5話決戦`）；别的写法容易和普通书名混淆，要隔着空格、下划线或括号，
/// 例如 `12話 (试看)`。
static CHAPTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:第\s*0*(\d{1,4}(?:\.\d{1,3})?)\s*[话話回].*|(?:0*(\d{1,4}(?:\.\d{1,3})?)\s*[话話]|ch(?:apter)?[\s._-]*0*(\d{1,4}(?:\.\d{1,3})?))(?:[\s._\-(（\[【].*)?)$",
    )
    .unwrap()
});

/// 附加篇：`番外`、`番外1`、`番外篇 2`、`特典`、`附录`、`外传`、`特别篇`、`Extra 1`、`Omake`。
/// 没有序号的算 0，后面可以像话一样跟别的字。
static EXTRA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:番外[篇编編]?|特典|附[录錄]|外[传傳]|特[别別][篇编編]|extra|omake)[\s._-]*(?:0*(\d{1,4}(?:\.\d{1,3})?))?(?:[\s._\-(（\[【].*)?$",
    )
    .unwrap()
});

/// 作品名加卷号：`作品 Vol.2`、`作品 v01`、`作品 第1卷`。单独的 `v` 要和作品名隔开、
/// 卷号至少两位，`[汉化组]v2` 这种表示第二版的写法不算。
static NAMED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(.*?\S)[\s._-]*(?:vol(?:ume)?[\s._-]*0*(\d{1,4})|[\s._-]v0*(\d{2,4})|第\s*0*(\d{1,4})\s*[卷巻册冊])$",
    )
    .unwrap()
});

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Volume {
    /// 归组依据，见 `series` 表的 `key` 列。
    pub(crate) key: String,
    pub(crate) series: String,
    /// 系列里的排序位置：正篇是卷号或话数（可以有小数），附加篇是 [`EXTRA_OFFSET`] 加它的序号。
    pub(crate) volume: f64,
    /// 名字只有卷号时在前面加上系列名作标题，搜作品名也能搜到各卷：写明是卷的换成
    /// “系列名 第N卷”，别的（数字、话、番外）写成“系列名 原名”，例如“结界女王 032”。
    pub(crate) title: Option<String>,
    /// 按上一层文件夹认出的番外，要等这一组里已经有正篇才归进去，见 [`Volume::joinable`]。
    pub(crate) needs_main: bool,
}

impl Volume {
    /// 能不能归进系列。只有番外的文件夹不当作系列：同人志书库里作者文件夹下的 `番外.zip`、
    /// `外传.zip` 不是哪部作品的附加篇。正篇和番外在同一次扫描里入库时番外可能先到，扫描结束前
    /// 的 [`App::assign_series`] 会再认一遍。
    pub(crate) fn joinable(&self, writer: &shuku_store::Writer) -> Result<bool> {
        Ok(!self.needs_main || writer.series_has_volume_below(&self.key, EXTRA_OFFSET)?)
    }
}

/// 名字里认出的序号。
enum Number<'a> {
    /// 写明是卷，标题写成“系列名 第N卷”。
    Volume(u32),
    /// 数字、话或附加篇，值是排序位置，标题写成“系列名 原名”。
    Bare(f64),
    /// 作品名加卷号，标题不变。
    Named(&'a str, u32),
}

fn first_number<T: std::str::FromStr>(captures: &regex::Captures, skip: usize) -> Option<T> {
    captures
        .iter()
        .skip(skip)
        .flatten()
        .next()?
        .as_str()
        .parse()
        .ok()
}

fn number(name: &str) -> Option<Number<'_>> {
    if let Some(captures) = BARE_VOLUME.captures(name) {
        return Some(Number::Volume(first_number(&captures, 1)?));
    }
    if let Some(captures) = BARE_NUMBER.captures(name) {
        return Some(Number::Bare(captures[1].parse().ok()?));
    }
    if let Some(captures) = CHAPTER.captures(name) {
        return Some(Number::Bare(first_number(&captures, 1)?));
    }
    if let Some(captures) = EXTRA.captures(name) {
        let number = first_number(&captures, 1).unwrap_or(0.0);
        return Some(Number::Bare(EXTRA_OFFSET + number));
    }
    let captures = NAMED.captures(name)?;
    let series = captures.get(1)?.as_str();
    let volume = first_number(&captures, 2)?;
    Some(Number::Named(
        series.trim_end_matches([' ', '.', '_', '-']),
        volume,
    ))
}

/// ComicInfo 的系列按名字归组，不管放在哪个文件夹；名字不分大小写。路径归组的依据是相对路径，
/// 不会以 `/` 开头，所以这里用 `/comicinfo/` 开头，两种依据不会撞上。
pub(crate) fn from_comicinfo(volume: shuku_metadata::Volume) -> Volume {
    Volume {
        key: format!("/comicinfo/{}", volume.series.to_lowercase()),
        series: volume.series,
        volume: volume.number.into(),
        title: None,
        needs_main: false,
    }
}

impl App {
    /// 给还不属于系列的已入库书按路径认卷，返回认出了几本。扫描结束前调用：认卷是在系列功能
    /// 之前入库的书，以及新加的卷让同一文件夹里原来的书凑成了系列。`walked` 是这次扫描走到的
    /// 条目（相对路径和是不是文件夹），不在里面的记录不动。标题只在还是初始标题、也不是从
    /// LRR 导入的时候才加上系列名。`by_top_dir` 见 [`detect`]。
    pub(crate) fn assign_series(
        &self,
        walked: &HashMap<String, bool>,
        by_top_dir: bool,
    ) -> Result<usize> {
        let mut found: Vec<(i64, Volume, String)> = self
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
                Some((id, detect(&path, title, by_top_dir)?, title.to_owned()))
            })
            .collect();
        // 番外要等同一组里有了正篇才归进去，先写正篇。
        found.sort_by_key(|(_, volume, _)| volume.needs_main);
        let mut assigned = 0;
        for batch in found.chunks(ASSIGN_BATCH) {
            assigned += self.write(|writer| {
                let mut assigned = 0;
                for (id, volume, file_title) in batch {
                    // 认卷和这次写入之间记录可能被删了。
                    let Some(archive) = writer.archive(*id)? else {
                        continue;
                    };
                    if !volume.joinable(writer)? {
                        continue;
                    }
                    assigned += 1;
                    writer.set_series(*id, &volume.key, &volume.series, volume.volume)?;
                    if let Some(title) = &volume.title
                        && archive.metadata.lrr_id.is_none()
                        && archive.metadata.title == *file_title
                    {
                        writer.edit_archive(*id, Some(title), None)?;
                    }
                }
                Ok(assigned)
            })?;
            self.listing_changed();
        }
        Ok(assigned)
    }
}

/// `relative` 是相对内容目录的路径，`name` 是新书的初始标题（去掉扩展名的文件名或文件夹名）。
/// 直接放在内容目录下的书不归组：没有文件夹名可以当系列名。`by_top_dir` 是扫描设置
/// `series_from_top_dir`：打开时按第一级文件夹归组，归组依据就是那个文件夹的路径，作品文件夹
/// 下直接放卷的系列和关着时归组依据相同，打开前已有的系列不变。
pub(crate) fn detect(relative: &str, name: &str, by_top_dir: bool) -> Option<Volume> {
    let path = Path::new(relative);
    let parent = path.parent()?;
    let folder = parent.file_name()?.to_str()?;
    let (key, series) = if by_top_dir {
        let top = path.iter().next()?.to_str()?;
        (top, top)
    } else {
        (parent.to_str()?, folder)
    };
    let name = name.trim();
    let volume = |volume: f64, title: Option<String>| Volume {
        key: key.to_owned(),
        series: series.to_owned(),
        volume,
        title,
        needs_main: !by_top_dir && volume >= EXTRA_OFFSET,
    };
    match number(name) {
        Some(Number::Volume(number)) => Some(volume(
            number.into(),
            Some(format!("{series} 第{number}卷")),
        )),
        Some(Number::Bare(position)) => Some(volume(position, Some(format!("{series} {name}")))),
        Some(Number::Named(_, number)) if by_top_dir => Some(volume(number.into(), None)),
        Some(Number::Named(named, number)) => Some(Volume {
            key: format!("{key}/{named}"),
            series: named.to_owned(),
            volume: number.into(),
            title: None,
            needs_main: false,
        }),
        // 名字里已经有作品名的不再重复加。
        None if by_top_dir => Some(volume(
            UNNUMBERED,
            (!name.contains(series)).then(|| format!("{series} {name}")),
        )),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{EXTRA_OFFSET, UNNUMBERED, Volume, detect};

    fn bare(key: &str, series: &str, volume: u32) -> Option<Volume> {
        Some(Volume {
            key: key.into(),
            series: series.into(),
            volume: volume.into(),
            title: Some(format!("{series} 第{volume}卷")),
            needs_main: false,
        })
    }

    fn named(key: &str, series: &str, volume: u32) -> Option<Volume> {
        Some(Volume {
            key: key.into(),
            series: series.into(),
            volume: volume.into(),
            title: None,
            needs_main: false,
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
                detect(&format!("漫画/{series}/{name}"), name, false),
                bare(&format!("漫画/{series}"), series, volume),
                "{name}"
            );
        }
    }

    #[test]
    fn bare_numbers_and_ranges_keep_the_name_after_the_series() {
        for (name, volume) in [("032", 32u32), ("1", 1), ("001-005", 1), ("006~011", 6)] {
            assert_eq!(
                detect(&format!("漫画/结界女王/{name}"), name, false),
                Some(Volume {
                    key: "漫画/结界女王".into(),
                    series: "结界女王".into(),
                    volume: volume.into(),
                    title: Some(format!("结界女王 {name}")),
                    needs_main: false,
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
                "回复术士的重来人生 第3卷",
                false
            ),
            named("散本/08/回复术士的重来人生", "回复术士的重来人生", 3)
        );
        assert_eq!(
            detect(
                "散本/07/[作者] ギルティサークル 第08巻.zip",
                "[作者] ギルティサークル 第08巻",
                false
            ),
            named(
                "散本/07/[作者] ギルティサークル",
                "[作者] ギルティサークル",
                8
            )
        );
        assert_eq!(
            detect("en/Berserk v01.cbz", "Berserk v01", false),
            named("en/Berserk", "Berserk", 1)
        );
        assert_eq!(
            detect(
                "同人/[作者] Some Story Vol.2.zip",
                "[作者] Some Story Vol.2",
                false
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
            assert_eq!(
                detect(&format!("本子/{name}.zip"), name, false),
                None,
                "{name}"
            );
        }
    }

    #[test]
    fn books_directly_in_the_content_directory_are_not_grouped() {
        assert_eq!(detect("01.zip", "01", false), None);
        assert_eq!(detect("作品 第1卷.zip", "作品 第1卷", false), None);
    }

    fn bare_named(key: &str, series: &str, name: &str, volume: f64) -> Option<Volume> {
        Some(Volume {
            key: key.into(),
            series: series.into(),
            volume,
            title: Some(format!("{series} {name}")),
            needs_main: false,
        })
    }

    #[test]
    fn chapter_names_keep_decimals_and_ignore_trailing_notes() {
        let series = "想要成为影之实力者";
        for (name, volume) in [
            ("第01話", 1.0),
            ("第10話", 10.0),
            ("第10.2話", 10.2),
            ("第60.1话", 60.1),
            ("第3回", 3.0),
            ("12話", 12.0),
            ("第26話_試看", 26.0),
            ("第27话_试看", 27.0),
            ("第28話 (试看)", 28.0),
            ("第5話 决战", 5.0),
            ("第6話決戦", 6.0),
            ("12話_试看", 12.0),
            ("Ch.5", 5.0),
            ("chapter 7.5", 7.5),
        ] {
            assert_eq!(
                detect(&format!("{series}/{name}"), name, false),
                bare_named(series, series, name, volume),
                "{name}"
            );
        }
        for name in ["12話目", "Chainsaw 3", "Chapter"] {
            assert_eq!(
                detect(&format!("{series}/{name}"), name, false),
                None,
                "{name}"
            );
        }
    }

    #[test]
    fn extras_sort_after_the_main_sequence_by_their_own_number() {
        let series = "学园默示录";
        for (name, number) in [
            ("番外", 0.0),
            ("番外1", 1.0),
            ("番外篇 2", 2.0),
            ("特典", 0.0),
            ("附录3", 3.0),
            ("外传_试看", 0.0),
            ("Extra 4", 4.0),
        ] {
            let mut expected = bare_named(series, series, name, EXTRA_OFFSET + number).unwrap();
            expected.needs_main = true;
            assert_eq!(
                detect(&format!("{series}/{name}"), name, false),
                Some(expected.clone()),
                "{name}"
            );
            // 第一级目录为一部作品时不必等正篇。
            expected.needs_main = false;
            assert_eq!(
                detect(&format!("{series}/{name}"), name, true),
                Some(expected),
                "{name}"
            );
        }
        assert_eq!(
            detect("本子/Extraordinary 3.zip", "Extraordinary 3", false),
            None
        );
    }

    #[test]
    fn top_folder_mode_groups_every_book_under_the_first_level_folder() {
        // 第一级文件夹下直接放的卷，归组依据和关着时一样。
        for name in ["VOL_1", "032", "第10.2話"] {
            let path = format!("学园默示录/{name}");
            assert_eq!(
                detect(&path, name, true),
                detect(&path, name, false),
                "{name}"
            );
        }
        // 认不出序号的也归进去，排在最后；名字里已有作品名时标题不变。
        assert_eq!(
            detect("夫妇以上/EPUB", "EPUB", true),
            bare_named("夫妇以上", "夫妇以上", "EPUB", UNNUMBERED)
        );
        assert_eq!(
            detect("夫妇以上/夫妇以上 特装版.zip", "夫妇以上 特装版", true),
            Some(Volume {
                key: "夫妇以上".into(),
                series: "夫妇以上".into(),
                volume: UNNUMBERED,
                title: None,
                needs_main: false,
            })
        );
        // 作品名加卷号的按第一级文件夹归组，不再按作品名分开。
        assert_eq!(
            detect("作品/[作者] 作品 第3卷.zip", "[作者] 作品 第3卷", true),
            named("作品", "作品", 3)
        );
        // 更深的层次也归到第一级文件夹。
        assert_eq!(
            detect("作品/第1卷/第01話", "第01話", true),
            bare_named("作品", "作品", "第01話", 1.0)
        );
        // 第一级文件夹本身是一本书、或者书直接放在内容目录下时不归组。
        assert_eq!(detect("单本", "单本", true), None);
        assert_eq!(detect("单本.zip", "单本", true), None);
    }
}
