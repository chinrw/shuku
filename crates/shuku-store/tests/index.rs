mod common;

use common::{metadata, pages};
use shuku_index::{ArchiveRecord, Index, Query, Sort, Term};
use shuku_store::{Store, Tag};

#[test]
fn exports_index_records_with_and_without_tags() {
    let mut store = Store::open(":memory:").unwrap();
    let mut records = Vec::new();
    records.extend(store.library().unwrap().index_records());
    assert!(records.is_empty());
    let first = store
        .save_archive(None, &metadata("first.cbz"), &pages())
        .unwrap();
    let second = store
        .save_archive(None, &metadata("second.cbz"), &[])
        .unwrap();
    store
        .replace_tags(
            first,
            &[
                Tag {
                    namespace: "Artist".into(),
                    value: "ÄLPHA,\nBeta".into(),
                },
                Tag {
                    namespace: String::new(),
                    value: "独立".into(),
                },
            ],
        )
        .unwrap();
    records.extend(store.library().unwrap().index_records());
    assert_eq!(
        records,
        vec![
            ArchiveRecord {
                id: first,
                title: "漫画 10".into(),
                date_added: 1_700_000_000,
                tags: vec!["artist:älpha,\nbeta".into(), "独立".into()],
                page_count: 2,
                is_new: true,
                series: None,
            },
            ArchiveRecord {
                id: second,
                title: "漫画 10".into(),
                date_added: 1_700_000_000,
                tags: Vec::new(),
                page_count: 0,
                is_new: true,
                series: None,
            },
        ]
    );
}

#[test]
fn ten_thousand_archives_with_ten_tags_build_the_same_index() {
    let mut store = Store::open(":memory:").unwrap();
    let mut direct_records = Vec::new();
    for number in 0..10_000 {
        let mut archive = metadata(&format!("{number}.cbz"));
        archive.title = format!("漫画 {}", 10_000 - number);
        archive.date_added = i64::from(number % 19);
        let id = store.save_archive(None, &archive, &[]).unwrap();
        let mut tags = vec![
            Tag {
                namespace: "Artist".into(),
                value: format!("Name{}", number % 127),
            },
            Tag {
                namespace: "language".into(),
                value: if number % 2 == 0 {
                    "日本語"
                } else {
                    "中文"
                }
                .into(),
            },
            Tag {
                namespace: "group".into(),
                value: format!("G{}", number % 31),
            },
        ];
        for slot in 0..7 {
            tags.push(Tag {
                namespace: String::new(),
                value: format!("Topic{slot}-{}", (number * (slot + 3)) % 97),
            });
        }
        assert_eq!(tags.len(), 10);
        store.replace_tags(id, &tags).unwrap();
        direct_records.push(ArchiveRecord {
            id,
            title: archive.title,
            date_added: archive.date_added,
            tags: tags
                .iter()
                .map(|tag| {
                    if tag.namespace.is_empty() {
                        tag.value.clone()
                    } else {
                        format!("{}:{}", tag.namespace, tag.value)
                    }
                })
                .collect(),
            ..Default::default()
        });
    }

    let mut exported = Vec::new();
    exported.extend(store.library().unwrap().index_records());
    assert_eq!(exported.len(), 10_000);
    for (actual, expected) in exported.iter().zip(&direct_records) {
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.title, expected.title);
        assert_eq!(actual.date_added, expected.date_added);
        let mut actual_tags = actual.tags.clone();
        let mut expected_tags: Vec<_> =
            expected.tags.iter().map(|tag| tag.to_lowercase()).collect();
        actual_tags.sort();
        expected_tags.sort();
        assert_eq!(actual_tags, expected_tags);
    }
    let direct = Index::build(direct_records).unwrap();
    let from_store = Index::build(exported).unwrap();
    let conditions = [
        (vec![], vec![]),
        (vec![Term::Tag("ARTIST:NAME42".into())], vec![]),
        (vec![Term::TagPrefix("artist:name1".into())], vec![]),
        (vec![Term::TagContains("TOPIC3-".into())], vec![]),
        (vec![Term::TitleContains("漫画 10".into())], vec![]),
        (vec![], vec![Term::Tag("language:中文".into())]),
        (
            vec![
                Term::Tag("language:中文".into()),
                Term::Tag("language:日本語".into()),
            ],
            vec![],
        ),
        (
            vec![
                Term::Tag("group:g0".into()),
                Term::TagPrefix("artist:name".into()),
            ],
            vec![Term::TagContains("topic1-2".into())],
        ),
        (vec![Term::Tag("missing".into())], vec![]),
    ];
    for sort in [Sort::Title, Sort::DateAddedNewest] {
        for (include, exclude) in &conditions {
            let mut query = Query {
                include: include.clone(),
                exclude: exclude.clone(),
                sort: sort.clone(),
                limit: 37,
                after: None,
                reverse: false,
                collapse_series: false,
            };
            loop {
                let expected = direct.search(&query).unwrap();
                assert_eq!(from_store.search(&query).unwrap(), expected, "{query:?}");
                if expected.next_after.is_none() {
                    break;
                }
                query.after = expected.next_after;
            }
        }
    }
}
