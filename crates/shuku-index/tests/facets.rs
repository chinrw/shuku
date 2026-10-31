use shuku_index::{ArchiveRecord, Index, Query, TagCount, Term};

fn record(id: i64, tags: &[&str]) -> ArchiveRecord {
    ArchiveRecord {
        id,
        title: format!("title {id}"),
        date_added: id,
        tags: tags.iter().map(|t| t.to_string()).collect(),
        ..Default::default()
    }
}

fn library() -> Index {
    Index::build(vec![
        record(1, &["language:japanese", "female:glasses", "artist:alpha"]),
        record(2, &["language:japanese", "artist:alpha"]),
        record(3, &["language:chinese", "female:glasses", "artist:beta"]),
        record(4, &["language:chinese", "female:ponytail", "artist:alpha"]),
        record(5, &["language:english"]),
    ])
    .unwrap()
}

fn counts(pairs: &[(&str, u64)]) -> Vec<TagCount> {
    pairs
        .iter()
        .map(|&(tag, count)| TagCount {
            tag: tag.to_string(),
            count,
        })
        .collect()
}

#[test]
fn facets_count_tags_within_the_result_most_common_first() {
    let index = library();
    let query = Query {
        include: vec![Term::Tag("artist:alpha".into())],
        ..Query::default()
    };

    let facets = index.facets(&query, 10);

    assert_eq!(
        facets,
        counts(&[
            ("artist:alpha", 3),
            ("language:japanese", 2),
            ("female:glasses", 1),
            ("female:ponytail", 1),
            ("language:chinese", 1),
        ])
    );
}

#[test]
fn facets_break_ties_by_tag_name_when_truncated() {
    let index = library();

    let facets = index.facets(&Query::default(), 3);

    // artist:alpha 有 3 本；有 3 个标签并列 2 本，按标签名取前两个。
    assert_eq!(
        facets,
        counts(&[
            ("artist:alpha", 3),
            ("female:glasses", 2),
            ("language:chinese", 2)
        ])
    );
}

#[test]
fn a_smaller_tag_wins_a_tie_by_name_against_a_larger_one() {
    let index = Index::build(vec![
        record(1, &["z:big", "a:small"]),
        record(2, &["z:big", "a:small"]),
        record(3, &["z:big", "x:out"]),
    ])
    .unwrap();
    let query = Query {
        exclude: vec![Term::Tag("x:out".into())],
        ..Query::default()
    };

    // z:big 总共的归档更多，但两个标签在结果里都是 2 本，按标签名排。
    assert_eq!(index.facets(&query, 1), counts(&[("a:small", 2)]));
    assert_eq!(
        index.facets(&query, 2),
        counts(&[("a:small", 2), ("z:big", 2)])
    );
}

#[test]
fn a_small_result_in_a_large_library_counts_each_archive() {
    // 结果只有 2 本，不到 40 本的 1/16，走逐本累加每本标签的分支。
    let mut records = vec![
        record(1, &["z:pick", "b:tie", "a:tie", "m:once"]),
        record(2, &["z:pick", "b:tie", "a:tie"]),
    ];
    records.extend((3..=40).map(|id| record(id, &["z:pick-not", "a:tie"])));
    let index = Index::build(records).unwrap();
    let query = Query {
        include: vec![Term::Tag("z:pick".into())],
        ..Query::default()
    };

    assert_eq!(
        index.facets(&query, 10),
        counts(&[("a:tie", 2), ("b:tie", 2), ("z:pick", 2), ("m:once", 1)])
    );
    assert_eq!(
        index.facets(&query, 2),
        counts(&[("a:tie", 2), ("b:tie", 2)])
    );
}

#[test]
fn a_limit_above_the_number_of_tags_returns_every_tag() {
    let index = library();

    let facets = index.facets(&Query::default(), usize::MAX);

    assert_eq!(
        facets,
        counts(&[
            ("artist:alpha", 3),
            ("female:glasses", 2),
            ("language:chinese", 2),
            ("language:japanese", 2),
            ("artist:beta", 1),
            ("female:ponytail", 1),
            ("language:english", 1),
        ])
    );
}

#[test]
fn facets_respect_exclusions() {
    let index = library();
    let query = Query {
        include: vec![Term::Tag("female:glasses".into())],
        exclude: vec![Term::Tag("language:chinese".into())],
        ..Query::default()
    };

    let facets = index.facets(&query, 10);

    assert_eq!(
        facets,
        counts(&[
            ("artist:alpha", 1),
            ("female:glasses", 1),
            ("language:japanese", 1),
        ])
    );
}

#[test]
fn facets_of_an_empty_result_or_a_zero_limit_are_empty() {
    let index = library();
    let disjoint = Query {
        include: vec![
            Term::Tag("language:japanese".into()),
            Term::Tag("language:chinese".into()),
        ],
        ..Query::default()
    };

    assert!(index.facets(&disjoint, 10).is_empty());
    assert!(index.facets(&Query::default(), 0).is_empty());
}

#[test]
fn suggestions_match_full_names_or_values_most_common_first() {
    let index = Index::build(vec![
        record(1, &["artist:alpha", "female:glasses", "Alps"]),
        record(2, &["artist:alpha", "group:alpine"]),
        record(3, &["artist:alpha", "group:alpine", "parody:beta"]),
        record(4, &["artist:albert"]),
    ])
    .unwrap();

    // 冒号后的值以 al 开头的标签和没有命名空间的 alps 都算，同数时按标签名排；
    // 值里只是包含 pha 的 alpha 不算。
    assert_eq!(
        index.suggest_tags("AL", 10),
        counts(&[
            ("artist:alpha", 3),
            ("group:alpine", 2),
            ("alps", 1),
            ("artist:albert", 1),
        ])
    );
    assert_eq!(
        index.suggest_tags("art", 10),
        counts(&[("artist:alpha", 3), ("artist:albert", 1)])
    );
    assert_eq!(
        index.suggest_tags("artist:alb", 10),
        counts(&[("artist:albert", 1)])
    );
    assert_eq!(index.suggest_tags("al", 1), counts(&[("artist:alpha", 3)]));
    assert!(index.suggest_tags("pha", 10).is_empty());
    assert!(index.suggest_tags("  ", 10).is_empty());
    assert!(index.suggest_tags("al", 0).is_empty());
}
