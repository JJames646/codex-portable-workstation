# ADR-0006：通过 PortableCodexRuntimePort 延后接入真实 Codex 载荷

## 状态

Accepted

## 背景

启动器、U 盘路径、UI、进程管理和本地数据需要先开发，但真实 U 盘版 Codex 载荷由用户后续提供。Windows 开发机当前存在 Microsoft Store 安装版 Codex，不能把安装版路径当作最终便携依赖，也不能通过批量停止 `Codex.exe` 测试便携进程管理。

## 决策

启动器依赖稳定的 `PortableCodexRuntimePort`，提供 `MissingRuntime`、`SmokeTestRuntime` 和后续 `CodexDesktopRuntime` 三个实现。

RC1 使用 SmokeTestRuntime 验证启动器完整流程；RC2 在用户提供载荷后接入 CodexDesktopRuntime。

## 正面影响

- 启动器开发不被真实 Codex 载荷阻塞。
- 不依赖电脑上安装版 Codex。
- 可以提前验证 Job Object、PID 树、停止和安全弹出。
- 后续替换载荷不需要重写 UI 和工作站状态机。

## 负面影响

- RC1 不能宣称已经启动真实 U 盘 Codex。
- 需要维护运行时 manifest 和额外的测试运行时。
- 最终仍必须进行真实 Codex 集成测试。

## 备选

- 直接使用 Microsoft Store Codex：拒绝，因为不能证明便携性并可能影响用户日常 Codex。
- 等待真实载荷后再开发：拒绝，因为会无意义阻塞 UI、路径和进程框架。
- 在启动器中写死 Codex 路径：拒绝，因为移动盘盘符和版本目录会变化。
