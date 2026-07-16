# Codex 便携工作站总体架构

## 1. 推荐架构

采用“Codex++ Fork 客户端 + 独立工作站模块 + Sub2API 服务端 + 自有更新控制面”的组合。

```text
┌────────────────────────────────────────────────────────────┐
│ 移动存储                                                   │
│                                                            │
│  Workstation Manager（Codex++ Fork / Tauri / React）       │
│  ├─ Workstation Core：路径、状态机、环境、备份、安全弹出   │
│  ├─ Codex++ Core：供应商、会话、插件、增强、CDP 启动       │
│  ├─ Secret Broker：加密凭据与进程环境注入                  │
│  ├─ Update Client：签名清单、下载、原子替换、回滚          │
│  └─ Connection Sidecar：仅连接允许的工作站服务             │
│             │                                              │
│             ├──启动──▶ Portable Codex Desktop              │
│             │          CODEX_HOME / CODEX_SQLITE_HOME      │
│             │                                              │
│             └──读取──▶ Workspace / Data / Backups          │
└────────────────────────────────────────────────────────────┘
                  │ HTTPS
                  ▼
┌────────────────────────────────────────────────────────────┐
│ 自有服务域名                                               │
│  ├─ Access Gateway / API Gateway                           │
│  ├─ activation-bridge：卡密、工作站绑定、续签与撤销       │
│  ├─ route-config：签名服务端点清单                          │
│  ├─ update-manifest：稳定更新清单                           │
│  └─ Sub2API：用户、Key、套餐、支付、用量和模型转发         │
└────────────────────────────────────────────────────────────┘
                  │
                  ▼
          合法授权的模型 API 服务
```

## 2. 为什么选 Codex++ Fork

### 方案 A：Codex++ Fork 作为正式客户端（推荐）

优点：

- 最大程度复用已存在功能。
- 当前 UI 原型可以直接迁移到 React/Tauri。
- 启动、供应商、协议转换、会话、插件和页面增强都在同一进程体系。
- 减少 WPF 与 Rust/Tauri 双 UI、双状态和 IPC 同步问题。

代价：

- 属于 Codex++ 衍生版本，必须履行 AGPL。
- 需要长期跟踪 Codex++ 上游。

### 方案 B：独立 WPF 启动器 + 原版 Codex++

优点：

- 主启动器可以保持闭源边界。
- Codex++ 可作为独立未修改程序分发。

缺点：

- UI 无法真正统一。
- 供应商、会话和插件能力需要跨进程调用或重复开发。
- Codex++ 自身状态仍不便携，最终大概率仍需 Fork。

### 方案 C：新客户端嵌入 Codex++ 管理页面

缺点明显：

- 没有稳定公开的远程管理 API。
- WebView 嵌套和本地权限边界复杂。
- 上游 UI 变化容易破坏集成。

因此不推荐。

## 3. 客户端模块

### 3.1 Workstation Shell

- 当前 UI 原型的正式实现。
- 负责导航、状态展示、错误提示和用户确认。
- 不直接读写敏感文件，通过 Tauri Commands 调用 Rust。

### 3.2 Portable Root Resolver

- 从 `current_exe()` 获取当前程序目录。
- 向上查找 `workstation.json`。
- 解析出 `USB_ROOT`。
- 所有产品路径通过 `PathLayout` 生成。
- 不使用固定盘符。

### 3.3 Launch Orchestrator

状态机：

```text
Idle
→ Checking
→ UpdateChecking
→ LoadingSecrets
→ PreparingDirectories
→ PreparingConnection
→ ApplyingProvider
→ StartingCodexPlusPlus
→ StartingCodex
→ Running
→ Stopping
→ FlushingData
→ CreatingRestorePoint
→ ReadyToEject
```

任何一步失败都必须返回结构化错误和可恢复动作。

### 3.4 Codex++ Compatibility Layer

- 设置 `CODEXPP_HOME`。
- 设置移动盘 Codex `--app-path`。
- 调用原有供应商、会话、插件和增强 Core。
- 关闭安装器、快捷方式、Watcher 和原更新器的默认入口。
- 将停止动作暴露为 Tauri Command。

### 3.5 Secret Broker

- 首次运行生成随机 vault key，并与加密凭据一起保存在移动盘受控目录。
- 凭据用 AES-256-GCM 加密，启动时自动加载，不要求 PIN。
- 配置中只写 `credential_id` 和 `env_key`。
- 启动子进程时注入真实 Key。
- 子进程退出后清理内存中的 Key。
- 不把 Key写入系统永久环境变量。
- 同盘保存 vault key 不能防止移动盘丢失后的专业提取，UI 必须明确提示。

### 3.6 Connection Sidecar

Sidecar 只解决两类问题：

1. 将 Codex 请求稳定转发到本产品自有 API 域名。
2. 在本地进程中安全附加工作站令牌或执行协议适配。

强制限制：

- 仅绑定 `127.0.0.1`。
- 只允许服务器签名清单中的 HTTPS 目标。
- 不支持任意域名、任意端口、SOCKS 全局代理或系统代理写入。
- 不转发电脑其他程序流量。
- 所有授权由服务器校验。

### 3.7 Update Client 与 Updater Stub

主程序运行时不能可靠替换自身，因此使用独立 `Workstation.Updater.exe`：

- 主程序下载并验证到 staging。
- 主程序正常退出。
- Updater 等待文件释放。
- 旧目录重命名为 `.previous`。
- 新目录原子切换。
- 启动健康检查。
- 失败时恢复 `.previous`。

