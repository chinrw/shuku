use shuku_index::{ArchiveRecord, Index, Query, SearchError, Sort, Term};

fn record(id: i64, title: &str, date_added: i64, tags: &[&str]) -> ArchiveRecord {
    ArchiveRecord {
        id,
        title: title.to_string(),
        date_added,
        tags: tags.iter().map(|t| t.to_string()).collect(),
        ..Default::default()
    }
}

fn library() -> Index {
    Index::build(vec![
        record(
            1,
            "魔法少女 Vol.10",
            300,
            &["language:japanese", "female:glasses", "artist:alpha"],
        ),
        record(
            2,
            "魔法少女 Vol.2",
            100,
            &["language:japanese", "artist:alpha"],
        ),
        record(
            3,
            "Summer Diary",
            200,
            &["language:chinese", "female:glasses", "artist:beta"],
        ),
        record(
            4,
            "妹妹的秘密",
            400,
            &["language:chinese", "female:ponytail", "artist:alpha2"],
        ),
        record(5, "Rainy Day", 500, &["language:english"]),
    ])
    .unwrap()
}

fn query(include: Vec<Term>) -> Query {
    Query {
        include,
        ..Query::default()
    }
}

fn ids(index: &Index, q: &Query) -> Vec<i64> {
    index.search(q).unwrap().ids
}

#[test]
fn empty_query_returns_every_archive_in_natural_title_order() {
    let index = library();

    let result = index.search(&Query::default()).unwrap();

    // "Vol.2" sorts before "Vol.10" because digit runs compare numerically.
    assert_eq!(result.ids, vec![5, 3, 4, 2, 1]);
    assert_eq!(result.total, 5);
}

#[test]
fn exact_tag_matches_only_that_tag() {
    let index = library();

    let result = ids(&index, &query(vec![Term::Tag("artist:alpha".into())]));

    assert_eq!(result, vec![2, 1]);
}

#[test]
fn tag_and_title_matching_ignores_case() {
    let index = library();

    assert_eq!(
        ids(&index, &query(vec![Term::Tag("Language:English".into())])),
        vec![5]
    );
    assert_eq!(
        ids(&index, &query(vec![Term::TitleContains("SUMMER".into())])),
        vec![3]
    );
}

#[test]
fn multiple_include_terms_intersect() {
    let index = library();

    let result = ids(
        &index,
        &query(vec![
            Term::Tag("language:japanese".into()),
            Term::Tag("female:glasses".into()),
        ]),
    );

    assert_eq!(result, vec![1]);
}

#[test]
fn disjoint_tags_return_an_empty_result() {
    let index = library();

    let result = index
        .search(&query(vec![
            Term::Tag("language:japanese".into()),
            Term::Tag("language:chinese".into()),
        ]))
        .unwrap();

    assert!(result.ids.is_empty());
    assert_eq!(result.total, 0);
    assert_eq!(result.next_after, None);
}

#[test]
fn exclude_terms_remove_matches() {
    let index = library();
    let q = Query {
        include: vec![Term::Tag("female:glasses".into())],
        exclude: vec![Term::Tag("language:chinese".into())],
        ..Query::default()
    };

    assert_eq!(ids(&index, &q), vec![1]);
}

#[test]
fn unknown_tag_matches_nothing_when_included_and_nothing_when_excluded() {
    let index = library();
    let excluded = Query {
        exclude: vec![Term::Tag("parody:none".into())],
        ..Query::default()
    };

    assert!(ids(&index, &query(vec![Term::Tag("parody:none".into())])).is_empty());
    assert_eq!(index.search(&excluded).unwrap().total, 5);
}

#[test]
fn tag_prefix_matches_every_tag_starting_with_it() {
    let index = library();

    // "artist:alpha" is a prefix of both "artist:alpha" and "artist:alpha2".
    let result = ids(&index, &query(vec![Term::TagPrefix("artist:alpha".into())]));

    assert_eq!(result, vec![4, 2, 1]);
}

#[test]
fn tag_substring_matches_across_namespaces() {
    let index = library();

    let result = ids(&index, &query(vec![Term::TagContains("glass".into())]));

    assert_eq!(result, vec![3, 1]);
}

