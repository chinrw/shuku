//! 内置的元数据功能：本地的文件名解析和 ComicInfo.xml，在线的 E-Hentai/ExHentai（可以
//! 把标签译成中文）、Bangumi 和 AniList。解析和联网在 `shuku-metadata` 里，这里负责设置、
//! 扫描时自动运行、在线来源的后台队列、待确认的候选和 HTTP 接口。结果都经
//! [`crate::edit::ArchiveEdit`] 写入：和 LRR 一样只加标签、不删已有的标签，来源地址和简介
//! 只在书还没有时写入。
//!
//! 扫描新增的书（不是从 LRR 迁移来的）先在扫描的事务里跑文件名和 ComicInfo，打开了在线来源
//! 的自动运行时再放进 `metadata_queue`，由后台逐本查。在线来源打开“先放候选”时，查到的结果
//! 存进 `metadata_candidate`，等人或 agent 改好、批准后才写入。
//!
//! - `network`：在线查找、限速、封面指纹和翻译表。
//! - `queue`：后台队列。
//! - `routes`：HTTP 接口。

mod network;
mod queue;
mod routes;

use crate::{
    App, Error, Result,
    edit::{ArchiveEdit, TagEdit},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shuku_metadata::{
    Metadata,
    anilist::AnilistOptions,
    bangumi::BangumiOptions,
    comicinfo,
    ehentai::{Cookies, EhOptions},
    filename::{FilenameOptions, FilenameParser},
};
use std::{
    io::Read,
    path::Path as FilePath,
    sync::{Arc, Mutex, PoisonError},
};

pub(crate) use network::MetadataNetwork;

/// 设置表里的键。
const SETTING_KEY: &str = "metadata";
/// ComicInfo.xml 一般只有几 KiB，超过这个大小当作损坏。
const COMICINFO_LIMIT: u64 = 1024 * 1024;
/// 在线来源的简介最多写这么多字，编辑接口的上限更宽。
const SUMMARY_LIMIT: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Provider {
    Filename,
    Comicinfo,
    Ehentai,
    Bangumi,
    Anilist,
}

impl Provider {
    /// 在线来源，后台队列按这个顺序挑。
    pub(crate) const NETWORK: [Provider; 3] =
        [Provider::Ehentai, Provider::Bangumi, Provider::Anilist];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Filename => "filename",
            Self::Comicinfo => "comicinfo",
            Self::Ehentai => "ehentai",
            Self::Bangumi => "bangumi",
            Self::Anilist => "anilist",
        }
    }

    pub(crate) fn network(name: &str) -> Result<Self> {
        Self::NETWORK
            .into_iter()
            .find(|provider| provider.as_str() == name)
            .ok_or_else(|| {
                Error::bad_request(format!(
                    "provider 必须是 ehentai、bangumi 或 anilist，收到 {name}"
                ))
            })
    }

    fn is_network(self) -> bool {
        Self::NETWORK.contains(&self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    /// 用找到的标题替换原标题，对应 LRR 的“允许插件替换标题”。Bangumi 和 AniList
    /// 查的是系列，不给标题。
    pub(crate) replace_title: bool,
    pub(crate) filename: FilenameSettings,
    pub(crate) comicinfo: ComicInfoSettings,
    pub(crate) ehentai: EhSettings,
    pub(crate) translation: TranslationSettings,
    pub(crate) bangumi: BangumiSettings,
    pub(crate) anilist: AnilistSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            replace_title: true,
            filename: FilenameSettings::default(),
            comicinfo: ComicInfoSettings::default(),
            ehentai: EhSettings::default(),
            translation: TranslationSettings::default(),
            bangumi: BangumiSettings::default(),
            anilist: AnilistSettings::default(),
        }
    }
}

impl Settings {
    /// 扫描到新书时是否放进这个在线来源的队列。
    fn auto(&self, provider: Provider) -> bool {
        match provider {
            Provider::Filename => self.filename.auto,
            Provider::Comicinfo => self.comicinfo.auto,
            Provider::Ehentai => self.ehentai.auto,
            Provider::Bangumi => self.bangumi.auto,
            Provider::Anilist => self.anilist.auto,
        }
    }

