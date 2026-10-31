//! 一次改很多本：加减标签、加进或移出静态分类，对应 LRR 的批量操作页。要改的书是
//! 一串 ID，或者和搜索相同的条件（`q`、`category`、`filter`）选出的全部结果。
//! 一批在一个事务里提交，要么全改，要么都不改。

use crate::{
    App, Error, Result,
    edit::{ArchiveEdit, TagEdit, parse_tag},
};
use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use shuku_store::Tag;

/// 一批最多改这么多本。30 万本时一次全改会让写事务持有写连接很久，进度上报都要等它。
const ARCHIVE_LIMIT: usize = 10_000;
/// 一批最多加减这么多个标签。
const TAG_LIMIT: usize = 100;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchBody {
    /// 和 `search` 二选一。
    ids: Option<Vec<i64>>,
    search: Option<Selection>,
    #[serde(default)]
    add_tags: Vec<String>,
    #[serde(default)]
    remove_tags: Vec<String>,
    add_to_category: Option<i64>,
    remove_from_category: Option<i64>,
}

/// 和搜索接口相同的条件。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    #[serde(default)]
    q: String,
    category: Option<i64>,
    #[serde(default)]
    filter: String,
}

struct Batch {
    /// 标签走和单本编辑相同的 [`ArchiveEdit`]，校验和写入只有一套。
    tags: Option<ArchiveEdit>,
    add_to_category: Option<i64>,
    remove_from_category: Option<i64>,
}

impl App {
    pub(crate) fn batch_router(&self) -> Router<App> {
        Router::new()
            .route("/api/batch", post(batch))
            .layer(axum::middleware::from_fn(crate::admin::same_origin))
    }

    /// 选出要改的书。按条件选时用快照，和用户在书库页上看到的结果一致。
    fn select(&self, ids: Option<Vec<i64>>, search: Option<Selection>) -> Result<Vec<i64>> {
        let ids = match (ids, search) {
            (Some(ids), None) => ids,
            (None, Some(search)) => {
                let filters = crate::api::parse_filters(&search.filter)?;
                let snapshot = self.snapshot();
                let (mut query, _) =
                    crate::api::build_query(&snapshot, &search.q, search.category, &filters)?;
                // 多取一本，才知道是不是超过了上限。
                query.limit = ARCHIVE_LIMIT + 1;
                snapshot
                    .index
                    .search(&query)
                    .map_err(Error::bad_request)?
                    .ids
            }
            _ => return Err(Error::bad_request("ids 和 search 必须给且只给一个")),
        };
        if ids.len() > ARCHIVE_LIMIT {
            return Err(Error::bad_request(format!(
                "一次最多改 {ARCHIVE_LIMIT} 本，请缩小范围"
            )));
        }
        let mut ids = ids;
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    /// 在一个事务里改完 `ids`，返回改了几本。按条件选出的书在快照之后被删掉的跳过；
    /// 明确给出的 ID 不存在时整批失败，返回 404。
    fn apply_batch(&self, ids: &[i64], batch: &Batch, skip_missing: bool) -> Result<usize> {
        let changed = self.write_categories(|writer| {
            // 一本都没选中时也要报出分类不存在或是动态分类，不能悄悄返回 0。
            for category in [batch.add_to_category, batch.remove_from_category]
                .into_iter()
                .flatten()
            {
                writer.require_static_category(category)?;
            }
            let mut changed = 0;
            for &id in ids {
                // 每一步都先检查归档在不在、再写，跳过的书没有留下一半的改动。
                let result = (|| -> Result<()> {
                    if let Some(category) = batch.add_to_category {
                        writer.set_category_member(category, id, true)?;
                    }
                    if let Some(category) = batch.remove_from_category {
                        writer.set_category_member(category, id, false)?;
                    }
                    if let Some(tags) = &batch.tags {
                        tags.apply(writer, id)?;
                    }
                    Ok(())
                })();
                match result {
                    Ok(()) => changed += 1,
                    Err(Error::Store(shuku_store::Error::ArchiveNotFound(_))) if skip_missing => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(changed)
        })?;
        if batch.tags.is_some() {
            self.listing_changed();
        }
        Ok(changed)
    }
}

async fn batch(
    State(app): State<App>,
    body: std::result::Result<Json<BatchBody>, JsonRejection>,
) -> Result<Json<Value>> {
    let Json(body) = body.map_err(Error::bad_request)?;
    let parse = |texts: &[String]| -> Result<Vec<Tag>> {
        if texts.len() > TAG_LIMIT {
            return Err(Error::bad_request(format!("一次最多 {TAG_LIMIT} 个标签")));
        }
        texts.iter().map(|text| parse_tag(text)).collect()
    };
    let (add, remove) = (parse(&body.add_tags)?, parse(&body.remove_tags)?);
    let tags = (!add.is_empty() || !remove.is_empty()).then(|| ArchiveEdit {
        tags: TagEdit::Merge { add, remove },
        ..ArchiveEdit::default()
    });
    if let Some(tags) = &tags {
        tags.validate()?;
    }
    let batch = Batch {
        tags,
        add_to_category: body.add_to_category,
        remove_from_category: body.remove_from_category,
    };
    if batch.tags.is_none()
        && batch.add_to_category.is_none()
        && batch.remove_from_category.is_none()
    {
        return Err(Error::bad_request(
            "至少要有 add_tags、remove_tags、add_to_category、remove_from_category 之一",
        ));
    }
    let skip_missing = body.search.is_some();
    app.blocking(move |app| {
        let ids = app.select(body.ids, body.search)?;
        let changed = app.apply_batch(&ids, &batch, skip_missing)?;
        tracing::info!(archives = changed, "批量修改");
        Ok(Json(json!({ "archives": changed })))
    })
    .await
}
