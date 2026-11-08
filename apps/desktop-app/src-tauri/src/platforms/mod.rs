use serde::{Deserialize, Serialize};

pub mod cnblogs;
pub mod csdn;
pub mod juejin;
pub mod shared;
pub mod wechat;

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
}

pub const SUPPORTED_PLATFORM_IDS: &[&str] = &[
    "cnblogs",
    "juejin",
    "zhihu",
    "csdn",
    "wechat",
    "segmentfault",
    "tencent",
    "aliyun",
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