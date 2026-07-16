# UI 与真实功能对接矩阵

## 1. 对接原则

正式客户端以 Codex++ 管理工具为底座。当前静态网页中的每个页面迁入 `apps/codex-plus-manager` 后，通过 Tauri Commands 调用：

1. Codex++ 已有 Rust Core。
2. 新增的 Workstation Core。
3. 独立 sidecar。
4. 用户服务器 API。

“复用”不代表完全不改；涉及便携路径、凭据或更新时仍需 Fork 适配。

## 2. 顶部全局操作

| UI | 对接能力 | 来源 | 处理方式 |
|---|---|---|---|
| 重新检测 | Codex 路径、版本、Codex++ 状态、磁盘、WebView2、数据目录、服务连接 | Codex++ `load_overview` + 新 `workstation_check` | 组合调用 |
| 启动 Codex | 应用供应商、启动服务连接、设置环境变量、通过 Codex++ 启动便携 Codex | Codex++ `launch_codex_plus` | 扩展为工作站状态机 |
| 停止 | 停止 Codex++ launcher、Codex、helper 和连接 sidecar | Codex++ 内部 watcher + 新命令 | 新增 `stop_codex_plus` |
| 弹出 U盘 | 等待 SQLite/文件句柄释放并调用 Windows 弹出接口 | 无 | 全新开发 |
| 主题切换 | 保存 UI 主题到移动盘 | 前端 + `save_settings` | 复用并便携化 |

## 3. 首页

| 区域 | 真实数据/动作 | 现有能力 | 缺口 |
|---|---|---|---|
| 当前工作区 | `Workspace/`、最近项目、盘符、容量 | 无统一接口 | 新 `workspace_status` |
| Codex 状态 | 进程、CDP、helper、版本 | Codex++ Overview/StatusStore | 改为 `CODEXPP_HOME` |
| 服务连接 | sidecar 状态、授权有效期、目标 API 可达 | Codex++ 无 | 新 connection service |
| 主仪表盘 | Codex/Codex++ 状态、版本、路径、项目、会话、供应商、插件和环境 | Codex++ Overview 已有一部分 | Workstation Overview 聚合 |
| 中转站附加仪表盘 | 已配置中转站后的用量、趋势、模型、余额、订阅 | Sub2API 用户接口已有 | 可选 API adapter |
| 查看详情 | 安全打开完整服务器网站 | Tauri 可创建远程 WebView | 必须隔离本地 IPC |
| 环境校验 | WebView2、写权限、空间、目录、运行时、网络、版本 | Codex++ 部分健康检查 | Workstation Health 聚合 |

中转站附加仪表盘建议接口：

```text
GET /api/v1/user/profile
GET /api/v1/usage/dashboard/snapshot-v2
GET /api/v1/subscriptions/summary
GET /v1/sub2api/billing
```

## 4. 供应商配置

| UI 功能 | Codex++ 对应 | 状态 |
|---|---|---|
| 供应商列表 | `load_settings` 中 `relay_profiles` | 已有 |
| 选择供应商 | `switch_relay_profile` | 已有，需 secret ref |
| 添加第三方 API | `save_settings` / provider import | 已有，需改安全模型 |
| 协议选择 | `RelayProtocol` | 已有 |
| 模型列表 | `fetch_relay_profile_models` | 已有 |
| 测试连接 | `test_relay_profile` | 已有 |
| Provider Doctor | `diagnose_relay_profile` | 已有 |
| Chat 转 Responses | `protocol_proxy` | 已有 |
| 官方中转站默认项 | 默认 RelayProfile + activation adapter | 新默认配置 |
| 便捷粘贴 API Key | 使用内置 Base URL/协议/模型预设，粘贴后自动填充并检测 | 新增 |
| 加密凭据 | 无 | 全新开发 |

必须改造：

- `RelayProfile` 不再保存真实 `api_key`。
- 新增 `credential_id`/`env_key` 引用。
- 配置生成使用 `env_key = "CODEX_WORKSTATION_API_KEY"`。
- 供应商测试由运行时 Secret Broker 注入临时 Key。
- 用户不设置 PIN；Secret Store 自动加载同盘密钥，并明确提示移动盘丢失风险。

## 5. 项目管理

| UI 功能 | 可复用 | 新增 |
|---|---|---|
| 列出 `Workspace/` 项目 | 无 | `workspace_inventory` |
| 最近项目 | Codex global state/SQLite 可读取 | 统一项目索引 |
| 关联会话数量 | Codex++ session storage | 按 cwd 聚合 |
| 新建/导入项目 | 文件系统能力 | 路径校验和模板 |
| 打开项目 | Codex++/Codex 启动参数待核实 | `open_workspace` |
| 项目移动 | Codex++ 会话项目移动 | 可复用 |
| Upstream worktree | Codex++ `upstream_worktree` | 已有 |
| “Scale 记忆” | 未发现明确上游模块 | 待确认，不先伪造 |

## 6. 会话与绘画

