# 开源范围与第三方说明

公开日期：2026-10-21。

## 本次公开的代码

仓库维护者将本仓库新增的云枢工作站代码 `workstation/` 和安装工具代码 `installer/` 按 **GNU Affero General Public License v3.0 only（AGPL-3.0-only）** 提供，完整许可证见根目录 `LICENSE`。根目录原 Codex++ 代码继续保留原 AGPL 许可证、作者和提交历史。

`workstation/` 是从已签名安装包提取的原始源码快照，`installer/` 是与包内原生模板匹配的历史构建源码。本次没有修改业务代码或历史包元数据。部分历史 `Cargo.toml` 中仍有 `license = "Proprietary"`；该字段保留用于原始版本复现，**不限制本次明确授予的 AGPL-3.0-only 源码权利**。该公开授权以本说明和根目录许可证为准。

原始 `SOURCE_PROVENANCE.json` 中的候选状态表示当时构建记录，不是对 2026-10-21 发布状态的重新声明。本次核验范围见发布说明。

## 成品与服务

现成 Windows 软件通过工作站卡密激活。用户可加入 QQ 群 **713993476** 免费申请；这不改变上述源码许可证授予的权利。源码公开不代表生产服务器、管理员签名私钥、模型 API 额度或第三方账号一并开放。

## 第三方组件

- Codex++ 的上游作者及历史代码按其原有 AGPL 许可证保留。
- Rust、Tauri、React、Vite 以及其他依赖适用各自许可证。依赖版本见各工程锁文件。
- 安装包中的 Codex、Claude Code、OpenClaw、Mihomo、Skills、插件和其他运行时属于独立第三方组件，**不因本仓库的 AGPL 声明而被重新授权**。使用和再分发须遵守对应组件条款。
- 图标来源保留在 [`ASSET_SOURCES.md`](../workstation/launcher/src/assets/ASSET_SOURCES.md)。品牌名称、商标、宣传截图和视频不作为 AGPL 软件源码授权的对象。
- 云枢品牌素材用于说明本项目；不得用本仓库或展示材料暗示第三方厂商对云枢提供官方背书。

本次不公开管理员 `admin-data`、生产签名私钥、用户密钥、数据库、会话或服务器部署凭据。
