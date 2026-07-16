# 安全模型

## 1. 保护对象

- 用户 API Key。
- 工作站卡密 entitlement、短期服务令牌和中转站 API Key。
- 项目源代码、会话、图片和本地记忆。
- 更新签名和发布链。
- activation-bridge 管理员凭据。
- 移动盘数据完整性。

## 2. 主要威胁

| 威胁 | 示例 |
|---|---|
| 移动盘丢失 | 他人读取 API Key、会话和项目 |
| 恶意电脑 | 读取进程内存、键盘输入或移动盘文件 |
| 日志泄露 | Codex++ settings/URL 中出现 Key |
| 更新供应链 | 下载被替换、GitHub 账号失陷 |
| 远程 WebView | 网站获得本地文件/进程权限 |
| 卡密盗用 | 卡密在激活前泄露或被批量猜测 |
| 服务端越权 | 客户端调用管理员接口 |
| 任意代理滥用 | sidecar 被其他程序当作通用代理 |

移动工作站无法防御已经完全失陷的宿主电脑。产品必须明确这一边界。

## 3. 无 PIN 本地凭据库

文件：

```text
Data/Secrets/vault.bin
Data/Secrets/vault.meta.json
```

用户已经明确不设置 PIN 或工作站解锁密码。建议：

- 加密：AES-256-GCM。
- 首次启动生成随机 256-bit `vault_key`。
- 每条记录独立 nonce。
- 密文包含版本和 AAD。
- `vault_key` 保存在同一移动存储的受控文件中，不写入普通设置和日志。
- 启动时自动加载，仅向工作站子进程注入当前供应商 secret。

不单独依赖 Windows DPAPI，因为 DPAPI 默认绑定单台电脑/用户，不适合跨电脑 U 盘。

安全边界：

- vault key 与密文都在同一移动存储中，因此该设计主要避免普通明文配置、误读、日志和诊断泄漏。
- 该设计不能防止移动存储丢失后被有能力的人员同时读取 vault key 和密文。
- UI 必须在首次导入 API Key 时明确提示该风险。
- 后续可以把 PIN 作为可选增强，但不能设为默认必填。

## 4. 运行时 secret 注入

1. Secret Broker从受控文件自动加载 vault key。
2. Secret Broker 解密当前供应商 Key。
3. 只给 Codex++/Codex 子进程设置指定环境变量。
4. `config.toml` 只保存 `env_key` 名称。
5. 不在命令行、URL 和永久环境变量中传递 Key。
6. 停止后清理 Secret Broker 内存。

注意：环境变量仍可被同一用户权限的恶意进程读取，因此不能把宿主电脑当作可信硬件。该方案的目标是避免静态泄露和日常日志泄露。

## 5. Codex++ 安全改造

- 禁止新凭据写入 `RelayProfile.api_key`。
- 禁止生成明文 `experimental_bearer_token`。
- 禁止自定义 Key 写入 `auth.json`。
- Provider import URL 不允许携带真实 Key。
- 诊断报告不序列化完整 settings。
- 日志统一 secret redactor。

旧数据迁移：

1. 检测旧明文 Key。
2. 用户确认后导入加密 vault。
3. 将 provider 改为 credential reference。
4. 安全重写 settings/config。
5. 提醒旧备份中仍可能包含明文。

## 6. 服务器认证

### 工作站卡密

- 工作站授权不要求用户账号。
- 卡密使用安全随机数生成并限制尝试次数。
- 卡密从首次成功激活开始计时。
- `installation_id` 绑定移动工作站实例而不是宿主电脑。
- 安装实例公钥绑定。
- 服务器签名 entitlement。
- 短期 access token。
- refresh token rotation。
- 可撤销。
- rate limit。
- 最大绑定数量和异常并发检测。

### 中转站网站

- 如果中转站网站本身需要账号，使用 HTTPS 和安全 Cookie。
- 工作站不保存网站账号密码，只接收用户主动复制粘贴的 API Key。

### 管理接口

- activation-bridge 到 Sub2API 管理接口使用独立服务器凭据。
- 管理凭据最小权限。
- 生产管理入口限制来源、MFA 和审计。
- 客户端永远不接收管理员 Key。

## 7. 远程 WebView

- 使用独立远程 WebviewWindow。
- 不注册本地危险 Tauri capabilities。
- 只允许 `https://account.example.com` 等精确域名。
- 禁止 `window.open` 任意 scheme。
- 外部下载进入受控目录并校验类型。
- 不允许远程页面读取 vault、文件系统、Shell 或 updater。
- Content Security Policy 与服务器 CSP 双重限制。

## 8. Sidecar

- 只绑定 loopback。
- 端口随机或冲突检测。
- 父进程会话认证。
- 目标域名 allowlist。
- 禁止任意 CONNECT。
- 限制并发、速率、请求大小和超时。
- 不缓存正文。
- 进程退出即失效。

## 9. 更新安全

- HTTPS 只是第一层，必须验证 Ed25519。
- 根公钥固定在 updater。
- 清单和每个组件都签名。
- 防止版本降级，除非用户主动选择已签名回滚版本。
- `revoked.json` 支持撤销。
- 解压防止 Zip Slip。
- 组件路径白名单。
- 更新器不跟随压缩包内符号链接写出目标目录。

## 10. 日志和诊断

允许：

- 版本。
- 组件状态。
- 错误代码。
- 脱敏 host。
- 相对路径。
- 请求 ID、耗时和状态码。

禁止：

- API Key。
- Authorization/Cookie。
- 用户网站密码。
- 管理员 Key。
- 完整项目源代码。
- 完整对话正文。
- 完整邮箱和个人信息。
- 带 token 的 URL。

诊断包生成后必须扫描：

```text
sk-
Bearer
Authorization
refresh_token
access_token
api_key
cookie
password
```

扫描命中后阻止导出并提示开发者修复。

## 11. 文件与路径安全

- 所有相对路径解析后必须仍在 `USB_ROOT` 允许目录内。
- 更新解压和恢复禁止 `..` 路径穿越。
- 不执行移动盘项目中的未知启动脚本作为环境配置。
- 外部输入的 EXE 路径必须验证文件类型、签名/哈希和目录边界。
- OS 命令使用参数数组，不拼接 Shell 字符串。

## 12. 隐私

- 本地项目和对话默认不上传到工作站后台。
- 仪表盘只上传模型用量和服务必要元数据。
- 环境失败统计必须匿名或可关闭。
- 发布前准备隐私政策、数据保留周期和删除机制。

## 13. 安全测试

- Secret 不落盘测试。
- 日志和诊断泄漏测试。
- 无 PIN 风险提示和同盘 vault key 安全边界测试。
- 路径穿越和恶意 ZIP 测试。
- 卡密暴力尝试、重复激活、绑定上限、令牌重放和撤销测试。
- 远程 WebView 权限隔离测试。
- Sidecar 任意目标和非父进程访问拒绝测试。
- 更新签名、损坏文件、撤销和回滚测试。
