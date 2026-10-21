use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::model::{Engine, EngineDetection};
use crate::paths::WorkstationPaths;

#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub engine: Engine,
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub current_dir: PathBuf,
    pub environment: Vec<(OsString, OsString)>,
    pub open_new_console: bool,
    pub loopback_port: Option<u16>,
}

pub fn detect_engine(paths: &WorkstationPaths, engine: Engine) -> EngineDetection {
    let (executable, auxiliary_paths) = match engine {
        Engine::Codex => (paths.codex_desktop.clone(), vec![paths.codex_cli.clone()]),
        Engine::Claude => (paths.claude_exe.clone(), Vec::new()),
        Engine::OpenClaw => (
            paths.openclaw_node.clone(),
            vec![paths.openclaw_entry.clone()],
        ),
    };
    let missing_paths = std::iter::once(&executable)
        .chain(auxiliary_paths.iter())
        .filter(|path| !path.is_file())
        .cloned()
        .collect::<Vec<_>>();
    EngineDetection {
        engine,
        ready: missing_paths.is_empty(),
        executable,
        auxiliary_paths,
        missing_paths,
    }
}

pub fn build_launch_spec(
    paths: &WorkstationPaths,
    engine: Engine,
    loopback_port: Option<u16>,
) -> Result<LaunchSpec> {
    build_launch_spec_internal(paths, engine, loopback_port, true)
}

pub(crate) fn build_launch_spec_without_provider(
    paths: &WorkstationPaths,
    engine: Engine,
    loopback_port: Option<u16>,
) -> Result<LaunchSpec> {
    build_launch_spec_internal(paths, engine, loopback_port, false)
}

fn build_launch_spec_internal(
    paths: &WorkstationPaths,
    engine: Engine,
    loopback_port: Option<u16>,
    include_provider: bool,
) -> Result<LaunchSpec> {
    let detection = detect_engine(paths, engine);
    if !detection.ready {
        let missing = detection
            .missing_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("；");
        bail!("{engine}主体不完整：{missing}");
    }

    let provider_environment = if include_provider {
        crate::provider::launch_environment(paths, engine)?
    } else {
        Vec::new()
    };
    match engine {
        Engine::Codex => {
            // Codex 仍只接收 CODEX_HOME 和 --user-data-dir，但两者指向本机 NTFS
            // 运行镜像。移动盘目录由 codex_machine 在启动/停止边界安全同步，避免
            // Chromium、LevelDB 和 SQLite WAL 直接高频写入 exFAT。
            let mut environment = provider_environment;
            let mut args = Vec::new();
            let runtime = crate::codex_machine::runtime_paths(paths);
            environment.push((
                OsString::from("CODEX_HOME"),
                runtime.home.as_os_str().to_os_string(),
            ));
            args.push(OsString::from(format!(
                "--user-data-dir={}",
                runtime.browser_data.display()
            )));
            Ok(LaunchSpec {
                engine,
                program: paths.codex_desktop.clone(),
                args,
                current_dir: paths.workspace.clone(),
                environment,
                open_new_console: false,
                loopback_port: None,
            })
        }
        Engine::Claude => {
            let mut environment = common_environment(paths)?;
            environment.extend(provider_environment);
            environment.push((
                OsString::from("CLAUDE_CONFIG_DIR"),
                paths.claude_home.as_os_str().to_os_string(),
            ));
            environment.push((
                OsString::from("CLAUDE_CODE_CACHE"),
                paths.cache.join("claude").into_os_string(),
            ));
            environment.push((
                OsString::from("NPM_CONFIG_CACHE"),
                paths.cache.join("npm").into_os_string(),
            ));
            environment.push((OsString::from("DISABLE_AUTOUPDATER"), OsString::from("1")));
            let current_dir = if include_provider {
                crate::provider::workspace_for_launch(paths, engine)?
                    .unwrap_or_else(|| paths.workspace.clone())
            } else {
                paths.workspace.clone()
            };
            Ok(LaunchSpec {
                engine,
                program: paths.claude_exe.clone(),
                args: Vec::new(),
                current_dir,
                environment,
                open_new_console: true,
                loopback_port: None,
            })
        }
        Engine::OpenClaw => {
            let port = loopback_port.context("OpenClaw启动前必须分配本地端口")?;
            let mut environment = common_environment(paths)?;
            environment.extend(provider_environment);
            let machine_cache = paths.machine_engine_cache(Engine::OpenClaw.slug());
            let temporary = machine_cache.join("temp");
            environment.push((
                OsString::from("OPENCLAW_HOME"),
                paths.openclaw_home.as_os_str().to_os_string(),
            ));
            environment.push((
                OsString::from("OPENCLAW_STATE_DIR"),
                paths.openclaw_home.as_os_str().to_os_string(),
            ));
            environment.push((
                OsString::from("OPENCLAW_CONFIG_PATH"),
                paths.openclaw_home.join("openclaw.json").into_os_string(),
            ));
            environment.push((OsString::from("OPENCLAW_NO_UPDATE"), OsString::from("1")));
            environment.push((OsString::from("TEMP"), temporary.as_os_str().to_os_string()));
            environment.push((OsString::from("TMP"), temporary.as_os_str().to_os_string()));
            // 程序与用户状态直接从移动盘运行；本机只保留可删除的编译和临时缓存。
            environment.push((
                OsString::from("NODE_COMPILE_CACHE"),
                machine_cache.join("node-compile-cache").into_os_string(),
            ));
            environment.push((
                OsString::from("NPM_CONFIG_CACHE"),
                machine_cache.join("npm-cache").into_os_string(),
            ));
            Ok(LaunchSpec {
                engine,
                program: paths.openclaw_node.clone(),
                args: vec![
                    paths.openclaw_entry.as_os_str().to_os_string(),
                    OsString::from("gateway"),
                    OsString::from("--port"),
                    OsString::from(port.to_string()),
                    OsString::from("--verbose"),
                ],
                current_dir: paths.openclaw_home.clone(),
                environment,
                open_new_console: false,
                loopback_port: Some(port),
            })
        }
    }
}

