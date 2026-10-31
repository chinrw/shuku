//! 用 EhTagTranslation 项目的 `db.text.json` 把 E-Hentai 的英文标签译成中文，
//! 代替 LRR 的 ETagCN 插件。命名空间按数据库里的 `frontMatters.name` 译，
//! 值按同一命名空间下的条目译；数据库里没有的保持原样。

use crate::Result;
use serde::Deserialize;
use shuku_store::Tag;
use std::collections::HashMap;

/// 载入后的翻译表，键都是小写。
#[derive(Debug, Default)]
pub struct Translations {
    namespaces: HashMap<String, Namespace>,
}

#[derive(Debug)]
struct Namespace {
    name: String,
    values: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Database {
    data: Vec<NamespaceEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NamespaceEntry {
    namespace: String,
    front_matters: FrontMatters,
    #[serde(default)]
    data: HashMap<String, ValueEntry>,
}

#[derive(Deserialize)]
struct FrontMatters {
    name: String,
}

#[derive(Deserialize)]
struct ValueEntry {
    name: String,
}

impl Translations {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let database: Database = serde_json::from_slice(bytes)?;
        let namespaces = database
            .data
            .into_iter()
            .map(|entry| {
                let values = entry
                    .data
                    .into_iter()
                    .filter(|(_, value)| !value.name.trim().is_empty())
                    .map(|(key, value)| (key.to_lowercase(), value.name.trim().to_owned()))
                    .collect();
                (
                    entry.namespace.to_lowercase(),
                    Namespace {
                        name: entry.front_matters.name.trim().to_owned(),
                        values,
                    },
                )
            })
            .collect();
        Ok(Self { namespaces })
    }

    /// 能译的值的总数，控制台用来确认数据库载入了。
    pub fn len(&self) -> usize {
        self.namespaces
            .values()
            .map(|namespace| namespace.values.len())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 没有命名空间的标签不译：E-Hentai 的标签都带命名空间。
    pub fn translate(&self, tag: &Tag) -> Tag {
        let Some(namespace) = self.namespaces.get(&tag.namespace.to_lowercase()) else {
            return tag.clone();
        };
        let value = namespace
            .values
            .get(&tag.value.to_lowercase())
            .cloned()
            .unwrap_or_else(|| tag.value.clone());
        let name = if namespace.name.is_empty() {
            tag.namespace.clone()
        } else {
            namespace.name.clone()
        };
        Tag {
            namespace: name,
            value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATABASE: &str = r#"{
        "head": {}, "version": 6,
        "data": [
            {"namespace": "rows", "frontMatters": {"name": "行名", "key": "rows"},
             "data": {"female": {"name": "女性", "intro": ""}}},
            {"namespace": "female", "frontMatters": {"name": "女性", "key": "female"},
             "data": {"big breasts": {"name": "巨乳", "intro": "", "links": ""}}},
            {"namespace": "reclass", "frontMatters": {"name": "重新分类"},
             "data": {"doujinshi": {"name": "同人志"}}}
        ]
    }"#;

    fn tag(namespace: &str, value: &str) -> Tag {
        Tag {
            namespace: namespace.into(),
            value: value.into(),
        }
    }

    #[test]
    fn known_namespaces_and_values_are_translated() {
        let translations = Translations::from_json(DATABASE.as_bytes()).unwrap();
        assert_eq!(translations.len(), 3);
        assert_eq!(
            translations.translate(&tag("female", "Big Breasts")),
            tag("女性", "巨乳")
        );
        assert_eq!(
            translations.translate(&tag("reclass", "doujinshi")),
            tag("重新分类", "同人志")
        );
        // 命名空间认得、值不认得时只译命名空间，和 ETagCN 一样。
        assert_eq!(
            translations.translate(&tag("female", "unknown")),
            tag("女性", "unknown")
        );
        assert_eq!(
            translations.translate(&tag("uploader", "someone")),
            tag("uploader", "someone")
        );
        assert_eq!(translations.translate(&tag("", "plain")), tag("", "plain"));
    }

    #[test]
    fn a_file_that_is_not_the_database_is_an_error() {
        assert!(Translations::from_json(b"{}").is_err());
        assert!(Translations::from_json(b"not json").is_err());
    }
}
