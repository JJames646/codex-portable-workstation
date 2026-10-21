use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::json;

use yunshu_workstation_runtime::bridge;
use yunshu_workstation_runtime::model::SkillTarget;
use yunshu_workstation_runtime::model::{CommandResponse, Engine};
use yunshu_workstation_runtime::paths::WorkstationPaths;
use yunshu_workstation_runtime::provider::{self, ConfigureProvider, ProviderProtocol};
use yunshu_workstation_runtime::runtime;
use zeroize::Zeroizing;

#[derive(Debug, Parser)]
#[command(
    name = "workstation-runtime",
    version,
    about = "云枢三引擎便携启动后端"
)]
struct Cli {
    /// 工作站根目录；省略时从当前EXE所在目录推导
    #[arg(long, global = true)]
    root: Option<PathBuf>,

    #[command(subcommand)]
    command: RuntimeCommand,
}

#[derive(Debug, Subcommand)]
enum RuntimeCommand {
    /// 输出内部 Runtime 源码指纹，供启动器拒绝新旧组件混装
    #[command(hide = true)]
    BuildId,
    /// 创建便携目录并写入干净的三引擎配置
    Prepare,
    /// 检测三引擎主体文件
    Detect {
        /// codex、claude或openclaw；省略时检测全部
        engine: Option<String>,
    },
    /// 启动并监督一个引擎
    Start { engine: String },
    /// 在当前控制台运行交互式引擎；阶段1仅用于Claude Code
    Console { engine: String },
    /// 请求一个引擎安全停止
    Stop {
        engine: String,
        #[arg(long, default_value_t = 30)]
        timeout_seconds: u64,
    },
    /// 安全停止全部受控引擎
    StopAll {
        #[arg(long, default_value_t = 30)]
        timeout_seconds: u64,
    },
    /// 读取引擎真实运行状态
    Status {
        /// codex、claude或openclaw；省略时读取全部
        engine: Option<String>,
    },
    /// 只执行离线版本探针，不启动正式会话
    Probe { engine: String },
    /// 暂存并校验本机监督 Runtime，不启动引擎
    #[command(hide = true)]
    SupervisorCheck { engine: String },
    /// 发布阶段生成单文件OpenClaw本机运行载荷，不包含任何用户数据
    #[command(hide = true)]
    PackOpenclawPayload {
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// 输出便携路径、配置与三引擎诊断信息
    Diagnose,
    /// 配置、查看、应用和测试三个智能体各自独立的供应商
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    /// 管理只绑定127.0.0.1的本地协议桥
    Bridge {
        #[command(subcommand)]
        command: BridgeCommand,
    },
    /// 管理Codex、Claude Code、OpenClaw三套便携Skills
    Skills {
        #[command(subcommand)]
        command: SkillsCommand,
    },
    /// 管理只服务于 Codex 的移动盘预置插件
    Plugins {
        #[command(subcommand)]
        command: PluginsCommand,
    },
    /// 内部监督命令，由start自动调用
    #[command(hide = true)]
    Supervise { engine: String },
    /// 内部离线验收命令：不读取供应商凭据
    #[command(hide = true)]
    SupervisePayload { engine: String },
}

#[derive(Debug, Subcommand)]
enum ProviderCommand {
    /// 交互式配置供应商；API Key仅从隐藏输入读取，不进入命令行
    Configure {
        engine: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        base_url: String,
        #[arg(long)]
        model: String,
        #[arg(long)]
        protocol: String,
        #[arg(long, default_value_t = 120)]
        timeout_seconds: u64,
    },
    /// 查看脱敏后的供应商配置状态
    Status {
        /// codex、claude或openclaw；省略时查看全部
        engine: Option<String>,
    },
    /// 把公开供应商配置重新应用到智能体原生配置
    Apply { engine: String },
    /// 通过本地协议桥发送最小请求，验证当前智能体供应商
    Test { engine: String },
}

#[derive(Debug, Subcommand)]
enum BridgeCommand {
    Start,
    Status,
    Stop,
    #[command(hide = true)]
    Serve,
}

#[derive(Debug, Subcommand)]
enum SkillsCommand {
    /// 只读检查本地SKILL.md、Markdown或ZIP
    Inspect { source: PathBuf },
    /// 生成可核验安装计划，不写入三个目标目录
    Plan {
        #[arg(long)]
        source: String,
        #[arg(long, default_value = "codex,claude,openclaw")]
        targets: String,
        /// 明确允许使用U盘Runtime安装声明的依赖
        #[arg(long)]
        install_dependencies: bool,
        /// 只使用已经验证的U盘缓存，不访问网络
        #[arg(long)]
        offline: bool,
    },
    /// 执行已经生成的安装计划
    Install { plan_id: String },
    /// 查看全部已安装Skill和哈希状态
    List,
    /// 校验一个Skill在所有目标中的真实文件
    Verify { slug: String },
    /// 从指定目标卸载Skill
    Uninstall {
        slug: String,
        #[arg(long, default_value = "codex,claude,openclaw")]
        targets: String,
    },
    /// 回滚崩溃后遗留的未完成事务
    Repair,
    /// 管理签名Catalog
    Catalog {
        #[command(subcommand)]
        command: SkillsCatalogCommand,
    },
    /// 浏览、缓存并校验SkillHub中文在线Skill
    Skillhub {
        #[command(subcommand)]
        command: SkillsSkillHubCommand,
    },
}

#[derive(Debug, Subcommand)]
enum SkillsCatalogCommand {
    /// 下载并验证全部已启用Catalog
    Refresh,
    /// 查看已经验证并缓存的Catalog
    List,
}

#[derive(Debug, Subcommand)]
enum SkillsSkillHubCommand {
    /// 读取SkillHub中文分类；--offline只读U盘缓存
    Categories {
        #[arg(long)]
        offline: bool,
    },
    /// 读取SkillHub推荐Skill；--offline只读U盘缓存
    Recommended {
        #[arg(long)]
        offline: bool,
    },
    /// 搜索SkillHub中文目录
    Search {
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        category: Option<String>,
        #[arg(long, default_value_t = 1)]
        page: u32,
        #[arg(long, default_value_t = 20)]
        page_size: u32,
        #[arg(long, default_value = "downloads")]
        sort_by: String,
        #[arg(long)]
        offline: bool,
    },
    /// 读取一个SkillHub Skill详情
    Detail {
        reference: String,
        #[arg(long)]
        offline: bool,
    },
    /// 下载到U盘缓存并执行官方Ed25519签名和内容哈希校验
    Cache { reference: String },
}

#[derive(Debug, Subcommand)]
enum PluginsCommand {
    /// 查看插件中心、预置快照与 Codex 配置状态
    Status,
    /// 浏览移动盘预置插件目录
    List {
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        category: Option<String>,
        #[arg(long, default_value_t = 1)]
        page: u32,
        #[arg(long, default_value_t = 24)]
        page_size: u32,
    },
    /// 查看单个插件详情
    Detail { id: String },
    /// 从移动盘预置载荷安装或重装一个 Codex 插件
    Install { id: String },
    /// 查看真实安装注册表
    Installed,
    /// 回滚未完成事务并修复可变盘符路径
    Repair,
    /// 启动仅绑定 127.0.0.1 的中文插件中心
    Serve {
        #[arg(long, default_value_t = 17642)]
        port: u16,
    },
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        let payload = json!({
            "ok": false,
            "error": error.to_string(),
        });
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&payload)
                .unwrap_or_else(|_| "{\"ok\":false,\"error\":\"unknown\"}".to_string())
        );
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = WorkstationPaths::resolve(cli.root)?;
    match cli.command {
        RuntimeCommand::BuildId => println!("{}", yunshu_workstation_runtime::RUNTIME_BUILD_ID),
        RuntimeCommand::Prepare => print_success(runtime::prepare(&paths)?)?,
        RuntimeCommand::Detect { engine } => {
            let engine = parse_optional_engine(engine)?;
            print_success(runtime::detect(&paths, engine))?;
        }
        RuntimeCommand::Start { engine } => {
            let engine = parse_engine(&engine)?;
            provider::apply(&paths, engine)?;
            if provider::requires_bridge(&paths, engine)? {
                bridge::ensure_running(&paths).await?;
            }
            print_success(runtime::start(&paths, engine)?)?;
        }
        RuntimeCommand::Console { engine } => {
            let engine = parse_engine(&engine)?;
            provider::apply(&paths, engine)?;
            if provider::requires_bridge(&paths, engine)? {
                bridge::ensure_running(&paths).await?;
            }
            runtime::run_console(&paths, engine)?;
        }
        RuntimeCommand::Stop {
            engine,
            timeout_seconds,
        } => {
            print_success(runtime::stop(
                &paths,
                parse_engine(&engine)?,
                Duration::from_secs(timeout_seconds),
            )?)?;
        }
        RuntimeCommand::StopAll { timeout_seconds } => {
            let engines = runtime::stop_all(&paths, Duration::from_secs(timeout_seconds))?;
            let bridge = bridge::stop(&paths).await?;
            print_success(serde_json::json!({"engines":engines,"bridge":bridge}))?;
        }
        RuntimeCommand::Status { engine } => match parse_optional_engine(engine)? {
            Some(engine) => print_success(runtime::status(&paths, engine)?)?,
            None => print_success(runtime::status_all(&paths)?)?,
        },
        RuntimeCommand::Probe { engine } => {
            print_success(runtime::probe(&paths, parse_engine(&engine)?)?)?;
        }
        RuntimeCommand::SupervisorCheck { engine } => {
            print_success(runtime::check_supervisor(&paths, parse_engine(&engine)?)?)?;
        }
        RuntimeCommand::PackOpenclawPayload { output } => {
            print_success(runtime::pack_openclaw_payload(&paths, output.as_deref())?)?;
        }
        RuntimeCommand::Diagnose => print_success(runtime::diagnose(&paths)?)?,
        RuntimeCommand::Provider { command } => match command {
            ProviderCommand::Configure {
                engine,
                name,
                base_url,
                model,
                protocol,
                timeout_seconds,
            } => {
                let api_key = rpassword::prompt_password("请输入API Key（输入内容不会显示）：")?;
                print_success(provider::configure(
                    &paths,
                    ConfigureProvider {
                        engine: parse_engine(&engine)?,
                        display_name: name,
                        base_url,
                        model,
                        models: None,
                        protocol: ProviderProtocol::from_str(&protocol)?,
                        workspace: None,
                        timeout_seconds,
                        api_key: Some(Zeroizing::new(api_key)),
                    },
                )?)?;
            }
            ProviderCommand::Status { engine } => match parse_optional_engine(engine)? {
                Some(engine) => print_success(provider::status(&paths, engine)?)?,
                None => print_success(provider::status_all(&paths)?)?,
            },
            ProviderCommand::Apply { engine } => {
                let engine = parse_engine(&engine)?;
                provider::apply(&paths, engine)?;
                print_success(provider::status(&paths, engine)?)?;
            }
            ProviderCommand::Test { engine } => {
                print_success(bridge::test_provider(&paths, parse_engine(&engine)?).await?)?;
            }
        },
        RuntimeCommand::Bridge { command } => match command {
            BridgeCommand::Start => print_success(bridge::start(&paths).await?)?,
            BridgeCommand::Status => print_success(bridge::status(&paths).await?)?,
            BridgeCommand::Stop => print_success(bridge::stop(&paths).await?)?,
            BridgeCommand::Serve => bridge::serve(paths).await?,
        },
        RuntimeCommand::Skills { command } => match command {
            SkillsCommand::Inspect { source } => {
                print_success(
                    yunshu_workstation_runtime::skills::transaction::inspect_local(&source)?,
                )?;
            }
            SkillsCommand::Plan {
                source,
                targets,
                install_dependencies,
                offline,
            } => {
                print_success(
                    yunshu_workstation_runtime::skills::transaction::create_plan_with_mode(
                        &paths,
                        &source,
                        parse_skill_targets(&targets)?,
                        install_dependencies,
                        offline,
                    )
                    .await?,
                )?;
            }
            SkillsCommand::Install { plan_id } => {
                print_success(
                    yunshu_workstation_runtime::skills::transaction::install_plan(
                        &paths, &plan_id,
                    )?,
                )?;
            }
            SkillsCommand::List => {
                print_success(
                    yunshu_workstation_runtime::skills::transaction::list_status(&paths)?,
                )?;
            }
            SkillsCommand::Verify { slug } => {
                print_success(
                    yunshu_workstation_runtime::skills::transaction::verify_skill(&paths, &slug)?,
                )?;
            }
            SkillsCommand::Uninstall { slug, targets } => {
                print_success(yunshu_workstation_runtime::skills::transaction::uninstall(
                    &paths,
                    &slug,
                    parse_skill_targets(&targets)?,
                )?)?;
            }
            SkillsCommand::Repair => {
                print_success(yunshu_workstation_runtime::skills::transaction::repair(
                    &paths,
                )?)?;
            }
            SkillsCommand::Catalog { command } => match command {
                SkillsCatalogCommand::Refresh => {
                    print_success(
                        yunshu_workstation_runtime::skills::catalog::refresh_all(&paths).await?,
                    )?;
                }
                SkillsCatalogCommand::List => {
                    print_success(yunshu_workstation_runtime::skills::catalog::list_cached(
                        &paths,
                    )?)?;
                }
            },
            SkillsCommand::Skillhub { command } => match command {
                SkillsSkillHubCommand::Categories { offline } => {
                    print_success(
                        yunshu_workstation_runtime::skills::skillhub::categories(&paths, offline)
                            .await?,
                    )?;
                }
                SkillsSkillHubCommand::Recommended { offline } => {
                    print_success(
                        yunshu_workstation_runtime::skills::skillhub::recommended(&paths, offline)
                            .await?,
                    )?;
                }
                SkillsSkillHubCommand::Search {
                    query,
                    category,
                    page,
                    page_size,
                    sort_by,
                    offline,
                } => {
                    print_success(
                        yunshu_workstation_runtime::skills::skillhub::search(
                            &paths,
                            query.as_deref(),
                            category.as_deref(),
                            page,
                            page_size,
                            &sort_by,
                            offline,
                        )
                        .await?,
                    )?;
                }
                SkillsSkillHubCommand::Detail { reference, offline } => {
                    print_success(
                        yunshu_workstation_runtime::skills::skillhub::detail(
                            &paths, &reference, offline,
                        )
                        .await?,
                    )?;
                }
                SkillsSkillHubCommand::Cache { reference } => {
                    print_success(
                        yunshu_workstation_runtime::skills::skillhub::download_package(
                            &paths, &reference,
                        )
                        .await?,
                    )?;
                }
            },
        },
        RuntimeCommand::Plugins { command } => match command {
            PluginsCommand::Status => {
                print_success(yunshu_workstation_runtime::plugins::status(&paths)?)?;
            }
            PluginsCommand::List {
                query,
                category,
                page,
                page_size,
            } => {
                print_success(yunshu_workstation_runtime::plugins::catalog::list(
                    &paths,
                    query.as_deref(),
                    category.as_deref(),
                    page,
                    page_size,
                )?)?;
            }
            PluginsCommand::Detail { id } => {
                print_success(yunshu_workstation_runtime::plugins::catalog::detail(
                    &paths, &id,
                )?)?;
            }
            PluginsCommand::Install { id } => {
                print_success(yunshu_workstation_runtime::plugins::installer::install(
                    &paths, &id,
                )?)?;
            }
            PluginsCommand::Installed => {
                print_success(
                    yunshu_workstation_runtime::plugins::installer::read_registry(&paths)?,
                )?;
            }
            PluginsCommand::Repair => {
                print_success(yunshu_workstation_runtime::plugins::installer::repair(
                    &paths,
                )?)?;
            }
            PluginsCommand::Serve { port } => {
                yunshu_workstation_runtime::plugins::server::serve_plugin_center(paths, port)
                    .await?;
            }
        },
        RuntimeCommand::Supervise { engine } => {
            runtime::supervise(&paths, parse_engine(&engine)?)?;
        }
        RuntimeCommand::SupervisePayload { engine } => {
            runtime::supervise_payload(&paths, parse_engine(&engine)?)?;
        }
    }
    Ok(())
}

fn parse_engine(value: &str) -> Result<Engine> {
    Engine::from_str(value)
}

fn parse_optional_engine(value: Option<String>) -> Result<Option<Engine>> {
    value.as_deref().map(parse_engine).transpose()
}

fn parse_skill_targets(value: &str) -> Result<Vec<SkillTarget>> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(SkillTarget::from_str)
        .collect()
}

fn print_success<T>(data: T) -> Result<()>
where
    T: Serialize,
{
    println!(
        "{}",
        serde_json::to_string_pretty(&CommandResponse::success(data))?
    );
    Ok(())
}
