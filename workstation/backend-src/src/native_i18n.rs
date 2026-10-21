use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::paths::WorkstationPaths;

pub const NATIVE_I18N_PAYLOAD: &str = "native-i18n-state-v1.json";
const PAYLOAD_SCHEMA_VERSION: u32 = 1;
const STATSIG_STORAGE_SUFFIX: &str = "685440364";
const DYNAMIC_CONFIG_ID: &str = "118392242";
const LAYER_CONFIG_ID: &str = "72216192";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeI18nPayload {
    schema_version: u32,
    storage_origin: String,
    database: String,
    key_derivation: String,
    statsig_storage_suffix: String,
    records: Vec<NativeI18nRecord>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeI18nRecord {
    kind: NativeI18nRecordKind,
    id: String,
    value_base64: String,
    sha256: String,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum NativeI18nRecordKind {
    DynamicConfig,
    LayerConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HelperInput<'a> {
    action: &'a str,
    module_root: PathBuf,
    database_path: &'a Path,
    statsig_storage_suffix: &'a str,
    dynamic_config: &'a Value,
    layer_config: &'a Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperOutput {
    verified: bool,
    #[serde(default)]
    matched_records: usize,
}

impl NativeI18nPayload {
    fn decoded_records(&self) -> Result<(Value, Value)> {
        if self.schema_version != PAYLOAD_SCHEMA_VERSION
            || self.storage_origin != "_app://-"
            || self.database != "Default/Local Storage/leveldb"
            || self.key_derivation != "statsigDjb2CurrentStableId"
            || self.statsig_storage_suffix != STATSIG_STORAGE_SUFFIX
        {
            bail!("原生中文状态载荷结构不受支持");
        }
        if self.records.len() != 2 {
            bail!("原生中文状态只能包含两个白名单记录");
        }

        let mut dynamic = None;
        let mut layer = None;
        for record in &self.records {
            validate_sha256(&record.sha256)?;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&record.value_base64)
                .context("原生中文状态不是有效的 Base64")?;
            if !sha256_bytes(&decoded).eq_ignore_ascii_case(&record.sha256) {
                bail!("原生中文状态记录哈希校验失败");
            }
            reject_sensitive_decoded_payload(&decoded)?;
            let value: Value =
                serde_json::from_slice(&decoded).context("原生中文状态记录不是有效的 JSON")?;
            validate_i18n_record(record, &value)?;
            match record.kind {
                NativeI18nRecordKind::DynamicConfig => dynamic = Some(value),
                NativeI18nRecordKind::LayerConfig => layer = Some(value),
            }
        }
        Ok((
            dynamic.context("缺少原生中文 dynamic config")?,
            layer.context("缺少原生中文 layer config")?,
        ))
    }
}

pub fn load_payload(path: &Path) -> Result<NativeI18nPayload> {
    let payload: NativeI18nPayload = serde_json::from_slice(
        &fs::read(path).with_context(|| format!("无法读取 {}", path.display()))?,
    )
    .with_context(|| format!("无法解析 {}", path.display()))?;
    payload.decoded_records()?;
    Ok(payload)
}

pub fn apply(
    paths: &WorkstationPaths,
    database_path: &Path,
    payload: &NativeI18nPayload,
) -> Result<()> {
    fs::create_dir_all(database_path)
        .with_context(|| format!("无法创建 {}", database_path.display()))?;
    let output = run_helper(paths, "apply", database_path, payload)?;
    if !output.verified || output.matched_records == 0 {
        bail!("原生中文状态写入后校验失败");
    }
    Ok(())
}

pub fn verify(
    paths: &WorkstationPaths,
    database_path: &Path,
    payload: &NativeI18nPayload,
) -> Result<bool> {
    if !database_path.is_dir() {
        return Ok(false);
    }
    Ok(run_helper(paths, "verify", database_path, payload)?.verified)
}

pub fn apply_with_local_runtime(
    paths: &WorkstationPaths,
    database_path: &Path,
    payload: &NativeI18nPayload,
    module_root: &Path,
    helper_root: &Path,
) -> Result<()> {
    fs::create_dir_all(database_path)
        .with_context(|| format!("无法创建 {}", database_path.display()))?;
    let output = run_helper_internal(
        paths,
        "apply",
        database_path,
        payload,
        Some((module_root, helper_root)),
    )?;
    if !output.verified || output.matched_records == 0 {
        bail!("原生中文状态写入后校验失败");
    }
    Ok(())
}

fn run_helper(
    paths: &WorkstationPaths,
    action: &str,
    database_path: &Path,
    payload: &NativeI18nPayload,
) -> Result<HelperOutput> {
    run_helper_internal(paths, action, database_path, payload, None)
}

fn run_helper_internal(
    paths: &WorkstationPaths,
    action: &str,
    database_path: &Path,
    payload: &NativeI18nPayload,
    local_runtime: Option<(&Path, &Path)>,
) -> Result<HelperOutput> {
    #[cfg(test)]
    if fs::read_to_string(&paths.bundled_node)
        .ok()
        .is_some_and(|value| value == "YUNSHU_NATIVE_I18N_TEST_OK")
    {
        return Ok(HelperOutput {
            verified: true,
            matched_records: 2,
        });
    }
    if !paths.bundled_node.is_file() {
        bail!("随盘 Node.js 不存在，无法处理 Codex 原生中文状态");
    }
    let (module_root, helper_root) = if let Some((module_root, helper_root)) = local_runtime {
        (module_root.to_path_buf(), helper_root.to_path_buf())
    } else {
        let module_root = paths
            .codex_desktop
            .parent()
            .context("Codex 主体路径不完整")?
            .join(
                "resources/plugins/openai-bundled/plugins/browser/scripts/node_modules/classic-level",
            );
        (module_root, paths.runtime_state.join("locale-init"))
    };
    if !module_root.join("index.js").is_file() {
        bail!("Codex 随附 classic-level 不存在");
    }
    let (dynamic_config, layer_config) = payload.decoded_records()?;
    let input = HelperInput {
        action,
        module_root,
        database_path,
        statsig_storage_suffix: STATSIG_STORAGE_SUFFIX,
        dynamic_config: &dynamic_config,
        layer_config: &layer_config,
    };

    fs::create_dir_all(&helper_root)
        .with_context(|| format!("无法创建 {}", helper_root.display()))?;
    let helper_path = helper_root.join("native-i18n-leveldb.cjs");
    fs::write(&helper_path, LEVELDB_HELPER.as_bytes())
        .with_context(|| format!("无法写入 {}", helper_path.display()))?;

    let mut child = Command::new(&paths.bundled_node)
        .arg(&helper_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法启动 Codex 原生中文状态处理器")?;
    child
        .stdin
        .take()
        .context("无法连接原生中文状态处理器")?
        .write_all(&serde_json::to_vec(&input)?)?;
    let output = child.wait_with_output()?;
    let _ = fs::remove_file(&helper_path);
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!("Codex 原生中文状态处理失败：{error}");
    }
    serde_json::from_slice(&output.stdout).context("原生中文状态处理器返回无效结果")
}

fn validate_i18n_record(record: &NativeI18nRecord, value: &Value) -> Result<()> {
    let object = value.as_object().context("原生中文记录不是对象")?;
    let allowed = [
        "name",
        "rule_id",
        "secondary_exposures",
        "version",
        "value",
        "id_type",
        "is_device_based",
        "is_in_layer",
        "explicit_parameters",
        "is_experiment_active",
        "is_user_in_experiment",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        bail!("原生中文记录包含未授权字段");
    }
    let localized = object
        .get("value")
        .and_then(Value::as_object)
        .context("原生中文记录 value 不是对象")?;
    if localized.len() != 2
        || localized.get("enable_i18n").and_then(Value::as_bool) != Some(true)
        || localized.get("locale_source").and_then(Value::as_str) != Some("IDE")
        || object.get("name").and_then(Value::as_str) != Some(record.id.as_str())
        || object
            .get("secondary_exposures")
            .and_then(Value::as_array)
            .is_none_or(|items| !items.is_empty())
    {
        bail!("原生中文记录没有启用官方中文状态");
    }
    match record.kind {
        NativeI18nRecordKind::DynamicConfig
            if record.id == DYNAMIC_CONFIG_ID
                && object.get("rule_id").and_then(Value::as_str) == Some("targetingGate")
                && object.get("is_in_layer").and_then(Value::as_bool) == Some(true) =>
        {
            Ok(())
        }
        NativeI18nRecordKind::LayerConfig
            if record.id == LAYER_CONFIG_ID
                && object.get("rule_id").and_then(Value::as_str) == Some("default") =>
        {
            Ok(())
        }
        _ => bail!("原生中文记录 ID 不在白名单中"),
    }
}

fn reject_sensitive_decoded_payload(bytes: &[u8]) -> Result<()> {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    let forbidden = [
        "cookie",
        "email",
        "access_token",
        "refresh_token",
        "api_key",
        "authorization",
        "device_id",
        "installation_id",
        "project",
        "conversation",
        "c:\\",
        "d:\\",
        "e:\\",
        "f:\\",
        "g:\\",
    ];
    if forbidden.iter().any(|value| text.contains(value)) {
        bail!("原生中文状态包含敏感字段或个人路径");
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        bail!("SHA-256格式不正确");
    }
    Ok(())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

const LEVELDB_HELPER: &str = r#"
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const input = JSON.parse(fs.readFileSync(0, 'utf8'));
const { ClassicLevel } = require(path.join(input.moduleRoot, 'index.js'));
const prefix = '_app://-\0\x01';

function decodeValue(value) {
  if (value.length > 1 && value[0] === 0) return value.subarray(1).toString('utf16le');
  if (value.length > 1 && value[0] === 1) return value.subarray(1).toString('utf8');
  return value.toString('utf8');
}

function encodeUtf16Json(value) {
  return Buffer.concat([Buffer.from([0]), Buffer.from(JSON.stringify(value), 'utf16le')]);
}

function encodeUtf8Json(value) {
  return Buffer.concat([Buffer.from([1]), Buffer.from(JSON.stringify(value), 'utf8')]);
}

function djb2(text) {
  let hash = 0;
  for (let index = 0; index < text.length; index += 1) {
    hash = ((hash << 5) - hash + text.charCodeAt(index)) | 0;
  }
  return String(hash >>> 0);
}

function cacheSuffix(userId, stableId) {
  const customIds = { stableID: stableId, source_surface_stable_id: stableId };
  const cids = Object.keys(customIds).sort().map((key) => `${key}-${customIds[key]}`).join(',');
  return djb2(`uid:${userId}|cids:${cids}|k:client-sYWqzCYMRkUg4DqqiZcR5DGTNl2iD7zNJY0HoeDLzxR`);
}

function minimalEvaluations() {
  return {
    feature_gates: {},
    dynamic_configs: { '118392242': input.dynamicConfig },
    layer_configs: { '72216192': input.layerConfig },
    live_entity_names: {},
    param_stores: {},
    time: 0,
    has_updates: true,
    evaluated_keys: {},
    sdkParams: {},
    generator: 'yunshu-local-i18n-seed',
    hash_used: 'djb2',
    target_app_used: 'codex'
  };
}

function patchWrapper(wrapper) {
  let evaluations;
  try { evaluations = JSON.parse(wrapper.data); } catch { evaluations = minimalEvaluations(); }
  evaluations.dynamic_configs = evaluations.dynamic_configs || {};
  evaluations.layer_configs = evaluations.layer_configs || {};
  evaluations.dynamic_configs['118392242'] = input.dynamicConfig;
  evaluations.layer_configs['72216192'] = input.layerConfig;
  wrapper.data = JSON.stringify(evaluations);
  wrapper.receivedAt = Date.now();
  return wrapper;
}

function isVerified(wrapper) {
  try {
    const evaluations = JSON.parse(wrapper.data);
    const dynamic = evaluations.dynamic_configs?.['118392242'];
    const layer = evaluations.layer_configs?.['72216192'];
    return dynamic?.value?.enable_i18n === true && dynamic?.value?.locale_source === 'IDE' &&
      layer?.value?.enable_i18n === true && layer?.value?.locale_source === 'IDE';
  } catch { return false; }
}

(async () => {
  const writable = input.action === 'apply';
  if (writable && !fs.existsSync(input.databasePath)) {
    fs.mkdirSync(input.databasePath, { recursive: true });
  }
  const db = new ClassicLevel(input.databasePath, {
    keyEncoding: 'buffer',
    valueEncoding: 'buffer',
    readOnly: !writable,
  });
  let generatedStableId = false;
  let matchedRecords = 0;
  try {
    await db.open();
    const stableKey = Buffer.from(`${prefix}statsig.stable_id.${input.statsigStorageSuffix}`, 'utf8');
    let stableId;
    try {
      const raw = decodeValue(await db.get(stableKey));
      stableId = JSON.parse(raw);
    } catch {
      if (!writable) {
        process.stdout.write(JSON.stringify({ verified: false, generatedStableId, matchedRecords }));
        return;
      }
      stableId = crypto.randomUUID();
      generatedStableId = true;
      await db.put(stableKey, encodeUtf8Json(stableId));
    }
    const suffixes = [cacheSuffix('', stableId), cacheSuffix(`ua-${stableId}`, stableId)];
    for (const suffix of suffixes) {
      const cacheKey = Buffer.from(`${prefix}statsig.cached.evaluations.${suffix}`, 'utf8');
      let wrapper;
      try { wrapper = JSON.parse(decodeValue(await db.get(cacheKey))); } catch {
        wrapper = { source: 'Bootstrap', data: JSON.stringify(minimalEvaluations()), receivedAt: Date.now() };
      }
      if (writable) await db.put(cacheKey, encodeUtf16Json(patchWrapper(wrapper)));
      const current = writable ? JSON.parse(decodeValue(await db.get(cacheKey))) : wrapper;
      if (isVerified(current)) matchedRecords += 1;
    }
  } finally {
    await db.close();
  }
  process.stdout.write(JSON.stringify({ verified: matchedRecords > 0, generatedStableId, matchedRecords }));
})().catch((error) => {
  process.stderr.write(String(error && error.stack ? error.stack : error));
  process.exitCode = 1;
});
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: NativeI18nRecordKind, id: &str, value: Value) -> NativeI18nRecord {
        let bytes = serde_json::to_vec(&value).unwrap();
        NativeI18nRecord {
            kind,
            id: id.to_string(),
            value_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
            sha256: sha256_bytes(&bytes),
        }
    }

    #[test]
    fn accepts_only_the_two_minimal_i18n_records() {
        let value = |id: &str, rule_id: &str, is_in_layer: bool| {
            serde_json::json!({
                "name": id,
                "rule_id": rule_id,
                "secondary_exposures": [],
                "version": 1,
                "value": {"enable_i18n": true, "locale_source": "IDE"},
                "id_type": "userID",
                "is_device_based": false,
                "explicit_parameters": [],
                "is_in_layer": is_in_layer
            })
        };
        let payload = NativeI18nPayload {
            schema_version: 1,
            storage_origin: "_app://-".to_string(),
            database: "Default/Local Storage/leveldb".to_string(),
            key_derivation: "statsigDjb2CurrentStableId".to_string(),
            statsig_storage_suffix: STATSIG_STORAGE_SUFFIX.to_string(),
            records: vec![
                record(
                    NativeI18nRecordKind::DynamicConfig,
                    DYNAMIC_CONFIG_ID,
                    value(DYNAMIC_CONFIG_ID, "targetingGate", true),
                ),
                record(
                    NativeI18nRecordKind::LayerConfig,
                    LAYER_CONFIG_ID,
                    value(LAYER_CONFIG_ID, "default", false),
                ),
            ],
        };
        assert!(payload.decoded_records().is_ok());
    }

    #[test]
    fn rejects_sensitive_values_after_base64_decoding() {
        assert!(reject_sensitive_decoded_payload(br#"{"cookie":"secret"}"#).is_err());
        assert!(reject_sensitive_decoded_payload(br#"{"path":"C:\\Users\\James"}"#).is_err());
    }
}
