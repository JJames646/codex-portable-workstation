# 云枢 AI 阶段十一 U 盘生产工具

本目录是与工作站主体隔离的阶段十一工程，包含：

- `yunshu-usb-writer-core`：母盘验证、发布清单、USB 写入事务、ZIP64 单 EXE 和 Ed25519 校验。
- `yunshu-usb-admin`：管理员批量写盘和用户安装包生成界面。
- `yunshu-usb-installer-native`：正式的 Rust + Win32 原生用户安装器模板，按普通 Windows 安装向导选择任意可写目录。
- `yunshu-usb-installer`：旧 Tauri 用户模板，仅保留供审计，不再用于正式生成。

## 安全边界

- 管理员直接/批量写盘仍不格式化、不分区、不清空目标盘。
- 用户原生安装器不再枚举或格式化物理磁盘；用户选择本机或移动盘上的安装目录，按文件大小和目录权限校验目标。
- 现有产品目录或根目录文件冲突时拒绝覆盖。
- 所有文件先写入目标盘暂存目录，逐文件 SHA-256 校验后再提交。
- 母盘只读作为输入；兼容 `交付资料/写入U盘根目录` 和现有阶段十的 `交付资料` 两种来源，内容映射到 U 盘根目录，包装目录不交付。
- 管理员私钥只保存在管理员工具旁的 `admin-data/production-signing.key`，不得提交、打印或交付。
- 用户安装器模板必须在构建时通过 `YUNSHU_INSTALLER_PUBLIC_KEY_HEX` 固定可信公钥；运行时不会信任安装包尾部自行声明的其他公钥。

## 构建

```text
cd apps/yunshu-usb-admin
npm ci
npm run build

cd ../..
cargo test --workspace
cargo build -p yunshu-usb-admin --release --features custom-protocol
set YUNSHU_INSTALLER_PUBLIC_KEY_HEX=<production-signing.pub中的64位十六进制公钥>
set CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS=-C target-feature=+crt-static
cargo build -p yunshu-usb-installer-native --release
```

`yunshu-usb-installer-native-template.exe` 是原生模板；只有管理员工具追加经过签名的完整母盘载荷后，才是可交付给用户的单 EXE 安装程序。新模板不使用 WebView2、PowerShell、Node 或 Python，并以静态 CRT 构建。单 EXE 仍包含完整离线母盘，因此文件体积不会因为向导变成小体积；变化是安装体验改为标准的“选择目录 → 安装 → 完成”。
