# Codex 便携工作站公开设计文档

本目录保存基于 Codex++ 二次开发的便携工作站技术设计。阶段 B 已将确认过的工作站 UI 整合进 Codex++ Manager，并接入真实本地概览；当前仍不包含真实便携 Codex 载荷，也不连接产品业务服务器。

核心文档：

- `ARCHITECTURE.md`：客户端、运行时、数据和未来服务端边界。
- `FEATURE_INTEGRATION_MATRIX.md`：UI 与 Codex++、新增 Workstation Core 的功能映射。
- `CODEXPLUSPLUS_INTEGRATION.md`：上游复用点和需要扩展的接口。
- `SECURITY_MODEL.md`：凭据、进程、诊断、安全退出和服务器权限边界。
- `UPDATE_PROTOCOL.md`：自有签名更新源、校验和回滚协议。
- `LICENSE_REVIEW.md`：上游许可证和对应源码义务。
- `adr/`：已经确认的关键架构决策。
- `../PHASE_B_IMPLEMENTATION_REPORT.md`：阶段 B 实现范围、验证结果和下一阶段边界。

当前已确认的浏览器交互原型位于 `prototype/workstation-launcher-ui/`。
