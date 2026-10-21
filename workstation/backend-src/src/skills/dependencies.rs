use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::paths::WorkstationPaths;
use crate::windows_process::configure_command;

use super::metadata::DependencySummary;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyInstallResult {
    pub detected: DependencySummary,
    pub python_installed: bool,
    pub node_installed: bool,
    pub notices: Vec<String>,
}

pub fn install_dependencies(
    paths: &WorkstationPaths,
    skill_root: &Path,
    detected: &DependencySummary,
) -> Result<DependencyInstallResult> {
    let mut result = DependencyInstallResult {
        detected: detected.clone(),
        python_installed: false,
        node_installed: false,
        notices: Vec::new(),
    };
    if detected.requirements_txt {
        install_python_requirements(paths, skill_root)?;
        result.python_installed = true;
    } else if detected.pyproject_toml {
        result.notices.push(
            "检测到pyproject.toml，但没有requirements.txt；为避免执行包构建脚本，本阶段未自动安装"
                .to_string(),
        );
    }

    if detected.package_json {
        install_node_packages(paths, skill_root, detected)?;
        result.node_installed = true;
    }
    if detected.yarn_lock {
        result
            .notices
            .push("检测到yarn.lock；使用U盘npm以--ignore-scripts模式安装".to_string());
    }
    Ok(result)
}

fn install_python_requirements(paths: &WorkstationPaths, skill_root: &Path) -> Result<()> {
    if !paths.bundled_python.is_file() {
        bail!("U盘Python不存在：{}", paths.bundled_python.display());
    }
    let virtual_environment = skill_root.join(".venv");
    if !virtual_environment
        .join("Scripts")
        .join("python.exe")
        .is_file()
    {
        run_checked(
            &paths.bundled_python,
            [
                OsString::from("-m"),
                OsString::from("venv"),
                virtual_environment.into_os_string(),
            ],
            skill_root,
            paths,
            "创建Skill Python虚拟环境",
        )?;
    }
    let python = skill_root.join(".venv").join("Scripts").join("python.exe");
    run_checked(
        &python,
        [
            OsString::from("-m"),
            OsString::from("pip"),
            OsString::from("install"),
            OsString::from("--disable-pip-version-check"),
            OsString::from("--no-input"),
            OsString::from("--cache-dir"),
            paths.skills_cache.join("python").into_os_string(),
            OsString::from("-r"),
            skill_root.join("requirements.txt").into_os_string(),
        ],
        skill_root,
        paths,
        "安装Skill Python依赖",
    )
}

fn install_node_packages(
    paths: &WorkstationPaths,
    skill_root: &Path,
    detected: &DependencySummary,
) -> Result<()> {
    let node_dir = paths.bundled_node.parent().context("U盘Node缺少父目录")?;
    let pnpm = node_dir.join("pnpm.cmd");
    let npm = node_dir.join("npm.cmd");
    if detected.pnpm_lock && pnpm.is_file() {
        run_checked(
            &pnpm,
            [
                OsString::from("install"),
                OsString::from("--frozen-lockfile"),
                OsString::from("--ignore-scripts"),
                OsString::from("--store-dir"),
                paths
                    .skills_cache
                    .join("node")
                    .join("pnpm-store")
                    .into_os_string(),
            ],
            skill_root,
            paths,
            "安装Skill pnpm依赖",
        )
    } else {
        if !npm.is_file() {
            bail!("U盘npm入口不存在：{}", npm.display());
        }
        let operation = if detected.package_lock {
            "ci"
        } else {
            "install"
        };
        run_checked(
            &npm,
            [
                OsString::from(operation),
                OsString::from("--ignore-scripts"),
                OsString::from("--no-audit"),
                OsString::from("--no-fund"),
            ],
            skill_root,
            paths,
            "安装Skill Node依赖",
        )
    }
}

fn run_checked<I>(
    program: &Path,
    args: I,
    current_dir: &Path,
    paths: &WorkstationPaths,
    action: &str,
) -> Result<()>
where
    I: IntoIterator<Item = OsString>,
{
    let path = child_path(paths)?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(current_dir)
        .env("PATH", path)
        .env(
            "NPM_CONFIG_CACHE",
            paths.skills_cache.join("node").join("npm"),
        )
        .env("PIP_CACHE_DIR", paths.skills_cache.join("python"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_command(&mut command, false);
    let output = command
        .output()
        .with_context(|| format!("{action}无法启动：{}", program.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let summary = stderr.lines().rev().take(12).collect::<Vec<_>>();
    bail!(
        "{action}失败，退出码={:?}：{}",
        output.status.code(),
        summary.into_iter().rev().collect::<Vec<_>>().join("\n")
    )
}

fn child_path(paths: &WorkstationPaths) -> Result<OsString> {
    let mut entries = vec![
        paths
            .bundled_node
            .parent()
            .context("U盘Node缺少父目录")?
            .to_path_buf(),
        paths
            .bundled_python
            .parent()
            .context("U盘Python缺少父目录")?
            .to_path_buf(),
    ];
    if let Some(existing) = env::var_os("PATH") {
        entries.extend(env::split_paths(&existing));
    }
    env::join_paths(entries).context("无法构造Skill依赖安装PATH")
}
