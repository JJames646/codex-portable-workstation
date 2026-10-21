use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::paths::WorkstationPaths;

pub const CODEX_LOCALE: &str = "zh-CN";
const SELECTED_LANGUAGES: &str = "zh-CN,zh";
const OWL_BOOTSTRAP_FILE: &str = "owl-feature-bootstrap-cache.json";
const I18N_VERIFICATION_MARKER: &str = "codex-i18n-verified-v1.json";
const REQUIRED_ENABLED_OWL_FEATURES: [&str; 6] = [
    "OwlAuth",
    "OwlAutofillAndPasswords",
    "OwlDownloads",
    "OwlPermissions",
    "OwlWebViewEnhancements",
    "OwlPrinting",
];
const REQUIRED_DISABLED_OWL_FEATURES: [&str; 3] =
    ["OwlExtensions", "OwlHistory", "OwlOpenAIGoLinks"];
const SAMPLE_PROFILE_PREFERENCES: [(&str, &str); 3] = [
    ("Default", "Default/Preferences"),
    ("codex-browser-app", "codex-browser-app/Preferences"),
    (
        "Default/Partitions/codex-browser-app",
        "Default/Partitions/codex-browser-app/Preferences",
    ),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocaleStatus {
    pub language_pack: PathBuf,
    pub language_pack_present: bool,
    pub config_locale: Option<String>,
    pub profile_locales: Vec<ProfileLocaleStatus>,
    pub feature_bootstrap: CodexFeatureBootstrapStatus,
    pub launch_language_argument: Option<String>,
    pub native_i18n_verified: bool,
    pub native_i18n_state: CodexNativeI18nState,
    pub verification_marker: PathBuf,
    pub ready: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CodexNativeI18nState {
    Missing,
    WrittenPendingVerification,
    Verified,
    VersionMismatch,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexFeatureBootstrapStatus {
    pub path: PathBuf,
    pub present: bool,
    pub enabled_owl_features: Vec<String>,
    pub disabled_owl_features: Vec<String>,
    pub ready: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OwlFeatureBootstrap {
    #[serde(default)]
    enabled_owl_feature_names: Vec<String>,
    #[serde(default)]
    disabled_owl_feature_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileLocaleStatus {
    pub profile: String,
    pub preferences: PathBuf,
    pub selected_languages: Option<String>,
    pub ready: bool,
}

pub fn ensure_codex_locale(paths: &WorkstationPaths) -> Result<CodexLocaleStatus> {
    let language_pack = language_pack_path(paths);
    if !language_pack.is_file() {
        bail!("Codex官方中文语言包不存在：{}", language_pack.display());
    }
    // 这里只做只读检查。官方原生中文还受远端 enable_i18n 功能开关及 Profile
    // 缓存控制，不能通过伪造 Owl 功能文件或强塞 --lang 参数来宣称“已中文化”。
    codex_locale_status(paths)
}

pub fn codex_locale_status(paths: &WorkstationPaths) -> Result<CodexLocaleStatus> {
    let language_pack = language_pack_path(paths);
    let config_locale = read_config_locale(&paths.codex_home.join("config.toml"))?;
    let mut profile_locales = Vec::new();
    for (profile, relative_preferences) in SAMPLE_PROFILE_PREFERENCES {
        let preferences = paths.codex_browser_data.join(relative_preferences);
        let selected_languages = read_profile_locale(&preferences)?;
        profile_locales.push(ProfileLocaleStatus {
            profile: profile.to_string(),
            preferences,
            ready: selected_languages.as_deref() == Some(SELECTED_LANGUAGES),
            selected_languages,
        });
    }
    let feature_bootstrap = feature_bootstrap_status(paths)?;
    let verification_marker = paths.runtime_state.join(I18N_VERIFICATION_MARKER);
    let native_i18n_state = crate::locale_init::native_i18n_state(paths);
    let native_i18n_verified = native_i18n_state == CodexNativeI18nState::Verified;
    let ready = language_pack.is_file()
        && config_locale.as_deref() == Some(CODEX_LOCALE)
        && profile_locales.iter().all(|profile| profile.ready)
        && feature_bootstrap.ready
        && native_i18n_verified;
    Ok(CodexLocaleStatus {
        language_pack_present: language_pack.is_file(),
        language_pack,
        config_locale,
        profile_locales,
        feature_bootstrap,
        // F 样本主进程没有显式 --lang 参数。渲染进程的语言由 Chromium/Profile
        // 自己派生，因此这里必须保持 None，避免诊断再次把无效参数当成成功条件。
        launch_language_argument: None,
        native_i18n_verified,
        native_i18n_state,
        verification_marker,
        ready,
    })
}

fn feature_bootstrap_path(paths: &WorkstationPaths) -> PathBuf {
    paths.codex_browser_data.join(OWL_BOOTSTRAP_FILE)
}

fn feature_bootstrap_status(paths: &WorkstationPaths) -> Result<CodexFeatureBootstrapStatus> {
    let path = feature_bootstrap_path(paths);
    if !path.is_file() {
        return Ok(CodexFeatureBootstrapStatus {
            path,
            present: false,
            enabled_owl_features: Vec::new(),
            disabled_owl_features: Vec::new(),
            ready: false,
        });
    }
    let bytes = fs::read(&path).with_context(|| format!("无法读取 {}", path.display()))?;
    let bootstrap: OwlFeatureBootstrap =
        serde_json::from_slice(&bytes).with_context(|| format!("无法解析 {}", path.display()))?;
    let ready = REQUIRED_ENABLED_OWL_FEATURES.iter().all(|required| {
        bootstrap
            .enabled_owl_feature_names
            .iter()
            .any(|candidate| candidate == required)
    }) && REQUIRED_DISABLED_OWL_FEATURES.iter().all(|required| {
        bootstrap
            .disabled_owl_feature_names
            .iter()
            .any(|candidate| candidate == required)
    });
    Ok(CodexFeatureBootstrapStatus {
        path,
        present: true,
        enabled_owl_features: bootstrap.enabled_owl_feature_names,
        disabled_owl_features: bootstrap.disabled_owl_feature_names,
        ready,
    })
}

fn language_pack_path(paths: &WorkstationPaths) -> PathBuf {
    paths
        .codex_desktop
        .parent()
        .unwrap_or(&paths.root)
        .join("locales")
        .join("zh-CN.pak")
}

fn read_profile_locale(path: &std::path::Path) -> Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("无法读取 {}", path.display()))?;
    let root: serde_json::Value =
        serde_json::from_slice(&bytes).with_context(|| format!("无法解析 {}", path.display()))?;
    Ok(root
        .get("intl")
        .and_then(serde_json::Value::as_object)
        .and_then(|intl| intl.get("selected_languages"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned))
}

fn read_config_locale(path: &std::path::Path) -> Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).with_context(|| format!("无法读取 {}", path.display()))?;
    let document = text
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("无法解析 {}", path.display()))?;
    Ok(document
        .get("desktop")
        .and_then(toml_edit::Item::as_table)
        .and_then(|desktop| desktop.get("localeOverride"))
        .and_then(toml_edit::Item::as_value)
        .and_then(toml_edit::Value::as_str)
        .map(ToOwned::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reads_sample_profile_locale_without_rewriting_it() {
        let temporary = tempdir().unwrap();
        let preferences = temporary.path().join("Default").join("Preferences");
        fs::create_dir_all(preferences.parent().unwrap()).unwrap();
        fs::write(
            &preferences,
            r#"{"intl":{"selected_languages":"zh-CN,zh"},"profile":{"name":"便携用户"}}"#
                .as_bytes(),
        )
        .unwrap();

        let before = fs::read(&preferences).unwrap();
        assert_eq!(
            read_profile_locale(&preferences).unwrap().as_deref(),
            Some(SELECTED_LANGUAGES)
        );
        assert_eq!(fs::read(&preferences).unwrap(), before);
    }

    #[test]
    fn locale_check_does_not_create_an_owl_bootstrap() {
        let temporary = tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("中文 工作站"));
        let status = feature_bootstrap_status(&paths).unwrap();
        assert!(!status.present);
        assert!(!status.ready);
        assert!(!feature_bootstrap_path(&paths).exists());
    }
}
