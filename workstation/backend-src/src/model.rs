use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Engine {
    Codex,
    Claude,
    #[serde(rename = "openclaw", alias = "open-claw")]
    OpenClaw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SkillTarget {
    Codex,
    Claude,
    #[serde(rename = "openclaw", alias = "open-claw")]
    OpenClaw,
}

impl SkillTarget {
    pub const ALL: [Self; 3] = [Self::Codex, Self::Claude, Self::OpenClaw];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenClaw => "openclaw",
        }
    }
}

impl fmt::Display for SkillTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.slug())
    }
}

impl FromStr for SkillTarget {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" | "claude-code" | "cloudcode" => Ok(Self::Claude),
            "openclaw" | "open-code" | "opencode" => Ok(Self::OpenClaw),
            other => bail!("不支持的Skill目标：{other}"),
        }
    }
}

impl Engine {
    pub const ALL: [Self; 3] = [Self::Codex, Self::Claude, Self::OpenClaw];

    pub fn slug(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenClaw => "openclaw",
        }
    }
}

impl fmt::Display for Engine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.slug())
    }
}

impl FromStr for Engine {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" | "claude-code" | "cloudcode" => Ok(Self::Claude),
            "openclaw" | "open-code" | "opencode" => Ok(Self::OpenClaw),
            other => bail!("不支持的引擎：{other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Starting,
    Running,
    Stopping,
    /// 受控进程已经退出，正在把本机 NTFS 运行镜像回写到移动盘。
    Saving,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineState {
    pub schema_version: u32,
    pub engine: Engine,
    pub status: RuntimeStatus,
    pub supervisor_pid: u32,
    pub process_pid: Option<u32>,
    pub executable: PathBuf,
    pub started_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    pub loopback_port: Option<u16>,
    #[serde(default)]
    pub window_ready: bool,
    #[serde(default)]
    pub window_process_id: Option<u32>,
    #[serde(default)]
    pub window_handle: Option<u64>,
    #[serde(default)]
    pub window_title: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineDetection {
    pub engine: Engine,
    pub ready: bool,
    pub executable: PathBuf,
    pub auxiliary_paths: Vec<PathBuf>,
    pub missing_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResponse<T>
where
    T: Serialize,
{
    pub ok: bool,
    pub data: T,
}

impl<T> CommandResponse<T>
where
    T: Serialize,
{
    pub fn success(data: T) -> Self {
        Self { ok: true, data }
    }
}

#[cfg(test)]
mod tests {
    use super::{Engine, SkillTarget};

    #[test]
    fn openclaw_serializes_with_frontend_engine_id() {
        assert_eq!(
            serde_json::to_string(&Engine::OpenClaw).unwrap(),
            "\"openclaw\""
        );
        assert_eq!(
            serde_json::to_string(&SkillTarget::OpenClaw).unwrap(),
            "\"openclaw\""
        );
    }

    #[test]
    fn legacy_openclaw_engine_id_remains_readable() {
        assert_eq!(
            serde_json::from_str::<Engine>("\"open-claw\"").unwrap(),
            Engine::OpenClaw
        );
        assert_eq!(
            serde_json::from_str::<SkillTarget>("\"open-claw\"").unwrap(),
            SkillTarget::OpenClaw
        );
    }
}
