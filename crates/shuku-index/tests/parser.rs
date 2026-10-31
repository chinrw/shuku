use shuku_index::{ParseError, ParsedQuery, Query, Term, parse_query};

fn either(text: &str) -> Term {
    Term::Any(vec![
        Term::TagContains(text.into()),
        Term::TitleContains(text.into()),
    ])
}

#[test]
fn parses_terms_exclusions_exact_markers_and_empty_queries() {
    let cases = [
        ("", Ok((vec![], vec![]))),
        (" \t\n", Ok((vec![], vec![]))),
        (" , , ", Ok((vec![], vec![]))),
        (
            " artist:Alpha ",
            Ok((vec![Term::TagPrefix("artist:Alpha".into())], vec![])),
        ),
        (
            "\"artist:Alpha\"",
            Ok((vec![Term::Tag("artist:Alpha".into())], vec![])),
        ),
        (
            "artist:Alpha$",
            Ok((vec![Term::Tag("artist:Alpha".into())], vec![])),
        ),
        (
            "\"artist:Alpha\"$",
            Ok((vec![Term::Tag("artist:Alpha".into())], vec![])),
        ),
        ("Summer", Ok((vec![either("Summer")], vec![]))),
        ("\"Summer\"", Ok((vec![either("Summer")], vec![]))),
        ("Summer$", Ok((vec![either("Summer")], vec![]))),
        (" 少女 ", Ok((vec![either("少女")], vec![]))),
        (
            " , artist:Alpha, 少女 , - language:en$ , -\"冬日\", ",
            Ok((
                vec![Term::TagPrefix("artist:Alpha".into()), either("少女")],
                vec![Term::Tag("language:en".into()), either("冬日")],
            )),
        ),
        ("-rain", Ok((vec![], vec![either("rain")]))),
        ("\"-rain\"", Ok((vec![either("-rain")], vec![]))),
        (
            "\"artist:Alpha, Beta\"",
            Ok((vec![Term::Tag("artist:Alpha, Beta".into())], vec![])),
        ),
        ("-", Err(ParseError::InvalidTerm("-".into()))),
        ("$", Err(ParseError::InvalidTerm("$".into()))),
        ("\"\"", Err(ParseError::InvalidTerm("\"\"".into()))),
        ("\"open", Err(ParseError::InvalidTerm("\"open".into()))),
        (
            "artist:\"Alpha\"",
            Err(ParseError::InvalidTerm("artist:\"Alpha\"".into())),
        ),
        ("good, -", Err(ParseError::InvalidTerm("-".into()))),
    ];

    for (input, expected) in cases {
        let expected = expected.map(|(include, exclude)| ParsedQuery {
            query: Query {
                include,
                exclude,
                ..Query::default()
            },
            degraded: false,
            read: Vec::new(),
        });
        assert_eq!(parse_query(input), expected, "{input:?}");
    }
}

#[test]
fn parses_trailing_wildcards_as_prefixes() {
    let cases = [
        ("artist:Al*", Term::TagPrefix("artist:Al".into())),
        ("artist:Al%", Term::TagPrefix("artist:Al".into())),
        ("artist:*", Term::TagPrefix("artist:".into())),
        ("Al*", either("Al")),
        ("Al%", either("Al")),
        ("*", either("")),
        ("\"artist:Al*\"", Term::TagPrefix("artist:Al".into())),
        ("artist:Al%$", Term::TagPrefix("artist:Al".into())),
    ];
    for (input, term) in cases {
        for excluded in [false, true] {
            let input = if excluded {
                format!("-{input}")
            } else {
                input.into()
            };
            let expected = ParsedQuery {
                query: Query {
                    include: if excluded { vec![] } else { vec![term.clone()] },
                    exclude: if excluded { vec![term.clone()] } else { vec![] },
                    ..Query::default()
                },
                degraded: false,
                read: Vec::new(),
            };
            assert_eq!(parse_query(&input), Ok(expected), "{input:?}");
        }
    }
}

#[test]
fn splits_inner_wildcards_into_required_fragments_and_marks_degradation() {
    let contains = |text: &str| Term::TagContains(text.into());
    let cases = [
        (
            "artist:Al*pha",
            vec![contains("artist:Al"), contains("pha")],
        ),
        (
            "artist:Al%pha",
            vec![contains("artist:Al"), contains("pha")],
        ),
        (
            "artist:Al?pha",
            vec![contains("artist:Al"), contains("pha")],
        ),
        (
            "artist:Al_pha",
            vec![contains("artist:Al"), contains("pha")],
        ),
        ("*artist:Al", vec![contains("artist:Al")]),
        ("artist:Al?", vec![contains("artist:Al")]),
        ("artist:Al**", vec![contains("artist:Al")]),
        (
            "\"artist:Al_pha\"",
            vec![contains("artist:Al"), contains("pha")],
        ),
        ("full_color", vec![either("full"), either("color")]),
        ("Al?pha$", vec![either("Al"), either("pha")]),
        (
            "?少女_日%记*",
            vec![either("少女"), either("日"), either("记")],
        ),
        ("?", vec![]),
    ];
    for (input, include) in cases {
        let expected = ParsedQuery {
            query: Query {
                include,
                ..Query::default()
            },
            degraded: true,
            read: Vec::new(),
        };
        assert_eq!(parse_query(input), Ok(expected), "{input:?}");
        let excluded = format!("-{input}");
        assert_eq!(
            parse_query(&excluded),
            Err(ParseError::UnsupportedWildcard(excluded.clone())),
        );
    }

    let parsed = parse_query("Al?pha, artist:Beta").unwrap();
    assert!(parsed.degraded);
    assert_eq!(
        parsed.query.include,
        vec![
            either("Al"),
            either("pha"),
            Term::TagPrefix("artist:Beta".into())
        ]
    );
}

