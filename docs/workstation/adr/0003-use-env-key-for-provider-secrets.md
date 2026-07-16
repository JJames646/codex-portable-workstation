# ADR-0003：供应商凭据使用加密 Vault 与 env_key

## 状态

Proposed

## 背景

Codex++ 上游会把 API Key 写入 settings、config 或 auth 文件，不符合移动盘丢失和诊断泄漏防护要求。

## 决策

真实 Key 只保存在使用同盘随机 vault key + AES-256-GCM 的 Secret Store 中。Codex provider 配置只保存 `env_key`，运行时对子进程注入。用户不设置 PIN，因此必须明确提示该方案不能防止移动盘丢失后的专业密钥提取。

## 正面影响

- 避免明文静态泄漏。
- 不进入命令行、URL 和普通诊断。
- 跨电脑可用。

## 负面影响

- 用户需要解锁 Vault。
- 宿主电脑被完全控制时仍可能读取进程内存/环境。
- 需要改造 Codex++ Provider 模型和测试流程。

## 备选

- Windows DPAPI：不适合跨电脑。
- auth.json 明文：安全性不足。
- 只使用本地 sidecar 固定 token：仍需保护 sidecar 的服务器凭据。
