use serde::{Deserialize, Serialize};

pub mod aliyun;
pub mod baijiahao;
pub mod bilibili;
pub mod cnblogs;
pub mod csdn;
pub mod fifty_one_cto;
pub mod juejin;
pub mod netease;
pub mod oschina;
pub mod shared;
pub mod tencentcloud;
pub mod toutiao;
pub mod wechat;
pub mod weibo;
pub mod webview;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    NotConfigured,
    AuthInvalid,
    PlatformError,
    NetworkError,
    NotImplemented,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformError {
    pub class: FailureClass,
    pub message: String,
}

impl PlatformError {
    pub fn new(class: FailureClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
        }
    }

    pub fn not_configured(message: impl Into<String>) -> Self {
        Self::new(FailureClass::NotConfigured, message)
    }

    #[cfg(test)]
    pub fn platform_error(message: impl Into<String>) -> Self {
        Self::new(FailureClass::PlatformError, message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PublishOperation {
    Create,
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResult {
    pub success: bool,
    pub post_id: String,
    pub edit_url: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<FailureClass>,
    pub operation: PublishOperation,
    #[serde(rename = "isPublished")]
    pub is_published: bool,
    /// 草稿模式落库的远端草稿 ID。部分平台（小红书）存草稿不返回正式文章 ID，
    /// 但必须把这个 ID 记下来，下次同步才能覆盖同一条草稿而不是新建。
    #[serde(rename = "draftId", skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<String>,
}

impl PublishResult {
    pub fn success(
        post_id: impl Into<String>,
        edit_url: impl Into<String>,
        message: impl Into<String>,
        operation: PublishOperation,
        is_published: bool,
    ) -> Self {
        Self {
            success: true,
            post_id: post_id.into(),
            edit_url: edit_url.into(),
            message: message.into(),
            failure_class: None,
            operation,
            is_published,
            draft_id: None,
        }
    }

    pub fn failure(
        class: FailureClass,
        message: impl Into<String>,
        operation: PublishOperation,
        is_published: bool,
    ) -> Self {
        Self {
            success: false,
            post_id: String::new(),
            edit_url: String::new(),
            message: message.into(),
            failure_class: Some(class),
            operation,
            is_published,
            draft_id: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PublishRequest {
    pub title: String,
    pub content: String,
    pub existing_id: Option<String>,
    pub is_update: bool,
    pub publish_status: bool,
    pub existing_published: bool,
    /// 逗号分隔的标签串，如 `"技术,开源"`。`None` 表示用户未填写。
    /// 各平台对标签的强制程度不同：CSDN 草稿可为空、发布必须非空。
    pub tags: Option<String>,
    /// 上一次草稿模式留下的远端草稿 ID（目前仅小红书使用）。与 `existing_id`
    /// 分开存放：`existing_id` 指向正式文章，草稿 ID 只在草稿箱内有效。
    pub existing_draft_id: Option<String>,
}

pub const SUPPORTED_PLATFORM_IDS: &[&str] = &[
    "cnblogs",
    "juejin",
    "zhihu",
    "csdn",
    "wechat",
    "segmentfault",
    "tencentcloud",
    "aliyun",
    "jianshu",
    "51cto",
    "oschina",
    "baijiahao",
    "netease",
    "bilibili",
    "weibo",
    "toutiao",
];

pub async fn dispatch(
    app: &tauri::AppHandle,
    platform: &str,
    request: &PublishRequest,
) -> PublishResult {
    let operation = if request.is_update {
        PublishOperation::Update
    } else {
        PublishOperation::Create
    };

    match platform {
        "cnblogs" => cnblogs::publish(app, request, operation).await,
        "csdn" => csdn::publish(app, request, operation).await,
        "juejin" => juejin::publish(app, request, operation).await,
        "wechat" => wechat::publish(app, request, operation).await,
        "zhihu" => webview::zhihu::run(app, request, operation).await,
        "jianshu" => webview::jianshu::run(app, request, operation).await,
        "segmentfault" => webview::segmentfault::run(app, request, operation).await,
        "xiaohongshu" => webview::xiaohongshu::run(app, request, operation).await,
        "51cto" => fifty_one_cto::publish(app, request, operation).await,
        "oschina" => oschina::publish(app, request, operation).await,
        "tencentcloud" => tencentcloud::publish(app, request, operation).await,
        "aliyun" => aliyun::publish(app, request, operation).await,
        "baijiahao" => baijiahao::publish(app, request, operation).await,
        "netease" => netease::publish(app, request, operation).await,
        "bilibili" => bilibili::publish(app, request, operation).await,
        "weibo" => weibo::publish(app, request, operation).await,
        "toutiao" => toutiao::publish(app, request, operation).await,
        id if SUPPORTED_PLATFORM_IDS.contains(&id) => PublishResult::failure(
            FailureClass::NotImplemented,
            format!("{} 的适配器尚未实现", id),
            operation,
            false,
        ),
        _ => PublishResult::failure(
            FailureClass::Unknown,
            format!("未知平台: {}", platform),
            operation,
            false,
        ),
    }
}