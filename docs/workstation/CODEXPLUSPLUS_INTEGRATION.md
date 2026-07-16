# Codex++ 二次开发与对接方案

## 1. 目标

将 Codex++ 从“安装在电脑上的外部管理工具”改造成“位于移动存储中的 Codex 便携工作站”，同时尽量保留上游功能和同步能力。

## 2. 集成方式

推荐创建正式 Fork：

```text
origin   = 你的 CodexPlusPlus Fork
upstream = https://github.com/BigPizzaV3/CodexPlusPlus.git
```

正式工作站不再把原版 Codex++ 当作独立可见应用，而是：

- `codex-plus-manager` 演进为 `Codex 便携工作站`。
- `codex-plus-launcher` 继续作为静默启动和 CDP 注入入口。
- `codex-plus-core` 和 `codex-plus-data` 保持上游模块边界。
- 新增 workstation crates，避免把 U 盘、服务端和更新逻辑塞进上游核心文件。

## 3. 必须修改的上游文件

### 3.1 Codex++ 状态目录

修改：

```text
crates/codex-plus-core/src/paths.rs
```

新增：

```text
CODEXPP_HOME
```

规则：

```text
如果 CODEXPP_HOME 存在且为目录：
  settings、status、logs、pending import、updates 全部写入该目录
否则：
  保持上游默认路径，兼容普通安装模式
```

测试：

- 有效目录。
- 空字符串。
- 不存在目录。
- 中文和空格路径。
- 只读目录。

### 3.2 用户脚本目录

修改：

```text
apps/codex-plus-manager/src-tauri/src/commands.rs
```

`user_scripts_config_dir()` 在便携模式下改为：

```text
<CODEXPP_HOME>\user_scripts
```

普通安装模式仍保留 `%APPDATA%\Codex++`。

### 3.3 便携启动参数

修改：

```text
apps/codex-plus-launcher/src/main.rs
crates/codex-plus-core/src/launcher.rs
```

保留现有：

```text
--app-path
--debug-port
--helper-port
```

新增建议：

```text
--portable-root <path>
--workspace-root <path>
--managed-update
```

`--portable-root` 只用于路径和运行模式，不携带任何 secret。

### 3.4 停止命令

修改：

```text
apps/codex-plus-manager/src-tauri/src/commands.rs
apps/codex-plus-manager/src-tauri/src/lib.rs
```

新增：

```text
stop_codex_plus
```

内部复用：

```text
watcher::stop_launcher_processes_and_wait()
watcher::stop_codex_processes_and_wait()
```

返回：

- 停止前进程。
- 等待时间。
- 残留进程。
- helper/sidecar 状态。

### 3.5 供应商凭据引用

修改：

```text
crates/codex-plus-core/src/settings.rs
crates/codex-plus-core/src/provider_import.rs
crates/codex-plus-core/src/relay_config.rs
crates/codex-plus-core/src/relay_switch.rs
apps/codex-plus-manager/src-tauri/src/commands.rs
apps/codex-plus-manager/src/App.tsx
```

建议数据结构：

```text
RelayProfile
├─ credential_id: Option<String>
├─ env_key: Option<String>
├─ api_key: 仅用于兼容旧数据，不再持久化新凭据
└─ secret_storage_mode: legacy | environment
```

正式工作站 provider 配置：

```toml
model_provider = "workstation"

[model_providers.workstation]
name = "Codex Workstation"
base_url = "https://api.example.com/v1"
wire_api = "responses"
env_key = "CODEX_WORKSTATION_API_KEY"
requires_openai_auth = false
```

禁止生成：

```toml
experimental_bearer_token = "真实Key"
```

禁止向 `auth.json` 写入自定义 API Key。

### 3.6 诊断包脱敏

修改：

```text
apps/codex-plus-manager/src-tauri/src/commands.rs
crates/codex-plus-core/src/diagnostic_log.rs
```

当前 `diagnostics_report()` 会包含完整 settings。正式版必须：

- 删除 `api_key`、`auth_contents`、token、cookie。
- Base URL 只保留 scheme/host，查询参数移除。
- 路径可保留移动盘相对路径，用户名目录匿名化。
- 生成后再次运行 secret scanner。

### 3.7 禁用上游自更新入口

修改：

```text
crates/codex-plus-core/src/update.rs
apps/codex-plus-manager/src-tauri/src/commands.rs
apps/codex-plus-manager/src/App.tsx
```

便携模式下：

- 不请求 `BigPizzaV3/CodexPlusPlus/releases/latest`。
- 不下载和启动 NSIS 安装程序。
- 更新 UI 调用 Workstation Update Client。

普通 Codex++ 安装模式可以保留上游行为，便于 Fork 回馈。

### 3.8 禁用系统安装副作用

便携模式默认关闭：

- 安装桌面/开始菜单快捷方式。
- 注册自启动 Watcher。
- 写系统级协议入口。
- 启动安装程序。

相关上游能力保留代码，但 UI 标记为高级选项或完全隐藏。

## 4. UI 迁移

当前原型页面迁入：

```text
apps/codex-plus-manager/src/
├─ features/home/
├─ features/providers/
├─ features/projects/
├─ features/sessions/
├─ features/plugins/
├─ features/enhancements/
├─ features/remote/
├─ features/data/
├─ features/connection/
├─ features/updates/
├─ features/maintenance/
└─ features/settings/
```

避免继续把全部界面堆在上游当前大型 `App.tsx` 中。迁移时按 feature 拆分，并保留统一的 Tauri invoke adapter。

## 5. 上游同步策略

每次同步：

1. `git fetch upstream`。
2. 建立临时 `upstream-sync/<date>` 分支。
3. Rebase 或 merge 上游 main。
4. 运行上游全部 Rust/TypeScript 测试。
5. 运行工作站便携、凭据、更新和 UI 回归测试。
6. 生成兼容性报告。
7. 只有通过后合并到自己的 main。

重点监控：

- `app_paths.rs`
- `launcher.rs`
- `settings.rs`
- `relay_config.rs`
- `protocol_proxy.rs`
- `commands.rs`
- 注入脚本和 Codex 页面选择器。

## 6. AGPL 交付要求

正式发布修改版时至少提供：

- 对应版本完整源码。
- 构建说明。
- 修改说明。
- AGPL-3.0 许可证。
- 上游版权声明。
- 用户可以获得“正在运行版本对应源码”的明确入口。

如果未来要求客户端闭源，应重新评估“独立启动器 + 未修改 Codex++ 外部进程”路线，而不是隐藏 Fork 源码。

## 7. 第一批 Codex++ 改造顺序

1. `CODEXPP_HOME`。
2. 便携模式和路径测试。
3. 暴露停止命令。
4. Secret reference 和 `env_key`。
5. 诊断脱敏。
6. 禁用原更新器/安装副作用。
7. 迁入首页与全局启动状态机。
8. 按页面迁移剩余 UI。