    /// 这个在线来源查到的结果是否先放进候选、确认后才写入。
    fn review(&self, provider: Provider) -> bool {
        match provider {
            Provider::Ehentai => self.ehentai.review,
            Provider::Bangumi => self.bangumi.review,
            Provider::Anilist => self.anilist.review,
            Provider::Filename | Provider::Comicinfo => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FilenameSettings {
    /// 扫描到新书时自动运行。
    pub(crate) auto: bool,
    #[serde(flatten)]
    pub(crate) options: FilenameOptions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ComicInfoSettings {
    pub(crate) auto: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct EhSettings {
    /// 扫描到新书时放进后台队列。
    pub(crate) auto: bool,
    /// 查到的结果先放进候选，确认后才写入。
    pub(crate) review: bool,
    #[serde(flatten)]
    pub(crate) options: EhOptions,
    /// 只写不读：设置接口只返回每个 cookie 有没有设置。
    pub(crate) cookies: Cookies,
}

/// E-Hentai 的结果译成中文，代替 LRR 的 ETagCN。数据库固定是数据目录里的
/// `db.text.json`：路径不能从没有认证的设置接口改，否则任何人都能让服务端去读别的文件。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TranslationSettings {
    pub(crate) enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct BangumiSettings {
    pub(crate) auto: bool,
    pub(crate) review: bool,
    #[serde(flatten)]
    pub(crate) options: BangumiOptions,
    /// 只写不读。没有令牌也能查，但搜不到被标成 NSFW 的条目。
    pub(crate) token: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AnilistSettings {
    pub(crate) auto: bool,
    pub(crate) review: bool,
    #[serde(flatten)]
    pub(crate) options: AnilistOptions,
}

/// 设置、翻译表、限速和后台队列的状态。
pub(crate) struct MetadataState {
    settings: Mutex<Arc<Settings>>,
    network: MetadataNetwork,
}

impl MetadataState {
    pub(crate) fn load(reader: &shuku_store::Reader) -> Result<Self> {
        let settings = reader.setting::<Settings>(SETTING_KEY)?.unwrap_or_default();
        Ok(Self {
            settings: Mutex::new(Arc::new(settings)),
            network: MetadataNetwork::default(),
        })
    }
}

/// 一次扫描里用的本地功能和要排队的在线来源，开始扫描时按当时的设置准备好。
pub(crate) struct ScanPlugins {
    filename: Option<FilenameParser>,
    comicinfo: bool,
    replace_title: bool,
    queued: Vec<Provider>,
}

impl ScanPlugins {
    /// 读新文件里的 ComicInfo.xml。读不了或解析不了只记日志，不影响入库。
    pub(crate) fn comicinfo(&self, path: &FilePath) -> Option<Metadata> {
        if !self.comicinfo {
            return None;
        }
        match read_comicinfo(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "ComicInfo.xml 读取失败");
                None
            }
        }
    }

    /// 新入库的书要写的元数据。结果通不过校验时只记日志，这本书照常入库。
    /// `name` 是去掉扩展名的文件名，`relative` 只用来记日志。
    pub(crate) fn edit_for_new(
        &self,
        relative: &str,
        name: &str,
        comicinfo: Option<Metadata>,
    ) -> Option<ArchiveEdit> {
        let mut metadata = Metadata::default();
        if let Some(parser) = &self.filename
            && let Some(parsed) = parser.parse(name)
        {
            metadata.merge(parsed);
        }
        if let Some(comicinfo) = comicinfo {
            metadata.merge(comicinfo);
        }
        if metadata.is_empty() {
            return None;
        }
        let edit = edit_from(proposal(metadata, self.replace_title), false, false);
        match edit.validate() {
            Ok(()) => Some(edit),
            Err(error) => {
                tracing::warn!(path = relative, %error, "文件名或 ComicInfo 的元数据无效，没有写入");
                None
            }
        }
    }

    /// 新书要放进哪些在线来源的队列。
    pub(crate) fn queued_providers(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.queued.iter().map(|provider| provider.as_str())
    }
}

impl App {
    fn metadata_settings(&self) -> Arc<Settings> {
        self.0
            .metadata
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn scan_plugins(&self) -> Result<ScanPlugins> {
        let settings = self.metadata_settings();
        Ok(ScanPlugins {
            filename: settings
                .filename
                .auto
                .then(|| FilenameParser::new(&settings.filename.options))
                .transpose()?,
            comicinfo: settings.comicinfo.auto,
            replace_title: settings.replace_title,
            queued: Provider::NETWORK
                .into_iter()
                .filter(|provider| settings.auto(*provider))
                .collect(),
        })
    }

    /// 在本地跑文件名或 ComicInfo，返回要写的内容，阻塞调用。
    fn run_local(&self, id: i64, provider: Provider) -> Result<Metadata> {
        let settings = self.metadata_settings();
        let archive = crate::api::require_archive(&*self.reader()?, id)?;
        let path = self.0.config.content_dir.join(&archive.metadata.path);
        let metadata = match provider {
            Provider::Filename => FilenameParser::new(&settings.filename.options)?
                .parse(&book_name(&path))
                .ok_or_else(|| nothing_found("文件名和正则匹配不上"))?,
            Provider::Comicinfo => {
                read_comicinfo(&path)?.ok_or_else(|| nothing_found("这本书里没有 ComicInfo.xml"))?
            }
            Provider::Ehentai | Provider::Bangumi | Provider::Anilist => {
                return Err(Error::bad_request("在线来源要联网，走 run_network"));
            }
        };
        Ok(proposal(metadata, settings.replace_title))
    }

    /// 不写入，返回按书现在的状态会写入的内容，和 [`App::apply_found`] 的取舍一样。阻塞调用。
    fn preview_found(&self, id: i64, metadata: Metadata) -> Result<Metadata> {
        let archive = crate::api::require_archive(&*self.reader()?, id)?;
        let edit = edit_from(
            metadata,
            archive.metadata.source_url.is_some(),
            !archive.metadata.summary.trim().is_empty(),
        );
        edit.validate()?;
        Ok(applied(&edit))
    }

    /// 在一个事务里写入，返回实际写入的内容。来源地址和简介按事务里读到的现状决定：
    /// 联网期间别的请求可能已经写了，那时不再覆盖。阻塞调用。
    fn apply_found(&self, id: i64, metadata: Metadata) -> Result<Metadata> {
        let applied = self.write(|writer| apply_in(writer, id, metadata))?;
        self.listing_changed();
        Ok(applied)
    }

    /// 合并设置的补丁、校验后保存，阻塞调用。
    fn save_metadata_settings(&self, patch: &Value) -> Result<Arc<Settings>> {
        // 读出、合并、提交和换上新设置都在这把锁里，两个同时到的补丁不会互相覆盖，
        // 内存里的设置也不会和库里的不一样。
        let mut cached = self
            .0
            .metadata
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let current =
            serde_json::to_value(&**cached).map_err(|error| Error::Write(error.to_string()))?;
        check_keys(patch, &current, "")?;
        let mut merged = current;
        merge(&mut merged, patch);
        let settings: Settings = serde_json::from_value(merged).map_err(Error::bad_request)?;
        FilenameParser::new(&settings.filename.options).map_err(Error::bad_request)?;
        if settings.anilist.options.min_tag_rank > 100 {
            return Err(Error::bad_request(
                "anilist.min_tag_rank 必须在 0 到 100 之间",
            ));
        }
        if settings.bangumi.options.tag_limit > 100 {
            return Err(Error::bad_request("bangumi.tag_limit 必须在 0 到 100 之间"));
        }
        let settings = Arc::new(settings);
        self.write(|writer| Ok(writer.set_setting(SETTING_KEY, &*settings)?))?;
        *cached = settings.clone();
        drop(cached);
        // 改了设置（例如换了 cookie）就从暂停的地方接着查。
        self.resume_metadata(None);
        Ok(settings)
    }
}

/// 在调用方的事务里写入，返回实际写入的内容。
fn apply_in(writer: &mut shuku_store::Writer<'_>, id: i64, metadata: Metadata) -> Result<Metadata> {
    let archive = writer.archive(id)?.ok_or_else(Error::not_found)?;
    let edit = edit_from(
        metadata,
        archive.metadata.source_url.is_some(),
        !archive.metadata.summary.trim().is_empty(),
    );
    edit.validate()?;
    edit.apply(writer, id)?;
    Ok(applied(&edit))
}

fn nothing_found(message: &str) -> Error {
    Error::Http {
        status: axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        message: message.into(),
    }
}

/// 归档里的 ComicInfo.xml；散图文件夹读文件夹里直接放着的那个。
fn read_comicinfo(path: &FilePath) -> Result<Option<Metadata>> {
    let bytes = if path.is_dir() {
        read_folder_comicinfo(path)?
    } else {
        shuku_archive::read_named_entry(path, comicinfo::FILE_NAME, COMICINFO_LIMIT)?
    };
    match bytes {
        Some(bytes) => Ok(Some(comicinfo::parse(&bytes)?)),
        None => Ok(None),
    }
}

fn read_folder_comicinfo(folder: &FilePath) -> Result<Option<Vec<u8>>> {
    let wanted = comicinfo::FILE_NAME.to_lowercase();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().to_lowercase() != wanted
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(entry.path())?
            .take(COMICINFO_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > COMICINFO_LIMIT {
            return Err(Error::File(format!("{} 太大", entry.path().display())));
        }
        return Ok(Some(bytes));
    }
    Ok(None)
}

/// 文件名解析用的名字：去掉扩展名的文件名；文件夹名里的点不是扩展名，用整个文件夹名。
fn book_name(path: &FilePath) -> String {
    let name = if path.is_dir() {
        path.file_name()
    } else {
        path.file_stem()
    };
    name.map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// 查到的结果按设置去掉不该改的部分，剩下的是要写的内容，也是候选里存的内容。
/// 写不进的标签（带逗号、换行这类）这里就丢掉，免得一个坏标签让整本写入失败。
fn proposal(mut metadata: Metadata, replace_title: bool) -> Metadata {
    if !replace_title {
        metadata.title = None;
    }
    metadata
        .tags
        .retain(|tag| crate::edit::valid_tags(std::slice::from_ref(tag)).is_ok());
    if let Some(summary) = &mut metadata.summary
        && summary.chars().count() > SUMMARY_LIMIT
    {
        *summary = summary.chars().take(SUMMARY_LIMIT).collect();
    }
    metadata
}

/// 要写的内容变成编辑：标签只加不删，已有来源地址和简介时不改它们。
fn edit_from(metadata: Metadata, has_source: bool, has_summary: bool) -> ArchiveEdit {
    ArchiveEdit {
        title: metadata.title,
        summary: metadata.summary.filter(|_| !has_summary),
        source_url: metadata.source_url.filter(|_| !has_source).map(Some),
        tags: TagEdit::Merge {
            add: metadata.tags,
            remove: Vec::new(),
        },
    }
}

/// 实际写入的内容，回给调用方显示。
fn applied(edit: &ArchiveEdit) -> Metadata {
    Metadata {
        title: edit.title.clone(),
        summary: edit.summary.clone(),
        tags: match &edit.tags {
            TagEdit::Merge { add, .. } => add.clone(),
            TagEdit::Keep | TagEdit::Replace(_) => Vec::new(),
        },
        source_url: edit.source_url.clone().flatten(),
        volume: None,
    }
}

/// 补丁里的每个键都要是设置里已有的，拼错的键不会被悄悄忽略。
fn check_keys(patch: &Value, current: &Value, prefix: &str) -> Result<()> {
    let Some(patch) = patch.as_object() else {
        return if prefix.is_empty() {
            Err(Error::bad_request("设置必须是 JSON 对象"))
        } else {
            Ok(())
        };
    };
    for (key, value) in patch {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let Some(existing) = current.get(key) else {
            return Err(Error::bad_request(format!("没有这项设置：{path}")));
        };
        if existing.is_object() {
            if !value.is_object() {
                return Err(Error::bad_request(format!("{path} 必须是 JSON 对象")));
            }
            check_keys(value, existing, &path)?;
        }
    }
    Ok(())
}

/// 把补丁里的对象逐层合并进去，其他值直接替换。
fn merge(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                merge(target.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (target, patch) => *target = patch.clone(),
    }
}

/// 返回给控制台的设置：cookie 和令牌只说有没有设置。
fn settings_json(settings: &Settings) -> Result<Value> {
    let mut value =
        serde_json::to_value(settings).map_err(|error| Error::Write(error.to_string()))?;
    let cookies = &settings.ehentai.cookies;
    if let Some(ehentai) = value.get_mut("ehentai").and_then(Value::as_object_mut) {
        ehentai.remove("cookies");
        ehentai.insert(
            "cookies_set".into(),
            json!({
                "ipb_member_id": !cookies.ipb_member_id.is_empty(),
                "ipb_pass_hash": !cookies.ipb_pass_hash.is_empty(),
                "star": !cookies.star.is_empty(),
                "igneous": !cookies.igneous.is_empty(),
            }),
        );
    }
    if let Some(bangumi) = value.get_mut("bangumi").and_then(Value::as_object_mut) {
        bangumi.remove("token");
        bangumi.insert(
            "token_set".into(),
            json!(!settings.bangumi.token.trim().is_empty()),
        );
    }
    Ok(value)
}

fn metadata_json(metadata: &Metadata) -> Value {
    let tags: Vec<_> = metadata
        .tags
        .iter()
        .map(|tag| json!({"namespace": tag.namespace, "value": tag.value}))
        .collect();
    json!({
        "title": metadata.title, "summary": metadata.summary, "tags": tags,
        "source_url": metadata.source_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_setting_keys_are_rejected_and_objects_merge() {
        let current = serde_json::to_value(Settings::default()).unwrap();
        assert!(check_keys(&json!({"filename": {"auto": true}}), &current, "").is_ok());
        assert!(check_keys(&json!({"bangumi": {"tag_limit": 3}}), &current, "").is_ok());
        assert!(check_keys(&json!({"filename": {"atuo": true}}), &current, "").is_err());
        assert!(check_keys(&json!({"filename": true}), &current, "").is_err());
        assert!(check_keys(&json!([]), &current, "").is_err());

        let mut merged = current;
        merge(
            &mut merged,
            &json!({"ehentai": {"exhentai": true, "cookies": {"igneous": "x"}},
                    "anilist": {"title": "romaji"}}),
        );
        let settings: Settings = serde_json::from_value(merged).unwrap();
        assert!(settings.ehentai.options.exhentai);
        assert_eq!(settings.ehentai.cookies.igneous, "x");
        assert_eq!(
            settings.anilist.options.title,
            shuku_metadata::anilist::TitleLanguage::Romaji
        );
        // 没提到的字段不变。
        assert!(settings.ehentai.options.use_thumbnail);
        assert!(settings.replace_title);
        assert!(settings.bangumi.options.prefer_chinese);
    }

    #[test]
    fn secrets_are_reported_but_not_returned() {
        let mut settings = Settings::default();
        settings.ehentai.cookies.ipb_pass_hash = "secret".into();
        settings.bangumi.token = "token-secret".into();
        let value = settings_json(&settings).unwrap();
        assert!(!value.to_string().contains("secret"));
        assert_eq!(value["ehentai"]["cookies_set"]["ipb_pass_hash"], true);
        assert_eq!(value["ehentai"]["cookies_set"]["igneous"], false);
        assert_eq!(value["bangumi"]["token_set"], true);
    }

    #[test]
    fn found_metadata_only_adds_and_keeps_what_the_book_already_has() {
        let metadata = Metadata {
            title: Some("New".into()),
            summary: Some("Summary".into()),
            tags: vec![
                shuku_store::Tag {
                    namespace: "artist".into(),
                    value: "a".into(),
                },
                shuku_store::Tag {
                    namespace: String::new(),
                    value: "bad, tag".into(),
                },
            ],
            source_url: Some("https://e-hentai.org/g/1/a/".into()),
            volume: None,
        };
        let kept = proposal(metadata.clone(), false);
        assert_eq!(kept.title, None);
        assert_eq!(kept.tags.len(), 1, "带逗号的标签丢掉");
        let edit = edit_from(kept, true, true);
        assert_eq!(edit.summary, None);
        assert_eq!(edit.source_url, None);
        assert!(
            matches!(&edit.tags, TagEdit::Merge { add, remove } if add.len() == 1 && remove.is_empty())
        );
        let edit = edit_from(proposal(metadata, true), false, false);
        assert_eq!(edit.title.as_deref(), Some("New"));
        assert_eq!(edit.summary.as_deref(), Some("Summary"));
        assert_eq!(
            edit.source_url,
            Some(Some("https://e-hentai.org/g/1/a/".into()))
        );
    }
}
