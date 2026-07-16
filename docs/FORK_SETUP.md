# Codex 便携工作站 Fork 配置

## 仓库关系

- 产品 Fork：`https://github.com/JJames646/codex-portable-workstation`
- 上游仓库：`https://github.com/BigPizzaV3/CodexPlusPlus`
- 审计基线标签：`v1.2.37`
- 审计基线提交：`2924333c6770497470090235f484519154698651`
- 阶段 A 分支：`codex/phase-a-baseline`

推荐远端配置：

```text
origin   https://github.com/JJames646/codex-portable-workstation.git
upstream https://github.com/BigPizzaV3/CodexPlusPlus.git
```

`upstream` 只用于拉取上游更新，不用于推送。上游更新必须先进入独立同步分支，经过兼容性审查和完整回归后才能合并到产品分支。

## 阶段 A 边界

阶段 A 只完成：

1. 固定并记录未修改的上游构建基线。
2. 在 Windows 11 x64 验证 Rust、TypeScript、Vite 和 Release 构建。
3. 迁移公开技术文档和已确认的静态 UI 原型。
4. 建立后续工作站开发所需的安全和目录规则。

阶段 A 不实现便携路径、真实 Codex 载荷、服务器连接、卡密、代理服务或更新逻辑。

## Windows 基线命令

```powershell
cargo fmt --all -- --check
cargo test --workspace

Set-Location apps\codex-plus-manager
npm ci
npm run check
npm test
npm run vite:build
npm run build
```

实际结果记录在 `docs/BASELINE_BUILD_REPORT.md`。

## AGPL 对应源码

本项目保留上游 AGPL-3.0 许可证。每个向接收者提供的二进制版本，都必须关联：

- 准确的源码 commit 或 tag。
- 对应构建说明。
- AGPL 许可证和第三方声明。
- 获取对应修改源码的明确方式。
