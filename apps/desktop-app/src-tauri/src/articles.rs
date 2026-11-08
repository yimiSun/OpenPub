use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri_plugin_store::StoreExt;

const ARTICLE_INDEX_KEY: &str = "articles:index";
const ARTICLE_KEY_PREFIX: &str = "articles:";
const SYNCMAP_KEY_PREFIX: &str = "syncmap:";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    pub id: String,
    pub title: String,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncEntry {
    pub id: String,
    pub published: bool,
    pub body_hash: String,
}

pub type SyncMapRecord = std::collections::BTreeMap<String, SyncEntry>;

pub fn normalize_content(content: &str) -> String {
    content.replace("\r\n", "\n").trim().to_string()
}

pub fn body_hash(content: &str) -> String {
    let normalized = normalize_content(content);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn article_key(id: &str) -> String {
    format!("{}{}", ARTICLE_KEY_PREFIX, id)
}

fn syncmap_key(article_id: &str) -> String {
    format!("{}{}", SYNCMAP_KEY_PREFIX, article_id)
}

fn read_index(store: &std::sync::Arc<tauri_plugin_store::Store<tauri::Wry>>) -> Vec<String> {
    store
        .get(ARTICLE_INDEX_KEY)
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default()
}

fn write_index(
    store: &std::sync::Arc<tauri_plugin_store::Store<tauri::Wry>>,
    ids: &[String],
) -> Result<(), String> {
    store.set(
        ARTICLE_INDEX_KEY,
        serde_json::to_value(ids).map_err(|e| format!("序列化文章索引失败: {}", e))?,
    );
    store.save().map_err(|e| format!("保存文章索引失败: {}", e))
}

fn open_store(app: &tauri::AppHandle) -> Result<std::sync::Arc<tauri_plugin_store::Store<tauri::Wry>>, String> {
    app.store("platform_config.json")
        .map_err(|e| format!("打开配置存储失败: {}", e))
}

fn now_string() -> String {
    format!("{:?}", std::time::SystemTime::now())
}

#[tauri::command]
pub async fn list_articles(app: tauri::AppHandle) -> Result<Vec<Article>, String> {
    let store = open_store(&app)?;
    let ids = read_index(&store);
    let mut articles = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(value) = store.get(&article_key(&id)) {
            if let Ok(article) = serde_json::from_value::<Article>(value.clone()) {
                articles.push(article);
            }
        }
    }
    Ok(articles)
}

#[tauri::command]
pub async fn create_article(
    app: tauri::AppHandle,
    title: String,
    content: String,
) -> Result<Article, String> {
    let store = open_store(&app)?;
    let id = uuid::Uuid::new_v4().to_string();
    let timestamp = now_string();
    let article = Article {
        id: id.clone(),
        title,
        content,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    };
    store.set(
        &article_key(&id),
        serde_json::to_value(&article).map_err(|e| format!("序列化文章失败: {}", e))?,
    );
    let mut ids = read_index(&store);
    ids.push(id);
    write_index(&store, &ids)?;
    store.save().map_err(|e| format!("保存文章失败: {}", e))?;
    Ok(article)
}

#[tauri::command]
pub async fn update_article(
    app: tauri::AppHandle,
    id: String,
    title: String,
    content: String,
) -> Result<Article, String> {
    let store = open_store(&app)?;
    let existing = store
        .get(&article_key(&id))
        .ok_or_else(|| format!("文章不存在: {}", id))?;
    let mut article = serde_json::from_value::<Article>(existing.clone())
        .map_err(|e| format!("解析文章失败: {}", e))?;
    article.title = title;
    article.content = content;
    article.updated_at = now_string();
    store.set(
        &article_key(&id),
        serde_json::to_value(&article).map_err(|e| format!("序列化文章失败: {}", e))?,
    );
    store.save().map_err(|e| format!("保存文章失败: {}", e))?;
    Ok(article)
}

