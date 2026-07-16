# 工作站更新与上游同步协议

## 1. 两层更新体系

```text
社区上游
├─ CodexPlusPlus
├─ unofficial-codex-app-offline
├─ Sub2API
└─ openai/codex 与官方文档
       ↓ 仅开发机/CI 定时监控
构建、测试、许可、兼容矩阵、签名
       ↓
自己的 GitHub Release 更新项目 / 镜像
├─ stable.json
├─ beta.json
├─ revoked.json
├─ Release assets
└─ 自有 updates 域名/CDN
       ↓
用户移动盘工作站
```

用户端永远不对社区仓库执行 `git pull`。

## 2. 客户端唯一更新源

初期更新源：

```text
https://github.com/<owner>/codex-workstation-releases/releases/latest/download/stable.json
```

客户端每次插入并启动时异步检查该项目的签名清单，不扫描源码差异，也不执行 `git pull`。

正式商业环境建议增加镜像：

```text
https://updates.example.com/workstation/stable.json
```

GitHub Release 作为发布源站和故障备用；国内用户客户端不应长期只依赖 GitHub 可达性。

仓库建议：

```text
codex-portable-workstation    公开 AGPL 客户端源码和 Codex++ Fork
codex-workstation-server      activation-bridge 和管理员后台
codex-workstation-releases    签名清单、发行包和发行说明，不放私钥
```

其中服务器仓库保持私有，客户端源码和发行仓库公开。实际创建仓库前确认用户 GitHub 登录状态和组织/账号名称。

公开客户端不能内置 GitHub PAT。私有测试更新应使用受控下载服务或短期签名 URL。

## 3. 组件划分

```text
workstation_manager
portable_codex
connection_service
runtime_assets
plugin_index
```

Sub2API 服务端不通过客户端更新协议更新。

## 4. stable.json 草案

```json
{
  "schema_version": 1,
  "channel": "stable",
  "release_id": "2026.07.16.1",
  "published_at": "2026-07-16T00:00:00Z",
  "minimum_updater_version": "1.0.0",
  "components": {
    "workstation_manager": {
      "version": "1.0.0",
      "url": "https://updates.example.com/assets/workstation-manager-1.0.0.zip",
      "size": 123,
      "sha256": "...",
      "signature": "..."
    },
    "portable_codex": {
      "version": "26.707.9981.0",
      "url": "https://updates.example.com/assets/portable-codex-26.707.9981.0.zip",
      "size": 123,
      "sha256": "...",
      "signature": "..."
    }
  },
  "compatibility": {
    "workstation_manager": "1.0.0",
    "portable_codex": "26.707.9981.0",
    "connection_service": "1.0.0"
  },
  "data_schema": {
    "minimum": 1,
    "maximum": 1
  },
  "release_notes_url": "https://updates.example.com/notes/2026.07.16.1",
  "manifest_signature": "..."
}
```

## 5. 签名

- 根 Ed25519 公钥固定在启动器/Updater 中。
- manifest 使用独立 release signing key 签名。
- 每个 asset 同时提供 SHA-256 和签名。
- 初期私钥可以由 GitHub Actions Secret 注入签名步骤，禁止写入仓库、Release 或客户端。
- 正式商业发布推荐将签名迁移到硬件/云 KMS 或离线签名机。
- 支持 `revoked.json` 撤销被发现有问题的 release。
- 公钥轮换必须由旧根密钥签名新公钥声明。

## 6. 启动检查

1. 读取本地 `manifest.json`。
2. 每次启动异步下载 GitHub Release 或镜像中的 `stable.json`，超时不阻塞启动。
3. 验证 HTTPS、schema、签名、发布时间和撤销状态。
4. 比较语义版本和兼容矩阵。
5. UI 显示更新点和变更摘要。
6. 用户选择更新、稍后或跳过本次。

安全修复可以支持强提醒，但除非存在明确不可运行或已撤销版本，不默认静默强制更新。

## 7. 下载

- 下载到 `Updater/staging/<release_id>/`。
- 支持断点续传时必须校验完整文件。
- 磁盘不足时在下载前失败。
- 先验证大小，再 SHA-256，再签名。
- staging 校验失败立即删除该包。

## 8. 应用更新

1. 创建 `Backups/pre-update-<release_id>`。
2. 停止 Codex、Codex++、helper 和 sidecar。
3. 启动独立 Updater。
4. 主程序退出。
5. Updater 等待文件句柄释放。
6. 旧组件目录改名为 `.previous`。
7. 新组件解压到临时目录。
8. 验证目录 manifest。
9. 原子切换为 current。
10. 启动健康检查。
11. 成功后保留有限数量 previous。
12. 失败则自动恢复旧目录。

## 9. 永不覆盖的数据

```text
Data/
Workspace/
Backups/
Credentials/ 或 Data/Secrets/
用户自定义配置
```

如果数据 schema 需要迁移：

- 必须先备份。
- 使用向前迁移脚本。
- 记录 migration journal。
- 回滚时恢复备份或保证旧版本可读。

## 10. 健康检查

- 主 EXE 可启动。
- Tauri WebView 可加载本地页面。
- Codex++ Core 版本正确。
- 移动盘 Codex 路径可识别。
- `CODEX_HOME` 和 `CODEXPP_HOME` 可读写。
- sidecar 可绑定 loopback。
- 不实际发送用户代码到远程。

## 11. 上游监控 CI

### Codex++

- 检查新 Release/tag。
- 拉取 Fork upstream。
- 编译 Rust/React。
- 运行工作站补丁回归。

### Portable Codex

- 检查新 Release/Store 版本。
- 私有构建。
- 验证 app 路径和至少 15 秒启动。
- 记录载荷来源和许可状态。

### Sub2API

- 检查新 Release/main 变更。
- 运行 adapter 契约测试。
- 只生成生产升级报告，不从客户端更新。

### OpenAI Codex

- 监控 provider schema、`CODEX_HOME`、`CODEX_SQLITE_HOME`、app-server 和插件格式变化。

## 12. 发布通道

- `stable`：只包含通过 Windows 实机测试的兼容组合。
- `beta`：较新上游和测试用户。
- `revoked`：阻止已知有严重问题的版本继续更新或运行特定功能。

## 13. GitHub Actions 建议

```text
monitor-codexplusplus.yml
monitor-portable-codex.yml
monitor-sub2api.yml
monitor-openai-codex.yml
build-client.yml
build-portable-codex.yml
compatibility-test.yml
publish-beta.yml
publish-stable.yml
rollback-release.yml
```

## 14. 原 Codex++ 更新器处理

便携模式下必须关闭：

```text
DEFAULT_LATEST_JSON_URL
perform_update() 启动安装程序
静默 launcher 自动拉起上游更新提示
```

避免工作站管理器绕过产品兼容矩阵自行升级。
