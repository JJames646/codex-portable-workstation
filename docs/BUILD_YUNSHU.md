# 云枢 v1.2.1 源码与构建

## 版本关系

- 安装包产品版本：`1.2.1`。
- 工作站 Build ID：`9303dc9771b80694`。
- `workstation/` 的 101 个源码及资源文件直接来自安装包内同名 Build ID 的源码目录，保持原字节；8 个 macOS `._` 元数据文件被排除。
- 历史工程包版本 `0.1.0` 与安装包产品版本不是同一字段，本次没有改写。
- 安装器模板 SHA-256：`89515adc1f53bd858e40958c8dd6eaa5a969316062ba791e7bf334a3d6dc100a`。与 2026-09-20 r4 构建记录一致。

## 环境

前端使用 Node.js 22 与 npm 锁文件。Windows 成品构建需要 Rust（支持 edition 2024）、MSVC C++ Build Tools、Windows SDK 和 Tauri 所需环境；运行工作站界面需要 WebView2。原生用户安装器本身不依赖 WebView2。

此源码不是完整便携母盘。Codex、Claude Code、OpenClaw 等第三方运行时、目录资源和模板须根据各自来源及条款准备，不能只编译启动器就视为完整安装包。

## 前端

在仓库根目录执行：

```bash
cd workstation/launcher
npm ci
npm run check
npm test
npm run build
```

## Rust 后端

在仓库根目录执行：

```bash
cargo test --manifest-path workstation/backend-src/Cargo.toml --locked
cargo build --manifest-path workstation/backend-src/Cargo.toml --release --locked
```

部分外部夹具测试和 Windows 集成测试依赖对应环境，不能用其他系统的检查代替。

## Windows 启动器

在 `workstation/launcher` 执行：

```bash
npm run tauri:build:portable
```

后端与启动器必须来自同一份源码。后端的 `build.rs` 根据源码内容和 `Cargo.toml` 生成 Build ID；改动代码或该文件后，Build ID 会变化。不要将新构建的单个文件混入已有安装包。

## 安装工具

构建命令及私钥边界见 [`installer/README.md`](../installer/README.md)。

原生模板需要在构建时设置 `YUNSHU_INSTALLER_PUBLIC_KEY_HEX` 为自有 Ed25519 公钥。公开的公钥用于验签，不能替代管理员私钥。仓库不提供云枢生产签名私钥；自行构建须使用自己的签名材料，不得把生成的私钥提交到仓库。

## 本次发布核验

本次对现有安装包执行了 Ed25519 验签、清单和载荷 SHA-256 校验、102,890 个文件的解压哈希比对与源码提取核对。源码归档和安装包另有 SHA-256 文件供下载核验。

本次发布没有重新编译 Windows EXE，也没有在 Windows 上重新安装运行，因此不把归档校验表述为新的 Windows 运行验收。