| UI 功能 | 对应能力 | 状态 |
|---|---|---|
| 会话列表 | `list_local_sessions` | 已有 |
| 删除会话 | `delete_local_session` | 已有 |
| 导出 Markdown | `codex-plus-data/markdown` | 已有 |
| Provider 同步 | `sync_providers_now` | 已有 |
| 会话索引清理 | preview/apply session cleanup | 已有 |
| 会话恢复 | Codex++ 删除前局部 backup | 需扩展为工作站恢复点 |
| 绘画记录 | `CODEX_HOME/generated_images` | 新索引 |
| 绘画缓存清理 | 文件系统 | 新增安全清理 |

## 7. 工具与插件

| UI 功能 | Codex++ 对应 | 状态 |
|---|---|---|
| 插件市场状态 | `plugin_marketplace_status` | 已有 |
| 修复插件市场 | `repair_plugin_marketplace` | 已有 |
| 远程插件市场 | remote marketplace commands | 已有 |
| Skill/Plugin/MCP 配置 | context entry commands | 已有 |
| 用户脚本市场 | `refresh_script_market` / `install_market_script` | 已有 |
| 启停用户脚本 | `set_user_script_enabled` | 已有 |
| 跨电脑保存 | 当前 `%APPDATA%\Codex++` | 必须改为 `CODEXPP_HOME` |

## 8. 页面增强

Codex++ 已有：

- 增强总开关。
- 会话删除和导出。
- 插件市场解锁。
- 中文界面。
- 粘贴修复。
- 会话宽度、滚动恢复、线程 ID。
- 服务层级、Goals、Stepwise。
- 图片覆盖层。
- 用户脚本和注入 watchdog。

工作站新增：

- 将全部设置保存到移动盘。
- 当前 UI 中的工作站快捷工具条。
- 失败降级：注入失败时仍能启动原始 Codex。

## 9. Zed 与远程项目

| UI 功能 | Codex++ 对应 | 状态 |
|---|---|---|
| Zed Remote 项目识别 | `list_zed_remote_projects` | 已有 |
| 打开 Zed Remote | `open_zed_remote` | 已有 |
| 忘记远程项目 | `forget_zed_remote_project` | 已有 |
| SSH remote worktree | `upstream_worktree/remote` | 已有基础 |
| 通用 SSH 主机档案 | 无完整工作站 UI | 新增 |
| 自动配置远端开发依赖 | 无 | 后续独立阶段 |

## 10. 数据与备份

| UI 功能 | 现有能力 | 缺口 |
|---|---|---|
| 会话 SQLite 识别 | Codex++ `codex_sqlite` | 已有 |
| 删除前局部备份 | `BackupStore` | 已有 |
| Provider 同步备份 | `provider_sync` | 已有 |
| 全工作站恢复点 | 无 | 新增 |
| 更新前快照 | 无 | 新增 |
| 异常退出恢复 | 无完整流程 | 新增 |
| U 盘空间/写入检测 | 无 | 新增 |
| 安全弹出 | 无 | 新增 |

## 11. 网络环境配置

Codex++ 可复用：

- 识别系统代理和代理环境变量冲突。
- 本地 Responses/Chat 协议转换。
- 供应商网络测试。

Codex++ 不能提供：

- 到你的服务器的专用长连接。
- 线路授权、卡密有效期和路由清单。
- 合规受限的边缘接入。

新增模块：

```text
ConnectionService sidecar
RouteManifestClient
ServiceEntitlementClient
HealthProbe
ProcessScopedEnvironmentBuilder
```

## 12. 更新中心

Codex++ 的 `check_update`/`perform_update` 只能作为参考，不直接接入正式按钮。

正式 UI 对接：

```text
check_workstation_update
download_workstation_update
apply_workstation_update
rollback_workstation_update
skip_update_once
```

组件：

- workstation-manager
- portable-codex
- connection-service
- plugin/skill index
- runtime assets

## 13. 安装维护

可复用：

- Codex 应用路径识别。
- Codex++ 概览和日志。
- 环境冲突检测。
- 插件市场修复。
- Watcher 进程检测逻辑。

新增：

- WebView2 固定运行时检测。
- 移动盘文件系统、速度、空间和写权限。
- manifest/hash 校验。
- 工作站目录修复。
- 从自己的更新仓库修复损坏组件。
- 诊断包脱敏。

默认不启用 Codex++ 的系统启动项、桌面快捷方式和安装入口。

## 14. 设置与恢复

| UI | 实现 |
|---|---|
| 启动检查更新 | Workstation Settings |
| 退出创建快照 | Backup Policy |
| 高级日志 | Log Level |
| 恢复出厂设置 | 停止全部进程、二次确认后删除 Workstation/Codex++ 配置、卡密、凭据、项目、会话、插件、Skills、绘画、备份和缓存 |
| 删除凭据 | 可单独执行，也包含在恢复出厂设置中 |
| 删除项目/会话 | 包含在完整恢复出厂设置中，必须清楚列出并不可撤销 |

## 15. 原型中需要后续修正的文案

- 静态原型已将首页改成 Codex++ Overview + Workstation Health；正式 Tauri 版本仍需绑定真实接口。
- 静态原型已将恢复出厂设置改为无账号密码、完整删除和确认文字；正式版本仍需实现安全删除边界。
- 静态原型已增加本产品中转站 API Key 便捷粘贴框；正式版本不得在浏览器状态或日志中保留 Key。
- “推荐中转站”遗留文案应统一为产品最终确定名称。
- 项目页“Scale 记忆”必须在数据来源核实后再保留。
