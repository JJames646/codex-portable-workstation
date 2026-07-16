# 阶段 B：工作站 UI 集成报告

## 1. 结论

阶段 B 已将确认过的“Codex 便携工作站”界面整合进 Codex++ Manager，并保留原有 Codex++ 功能页面作为工作站模块。

当前客户端已经具备：

- 工作站统一外壳、导航、浅色和深色主题。
- 首页真实读取 Codex++ 本地概览、设置、会话、扩展和远程项目数据。
- 按真实会话 `cwd` 聚合项目。
- 数据、网络环境和更新中心的阶段化页面。
- 中转站数据服务和工作站控制服务两个明确的 `NotConnected` Adapter。
- 浏览器预览、空状态、部分失败状态和重新检测交互。

当前客户端仍不是 U 盘便携运行时。便携根目录、受控停止、安全弹出和真实服务器连接分别属于后续阶段。

### 1.1 阶段 B.1 修正

根据 Windows 首轮体验反馈，阶段 B.1 完成以下修正：

- 不再重新估算工作站视觉样式，直接以
  `prototype/workstation-launcher-ui/styles.css` 为视觉数据源。
- 将原型样式作为最终 CSS 覆盖层加载，避免被 Codex++ 原有全局样式覆盖。
- 首页卡片尺寸、侧栏、顶栏、字体层级、深浅色主题、仪表盘和红色渐变停止按钮与确认过的网页原型保持一致。
- 原型中的演示数字没有进入正式客户端，页面继续显示 Codex++ 真实本地数据。
- 更新提醒每次完整启动最多弹出一次；进入更新中心不会重复弹窗。
- 新增“稍后提醒”“忽略此版本”和“恢复此版本提醒”。
- Windows 右上角关闭按钮改为退出管理器，不再隐藏到托盘。
- Windows Manifest 增加 `PerMonitorV2` DPI 声明，改善 WebView2 在 Windows 缩放环境下的文字清晰度。
- 中英文切换功能保持不变。

## 2. 主要实现

### 2.1 工作站外壳

Codex++ Manager 已改为工作站信息架构：

- 首页
- 供应商配置
- 项目管理
- 会话与绘画
- 工具与插件
- 页面增强
- Zed 远程项目
- 数据与备份
- 网络环境配置
- 更新中心
- 安装维护
- 设置

脚本市场、推荐内容、环境冲突检测、日志与诊断等原 Codex++ 页面仍可从模块入口打开，没有删除上游功能。

应用名称、窗口标题和默认窗口尺寸已经统一为工作站：

```text
Codex 便携工作站
1280 × 860
最小尺寸 960 × 720
```

### 2.2 本地工作站聚合接口

新增：

```text
apps/codex-plus-manager/src/lib/workstation-api.ts
```

该接口通过现有 Tauri Commands 并行读取：

- `load_overview`
- `load_settings`
- `list_local_sessions`
- `read_live_context_entries`
- `list_zed_remote_projects`
- `check_relay_environment`

聚合器支持：

- 全部成功。
- 部分命令失败但保留其余真实数据。
- Tauri 调用抛出异常。
- 命令返回结构化 `status: failed`。
- 浏览器预览时明确显示本地数据不可用，不填充演示数字。

### 2.3 服务器接口边界

阶段 B 只建立两个客户端接口占位：

```text
relay-service
workstation-control
```

两者固定返回：

```text
status: not_connected
endpointConfigured: false
```

没有写入服务器域名，没有连接 Sub2API，没有卡密激活，没有管理员接口，也没有模拟余额、延迟、套餐或更新成功。

### 2.4 安全边界

- “启动 Codex”暂时继续复用 Codex++ 原有启动命令。
- “停止”“停止服务”“安全弹出 U盘”保持禁用。
- 没有新增按进程名批量结束 Codex 的逻辑。
- 没有修改 Windows 全局代理或永久环境变量。
- 没有把 API Key、Token 或密码加入新代码。
- Windows Release EXE 没有远程启动，避免阶段 C 便携隔离完成前读取或写入用户真实 Codex 本机目录。

## 3. 新增和修改的主要文件

