# AGENTS.md

本文件为 Codex 便携工作站 Fork 的工作规范，指导 agent 在本仓库工作。

## 项目概述

本仓库是 [BigPizzaV3/CodexPlusPlus](https://github.com/BigPizzaV3/CodexPlusPlus) 的公开 fork，目标是在保留 Codex++ 现有能力的基础上开发 Windows「Codex 便携工作站」。

当前基线固定为上游 `v1.2.37`。阶段 A 只迁移公开技术文档、UI 原型并验证 Windows 构建，不改变上游业务功能。后续阶段必须按 `docs/workstation/` 中的架构和安全边界实施。

## 仓库结构

- `crates/codex-plus-core/` — 核心 Rust 库（配置生成、catalog 解析、数据模型）
- `apps/codex-plus-manager/` — Tauri 桌面应用，前端 React+TS
- `crates/codex-plus-data/` — 数据持久化
- `docs/workstation/` — 便携工作站公开架构、安全、集成和更新设计
- `prototype/workstation-launcher-ui/` — 已确认的静态 UI 交互原型

## 工作站开发边界

- 不写死 Windows 盘符，移动盘根目录必须从当前 EXE 位置推导。
- `CODEX_HOME`、`CODEX_SQLITE_HOME`、`CODEXPP_HOME` 只对子进程设置。
- 不修改 Windows 全局代理、路由表或永久环境变量。
- 用户数据只归入移动盘 `Data/`、`Workspace/`、`Backups/`。
- 不实现通用 VPN、任意目标代理或上游权限绕过。
- 两套业务服务器在接入阶段前必须使用明确的 `NotConnected` Adapter。

## 安全规则

- 禁止批量删除、rm -rf、rmdir /s
- 删除只能单个文件，删除前确认
- 禁止 sudo、提权、curl | bash
- 禁止泄露密钥、.env、auth.json、config.toml 凭据
- 覆盖文件前确认
- 不擅自改 Cargo.toml、package.json、.gitignore（除非任务必需）

## 命令执行

- 执行 bash 命令前确认
- 不运行未知脚本、不擅自装依赖
- 测试用 cargo test，不另起工具链

## 编码规范

- 对话用中文，代码可用英文，注释尽量中文
- 保持上游代码风格统一（Rust 标准、React+TS）
- 工作站改动隔离、opt-in，保持普通 Codex++ 安装模式兼容
- 不做需求外的操作

## 测试约定

- 沿用上游 `#[test]` + tempfile 风格（见 `crates/codex-plus-core/tests/relay_config.rs`）
- 断言读 config.toml 文本，如 `assert!(config.contains("model_catalog_json"))`
- 改行为要同步改/加对应测试

## 与上游同步

- `upstream` = https://github.com/BigPizzaV3/CodexPlusPlus.git
- `origin` = https://github.com/JJames646/codex-portable-workstation.git
- 阶段分支使用 `codex/` 前缀
- 定期 `git fetch upstream && git rebase upstream/main` 保持同步
- 每个发行二进制必须关联准确源码 commit/tag、构建说明、AGPL 许可证和第三方声明
