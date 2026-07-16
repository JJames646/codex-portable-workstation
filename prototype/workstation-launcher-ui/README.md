# Codex 便携工作站 UI 原型

此目录保存 2026-07-16 确认的浏览器交互原型。

它是正式 Windows 客户端的界面参考，不是生产软件，所有进程、更新、卡密、服务器和安全弹出操作目前都是前端模拟。

## 本地查看

```bash
python3 -m http.server 8787
```

浏览器打开 `http://127.0.0.1:8787/`。

## 正式实现方向

原型将在阶段 B 迁移到 Codex++ Fork 的 `apps/codex-plus-manager` React/Tauri 工程。迁移时保留现有信息架构，但按钮必须按 `docs/workstation/FEATURE_INTEGRATION_MATRIX.md` 对接真实 Tauri Commands、Codex++ Core、工作站模块和服务器 Adapter。

## 当前限制

- 页面中的版本、用量、路径、项目和状态是演示数据。
- “我的中转站”详情页尚未加载远程网站。
- 环境检测、启动、停止、弹出、更新、卡密和恢复出厂设置尚未操作真实系统。
- 正式版本不得直接把远程网站放入拥有 Tauri 本地权限的 WebView 上下文。
- 当前阶段的两套业务服务器保持 `NotConnected`，不得把演示状态当作真实联网结果。
