use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub fn from_message(message: impl Into<String>) -> Self {
        let message = message.into();
        let code = classify(&message).to_string();
        Self { code, message }
    }
}

impl From<anyhow::Error> for CommandError {
    fn from(error: anyhow::Error) -> Self {
        Self::from_message(error.to_string())
    }
}

impl From<std::io::Error> for CommandError {
    fn from(error: std::io::Error) -> Self {
        Self::from_message(error.to_string())
    }
}

fn classify(message: &str) -> &'static str {
    let normalized = message.to_lowercase();
    if normalized.contains("主体不完整") || normalized.contains("缺少受控启动后端") {
        "payload_missing"
    } else if normalized.contains("尚未配置") || normalized.contains("首次保存供应商") {
        "provider_missing"
    } else if normalized.contains("凭据") || normalized.contains("api key") {
        "credential_missing"
    } else if normalized.contains("协议") {
        "protocol_unsupported"
    } else if normalized.contains("端口") {
        "bridge_port_busy"
    } else if normalized.contains("相对路径")
        || normalized.contains("越出")
        || normalized.contains("不在工作站")
    {
        "path_not_portable"
    } else if normalized.contains("只读") || normalized.contains("permission denied") {
        "disk_read_only"
    } else if normalized.contains("空间") || normalized.contains("no space") {
        "disk_full"
    } else if normalized.contains("事务") || normalized.contains("repair") {
        "transaction_recovery_required"
    } else if normalized.contains("教程") && normalized.contains("许可") {
        "tutorial_not_licensed"
    } else if normalized.contains("网络")
        || normalized.contains("连接")
        || normalized.contains("http")
    {
        "network_error"
    } else {
        "operation_failed"
    }
}

pub type CommandResult<T> = Result<T, CommandError>;
