use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::Manager;
use tauri_plugin_store::StoreExt;

use crate::articles::{Article, SyncMapRecord};

pub const CONFIG_STORE_FILE: &str = "platform_config.json";
pub const CREDENTIAL_STORE_FILE: &str = "credentials.json";

const ARTICLE_INDEX_KEY: &str = "articles:index";
const ARTICLE_KEY_PREFIX: &str = "articles:";
const SYNCMAP_KEY_PREFIX: &str = "syncmap:";
const PLATFORM_CONFIG_SUFFIX: &str = "_config";
const COOKIE_SUFFIX: &str = "_cookie";
const EXPORT_VERSION: u32 = 1;

// 与 packages/core/src/platforms/registry.ts 中 credentialKind 为 "cookie" 的平台保持一致。
// 只有这些平台才可能在 credentials.json 里存在可解密的 {id}_cookie 密文；
// 博客园走 MetaWeblog Token（credentialKind: "metaweblog"），配置在 platform_config.json 里，
// 不存在 Cookie 凭据。channelType 为 "webview" 的平台（知乎、简书、思否、小红书、抖音）
// 登录态由通道窗口自己持有，同样不写 DPAPI Cookie，一律不列入。
// 新增平台时两边必须同步，否则导出会把凭据错当成 Cookie 去解密。
const COOKIE_CREDENTIAL_PLATFORMS: &[&str] = &[
    "juejin",
    "csdn",
    "tencentcloud",
    "aliyun",
    "wechat",
    "51cto",
    "oschina",
    "baijiahao",
    "netease",
    "bilibili",
    "weibo",
    "toutiao",
];

fn is_cookie_credential_platform(id: &str) -> bool {
    COOKIE_CREDENTIAL_PLATFORMS.contains(&id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StoreKind {
    PlatformConfig,
    Credentials,
}

pub trait ConfigBackend {
    fn read_all(&self, kind: StoreKind) -> Result<BTreeMap<String, Value>, String>;
    fn replace_all(&self, kind: StoreKind, entries: &BTreeMap<String, Value>) -> Result<(), String>;
}

pub struct TauriBackend<'a> {
    app: &'a tauri::AppHandle,
}

impl<'a> TauriBackend<'a> {
    pub fn new(app: &'a tauri::AppHandle) -> Self {
        Self { app }
    }

    fn file_of(kind: StoreKind) -> &'static str {
        match kind {
            StoreKind::PlatformConfig => CONFIG_STORE_FILE,
            StoreKind::Credentials => CREDENTIAL_STORE_FILE,
        }
    }
}

impl<'a> ConfigBackend for TauriBackend<'a> {
    fn read_all(&self, kind: StoreKind) -> Result<BTreeMap<String, Value>, String> {
        let store = self
            .app
            .store(Self::file_of(kind))
            .map_err(|e| format!("打开存储失败: {}", e))?;
        Ok(store.entries().into_iter().collect())
    }