#[test]
fn title_substring_matches_two_cjk_characters() {
    let index = library();

    assert_eq!(
        ids(&index, &query(vec![Term::TitleContains("妹妹".into())])),
        vec![4]
    );
    assert_eq!(
        ids(&index, &query(vec![Term::TitleContains("少女".into())])),
        vec![2, 1]
    );
}

#[test]
fn date_sort_puts_newest_first() {
    let index = library();
    let q = Query {
        sort: Sort::DateAddedNewest,
        ..Query::default()
    };

    assert_eq!(ids(&index, &q), vec![5, 4, 1, 3, 2]);
}

#[test]
fn pages_continue_after_the_cursor_without_gaps_or_repeats() {
    let index = library();
    let first = index
        .search(&Query {
            limit: 2,
            ..Query::default()
        })
        .unwrap();

    let second = index
        .search(&Query {
            limit: 2,
            after: first.next_after,
            ..Query::default()
        })
        .unwrap();
    let third = index
        .search(&Query {
            limit: 2,
            after: second.next_after,
            ..Query::default()
        })
        .unwrap();

    assert_eq!(first.ids, vec![5, 3]);
    assert_eq!(second.ids, vec![4, 2]);
    assert_eq!(third.ids, vec![1]);
    assert_eq!(third.next_after, None);
    assert_eq!(third.total, 5);
}

#[test]
fn sparse_results_page_in_sort_order() {
    let index = library();
    let q = Query {
        include: vec![Term::Tag("artist:alpha".into())],
        sort: Sort::DateAddedNewest,
        limit: 1,
        ..Query::default()
    };

    let first = index.search(&q).unwrap();
    let second = index
        .search(&Query {
            after: first.next_after,
            ..q.clone()
        })
        .unwrap();

    assert_eq!(first.ids, vec![1]);
    assert_eq!(second.ids, vec![2]);
}

#[test]
fn cursor_for_an_unknown_archive_is_rejected() {
    let index = library();

    let err = index
        .search(&Query {
            after: Some(99),
            ..Query::default()
        })
        .unwrap_err();

    assert_eq!(err, SearchError::UnknownCursor(99));
}

#[test]
fn duplicate_archive_ids_are_rejected_at_build_time() {
    let result = Index::build(vec![record(1, "a", 0, &[]), record(1, "b", 0, &[])]);

    assert!(result.is_err());
}

#[test]
fn random_picks_only_matches_and_none_when_empty() {
    let index = Index::build(vec![
        record(1, "a", 0, &["artist:alpha"]),
        record(2, "b", 0, &["artist:beta"]),
        record(3, "c", 0, &["artist:alpha"]),
    ])
    .unwrap();
    let alpha = query(vec![Term::Tag("artist:alpha".into())]);
    let picked: std::collections::BTreeSet<_> = (0..20)
        .filter_map(|seed| index.random(&alpha, seed))
        .collect();
    assert_eq!(picked, [1, 3].into());
    assert_eq!(index.random(&query(vec![Term::Tag("x".into())]), 7), None);
}

#[test]
fn namespace_sort_uses_the_naturally_smallest_value_of_each_book() {
    let index = Index::build(vec![
        record(1, "a", 0, &["artist:unit2", "artist:unit10"]),
        record(2, "b", 0, &["artist:unit3"]),
        record(3, "c", 0, &[]),
    ])
    .unwrap();
    let result = index
        .search(&Query {
            sort: Sort::Namespace("artist".into()),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(result.ids, [1, 2, 3]);
}

#[test]
fn offset_pages_match_cursor_pages() {
    let index = library();
    let page = |offset| {
        index
            .search(&Query {
                limit: 2,
                offset,
                ..Query::default()
            })
            .unwrap()
    };
    assert_eq!(page(0).ids, vec![5, 3]);
    assert_eq!(page(2).ids, vec![4, 2]);
    assert_eq!(page(2).next_after, Some(2));
    assert_eq!(page(4).ids, vec![1]);
    assert_eq!(page(4).next_after, None);
    assert!(page(10).ids.is_empty());
    assert_eq!(page(10).total, 5);
}
