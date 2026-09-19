# OGG Switch

English | 简体中文（当前）

**Grok Build 与 Oh My Pi 的统一桌面配置工具。**

[![Release](https://img.shields.io/github/v/release/akiteet/ogg-switch?style=flat-square)](https://github.com/akiteet/ogg-switch/releases)
[![License](https://img.shields.io/github/license/akiteet/ogg-switch?style=flat-square)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue?style=flat-square)](#下载)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB?style=flat-square)](https://tauri.app)

OGG Switch 将两个最强大的 AI 编程运行时——xAI 的 **Grok Build** 与
**Oh My Pi (OMP)**——整合到同一个桌面界面中。它会读取你现有的本地配置，以清晰、
可搜索的目录形式呈现，并让两个运行时始终保持同步，无需你再手动编辑 YAML 文件。

OGG Switch 基于 **Tauri 2** 与 **Rust** 构建，是一个约 25 MB 的原生二进制程序，
零遥测、不依赖云端。你的密钥与配置永远不会离开本机。

---

## 功能亮点

**精选 100+ 供应商的目录**
- 20 个 OAuth 登录供应商——Anthropic、OpenAI Codex、GitHub Copilot、Cursor、
  Gemini CLI、Antigravity、Devin、GitLab Duo、Kimi Code、Z.AI Coding Plan、
  Muse Code 等
- 80+ 个 API Key 供应商，覆盖官方 API、网关、聚合器与本地引擎
  （Ollama、LM Studio、llama.cpp、vLLM）
- 每个预设的认证方式都与上游运行时自身的鉴权策略对齐，OAuth / API Key
  的划分始终准确

**Oh My Pi 的语义角色编排**
- 十个专用角色——`default`、`smol`、`slow`、`plan`、`commit`、`vision`、
  `designer`、`task`、`advisor`、`tiny`——每个角色映射到一组供应商/模型，
  并可选配推理深度
- 角色修改会对照实时目录进行校验，并以原子方式写回

**双引擎，单界面**
- Grok Build 模式管理 xAI 运行时（TOML 配置、模型回退链、路由）
- Oh My Pi 模式直接操作 `models.yml` / `config.yml`，供应商库会保留已移除的
  条目，随时可恢复
- 一键将现有生效配置导入为受管配置档

**生产级基础设施**
- 本地代理，支持自动故障转移与按供应商的健康检查
- 按供应商与模型细分的用量统计，附带成本追踪
- 会话浏览器、技能管理器、MCP 控制台、提示词库
- 拖拽排序、批量导入导出、备份与恢复
- WebDAV 与 S3 配置同步
- 应用内自动更新，逐个校验签名，更新直接来自 GitHub Releases

**本地优先的设计**
- 无遥测、无账号、无后台网络请求
- 配置以纯文本文件保存在 `~/.ogg-switch/` 下
- 原子写入并自动备份

**四种界面语言**——英语、简体中文、繁体中文、日语

---

## 下载

预构建安装包已发布在 [Releases](https://github.com/akiteet/ogg-switch/releases) 页面：

| 平台 | 安装包 |
|------|--------|
| Windows | `.msi` 安装器 / `.exe`（NSIS） |
| macOS | `.dmg`（Apple Silicon 与 Intel） |
| Linux | `.AppImage` / `.deb` |

> **macOS 说明** —— 正式版构建尚未签名，Gatekeeper 可能会在首次启动时要求确认
> （`系统设置 → 隐私与安全性 → 仍要打开`）。公证已列入路线图。

### 自动更新

OGG Switch 会在启动时检查 GitHub Releases，并可直接就地安装更新。每个更新包都会
用编译进应用内的公钥校验，被篡改的下载文件会在应用之前被拒绝。

- **Windows** —— 发布了两种安装包。`.msi` 按机器安装（一次 UAC 提示）并注册一个
  特权更新任务，因此后续更新可静默应用。NSIS `.exe` 按用户安装，始终无需
  管理员权限。
- **Linux** —— 应用内更新适用于 AppImage 构建；`.deb` 用户应通过包管理器升级。
- **macOS** —— 更新器会就地替换 `.app` 包。由于构建尚未公证，更新后 macOS
  仍可能再次要求确认。

便携版没有更新通道；「检查更新」会引导你前往
[Releases](https://github.com/akiteet/ogg-switch/releases) 页面。

---

## 快速开始

环境要求：[Node.js](https://nodejs.org) 20+、[pnpm](https://pnpm.io) 10+，以及
Rust 工具链（[rustup](https://rustup.rs)）。

```bash
git clone https://github.com/akiteet/ogg-switch.git
cd ogg-switch
pnpm install
pnpm tauri dev        # development build
pnpm tauri build      # production installers
```

打包正式发布安装包需要更新器签名密钥，因为更新产物在打包过程中会被签名：

```powershell
pnpm tauri signer generate -w $env:USERPROFILE\.tauri\ogg-switch.key   # once
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$env:USERPROFILE\.tauri\ogg-switch.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "<your password>"
pnpm tauri build
```

对应的公钥保存在 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey` 字段。
日常开发可以用 `pnpm tauri build --no-bundle` 完全跳过打包。

运行测试：

```bash
pnpm test:unit        # frontend (Vitest)
cargo test            # Rust core
```

---

## 配置布局

OGG Switch 直接操作运行时本身已经在用的配置文件——不会把任何内容存进私有格式。

| 运行时 | 文件 |
|--------|------|
| OGG Switch | `~/.ogg-switch/`（配置档、代理状态、用量数据） |
| Oh My Pi | `~/.omp/agent/models.yml`（供应商）与 `~/.omp/agent/config.yml`（角色、重试链） |

---

## 架构

```
┌────────────────────────────────────────────────
│  React 19 + TypeScript (Vite)                  │
│  shadcn/ui · TanStack Query · framer-motion    │
├────────────────────────────────────────────────┤
│  Tauri 2 命令桥接 (IPC)                        │
├────────────────────────────────────────────────┤
│  Rust 核心                                     │
│  · 运行时适配器 (Grok Build / Oh My Pi)        │
│  · 本地代理 + 故障转移数据平面                 │
│  · SQLite 状态存储 · 原子文件写入器            │
│  · 用量聚合 · 会话与技能索引                   │
└────────────────────────────────────────────────┘
```

---

## 路线图

- 全平台代码签名与公证
- 供应商插件与外部预设源
- 跨设备配置档同步改进

---

## 致谢

OGG Switch 是 [cc-switch](https://github.com/farion1231/cc-switch)（MIT）的衍生作品，
本项目在其应用外壳与供应商管理的基础上构建而成。上游版权声明保留在
[LICENSE](LICENSE) 中，并在 [NOTICE.md](NOTICE.md) 中进一步说明。

同时感谢 [Oh My Pi](https://github.com/can1357/oh-my-pi) 项目提供了本工具所集成的
运行时，感谢 [Tauri](https://tauri.app) 团队提供桌面框架，感谢
[shadcn/ui](https://ui.shadcn.com) 与 [Lobe Icons](https://github.com/lobehub/lobe-icons)
的维护者提供 UI 与品牌素材。

---

## 许可证

[MIT](LICENSE) © 2026 akiteet