#[test]
fn pages_and_read_conditions_become_comparisons() {
    use shuku_index::{Comparison, ReadCondition};
    let pages = |comparison| Ok((vec![Term::Pages(comparison)], vec![], vec![]));
    let read = |comparison, excluded| {
        Ok((
            vec![],
            vec![],
            vec![ReadCondition {
                comparison,
                excluded,
            }],
        ))
    };
    let cases = [
        ("pages:>20", pages(Comparison::Greater(20))),
        ("pages:<20", pages(Comparison::Less(20))),
        ("pages:>=20", pages(Comparison::AtLeast(20))),
        ("pages:<=20", pages(Comparison::AtMost(20))),
        ("pages:=20", pages(Comparison::Equal(20))),
        ("pages:20", pages(Comparison::Equal(20))),
        ("PAGES:>20", pages(Comparison::Greater(20))),
        ("pages: >20", pages(Comparison::Greater(20))),
        ("\"pages:>20\"", pages(Comparison::Greater(20))),
        (
            "-pages:<5",
            Ok((vec![], vec![Term::Pages(Comparison::Less(5))], vec![])),
        ),
        ("read:0", read(Comparison::Equal(0), false)),
        ("read:>0$", read(Comparison::Greater(0), false)),
        ("-read:>0", read(Comparison::Greater(0), true)),
        ("pages:!=20", Err("pages:!=20")),
        ("rating:>=3", Err("rating:>=3")),
        ("rating:!=3", Err("rating:!=3")),
        ("pages:>", Err("pages:>")),
        ("pages:>abc", Err("pages:>abc")),
        ("read:>-1", Err("read:>-1")),
    ];
    for (input, expected) in cases {
        let actual = parse_query(input)
            .map(|parsed| (parsed.query.include, parsed.query.exclude, parsed.read));
        let expected =
            expected.map_err(|raw: &str| ParseError::UnsupportedNumericCondition(raw.into()));
        assert_eq!(actual, expected, "{input:?}");
    }

    let parsed = parse_query("artist:Alpha, pages:>20, read:0").unwrap();
    assert_eq!(
        parsed.query.include,
        vec![
            Term::TagPrefix("artist:Alpha".into()),
            Term::Pages(Comparison::Greater(20))
        ]
    );
    assert_eq!(parsed.read.len(), 1);
    assert_eq!(
        parse_query("Al?pha, pages:!=20"),
        Err(ParseError::UnsupportedNumericCondition("pages:!=20".into()))
    );

    assert_eq!(
        parse_query("volume:20, volume:30$").unwrap().query.include,
        vec![
            Term::TagPrefix("volume:20".into()),
            Term::Tag("volume:30".into())
        ],
    );
}

#[test]
fn numeric_field_names_do_not_block_ordinary_tag_terms() {
    let cases = [
        ("pages:color", Ok(Term::TagPrefix("pages:color".into()))),
        ("read:later$", Ok(Term::Tag("read:later".into()))),
        ("pages:*", Ok(Term::TagPrefix("pages:".into()))),
        ("pages:-draft", Ok(Term::TagPrefix("pages:-draft".into()))),
        (
            "pages:-20",
            Err(ParseError::UnsupportedNumericCondition("pages:-20".into())),
        ),
        (
            "read:+1",
            Err(ParseError::UnsupportedNumericCondition("read:+1".into())),
        ),
        (
            "pages:999999999999999999999999999999",
            Err(ParseError::UnsupportedNumericCondition(
                "pages:999999999999999999999999999999".into(),
            )),
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(
            parse_query(input).map(|parsed| parsed.query.include),
            expected.map(|term| vec![term]),
            "{input:?}",
        );
    }
}

#[test]
fn backslash_makes_the_next_character_literal() {
    let tag = |text: &str| Term::Tag(text.into());
    let prefix = |text: &str| Term::TagPrefix(text.into());
    let cases = [
        (r#""artist:foo\_bar""#, vec![tag("artist:foo_bar")], vec![]),
        (r#""artist:foo\?bar""#, vec![tag("artist:foo?bar")], vec![]),
        (r#""artist:foo\*bar""#, vec![tag("artist:foo*bar")], vec![]),
        (r#""artist:foo\%bar""#, vec![tag("artist:foo%bar")], vec![]),
        (r#""artist:foo\*""#, vec![tag("artist:foo*")], vec![]),
        (r"artist:foo\*", vec![prefix("artist:foo*")], vec![]),
        (r"artist:foo\$", vec![prefix("artist:foo$")], vec![]),
        (r#"-"artist:foo\_bar""#, vec![], vec![tag("artist:foo_bar")]),
        (r"a\,b", vec![either("a,b")], vec![]),
        (
            r#""title:say \"hi\"""#,
            vec![tag(r#"title:say "hi""#)],
            vec![],
        ),
        (r"a\\b", vec![either(r"a\b")], vec![]),
        (r"trail\", vec![either(r"trail\")], vec![]),
        (r"foo\ ,bar", vec![either("foo "), either("bar")], vec![]),
        (r"foo\ ", vec![either("foo ")], vec![]),
        (r"\-rain", vec![either("-rain")], vec![]),
        (r#"\"artist:a\""#, vec![prefix(r#""artist:a""#)], vec![]),
        (
            r#""artist:foo\_bar", -"group:x\,y""#,
            vec![tag("artist:foo_bar")],
            vec![tag("group:x,y")],
        ),
    ];
    for (input, include, exclude) in cases {
        let expected = ParsedQuery {
            query: Query {
                include,
                exclude,
                ..Query::default()
            },
            degraded: false,
            read: Vec::new(),
        };
        assert_eq!(parse_query(input), Ok(expected), "{input:?}");
    }
}
