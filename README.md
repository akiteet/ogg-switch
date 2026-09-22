# OGG Switch

English | [简体中文](README.zh-CN.md)

**One control plane for Grok Build, Oh My Pi and Antigravity.**

[![Release](https://img.shields.io/github/v/release/akiteet/ogg-switch?style=flat-square)](https://github.com/akiteet/ogg-switch/releases)
[![License](https://img.shields.io/github/license/akiteet/ogg-switch?style=flat-square)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue?style=flat-square)](#download)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB?style=flat-square)](https://tauri.app)

OGG Switch unifies three of the most capable AI coding runtimes — xAI's **Grok Build**,
**Oh My Pi (OMP)** and Google's **Antigravity CLI (agy)** — behind a single desktop
interface. It reads your existing local configuration, presents it as a clean, searchable
catalog, and keeps the runtimes in sync without ever asking you to hand-edit a YAML file.

Built with **Tauri 2** and **Rust**, OGG Switch is a ~25 MB native binary with zero telemetry
and no cloud dependency. Your keys and configuration never leave your machine.

---

## Highlights

**A catalog of 100+ curated providers**
- 20 OAuth sign-in providers — Anthropic, OpenAI Codex, GitHub Copilot, Cursor, Gemini CLI,
  Antigravity, Devin, GitLab Duo, Kimi Code, Z.AI Coding Plan, Muse Code and more
- 80+ API-key providers across official APIs, gateways, aggregators and local engines
  (Ollama, LM Studio, llama.cpp, vLLM)
- Every preset's authentication model is aligned with the upstream runtime's own auth
  policies, so the OAuth/API-key split is always accurate

**Semantic role orchestration for Oh My Pi**
- All fifteen built-in roles — the ten chat roles (`default`, `smol`, `slow`, `vision`,
  `plan`, `commit`, `tiny`, `memory`, `task`, `advisor`) plus the five kind roles
  (`image`, `web`, `speech`, `dictation`, `judge`) — each mapped to a provider/model
  pair with optional reasoning depth. Custom `modelRoles` keys in `config.yml` are
  listed too, so nothing configured in OMP stays hidden from the UI
- Role candidates come from OMP's own catalog (including its synthetic `web` and
  `local` providers), and role edits are written back atomically

**Dual-engine, one interface**
- Grok Build mode manages the xAI runtime (TOML configuration, model fallback chains,
  routing)
- Oh My Pi mode works directly on `models.yml` / `config.yml`, with a provider library
  that keeps removed entries recoverable
- Antigravity mode manages Google's `agy`: API-key/relay providers via
  `~/.gemini/antigravity-cli/settings.json` plus persistent `GEMINI_API_KEY` /
  `GOOGLE_GEMINI_BASE_URL` environment variables, and multi-account Google login by
  snapshotting/restoring `antigravity-oauth-token`
- One click to import an existing live configuration as a managed profile

**Production-grade plumbing**
- Local proxy with automatic failover and per-provider health checks
- Usage analytics split by provider and model, with cost tracking
- Session browser, skills manager, MCP console, prompt library
- Drag-and-drop ordering, bulk import/export, backup and restore
- WebDAV and S3 configuration sync
- Signature-verified in-app auto-update delivered straight from GitHub Releases

**Local-first by design**
- No telemetry, no accounts, no background network calls
- Configuration is stored in plain files under `~/.ogg-switch/`
- Atomic writes with automatic backups

**Four languages** — English, Simplified Chinese, Traditional Chinese, Japanese

---

## Download

Prebuilt installers are published on the [Releases](https://github.com/akiteet/ogg-switch/releases) page:

| Platform | Package |
|----------|---------|
| Windows | `.msi` installer / `.exe` (NSIS) |
| macOS | `.dmg` (Apple Silicon and Intel) |
| Linux | `.AppImage` / `.deb` |

> **macOS note** — release builds are not code-signed yet, so Gatekeeper may ask for
> confirmation on first launch (`System Settings → Privacy & Security → Open Anyway`).
> Notarization is on the roadmap.

### Auto-update

OGG Switch checks GitHub Releases on launch and can install updates in place. Every
artifact is verified against the public key compiled into the app, so a tampered
download is rejected before it is applied.

- **Windows** — two installers are published. The `.msi` installs per-machine (one UAC
  prompt) and registers a privileged update task, so later updates apply silently. The
  NSIS `.exe` installs per-user and never needs administrator rights.
- **Linux** — in-app updates work for the AppImage build; `.deb` users should upgrade
  through their package manager.
- **macOS** — the updater replaces the `.app` bundle in place. Because builds are not
  notarized yet, macOS may still ask for confirmation once after an update.

Portable builds have no update channel; **Check for updates** will point you at the
[Releases](https://github.com/akiteet/ogg-switch/releases) page instead.

---

## Quick Start

Requirements: [Node.js](https://nodejs.org) 20+, [pnpm](https://pnpm.io) 10+, and a Rust
toolchain ([rustup](https://rustup.rs)).

```bash
git clone https://github.com/akiteet/ogg-switch.git
cd ogg-switch
pnpm install
pnpm tauri dev        # development build
pnpm tauri build      # production installers
```

Packaging release installers requires the updater signing key, because update artifacts
are signed during bundling:

```powershell
pnpm tauri signer generate -w $env:USERPROFILE\.tauri\ogg-switch.key   # once
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$env:USERPROFILE\.tauri\ogg-switch.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "<your password>"
pnpm tauri build
```

The matching public key lives in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`.
For day-to-day work you can skip bundling entirely with `pnpm tauri build --no-bundle`.

Run the test suites:

```bash
pnpm test:unit        # frontend (Vitest)
cargo test            # Rust core
```

---

## Configuration Layout

OGG Switch operates on the configuration files that the runtimes already use — nothing is
stored in a proprietary format.

| Runtime | Files |
|---------|-------|
| OGG Switch | `~/.ogg-switch/` (profiles, proxy state, usage data) |
| Oh My Pi | `~/.omp/agent/models.yml` (providers) and `~/.omp/agent/config.yml` (roles, retry chains) |
| Antigravity (agy) | `~/.gemini/antigravity-cli/settings.json` (`modelProvider`), persistent env vars (`GEMINI_API_KEY`, `GOOGLE_GEMINI_BASE_URL`) and `~/.gemini/antigravity-cli/antigravity-oauth-token` (Google account snapshots) |

---

## Architecture

```
┌────────────────────────────────────────────────
│  React 19 + TypeScript (Vite)                  │
│  shadcn/ui · TanStack Query · framer-motion    │
├────────────────────────────────────────────────┤
│  Tauri 2 command bridge (IPC)                  │
├────────────────────────────────────────────────┤
│  Rust core                                     │
│  · runtime adapters (Grok Build / Oh My Pi)    │
│  · local proxy + failover data plane           │
│  · SQLite state · atomic file writers          │
│  · usage aggregation · session & skill indexing│
└────────────────────────────────────────────────┘
```

---

## Roadmap

- Code signing and notarization for all platforms
- Provider plugins and external preset sources
- Cross-device profile sync improvements

---

## Acknowledgments

OGG Switch is a derivative work of [cc-switch](https://github.com/farion1231/cc-switch)
(MIT), whose application shell and provider-management foundation this project builds upon.
The upstream copyright notice is preserved in [LICENSE](LICENSE) and further documented in
[NOTICE.md](NOTICE.md).

Thanks also to the [Oh My Pi](https://github.com/can1357/oh-my-pi) project for the runtime
this tool integrates with, the [Tauri](https://tauri.app) team for the desktop framework,
and the maintainers of [shadcn/ui](https://ui.shadcn.com) and
[Lobe Icons](https://github.com/lobehub/lobe-icons) for the UI and brand assets.

---

## License

[MIT](LICENSE) © 2026 akiteet