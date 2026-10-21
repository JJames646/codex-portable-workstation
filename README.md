<div align="center">

**English** | [简体中文](README.zh-CN.md)

# Yunshu AI Workstation · 云枢 AI 便携工作站

**Source code for the launcher, runtime backend and installer of a portable AI workspace for Windows.**

Rust · Tauri 2 · React · TypeScript · AGPL-3.0-only

[**Download / 下载**](https://github.com/JJames646/codex-portable/releases/tag/v1.2.1) · [**Source / 源码**](workstation) · [**Installer / 安装器**](installer) · [**Build / 构建**](docs/BUILD_YUNSHU.md)

![Yunshu AI Workstation: the existing application interface in Simplified Chinese](docs/images/overview.jpg)

</div>

> **Looking for the ready-to-use application?** Visit the [download repository](https://github.com/JJames646/codex-portable). The packaged software requires an activation code. Join **QQ group `713993476`** to request one for free. Workstation activation does not include model API credits; configure your own model service.

**Interface language:** The current application, screenshots and promotional video are in Simplified Chinese. The project README is available in English and Chinese.

## What This Source Matches

This repository publishes the workstation source corresponding to the existing **v1.2.1 Windows installer**, together with native installer source checked against historical build records.

| Component | Location and relationship |
| --- | --- |
| Current workstation frontend and Tauri launcher | [`workstation/launcher`](workstation/launcher) |
| Current Rust runtime backend | [`workstation/backend-src`](workstation/backend-src) |
| Source provenance from the installer | [`workstation/SOURCE_PROVENANCE.json`](workstation/SOURCE_PROVENANCE.json) |
| Native installer, administrator packaging tool and core library | [`installer`](installer) |
| Version, file hashes and release verification | [`docs/releases/v1.2.1.md`](docs/releases/v1.2.1.md) (Chinese) |
| Upstream Codex++ and early workstation code | Root-level `apps/`, `crates/` and `assets/`, with the original history preserved |

The workstation source was extracted from `source/release-9303dc9771b80694/` inside the installer. **All 101 source and resource files match their original hashes.** Eight macOS `._` metadata files are excluded from the published source. The current product lives in `workstation/`; the root-level Codex++ build entry belongs to the earlier codebase. Installer code comes from the September 20, 2026 r4 source snapshot, whose template hash matches the template embedded in the v1.2.1 package.

The source ZIP contains the current workstation and installer. The earlier Codex++ project and commit history are available in this GitHub repository.

## Capabilities

- Configure and launch Codex, Claude Code and OpenClaw from one workspace.
- Use AI Chat, a separate image generation workspace and local result management.
- Browse and install Skills and plugins, with target engine selection.
- Derive the working directory from the application location and manage transferable data and child-process environments.
- Access Chinese tutorials, workstation settings and activation.
- Install through a native Windows wizard with signed manifests and file integrity checks.

| AI Chat | Skills management |
| --- | --- |
| ![AI Chat workspace](docs/images/chat.jpg) | ![Skills management](docs/images/skills.jpg) |

## Build and Verify

Start with the [full build guide (Chinese)](docs/BUILD_YUNSHU.md). To check the frontend:

```bash
cd workstation/launcher
npm ci
npm run check
npm test
npm run build
```

To test the backend:

```bash
cargo test --manifest-path workstation/backend-src/Cargo.toml --locked
```

The Windows application requires the appropriate Rust / MSVC toolchain and WebView2. This source repository does not include complete third-party runtimes, developer credentials, administrator signing private keys or production server configuration.

## Software, Source and Activation

**Packaged software:** Download and install the application, then join QQ group **713993476** to request a free workstation activation code. You can also search for **"云枢 AI"** on Xianyu (闲鱼) for inquiries.

**Source code:** Provided under [AGPL-3.0-only](LICENSE), subject to the [open-source scope statement (Chinese)](docs/OPEN_SOURCE.md). The original source snapshot retains historical package metadata; the repository license and scope statement define the current public grant. Model service accounts, API credits, third-party applications and trademarks are outside that source-code grant.

## Video and Feedback

[![Watch / download the product video](docs/images/demo-poster.jpg)](https://github.com/JJames646/codex-portable/releases/download/v1.2.1/Yunshu-AI-Workstation-Demo.mp4)

Report reproducible problems in [Issues](https://github.com/JJames646/codex-portable-workstation/issues). Include the source version, operating system and steps to reproduce. Do not attach API keys, activation codes or private conversations.

## Upstream and Acknowledgments

This repository retains its fork relationship with [BigPizzaV3/CodexPlusPlus](https://github.com/BigPizzaV3/CodexPlusPlus), the commit history and the AGPL license. The early workstation evolved from that project; the current additions are presented separately in `workstation/` and `installer/`.

[Archived upstream Chinese README](docs/upstream/CodexPlusPlus-README.zh.md) · [Archived upstream English README](docs/upstream/CodexPlusPlus-README.en.md) · [Contributing](CONTRIBUTING.md)

Third-party software and brand names belong to their respective owners. This is an independently maintained project and is not officially endorsed by OpenAI, Anthropic or other third parties.
