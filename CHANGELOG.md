# Changelog

All notable changes to OGG Switch will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.1.1] - 2026-09-22

### Added

- Oh My Pi: the role manager covers OMP's full role set — the ten chat roles plus
  `image`, `web`, `speech`, `dictation` and `judge` — and custom roles can be created
  with any name; they are written to `config.yml:modelRoles` like built-in roles and
  can be edited or deleted again
- Oh My Pi: role candidates come from OMP's own catalog, including its synthetic
  `web` and `local` providers
- Provider forms: drag-and-drop ordering for the model list rows in Oh My Pi and
  Grok Build

### Fixed

- Oh My Pi: role candidates are limited to the model kinds each role accepts, so an
  assignment always resolves inside OMP; the dialog explains when a provider's own
  models are out of scope for the selected role
- Oh My Pi: duplicating a provider creates a new library entry with its own id
  (`<id>-copy`, `-copy-2`, …) and leaves `models.yml` untouched — confirm it with
  "Add" when you want it active
- Oh My Pi: editing a provider keeps its API key, custom headers and
  "send Authorization header" setting
- Oh My Pi: "fetch models" prefers the provider's upstream `/models` listing, with
  OMP's catalog as the fallback and as the source of context window / reasoning
  metadata
- Oh My Pi: local providers without an API key (Ollama, LM Studio, …) can be saved
- Oh My Pi: role entries this app cannot represent survive writes to `config.yml`

## [1.1.0] - 2026-09-21

### Added

- Antigravity CLI (agy) as a third managed runtime: provider switching through
  `~/.gemini/antigravity-cli/settings.json` and persistent environment variables,
  a Google account pool that snapshots and restores the current agy sign-in,
  session browsing with resume via `agy --conversation <id>`, usage analytics,
  and the official product icon alongside two ready-made presets
- Provider switch timeline: historical usage is attributed to the provider that
  was active when the session ran
- Icon metadata for 53 providers, enabling search and theme coloring in the
  icon picker

### Changed

- OGG Switch manages three runtimes: Grok Build, Antigravity and Oh My Pi. Interfaces
  and commands inherited from upstream for other agents have been removed; existing
  provider records stay in the database but are no longer surfaced.
- The local proxy targets Grok Build only.
- Database schema upgraded to v20. Migration runs automatically and takes a backup
  first; databases written by v1.1.0 cannot be opened by older versions.

### Fixed

- Usage dashboard: Antigravity experiment flags no longer appear as model names
- Usage dashboard: Antigravity entries carry the session's real event time instead
  of the import time
- Usage dashboard: OMP reports per-request usage again
- OMP providers show their configured display name in model and role selectors

## [1.0.0] - 2026-09-17

First public release.

### Added

- Dual-engine architecture: Grok Build and Oh My Pi managed from one interface
- Provider catalog with 100+ presets — 20 OAuth sign-in providers plus API-key,
  gateway, aggregator and local-engine providers, with authentication models aligned
  to each runtime's own auth policies
- Oh My Pi semantic role orchestration (10 roles) written natively to
  `~/.omp/agent/config.yml`
- Oh My Pi provider library: entries removed from the live configuration remain
  recoverable and can be re-added with one click
- Local proxy with automatic failover, per-provider health checks and stream verification
- Usage analytics segmented by provider and model, with cost tracking
- Session browser, skills manager, MCP console and prompt library
- WebDAV and S3 configuration sync, plus backup and restore
- Four UI languages: English, Simplified Chinese, Traditional Chinese, Japanese
- Windows installers (`.msi` and NSIS), macOS `.dmg` and Linux `.AppImage` / `.deb`
- Signature-verified in-app auto-update served from GitHub Releases
- Multi-platform release pipeline (Windows, macOS, Linux)

### Changed

- Rebuilt the provider preset catalog against the upstream runtime's auth policy
  definitions to guarantee correct OAuth / API-key classification
- Unified provider branding assets under a single icon registry

### Fixed

- Configuration writes are atomic and self-healing — no partially written YAML
- Non-empty model name fallbacks prevent upstream schema validation errors
- Provider removal cleans dangling references from retry chains and provider order
- Role editor dialog is no longer clipped by the window header
- View transitions no longer flash during navigation

---

## Heritage

OGG Switch builds on [cc-switch](https://github.com/farion1231/cc-switch) v3.20.3 (MIT),
whose Grok Build configuration and proxy foundations this project extends. See
[NOTICE.md](NOTICE.md) for details.