## 4. 服务器模块

### 4.1 Sub2API

保持独立部署，负责：

- 中转站网站用户和登录。
- 中转站用户 API Key。
- 余额、订阅、支付和卡密。
- 用量统计和仪表盘。
- 模型路由、计费、限流和转发。

### 4.2 activation-bridge

独立服务，职责：

- 工作站卡密生成规则和激活验证。
- 以移动存储为单位绑定 `installation_id` 和公钥。
- 从首次激活开始计算有效期。
- 控制每张卡密的最大绑定数量。
- 签发可离线验证的 entitlement 和短期工作站访问令牌。
- 续签、解绑、禁用和异常并发审计。
- 查询本产品中转站套餐权益和连接服务有效期。

它可以在服务器端调用 Sub2API 管理接口，但管理员秘密只保留在服务器环境变量或 Secret Manager。

### 4.3 Route Config

返回签名端点清单：

```json
{
  "schema_version": 1,
  "issued_at": "2026-07-16T00:00:00Z",
  "expires_at": "2026-07-17T00:00:00Z",
  "routes": [
    {
      "id": "primary",
      "api_base_url": "https://api.example.com/v1",
      "health_url": "https://api.example.com/health",
      "priority": 100
    }
  ],
  "signature": "..."
}
```

客户端只在这些端点间做健康检测和故障切换。

## 5. 数据布局

```text
Codex-Workstation/
├─ Codex便携工作站.exe
├─ workstation.json
├─ manifest.json
├─ App/
│  ├─ Workstation/
│  ├─ Codex/
│  │  └─ app/
│  └─ Sidecars/
├─ Data/
│  ├─ Codex/
│  │  └─ .codex/
│  │     ├─ config.toml
│  │     ├─ sessions/
│  │     ├─ archived_sessions/
│  │     ├─ skills/
│  │     ├─ sqlite/
│  │     ├─ generated_images/
│  │     └─ logs/
│  ├─ CodexPlusPlus/
│  │  ├─ settings.json
│  │  ├─ latest-status.json
│  │  ├─ user_scripts/
│  │  └─ logs/
│  ├─ Secrets/
│  ├─ Index/
│  └─ Temp/
├─ Workspace/
├─ Backups/
│  ├─ latest/
│  ├─ previous/
│  └─ emergency/
├─ Updater/
│  ├─ staging/
│  └─ previous/
├─ Diagnostics/
├─ Logs/
└─ Licenses/
```

## 6. 启动数据流

```text
用户点击启动
→ Health 检查移动盘
→ 解锁 Secret Store
→ 获取/验证 route manifest
→ 检查工作站服务权益
→ 启动 Connection Sidecar
→ 生成不含明文 Key 的 Codex provider config
→ 注入 CODEX_HOME / CODEX_SQLITE_HOME / CODEXPP_HOME / env_key
→ Codex++ 通过 --app-path 启动移动盘 Codex
→ CDP 注入页面增强
→ 进入 Running
```

## 7. 停止和安全弹出数据流

```text
用户点击停止
→ 请求 Codex 正常退出
→ 停止 Codex++ helper/launcher
→ 停止 Connection Sidecar
→ 等待 SQLite checkpoint 和 WAL/SHM 释放
→ 写入 workstation.json 正常退出标记
→ 创建 latest 恢复点
→ 进入 ReadyToEject
→ 用户点击弹出
→ 检查移动盘文件句柄
→ Windows 安全弹出
```

## 8. 非功能要求

### 性能

- 本地 UI 首屏目标：2 秒内可操作。
- 离线环境检测目标：3 秒内完成。
- 在线检查不阻塞启动，单请求超时 3–5 秒。
- Sidecar 不缓存用户代码和提示内容。

### 可用性

- 更新服务不可用时继续启动本地版本。
- Sub2API 仪表盘不可用时不影响本地项目浏览。
- 页面增强失败时降级为无增强 Codex。
- Connection Sidecar 失败时显示明确错误，不静默改全局网络。

### 数据可靠性

- 移动盘写入前检查剩余空间。
- SQLite 关闭前等待 WAL/SHM。
- 更新和关键配置变更前创建恢复点。
- 默认推荐 NTFS；exFAT 显示可靠性和权限警告。

### 安全

- 全部远程接口 HTTPS。
- 设备令牌短期有效、可撤销。
- 更新和线路清单使用签名。
- 远程 WebView 不拥有本地 Tauri Command 权限。
- 所有诊断包执行敏感信息扫描。

## 9. 主要失败模式

| 失败 | 用户影响 | 处理 |
|---|---|---|
| U 盘只读或空间不足 | 无法启动或保存 | 启动前阻止并显示修复建议 |
| `CODEX_HOME` 不存在 | Codex++ 回退电脑本地目录 | 启动前强制创建并校验 |
| Codex++ 注入失败 | 页面增强不可用 | 降级启动原始 Codex |
| 服务器不可达 | 仪表盘/服务连接不可用 | 本地功能继续；需要服务的模型不可调用 |
| 更新签名错误 | 可能为损坏或攻击 | 删除 staging，继续旧版 |
| 新版本健康检查失败 | 新版不可用 | 自动回滚 |
| SQLite 被占用 | 无法弹出 | 列出占用进程，不默认强杀 |
| 移动盘意外拔出 | 数据可能损坏 | 下次启动检测异常退出并提供恢复点 |
