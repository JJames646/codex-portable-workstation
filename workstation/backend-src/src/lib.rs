pub mod bridge;
pub mod codex_machine;
pub mod codex_proxy;
pub mod config;
pub mod engine;
pub mod locale;
pub mod locale_init;
pub mod model;
pub mod native_i18n;
pub mod paths;
pub mod plugins;
pub mod protocol;
pub mod provider;
pub mod runtime;
pub mod skills;
pub mod vault;
pub mod windows_process;

/// 启动器内置 Runtime 与外部 supervisor 的源码指纹。
pub const RUNTIME_BUILD_ID: &str = env!("YUNSHU_RUNTIME_BUILD_ID");
