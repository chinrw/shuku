use shuku_index::{ArchiveRecord, Index, Query, Term};

#[test]
fn any_unions_children_and_composes_with_inclusion_and_exclusion() {
    let index = Index::build(
        [
            (1, "A summer", vec!["season:summer", "language:ja"]),
            (2, "B winter", vec!["season:summer", "language:en"]),
            (3, "C summer", vec!["language:ja"]),
            (4, "D winter", vec!["language:ja"]),
        ]
        .map(|(id, title, tags)| ArchiveRecord {
            id,
            title: title.into(),
            date_added: id,
            tags: tags.into_iter().map(String::from).collect(),
            ..Default::default()
        }),
    )
    .unwrap();
    let any = Term::Any(vec![
        Term::TagContains("SUMMER".into()),
        Term::TitleContains("summer".into()),
    ]);

    let cases = [
        (vec![any.clone()], vec![], vec![1, 2, 3]),
        (
            vec![any.clone(), Term::Tag("language:ja".into())],
            vec![],
            vec![1, 3],
        ),
        (vec![], vec![any.clone()], vec![4]),
        (
            vec![Term::Tag("language:ja".into())],
            vec![any.clone()],
            vec![4],
        ),
        (vec![Term::Any(vec![])], vec![], vec![]),
        (vec![], vec![Term::Any(vec![])], vec![1, 2, 3, 4]),
        (
            vec![Term::Any(vec![any, Term::Tag("missing".into())])],
            vec![],
            vec![1, 2, 3],
        ),
    ];
    for (include, exclude, expected) in cases {
        let query = Query {
            include,
            exclude,
            ..Query::default()
        };
        let result = index.search(&query).unwrap();
        assert_eq!(result.total, expected.len() as u64, "{query:?}");
        assert_eq!(result.ids, expected, "{query:?}");
    }
}