#[tauri::command]
pub async fn delete_article(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let store = open_store(&app)?;
    store.delete(&article_key(&id));
    store.delete(&syncmap_key(&id));
    let ids: Vec<String> = read_index(&store)
        .into_iter()
        .filter(|existing| existing != &id)
        .collect();
    write_index(&store, &ids)?;
    store.save().map_err(|e| format!("保存文章失败: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn load_sync_map(app: tauri::AppHandle, article_id: String) -> Result<SyncMapRecord, String> {
    let store = open_store(&app)?;
    let record = store
        .get(&syncmap_key(&article_id))
        .and_then(|v| serde_json::from_value::<SyncMapRecord>(v.clone()).ok())
        .unwrap_or_default();
    Ok(record)
}

#[tauri::command]
pub async fn save_sync_entry(
    app: tauri::AppHandle,
    article_id: String,
    platform: String,
    entry: SyncEntry,
) -> Result<(), String> {
    let store = open_store(&app)?;
    let mut record = store
        .get(&syncmap_key(&article_id))
        .and_then(|v| serde_json::from_value::<SyncMapRecord>(v.clone()).ok())
        .unwrap_or_default();
    record.insert(platform, entry);
    store.set(
        &syncmap_key(&article_id),
        serde_json::to_value(&record).map_err(|e| format!("序列化映射失败: {}", e))?,
    );
    store.save().map_err(|e| format!("保存映射失败: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn clear_sync_entry(
    app: tauri::AppHandle,
    article_id: String,
    platform: String,
) -> Result<(), String> {
    let store = open_store(&app)?;
    let mut record = store
        .get(&syncmap_key(&article_id))
        .and_then(|v| serde_json::from_value::<SyncMapRecord>(v.clone()).ok())
        .unwrap_or_default();
    record.remove(&platform);
    store.set(
        &syncmap_key(&article_id),
        serde_json::to_value(&record).map_err(|e| format!("序列化映射失败: {}", e))?,
    );
    store.save().map_err(|e| format!("保存映射失败: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_content_yields_same_hash() {
        assert_eq!(body_hash("# 标题\n正文"), body_hash("# 标题\n正文"));
    }

    #[test]
    fn whitespace_differences_yield_same_hash() {
        assert_eq!(body_hash("  # 标题\n正文  \n"), body_hash("# 标题\n正文"));
    }

    #[test]
    fn crlf_and_lf_yield_same_hash() {
        assert_eq!(body_hash("a\r\nb"), body_hash("a\nb"));
    }

    #[test]
    fn content_change_yields_different_hash() {
        assert_ne!(body_hash("正文 A"), body_hash("正文 B"));
    }

    #[test]
    fn hash_is_lowercase_hex_of_64_chars() {
        let hash = body_hash("内容");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn key_prefixes_do_not_overlap() {
        let article_id = "abc-uuid";
        assert_eq!(syncmap_key(article_id), "syncmap:abc-uuid");
        assert_eq!(article_key(article_id), "articles:abc-uuid");
        assert_ne!(syncmap_key(article_id), format!("{}_config", "cnblogs"));
        assert_ne!(syncmap_key(article_id), format!("{}_cookie", "cnblogs"));
    }

    #[test]
    fn syncmap_value_shape_round_trips() {
        let mut record = SyncMapRecord::new();
        record.insert(
            "cnblogs".to_string(),
            SyncEntry {
                id: "123".to_string(),
                published: false,
                body_hash: body_hash("正文"),
            },
        );
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["cnblogs"]["published"], serde_json::json!(false));
        assert!(json["cnblogs"]["bodyHash"].is_string());
        let parsed: SyncMapRecord = serde_json::from_value(json).unwrap();
        assert_eq!(parsed["cnblogs"].id, "123");
    }

    #[test]
    fn article_serializes_with_camel_case() {
        let article = Article {
            id: "id-1".to_string(),
            title: "标题".to_string(),
            content: "正文".to_string(),
            created_at: "t1".to_string(),
            updated_at: "t2".to_string(),
        };
        let json = serde_json::to_value(&article).unwrap();
        assert!(json["createdAt"].is_string());
        assert!(json["updatedAt"].is_string());
    }
}