    fn replace_all(&self, kind: StoreKind, entries: &BTreeMap<String, Value>) -> Result<(), String> {
        let store = self
            .app
            .store(Self::file_of(kind))
            .map_err(|e| format!("打开存储失败: {}", e))?;
        store.clear();
        for (key, value) in entries {
            store.set(key.clone(), value.clone());
        }
        store.save().map_err(|e| format!("保存存储失败: {}", e))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigBundle {
    pub version: u32,
    pub exported_at: String,
    pub platforms: BTreeMap<String, Value>,
    pub articles: Vec<Value>,
    pub sync_map: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportPlan {
    pub config: BTreeMap<String, Value>,
    pub credentials: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub platforms: usize,
    pub articles: usize,
    pub sync_maps: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResetReport {
    pub config_keys: usize,
    pub credential_keys: usize,
}

fn platform_id_of_config_key(key: &str) -> Option<&str> {
    key.strip_suffix(PLATFORM_CONFIG_SUFFIX)
        .filter(|id| !id.is_empty())
}

fn platform_id_of_cookie_key(key: &str) -> Option<&str> {
    key.strip_suffix(COOKIE_SUFFIX)
        .filter(|id| !id.is_empty())
}

fn cookie_of(config: &Value) -> Option<String> {
    config
        .get("cookie")
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn with_cookie(config: &Value, cookie: &str) -> Value {
    let mut object = match config {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    object.insert("cookie".to_string(), Value::String(cookie.to_string()));
    Value::Object(object)
}

fn without_cookie(config: &Value) -> Value {
    match config {
        Value::Object(map) => {
            let mut cleaned = map.clone();
            cleaned.remove("cookie");
            Value::Object(cleaned)
        }
        other => other.clone(),
    }
}

pub fn build_bundle<B: ConfigBackend>(
    backend: &B,
    mut resolve_cookie: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<ConfigBundle, String> {
    let config_entries = backend.read_all(StoreKind::PlatformConfig)?;
    let credential_entries = backend.read_all(StoreKind::Credentials)?;

    let mut platform_ids: Vec<String> = Vec::new();
    for key in config_entries.keys() {
        if let Some(id) = platform_id_of_config_key(key) {
            platform_ids.push(id.to_string());
        }
    }
    for key in credential_entries.keys() {
        if let Some(id) = platform_id_of_cookie_key(key) {
            if !is_cookie_credential_platform(id) {
                continue;
            }
            if !platform_ids.iter().any(|existing| existing == id) {
                platform_ids.push(id.to_string());
            }
        }
    }
    platform_ids.sort();

    let mut platforms = BTreeMap::new();
    for id in &platform_ids {
        let config = config_entries
            .get(&format!("{}{}", id, PLATFORM_CONFIG_SUFFIX))
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        let cookie = match cookie_of(&config) {
            Some(existing) => Some(existing),
            // 只有 Cookie 类凭据才去凭据库解密；博客园的 MetaWeblog Token 不是 Cookie 密文，
            // 强行解密会报「长度不是 4 的倍数」并拖垮整份导出。
            None if is_cookie_credential_platform(id) => match resolve_cookie(id) {
                Ok(cookie) => cookie,
                // 单个平台的密文损坏（旧版本明文残留、DPAPI 换机失效等）时跳过该平台的凭据，
                // 导出其余数据照常完成，绝不因为一个平台阻塞整份备份。
                Err(_) => None,
            },
            None => None,
        };
        let merged = match cookie {
            Some(cookie) => with_cookie(&config, &cookie),
            None => config,
        };
        platforms.insert(id.clone(), merged);
    }

    let index: Vec<String> = config_entries
        .get(ARTICLE_INDEX_KEY)
        .and_then(|value| serde_json::from_value::<Vec<String>>(value.clone()).ok())
        .unwrap_or_default();

    let mut article_ids = index.clone();
    for key in config_entries.keys() {
        if let Some(id) = key.strip_prefix(ARTICLE_KEY_PREFIX) {
            if id != "index" && !article_ids.iter().any(|existing| existing == id) {
                article_ids.push(id.to_string());
            }
        }
    }

    let mut articles = Vec::new();
    for id in &article_ids {
        if let Some(value) = config_entries.get(&format!("{}{}", ARTICLE_KEY_PREFIX, id)) {
            articles.push(value.clone());
        }
    }

    let mut sync_map = BTreeMap::new();
    for (key, value) in &config_entries {
        if let Some(id) = key.strip_prefix(SYNCMAP_KEY_PREFIX) {
            if !id.is_empty() {
                sync_map.insert(id.to_string(), value.clone());
            }
        }
    }

    Ok(ConfigBundle {
        version: EXPORT_VERSION,
        exported_at: crate::settings_timestamp(),
        platforms,
        articles,
        sync_map,
    })
}

pub fn parse_bundle(raw: &str) -> Result<ConfigBundle, String> {
    let bundle: ConfigBundle =
        serde_json::from_str(raw).map_err(|e| format!("配置文件解析失败: {}", e))?;
    validate_bundle(&bundle)?;
    Ok(bundle)
}

fn validate_bundle(bundle: &ConfigBundle) -> Result<(), String> {
    if bundle.version != EXPORT_VERSION {
        return Err(format!(
            "配置文件版本 {} 不受支持（当前支持 {}）",
            bundle.version, EXPORT_VERSION
        ));
    }

    for (platform, config) in &bundle.platforms {
        if platform.trim().is_empty() {
            return Err("配置文件包含空的平台标识".to_string());
        }
        if !matches!(config, Value::Object(_)) {
            return Err(format!("平台 {} 的配置不是对象", platform));
        }
    }

    let mut seen_ids: Vec<String> = Vec::new();
    for value in &bundle.articles {
        let article: Article = serde_json::from_value(value.clone())
            .map_err(|e| format!("文章结构不合法: {}", e))?;
        if article.id.trim().is_empty() {
            return Err("文章缺少 id".to_string());
        }
        if seen_ids.iter().any(|existing| existing == &article.id) {
            return Err(format!("文章 id 重复: {}", article.id));
        }
        seen_ids.push(article.id);
    }

    for (article_id, value) in &bundle.sync_map {
        if article_id.trim().is_empty() {
            return Err("同步映射包含空的文章 id".to_string());
        }
        let _record: SyncMapRecord = serde_json::from_value(value.clone())
            .map_err(|e| format!("文章 {} 的同步映射结构不合法: {}", article_id, e))?;
    }

    Ok(())
}

pub fn plan_import(
    bundle: &ConfigBundle,
    mut encrypt_cookie: impl FnMut(&str, &str) -> Result<String, String>,
) -> Result<ImportPlan, String> {
    let mut config = BTreeMap::new();
    let mut credentials = BTreeMap::new();

    for (platform, value) in &bundle.platforms {
        let config_key = format!("{}{}", platform, PLATFORM_CONFIG_SUFFIX);
        let cookie = cookie_of(value);
        match &cookie {
            Some(cookie) => {
                config.insert(config_key, value.clone());
                credentials.insert(
                    format!("{}{}", platform, COOKIE_SUFFIX),
                    Value::String(encrypt_cookie(platform, cookie)?),
                );
            }
            None => {
                config.insert(config_key, without_cookie(value));
            }
        }
    }

    let mut article_ids: Vec<String> = Vec::new();
    for value in &bundle.articles {
        let article: Article = serde_json::from_value(value.clone())
            .map_err(|e| format!("文章结构不合法: {}", e))?;
        config.insert(
            format!("{}{}", ARTICLE_KEY_PREFIX, article.id),
            serde_json::to_value(&article).map_err(|e| format!("序列化文章失败: {}", e))?,
        );
        article_ids.push(article.id);
    }
    config.insert(
        ARTICLE_INDEX_KEY.to_string(),
        serde_json::to_value(&article_ids).map_err(|e| format!("序列化文章索引失败: {}", e))?,
    );

    for (article_id, value) in &bundle.sync_map {
        let record: SyncMapRecord = serde_json::from_value(value.clone())
            .map_err(|e| format!("文章 {} 的同步映射结构不合法: {}", article_id, e))?;
        if record.is_empty() {
            continue;
        }
        config.insert(
            format!("{}{}", SYNCMAP_KEY_PREFIX, article_id),
            serde_json::to_value(&record).map_err(|e| format!("序列化同步映射失败: {}", e))?,
        );
    }

    Ok(ImportPlan { config, credentials })
}

pub fn import_with_rollback<B: ConfigBackend>(
    backend: &B,
    bundle: &ConfigBundle,
    encrypt_cookie: impl FnMut(&str, &str) -> Result<String, String>,
) -> Result<ImportReport, String> {
    let config_snapshot = backend.read_all(StoreKind::PlatformConfig)?;
    let credential_snapshot = backend.read_all(StoreKind::Credentials)?;

    let plan = plan_import(bundle, encrypt_cookie)?;

    let attempt = backend
        .replace_all(StoreKind::PlatformConfig, &plan.config)
        .and_then(|_| backend.replace_all(StoreKind::Credentials, &plan.credentials));

    match attempt {
        Ok(()) => Ok(ImportReport {
            platforms: bundle.platforms.len(),
            articles: bundle.articles.len(),
            sync_maps: bundle.sync_map.len(),
        }),
        Err(err) => {
            let config_restore = backend.replace_all(StoreKind::PlatformConfig, &config_snapshot);
            let credential_restore =
                backend.replace_all(StoreKind::Credentials, &credential_snapshot);
            let rollback_failure = match (config_restore, credential_restore) {
                (Ok(()), Ok(())) => None,
                (config_err, credential_err) => Some(format!(
                    "回滚未完全成功（配置: {}，凭据: {}），请手动核对数据",
                    config_err.map(|_| "成功").unwrap_or("失败"),
                    credential_err.map(|_| "成功").unwrap_or("失败"),
                )),
            };
            Err(match rollback_failure {
                Some(note) => format!("导入失败: {}；{}", err, note),
                None => format!("导入失败: {}，数据已还原为导入前的状态", err),
            })
        }
    }
}

pub fn collect_reset_keys(
    kind: StoreKind,
    entries: &BTreeMap<String, Value>,
    platform_ids: &[String],
) -> Vec<String> {
    let mut discovered: Vec<String> = platform_ids.to_vec();
    for key in entries.keys() {
        if let Some(id) = platform_id_of_config_key(key) {
            if !discovered.iter().any(|existing| existing == id) {
                discovered.push(id.to_string());
            }
        }
    }

    let mut keys: Vec<String> = entries
        .keys()
        .filter(|key| match kind {
            StoreKind::PlatformConfig => {
                key.as_str() == ARTICLE_INDEX_KEY
                    || key.starts_with(ARTICLE_KEY_PREFIX)
                    || key.starts_with(SYNCMAP_KEY_PREFIX)
                    || discovered.iter().any(|id| {
                        key.strip_suffix(PLATFORM_CONFIG_SUFFIX) == Some(id.as_str())
                    })
            }
            StoreKind::Credentials => key.ends_with(COOKIE_SUFFIX),
        })
        .cloned()
        .collect();
    keys.sort();
    keys
}

pub fn reset_with_report<B: ConfigBackend>(
    backend: &B,
    platform_ids: &[String],
) -> Result<ResetReport, String> {
    let config_entries = backend.read_all(StoreKind::PlatformConfig)?;
    let credential_entries = backend.read_all(StoreKind::Credentials)?;

    let config_keys = collect_reset_keys(StoreKind::PlatformConfig, &config_entries, platform_ids);
    let credential_keys =
        collect_reset_keys(StoreKind::Credentials, &credential_entries, platform_ids);

    let mut kept_config = config_entries.clone();
    for key in &config_keys {
        kept_config.remove(key);
    }
    let mut kept_credentials = credential_entries.clone();
    for key in &credential_keys {
        kept_credentials.remove(key);
    }

    backend.replace_all(StoreKind::PlatformConfig, &kept_config)?;
    backend.replace_all(StoreKind::Credentials, &kept_credentials)?;

    Ok(ResetReport {
        config_keys: config_keys.len(),
        credential_keys: credential_keys.len(),
    })
}

fn exports_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("获取配置目录失败: {}", e))?
        .join("exports");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建导出目录失败: {}", e))?;
    Ok(dir)
}

fn export_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(exports_dir(app)?.join(format!("openpub-config-{}.json", crate::settings_timestamp())))
}

fn write_atomically(path: &Path, content: &str) -> Result<(), String> {
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, content).map_err(|e| format!("写入临时文件失败: {}", e))?;
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("替换目标文件失败: {}", e)
    })
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportFileInfo {
    pub path: String,
    pub file_name: String,
    pub modified_at: u64,
    pub size_bytes: u64,
}

#[tauri::command]
pub async fn list_export_files(app: tauri::AppHandle) -> Result<Vec<ExportFileInfo>, String> {
    let dir = exports_dir(&app)?;
    let mut files = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("读取导出目录失败: {}", e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let modified_at = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        files.push(ExportFileInfo {
            path: path.to_string_lossy().to_string(),
            file_name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string(),
            modified_at,
            size_bytes: metadata.len(),
        });
    }
    files.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
    Ok(files)
}

#[tauri::command]
pub async fn export_config(
    app: tauri::AppHandle,
    target_path: Option<String>,
) -> Result<String, String> {
    let backend = TauriBackend::new(&app);
    let bundle = build_bundle(&backend, |platform| crate::load_stored_cookie(&app, platform))?;

    let path = match target_path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => export_path(&app)?,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目标目录失败: {}", e))?;
    }

    let content = serde_json::to_string_pretty(&bundle)
        .map_err(|e| format!("序列化配置失败: {}", e))?;
    write_atomically(&path, &content)?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn import_config(app: tauri::AppHandle, source_path: String) -> Result<ImportReport, String> {
    let path = PathBuf::from(source_path.trim());
    if !path.exists() {
        return Err(format!("配置文件不存在: {}", path.to_string_lossy()));
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("读取配置文件失败: {}", e))?;
    let bundle = parse_bundle(&raw)?;
    let backend = TauriBackend::new(&app);
    import_with_rollback(&backend, &bundle, |platform, cookie| {
        crate::encrypt_cookie_to_base64(platform, cookie)
    })
}

#[tauri::command]
pub async fn reset_all_data(
    app: tauri::AppHandle,
    platform_ids: Vec<String>,
) -> Result<ResetReport, String> {
    let backend = TauriBackend::new(&app);
    reset_with_report(&backend, &platform_ids)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    pub removed_credential_keys: usize,
}

fn collect_invalid_credential_keys(entries: &BTreeMap<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = entries
        .keys()
        .filter(|key| {
            key.ends_with(COOKIE_SUFFIX)
                && platform_id_of_cookie_key(key)
                    .map(|id| !is_cookie_credential_platform(id))
                    .unwrap_or(false)
        })
        .cloned()
        .collect();
    keys.sort();
    keys
}

pub fn cleanup_invalid_credentials<B: ConfigBackend>(
    backend: &B,
) -> Result<CleanupReport, String> {
    let credential_entries = backend.read_all(StoreKind::Credentials)?;
    let keys = collect_invalid_credential_keys(&credential_entries);

    let mut kept = credential_entries.clone();
    for key in &keys {
        kept.remove(key);
    }
    backend.replace_all(StoreKind::Credentials, &kept)?;

    Ok(CleanupReport {
        removed_credential_keys: keys.len(),
    })
}

#[tauri::command]
pub async fn cleanup_invalid_credentials_cmd(
    app: tauri::AppHandle,
) -> Result<CleanupReport, String> {
    let backend = TauriBackend::new(&app);
    cleanup_invalid_credentials(&backend)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeBackend {
        config: RefCell<BTreeMap<String, Value>>,
        credentials: RefCell<BTreeMap<String, Value>>,
        calls: RefCell<usize>,
        fail_on_calls: Vec<usize>,
    }

    impl FakeBackend {
        fn new(
            config: BTreeMap<String, Value>,
            credentials: BTreeMap<String, Value>,
            fail_on_calls: Vec<usize>,
        ) -> Self {
            Self {
                config: RefCell::new(config),
                credentials: RefCell::new(credentials),
                calls: RefCell::new(0),
                fail_on_calls,
            }
        }

        fn empty() -> Self {
            Self::new(BTreeMap::new(), BTreeMap::new(), Vec::new())
        }

        fn slot(&self, kind: StoreKind) -> &RefCell<BTreeMap<String, Value>> {
            match kind {
                StoreKind::PlatformConfig => &self.config,
                StoreKind::Credentials => &self.credentials,
            }
        }

        fn snapshot(&self, kind: StoreKind) -> BTreeMap<String, Value> {
            self.slot(kind).borrow().clone()
        }
    }

    impl ConfigBackend for FakeBackend {
        fn read_all(&self, kind: StoreKind) -> Result<BTreeMap<String, Value>, String> {
            Ok(self.snapshot(kind))
        }

        fn replace_all(
            &self,
            kind: StoreKind,
            entries: &BTreeMap<String, Value>,
        ) -> Result<(), String> {
            let call = {
                let mut calls = self.calls.borrow_mut();
                *calls += 1;
                *calls
            };
            self.slot(kind).borrow_mut().clear();
            if self.fail_on_calls.contains(&call) {
                return Err(format!("模拟第 {} 次写入失败", call));
            }
            *self.slot(kind).borrow_mut() = entries.clone();
            Ok(())
        }
    }

    fn article(id: &str, title: &str) -> Value {
        serde_json::json!({
            "id": id,
            "title": title,
            "content": format!("{} 的正文", title),
            "createdAt": "100",
            "updatedAt": "200",
        })
    }

    fn sync_entry(platform: &str) -> Value {
        serde_json::json!({
            "id": format!("remote-{}", platform),
            "published": true,
            "bodyHash": "abc",
        })
    }

    fn stub_encrypt(platform: &str, cookie: &str) -> Result<String, String> {
        Ok(format!("enc:{}:{}", platform, cookie))
    }

    fn bundle_from(platforms: Vec<(&str, Value)>, articles: Vec<Value>, sync_map: Vec<(&str, Value)>) -> ConfigBundle {
        ConfigBundle {
            version: EXPORT_VERSION,
            exported_at: "123".to_string(),
            platforms: platforms
                .into_iter()
                .map(|(id, value)| (id.to_string(), value))
                .collect(),
            articles,
            sync_map: sync_map
                .into_iter()
                .map(|(id, value)| (id.to_string(), value))
                .collect(),
        }
    }

    fn seeded_backend() -> FakeBackend {
        let mut config = BTreeMap::new();
        config.insert(
            "juejin_config".to_string(),
            serde_json::json!({ "cookie": "sessionid=manual" }),
        );
        config.insert(
            "cnblogs_config".to_string(),
            serde_json::json!({ "apiUrl": "https://rpc.cnblogs.com/metaweblog/x", "username": "u", "token": "t" }),
        );
        config.insert("articles:index".to_string(), serde_json::json!(["a1", "a2"]));
        config.insert("articles:a1".to_string(), article("a1", "第一篇"));
        config.insert("articles:a2".to_string(), article("a2", "第二篇"));
        config.insert(
            "syncmap:a1".to_string(),
            serde_json::json!({ "juejin": sync_entry("juejin") }),
        );

        let mut credentials = BTreeMap::new();
        credentials.insert(
            "csdn_cookie".to_string(),
            serde_json::json!("cipher-csdn"),
        );

        FakeBackend::new(config, credentials, Vec::new())
    }

    #[test]
    fn export_contains_all_three_sections() {
        let backend = seeded_backend();
        let bundle = build_bundle(&backend, |platform| {
            Ok(Some(format!("stored-{}", platform)))
        })
        .expect("导出应成功");

        assert_eq!(bundle.version, EXPORT_VERSION);
        assert_eq!(bundle.platforms.len(), 3);
        assert_eq!(bundle.articles.len(), 2);
        assert_eq!(bundle.sync_map.len(), 1);
        assert!(bundle.platforms.contains_key("cnblogs"));
        assert!(bundle.platforms.contains_key("juejin"));
        assert!(bundle.platforms.contains_key("csdn"));
    }

    #[test]
    fn export_merges_stored_cookie_into_platform_without_manual_cookie() {
        let backend = seeded_backend();
        let bundle = build_bundle(&backend, |platform| {
            Ok(Some(format!("stored-{}", platform)))
        })
        .expect("导出应成功");

        let csdn = &bundle.platforms["csdn"];
        assert_eq!(csdn["cookie"], Value::String("stored-csdn".to_string()));
    }

    #[test]
    fn export_prefers_manual_cookie_over_stored_cookie() {
        let backend = seeded_backend();
        let bundle = build_bundle(&backend, |platform| {
            Ok(Some(format!("stored-{}", platform)))
        })
        .expect("导出应成功");

        assert_eq!(
            bundle.platforms["juejin"]["cookie"],
            Value::String("sessionid=manual".to_string())
        );
    }

    #[test]
    fn export_skips_credential_lookup_for_non_cookie_platforms() {
        let mut backend = seeded_backend();
        backend
            .credentials
            .borrow_mut()
            .insert("cnblogs_cookie".to_string(), serde_json::json!("plaintext-token"));
        let calls = std::cell::RefCell::new(Vec::<String>::new());

        let bundle = build_bundle(&backend, |platform| {
            calls.borrow_mut().push(platform.to_string());
            Err(format!("{} 的 Cookie 密文格式非法: 长度不是 4 的倍数", platform))
        })
        .expect("博客园的 MetaWeblog Token 不应阻塞导出");

        assert!(
            !calls.borrow().iter().any(|id| id == "cnblogs"),
            "博客园不是 Cookie 类凭据，不应尝试解密"
        );
        assert!(bundle.platforms["cnblogs"].get("cookie").is_none());
    }

    #[test]
    fn export_survives_broken_cookie_ciphertext() {
        let backend = seeded_backend();
        let bundle = build_bundle(&backend, |platform| {
            Err(format!("{} 的 Cookie 密文格式非法: 长度不是 4 的倍数", platform))
        })
        .expect("单个平台密文损坏不应阻塞整份导出");

        assert!(bundle.platforms.contains_key("csdn"));
        assert!(bundle.platforms["csdn"].get("cookie").is_none());
        assert_eq!(
            bundle.platforms["juejin"]["cookie"],
            Value::String("sessionid=manual".to_string()),
            "手动填写的 Cookie 不受解密失败影响"
        );
    }

    #[test]
    fn cleanup_removes_only_non_cookie_platform_credentials() {
        let mut backend = seeded_backend();
        backend.credentials.borrow_mut().insert(
            "cnblogs_cookie".to_string(),
            Value::String("enc:cnblogs:token".to_string()),
        );
        backend.credentials.borrow_mut().insert(
            "juejin_cookie".to_string(),
            Value::String("enc:juejin:session".to_string()),
        );

        let report = cleanup_invalid_credentials(&backend).expect("清理应成功");
        assert_eq!(report.removed_credential_keys, 1);
        let kept = backend.snapshot(StoreKind::Credentials);
        assert!(!kept.contains_key("cnblogs_cookie"));
        assert!(
            kept.contains_key("juejin_cookie"),
            "合法 cookie 类平台凭据不得被误删"
        );
    }

    #[test]
    fn cleanup_is_noop_when_no_invalid_credentials() {
        let mut backend = seeded_backend();
        backend.credentials.borrow_mut().insert(
            "juejin_cookie".to_string(),
            Value::String("enc:juejin:session".to_string()),
        );

        let report = cleanup_invalid_credentials(&backend).expect("清理应成功");
        assert_eq!(report.removed_credential_keys, 0);
        assert!(backend
            .snapshot(StoreKind::Credentials)
            .contains_key("juejin_cookie"));
    }

    #[test]
    fn export_keeps_article_order_from_index_and_appends_orphans() {
        let mut backend = seeded_backend();
        backend
            .config
            .borrow_mut()
            .insert("articles:a0".to_string(), article("a0", "孤儿"));

        let bundle = build_bundle(&backend, |_| Ok(None)).expect("导出应成功");
        let ids: Vec<String> = bundle
            .articles
            .iter()
            .map(|value| value["id"].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(ids, vec!["a1", "a2", "a0"]);
    }

    #[test]
    fn exported_json_can_be_parsed_back_to_same_structure() {
        let backend = seeded_backend();
        let bundle = build_bundle(&backend, |_| Ok(None)).expect("导出应成功");
        let json = serde_json::to_string_pretty(&bundle).expect("序列化应成功");
        let parsed = parse_bundle(&json).expect("解析应成功");
        assert_eq!(parsed, bundle);
    }

    #[test]
    fn import_success_replaces_previous_data() {
        let backend = seeded_backend();
        let bundle = bundle_from(
            vec![("juejin", serde_json::json!({ "cookie": "sessionid=new" }))],
            vec![article("b1", "新文章")],
            Vec::new(),
        );

        let report =
            import_with_rollback(&backend, &bundle, stub_encrypt).expect("导入应成功");

        assert_eq!(report.platforms, 1);
        assert_eq!(report.articles, 1);
        let config = backend.snapshot(StoreKind::PlatformConfig);
        assert!(!config.contains_key("articles:a1"), "旧文章应被清除");
        assert!(!config.contains_key("cnblogs_config"), "旧平台配置应被清除");
        assert!(config.contains_key("articles:b1"));
        assert_eq!(config["articles:index"], serde_json::json!(["b1"]));
        let credentials = backend.snapshot(StoreKind::Credentials);
        assert_eq!(
            credentials["juejin_cookie"],
            Value::String("enc:juejin:sessionid=new".to_string())
        );
        assert!(!credentials.contains_key("csdn_cookie"), "旧凭据应被清除");
    }

    #[test]
    fn import_writes_encrypted_cookie_and_strips_it_when_absent() {
        let backend = FakeBackend::empty();
        let bundle = bundle_from(
            vec![
                ("juejin", serde_json::json!({ "cookie": "sessionid=x" })),
                ("cnblogs", serde_json::json!({ "token": "t" })),
            ],
            Vec::new(),
            Vec::new(),
        );
        import_with_rollback(&backend, &bundle, stub_encrypt).expect("导入应成功");

        let credentials = backend.snapshot(StoreKind::Credentials);
        assert_eq!(
            credentials["juejin_cookie"],
            Value::String("enc:juejin:sessionid=x".to_string())
        );
        assert!(!credentials.contains_key("cnblogs_cookie"));
        let config = backend.snapshot(StoreKind::PlatformConfig);
        assert!(!config["cnblogs_config"].get("cookie").is_some());
    }

    #[test]
    fn import_rolls_back_when_first_store_write_fails() {
        let backend = seeded_backend();
        let before_config = backend.snapshot(StoreKind::PlatformConfig);
        let before_credentials = backend.snapshot(StoreKind::Credentials);
        let failing = FakeBackend::new(before_config.clone(), before_credentials.clone(), vec![1]);

        let bundle = bundle_from(
            vec![("juejin", serde_json::json!({ "cookie": "sessionid=new" }))],
            vec![article("b1", "新文章")],
            Vec::new(),
        );

        let err = import_with_rollback(&failing, &bundle, stub_encrypt).expect_err("应失败");
        assert!(err.contains("导入失败"));
        assert_eq!(failing.snapshot(StoreKind::PlatformConfig), before_config);
        assert_eq!(failing.snapshot(StoreKind::Credentials), before_credentials);
    }

    #[test]
    fn import_rolls_back_when_second_store_write_fails() {
        let backend = seeded_backend();
        let before_config = backend.snapshot(StoreKind::PlatformConfig);
        let before_credentials = backend.snapshot(StoreKind::Credentials);
        let failing = FakeBackend::new(before_config.clone(), before_credentials.clone(), vec![2]);

        let bundle = bundle_from(
            vec![("juejin", serde_json::json!({ "cookie": "sessionid=new" }))],
            vec![article("b1", "新文章")],
            Vec::new(),
        );

        let err = import_with_rollback(&failing, &bundle, stub_encrypt).expect_err("应失败");
        assert!(
            err.contains("还原"),
            "回滚成功时错误信息应说明数据已还原，实际: {}",
            err
        );
        assert_eq!(failing.snapshot(StoreKind::PlatformConfig), before_config);
        assert_eq!(failing.snapshot(StoreKind::Credentials), before_credentials);
    }

    #[test]
    fn import_reports_when_rollback_itself_fails() {
        let backend = seeded_backend();
        let before_config = backend.snapshot(StoreKind::PlatformConfig);
        let before_credentials = backend.snapshot(StoreKind::Credentials);
        let failing = FakeBackend::new(
            before_config,
            before_credentials,
            vec![1, 2, 3, 4],
        );

        let bundle = bundle_from(
            vec![("juejin", serde_json::json!({ "cookie": "sessionid=new" }))],
            Vec::new(),
            Vec::new(),
        );

        let err = import_with_rollback(&failing, &bundle, stub_encrypt).expect_err("应失败");
        assert!(
            err.contains("回滚未完全成功"),
            "回滚失败必须如实报告，实际: {}",
            err
        );
    }

    #[test]
    fn import_rejects_unsupported_version() {
        let mut bundle = bundle_from(Vec::new(), Vec::new(), Vec::new());
        bundle.version = 99;
        let err = validate_bundle(&bundle).expect_err("不支持的版本应被拒");
        assert!(err.contains("不受支持"));
    }

    #[test]
    fn import_rejects_missing_required_field() {
        let raw = r#"{"version":1,"exportedAt":"1","platforms":{}}"#;
        let err = parse_bundle(raw).expect_err("缺字段应被拒");
        assert!(err.contains("解析失败"));
    }

    #[test]
    fn import_rejects_non_object_platform_config() {
        let bundle = bundle_from(
            vec![("juejin", Value::String("not-an-object".to_string()))],
            Vec::new(),
            Vec::new(),
        );
        assert!(validate_bundle(&bundle).is_err());
    }

    #[test]
    fn import_rejects_duplicate_article_ids() {
        let bundle = bundle_from(
            Vec::new(),
            vec![article("a1", "一"), article("a1", "二")],
            Vec::new(),
        );
        let err = validate_bundle(&bundle).expect_err("重复 id 应被拒");
        assert!(err.contains("重复"));
    }

    #[test]
    fn import_rejects_invalid_sync_map_structure() {
        let bundle = bundle_from(
            Vec::new(),
            Vec::new(),
            vec![("a1", serde_json::json!({ "juejin": { "id": 1 } }))],
        );
        assert!(validate_bundle(&bundle).is_err());
    }

    #[test]
    fn roundtrip_keeps_platforms_articles_and_sync_map() {
        let backend = seeded_backend();
        let bundle = build_bundle(&backend, |_| Ok(None)).expect("导出应成功");

        let target = FakeBackend::empty();
        import_with_rollback(&target, &bundle, stub_encrypt).expect("导入应成功");

        let config = target.snapshot(StoreKind::PlatformConfig);
        assert_eq!(config["cnblogs_config"], bundle.platforms["cnblogs"]);
        assert_eq!(config["articles:index"], serde_json::json!(["a1", "a2"]));
        assert_eq!(config["articles:a1"], bundle.articles[0]);
        assert_eq!(config["syncmap:a1"], bundle.sync_map["a1"]);
        let credentials = target.snapshot(StoreKind::Credentials);
        assert_eq!(
            credentials["juejin_cookie"],
            Value::String("enc:juejin:sessionid=manual".to_string())
        );
    }

    #[test]
    fn reset_clears_articles_syncmaps_and_platform_configs() {
        let mut config = BTreeMap::new();
        config.insert("articles:index".to_string(), serde_json::json!(["a1"]));
        config.insert("articles:a1".to_string(), article("a1", "一"));
        config.insert("syncmap:a1".to_string(), serde_json::json!({}));
        config.insert("juejin_config".to_string(), serde_json::json!({}));
        config.insert("theme".to_string(), serde_json::json!("dark"));
        config.insert("foo_config".to_string(), serde_json::json!({}));

        let keys = collect_reset_keys(StoreKind::PlatformConfig, &config, &["juejin".to_string()]);
        assert_eq!(
            keys,
            vec![
                "articles:a1".to_string(),
                "articles:index".to_string(),
                "foo_config".to_string(),
                "juejin_config".to_string(),
                "syncmap:a1".to_string(),
            ]
        );
        assert!(!keys.contains(&"theme".to_string()), "非业务键不得被清除");
    }

    #[test]
    fn reset_clears_every_cookie_but_keeps_other_credentials() {
        let mut credentials = BTreeMap::new();
        credentials.insert("juejin_cookie".to_string(), serde_json::json!("c1"));
        credentials.insert("zhihu_cookie".to_string(), serde_json::json!("c2"));
        credentials.insert("app_secret".to_string(), serde_json::json!("s"));

        let keys = collect_reset_keys(StoreKind::Credentials, &credentials, &[]);
        assert_eq!(
            keys,
            vec!["juejin_cookie".to_string(), "zhihu_cookie".to_string()]
        );
    }

    #[test]
    fn reset_with_report_removes_keys_and_keeps_preferences() {
        let mut config = BTreeMap::new();
        config.insert("articles:index".to_string(), serde_json::json!(["a1"]));
        config.insert("articles:a1".to_string(), article("a1", "一"));
        config.insert("syncmap:a1".to_string(), serde_json::json!({}));
        config.insert("juejin_config".to_string(), serde_json::json!({}));
        config.insert("ui_pref".to_string(), serde_json::json!("kept"));

        let mut credentials = BTreeMap::new();
        credentials.insert("juejin_cookie".to_string(), serde_json::json!("c1"));

        let backend = FakeBackend::new(config, credentials, Vec::new());
        let report = reset_with_report(&backend, &["juejin".to_string()]).expect("清空应成功");

        assert_eq!(report.config_keys, 4);
        assert_eq!(report.credential_keys, 1);
        let config_after = backend.snapshot(StoreKind::PlatformConfig);
        assert_eq!(config_after.len(), 1);
        assert_eq!(config_after["ui_pref"], serde_json::json!("kept"));
        assert!(backend.snapshot(StoreKind::Credentials).is_empty());
    }

    #[test]
    fn reset_on_empty_store_reports_zero() {
        let backend = FakeBackend::empty();
        let report = reset_with_report(&backend, &[]).expect("清空应成功");
        assert_eq!(report.config_keys, 0);
        assert_eq!(report.credential_keys, 0);
    }
}