```text
apps/codex-plus-manager/src/features/home/HomeScreen.tsx
apps/codex-plus-manager/src/features/projects/ProjectsScreen.tsx
apps/codex-plus-manager/src/features/data/DataScreen.tsx
apps/codex-plus-manager/src/features/connection/ConnectionScreen.tsx
apps/codex-plus-manager/src/features/updates/UpdatesScreen.tsx
apps/codex-plus-manager/src/features/workstation/components.tsx
apps/codex-plus-manager/src/features/workstation/workstation.css
apps/codex-plus-manager/src/features/workstation/prototype-fidelity.css
apps/codex-plus-manager/src/lib/workstation-api.ts
apps/codex-plus-manager/src/workstation-api.test.ts
apps/codex-plus-manager/src/update-policy.ts
apps/codex-plus-manager/src/update-policy.test.ts
apps/codex-plus-manager/src/App.tsx
apps/codex-plus-manager/src/main.tsx
apps/codex-plus-manager/src/styles.css
apps/codex-plus-manager/index.html
apps/codex-plus-manager/src-tauri/src/commands.rs
apps/codex-plus-manager/src-tauri/src/lib.rs
apps/codex-plus-manager/src-tauri/windows-app-manifest.xml
apps/codex-plus-manager/src-tauri/tauri.conf.json
apps/codex-plus-manager/src-tauri/tests/windows_subsystem.rs
crates/codex-plus-core/src/paths.rs
```

## 4. 验证结果

### 4.1 macOS 开发端

```text
npm run check        通过
npm test             通过，22 项测试
npm run vite:build   通过
```

阶段 B.1 重新以 `1280 × 860` 截取已确认原型和 React 工作站首页进行逐项对照。
最终页面的侧栏宽度、顶栏高度、三张状态卡、仪表盘布局、环境校验列表和字体层级已对齐，
页面无控制台错误。

### 4.2 Windows 11 x64

在 `D:\Dev\codex-portable` 完成：

```text
npm run check                       通过
npm test                            通过，22 项测试
cargo fmt --all -- --check          通过
cargo test --workspace --offline    通过
npm run build                       通过
```

测试子进程继续使用：

```text
NO_PROXY=127.0.0.1,localhost
```

没有修改 Windows 系统代理。

### 4.3 Windows 构建产物

```text
路径：D:\Dev\codex-portable\target\release\codex-plus-plus-manager.exe
大小：26,584,064 bytes
SHA-256：B3CCC68D255E48C199215F01B093283169CAA01BF97A5EC8EF5307752B55B400
```

该哈希只对应本次本地阶段 B 构建，不是正式发布签名。

构建时原 `target\release\codex-plus-plus.exe` 正被现有 Codex 启动链使用，因此没有强制停止该进程。
新版管理器先在独立目标目录构建，再单独替换管理器 EXE。旧管理器备份为：

```text
D:\Dev\codex-portable\target\release\codex-plus-plus-manager.phase-b-ui.bak.exe
```

## 5. 已知事项

1. Vite 主 JavaScript chunk 约为 515 kB，构建成功但有大于 500 kB 的性能提示，后续可按页面拆包。
2. 上游 npm 依赖仍报告 1 个 low 和 1 个 high vulnerability，本阶段没有执行可能改变锁文件的 `npm audit fix`。
3. 阶段 B 没有实现 `CODEXPP_HOME`、移动盘根目录或数据迁移。
4. 阶段 B 没有实现受控停止、安全弹出、签名更新或服务器连接。
5. Windows EXE 已完成原生编译，但为保护用户现有 Codex 数据，没有在未隔离状态下远程启动。
   右上角关闭和 DPI 效果需要用户在交互式 Windows 桌面手动打开新版 EXE 做最终目视确认。

## 6. 下一阶段

阶段 C 实现 Codex++ 便携根目录：

- 从当前 EXE 所在目录推导移动盘根目录。
- 实现子进程级 `CODEXPP_HOME`。
- 将工作站设置、状态、日志和用户脚本归入移动盘 `Data/`。
- 保持安装模式兼容。
- 验证中文路径、空格路径、盘符变化、只读和空间不足。
