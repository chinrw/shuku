use shuku_index::{ArchiveRecord, Index};

fn record(id: i64, title: &str, tags: &[&str]) -> ArchiveRecord {
    ArchiveRecord {
        id,
        title: title.to_string(),
        date_added: id,
        tags: tags.iter().map(|t| t.to_string()).collect(),
        ..Default::default()
    }
}

fn volume(id: i64, title: &str, tags: &[&str], series: i64, volume: u32) -> ArchiveRecord {
    ArchiveRecord {
        series: Some((series, volume)),
        ..record(id, title, tags)
    }
}

#[test]
fn rarer_shared_tags_rank_higher() {
    let index = Index::build(vec![
        record(1, "a", &["artist:alpha", "female:glasses", "parody:x"]),
        // 只共有一个两本都有的作者标签，比共有两个更常见的标签分高。
        record(2, "b", &["artist:alpha"]),
        record(3, "c", &["female:glasses", "parody:x"]),
        record(4, "d", &["female:glasses", "parody:x", "other:y"]),
        record(5, "e", &["female:glasses"]),
        record(6, "f", &["parody:x"]),
        record(7, "g", &["female:glasses", "parody:x"]),
        record(8, "h", &["female:glasses", "parody:x"]),
        record(9, "i", &["unrelated"]),
    ])
    .unwrap();

    // 作者 ln(9/2)≈1.50；female:glasses 和 parody:x 各 ln(9/6)≈0.41。
    // 3、4、7、8 同分，按标题排；没有共同标签的 9 和这本自己不列。
    assert_eq!(index.similar(1, 10), [2, 3, 4, 7, 8, 5, 6]);
    assert_eq!(index.similar(1, 2), [2, 3]);
    assert_eq!(index.similar(1, 0), Vec::<i64>::new());
    assert_eq!(index.similar(42, 10), Vec::<i64>::new());
    assert_eq!(index.similar(9, 10), Vec::<i64>::new());
}

#[test]
fn tags_on_most_of_a_large_library_are_ignored() {
    let mut records: Vec<_> = (1..=1000)
        .map(|id| record(id, &format!("filler {id}"), &["language:japanese"]))
        .collect();
    records.push(record(
        1001,
        "target",
        &["language:japanese", "artist:alpha"],
    ));
    records.push(record(1002, "same artist", &["artist:alpha"]));
    let index = Index::build(records).unwrap();

    assert_eq!(index.similar(1001, 10), [1002]);
    assert_eq!(index.similar(1, 10), Vec::<i64>::new());
}

#[test]
fn small_libraries_keep_tags_that_cover_a_tenth_of_them() {
    // 20 本里 5 本带的标签超过 1/10，但不到 100 本，仍然算数。
    let mut records: Vec<_> = (1..=15)
        .map(|id| record(id, &format!("other {id}"), &[]))
        .collect();
    records.extend((16..=20).map(|id| record(id, &format!("tagged {id}"), &["artist:alpha"])));
    let index = Index::build(records).unwrap();

    assert_eq!(index.similar(16, 10), [17, 18, 19, 20]);
}

#[test]
fn series_volumes_are_left_out_or_collapsed_to_one() {
    let tags = &["artist:alpha"];
    let index = Index::build(vec![
        volume(1, "own 1", tags, 10, 1),
        volume(2, "own 2", tags, 10, 2),
        // 另一个系列同分的几卷只留卷号最小的。
        volume(3, "other 3", tags, 20, 3),
        volume(4, "other 2", tags, 20, 2),
        // 得分更高的一卷优先于卷号小的。
        volume(5, "third 1", tags, 30, 1),
        volume(6, "third 2", &["artist:alpha", "female:glasses"], 30, 2),
        record(7, "single", &["artist:alpha", "female:glasses"]),
        record(8, "filler", &[]),
        record(9, "filler", &[]),
        record(10, "filler", &[]),
    ])
    .unwrap();

    assert_eq!(index.similar(1, 10), [4, 7, 5]);
    assert_eq!(index.similar(7, 10), [6, 4, 1]);
}

#[test]
fn equal_volumes_of_another_series_pick_the_first_title() {
    // 结果不依赖构建顺序。
    for order in [[2, 3], [3, 2]] {
        let mut records = vec![record(1, "target", &["t"]), record(4, "untagged", &[])];
        for id in order {
            let title = if id == 2 { "Z" } else { "A" };
            records.push(volume(id, title, &["t"], 20, 1));
        }
        let index = Index::build(records).unwrap();
        assert_eq!(index.similar(1, 10), [3]);
    }
}
