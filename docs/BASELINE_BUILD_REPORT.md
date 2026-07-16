# 阶段 A：Windows 构建基线报告

## 1. 结论

Codex++ 上游 `v1.2.37` 在目标 Windows 11 x64 开发机上完成了未修改源码的基线验证。

结果：

- Rust 格式检查通过。
- TypeScript 类型检查通过。
- 前端测试通过，14 项测试全部成功。
- Vite 生产构建通过。
- Rust 全工作区测试通过。
- Windows Release 构建通过。
- 生成静默启动器和桌面管理器两个 EXE。

阶段 A 未修改 Codex++ 业务源码，也未连接工作站业务服务器。

## 2. 源码基线

| 项目 | 值 |
|---|---|
| 上游仓库 | `BigPizzaV3/CodexPlusPlus` |
| 上游标签 | `v1.2.37` |
| 基线提交 | `2924333c6770497470090235f484519154698651` |
| 产品分支 | `codex/phase-a-baseline` |
| Windows 工作目录 | `D:\Dev\codex-portable` |

Windows 构建前后的 Git 内容哈希一致。构建产物和依赖目录均未提交到仓库。

## 3. Windows 开发环境

| 工具 | 版本 |
|---|---|
| Windows | Windows 11 x64，版本 `10.0.22621`，Build `22621` |
| Git | `2.55.0.windows.3` |
| Node.js | `v24.18.0` |
| npm | `11.16.0` |
| rustc | `1.97.0 (2d8144b78 2026-07-07)` |
| cargo | `1.97.0 (c980f4866 2026-06-30)` |

日常构建使用普通用户权限和 `cmd.exe`。Windows PowerShell 的脚本执行策略会阻止直接执行 `npm.ps1`，因此自动化命令应使用 `npm.cmd` 或 `cmd /c npm ...`，不需要修改系统执行策略。

## 4. 验证结果

### 4.1 Rust 格式

```powershell
cargo fmt --all -- --check
```

结果：通过。

### 4.2 前端依赖

```powershell
Set-Location apps\codex-plus-manager
npm ci
```

结果：通过，安装 101 个 package，审计 102 个 package。

上游锁文件当前报告：

- 1 个 low vulnerability。
- 1 个 high vulnerability。
- `esbuild@0.25.12` install script 未显式加入 allow-scripts 的提示。

阶段 A 不执行 `npm audit fix`，因为它可能改变上游锁文件和基线行为。应在后续依赖审计任务中单独评估并测试。

### 4.3 TypeScript

```powershell
npm run check
```

结果：通过。

### 4.4 前端测试

```powershell
npm test
```

结果：通过，14 项测试全部成功。

### 4.5 Vite 生产构建

```powershell
npm run vite:build
```

结果：通过，Vite `6.4.2` 完成 1607 个模块转换。

### 4.6 Rust 全工作区测试

首次执行：

```powershell
cargo test --workspace
```

编译成功，但上游 `ads` 集成测试中的本地 HTTP 回退用例收到 `127.0.0.1` 的 502。

根因是开发机当前用户启用了本地系统代理。测试使用的 Reqwest 客户端没有按系统代理绕过列表正确处理测试随机端口，把 localhost 请求送入了代理。没有关闭或修改 Windows 系统代理。

只对测试子进程临时增加：

```powershell
$env:NO_PROXY = "127.0.0.1,localhost"
cargo test --workspace --offline
```

结果：全工作区测试全部通过。

该现象属于可复现的上游测试环境兼容点。后续 Windows CI 应显式为本地集成测试设置 `NO_PROXY`，产品运行时不得依赖修改系统全局代理。

### 4.7 Windows Release 构建

```powershell
Set-Location apps\codex-plus-manager
npm run build
```

结果：通过。

上游 `tauri.conf.json` 当前设置 `"bundle.active": false`，因此本命令生成 Release EXE，不生成 MSI、NSIS 等安装包。这符合当前基线，不代表便携版最终发布格式已经完成。

## 5. Release 产物

| 文件 | 大小 | SHA-256 |
|---|---:|---|
| `target\release\codex-plus-plus.exe` | 13,470,208 bytes | `B3F1C1E563AB0777EFF67542B007F8E87CDADD85C9E4E2855FD3F62784093E0C` |
| `target\release\codex-plus-plus-manager.exe` | 26,489,344 bytes | `5F542C6738600695BE07CF190BC66E0F4443F9A40517F32E4CE3AD2B4B2051DC` |

这些哈希只对应本次本地基线构建，不是正式发布签名。

## 6. UI 原型和公开文档

阶段 A 已迁入：

- `prototype/workstation-launcher-ui/`：已确认的静态交互原型。
- `docs/workstation/`：架构、功能对接、安全、更新、许可和 ADR。
- `docs/FORK_SETUP.md`：Fork、上游和基线说明。

验证：

```text
node --check prototype/workstation-launcher-ui/app.js
```

结果：通过。

原型中的私人主机名称和地址已经替换为非真实示例。服务器状态仍为前端模拟，阶段 B 必须改为明确的 `NotConnected` Adapter。

## 7. 阶段 A 风险记录

1. npm 审计仍有 1 个 low 和 1 个 high vulnerability，需要后续专项审计。
2. Windows 系统代理会影响 Reqwest localhost 测试，CI 和测试脚本需要使用进程级 `NO_PROXY`。
3. 上游目前只生成 EXE，不生成安装包；便携工作站最终采用目录/ZIP 交付时需要新增打包清单、哈希和签名流程。
4. 当前产物仍是原版 Codex++，不是便携工作站，也不能宣称已从 U 盘启动真实 Codex。
5. 真实便携 Codex 应用载荷和商业再分发许可仍属于后续独立 Gate。

## 8. 下一阶段入口

阶段 A 基线通过后，可以进入阶段 B：

- 将静态原型迁移到 `apps/codex-plus-manager`。
- 保持现有 UI 设计和信息架构。
- 建立 `workstation-api.ts`。
- 首页对接真实 Codex++ 本地概览。
- 两套业务服务器先使用 `NotConnected` Adapter。
- 不接入卡密、管理员后台或工作站连接服务。
