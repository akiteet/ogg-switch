# Changelog

All notable changes to OGG Switch will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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