pub fn build_probe_spec(paths: &WorkstationPaths, engine: Engine) -> Result<LaunchSpec> {
    let port = (engine == Engine::OpenClaw).then_some(18789);
    let mut spec = build_launch_spec_internal(paths, engine, port, false)?;
    spec.open_new_console = false;
    match engine {
        Engine::Codex => {
            spec.program = paths.codex_cli.clone();
            spec.args = vec![OsString::from("--version")];
            spec.environment.retain(|(key, _)| key != "CODEX_HOME");
            spec.environment.push((
                OsString::from("CODEX_HOME"),
                paths.codex_home.as_os_str().to_os_string(),
            ));
        }
        Engine::Claude => {
            spec.args = vec![OsString::from("--version")];
        }
        Engine::OpenClaw => {
            spec.args = vec![
                paths.openclaw_entry.as_os_str().to_os_string(),
                OsString::from("--version"),
            ];
            spec.current_dir = paths.openclaw_home.clone();
        }
    }
    Ok(spec)
}

fn common_environment(paths: &WorkstationPaths) -> Result<Vec<(OsString, OsString)>> {
    let mut path_entries = vec![
        paths
            .bundled_node
            .parent()
            .context("随盘Node缺少父目录")?
            .to_path_buf(),
        paths
            .bundled_python
            .parent()
            .context("随盘Python缺少父目录")?
            .to_path_buf(),
    ];
    if let Some(existing) = env::var_os("PATH") {
        path_entries.extend(env::split_paths(&existing));
    }
    let joined_path = env::join_paths(path_entries).context("无法构造便携PATH")?;
    Ok(vec![
        (
            OsString::from("AI_WORKSTATION_ROOT"),
            paths.root.as_os_str().to_os_string(),
        ),
        (OsString::from("PATH"), joined_path),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_launch_keeps_both_data_layers_on_the_portable_root() {
        let root = std::env::temp_dir().join("中文 路径").join("dev2");
        let paths = WorkstationPaths::from_root(root.clone());
        let environment = common_environment(&paths).unwrap();
        assert!(
            environment
                .iter()
                .any(|(key, value)| { key == "AI_WORKSTATION_ROOT" && value == root.as_os_str() })
        );
        let spec = build_launch_spec_internal(&paths, Engine::Codex, None, false).unwrap_err();
        assert!(spec.to_string().contains("主体不完整"));
    }

    #[test]
    fn codex_launch_uses_the_local_ntfs_mirror_with_the_minimal_argument_contract() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("中文 路径").join("dev2");
        let paths = WorkstationPaths::from_root(root);
        for file in [
            &paths.codex_desktop,
            &paths.codex_cli,
            &paths.bundled_node,
            &paths.bundled_python,
        ] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"test").unwrap();
        }
        let spec = build_launch_spec_internal(&paths, Engine::Codex, None, false).unwrap();
        assert!(
            spec.args
                .iter()
                .any(|argument| argument.to_string_lossy().starts_with("--user-data-dir="))
        );
        assert_eq!(spec.args.len(), 1);
        assert!(spec.environment.iter().any(|(key, value)| {
            key == "CODEX_HOME" && value == paths.codex_machine_home().as_os_str()
        }));
        assert!(spec.args.iter().any(|argument| {
            argument.to_string_lossy()
                == format!(
                    "--user-data-dir={}",
                    paths.codex_machine_browser_data().display()
                )
        }));
        for forbidden in [
            "AI_WORKSTATION_ROOT",
            "PATH",
            "CODEX_SQLITE_HOME",
            "TEMP",
            "TMP",
            "LANG",
            "LC_ALL",
        ] {
            assert!(
                spec.environment.iter().all(|(key, _)| key != forbidden),
                "Codex 不应收到额外环境变量 {forbidden}"
            );
        }
        assert!(spec.args.iter().all(|argument| {
            let argument = argument.to_string_lossy();
            !argument.starts_with("--lang=")
                && !argument.starts_with("--disk-cache-dir=")
                && !argument.starts_with("--disk-cache-size=")
                && !argument.starts_with("--media-cache-size=")
        }));
    }

    #[test]
    fn codex_probe_contract_does_not_consume_pending_proxy_initialization() {
        let temporary = tempfile::tempdir().unwrap();
        let paths = WorkstationPaths::from_root(temporary.path().join("portable"));
        paths.prepare_layout().unwrap();
        for file in [&paths.codex_desktop, &paths.codex_cli] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"test").unwrap();
        }
        crate::locale_init::save_proxy_config(
            &paths,
            crate::locale_init::CodexInitProxySaveInput {
                protocol: crate::locale_init::CodexInitProxyProtocol::Http,
                host: "127.0.0.1".to_string(),
                port: 7890,
                username: None,
                password: None,
            },
        )
        .unwrap();
        crate::locale_init::activate_proxy_for_next_codex_start(&paths).unwrap();

        let spec = build_launch_spec_internal(&paths, Engine::Codex, None, false).unwrap();
        assert_eq!(spec.args.len(), 1);
        assert!(
            crate::locale_init::take_codex_proxy_launch(&paths)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn openclaw_launch_uses_portable_program_and_portable_state() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("中文 路径").join("dev2");
        let paths = WorkstationPaths::from_root(root);
        for file in [
            &paths.openclaw_node,
            &paths.openclaw_entry,
            &paths.bundled_node,
            &paths.bundled_python,
        ] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"test").unwrap();
        }

        let spec =
            build_launch_spec_internal(&paths, Engine::OpenClaw, Some(18789), false).unwrap();

        assert_eq!(spec.program, paths.openclaw_node);
        assert_eq!(spec.args.first().unwrap(), paths.openclaw_entry.as_os_str());
        assert!(spec.environment.iter().any(|(key, value)| {
            key == "OPENCLAW_HOME" && value == paths.openclaw_home.as_os_str()
        }));
        assert!(spec.environment.iter().any(|(key, value)| {
            key == "OPENCLAW_STATE_DIR" && value == paths.openclaw_home.as_os_str()
        }));
        assert!(spec.environment.iter().any(|(key, value)| {
            key == "NODE_COMPILE_CACHE"
                && value
                    == paths
                        .machine_engine_cache(Engine::OpenClaw.slug())
                        .join("node-compile-cache")
                        .as_os_str()
        }));
        assert!(spec.program.starts_with(&paths.root));
        assert!(PathBuf::from(spec.args.first().unwrap()).starts_with(&paths.root));
    }
}
