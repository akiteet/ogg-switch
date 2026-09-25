# Changelog

All notable changes to OGG Switch will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Grok Build: official usage no longer shows a fake "0%". The billing query moved
  from the gRPC endpoint (whose response carries no usage percent for free plans,
  which the old heuristic parser silently papered over with a zero) to the supported
  JSON billing API; when a plan reports no percentage the card shows "usage
  unknown" with the reset countdown instead of a fabricated number
- Oh My Pi: the quota windows on OAuth provider cards get a manual refresh button —
  it runs omp's own usage command so numbers reflect what the provider reports right
  now (omp respects its own rate-limit-safe caching), instead of waiting for OMP to
  happen to record new snapshots



### Fixed

- Antigravity sessions stay alive: when the access token nears expiry it is
  refreshed automatically from the refresh token agy already stores, and the
  refreshed credentials are written back to where agy reads them. The OAuth client
  needed for the refresh is extracted from the locally installed agy at runtime —
  nothing is embedded in the repository, and a missing or outdated agy falls back to
  the previous "run agy to sign in again" hint
- Oh My Pi: OAuth provider cards' quota windows refresh on their own now — 60-second
  polling, and the refresh events from background session sync actually reach them
  (the query key sat outside the namespace those events invalidate, so the numbers
  used to freeze at whatever was read first)

## [1.1.3] - 2026-09-25

### Added

- Antigravity official subscription quota: the card shows real usage for the two
  model families Antigravity itself uses — Gemini models and Claude / GPT models —
  with reset countdowns, sourced from the same Cloud Code endpoints agy calls
- Oh My Pi: OAuth provider cards show the quota windows Oh My Pi records (the same
  data the usage dashboard uses). Google accounts render as the same two families as
  the Antigravity card, with duplicated shared windows deduplicated
- Oh My Pi: the provider catalog is aligned with omp's own documentation — presets
  are grouped into OAuth login / API Key (built-in) / Common providers, and two chat
  providers omp supports were picked up from omp's auth rules (Command Code,
  Abliteration)
- Balance queries for providers without a built-in balance API explain what is
  supported and how to switch to a custom usage script, instead of failing with a
  bare error
- Full multi-language coverage: every string added since 1.1.2 is available in all
  four languages (English, Simplified Chinese, Traditional Chinese, Japanese), and
  backend errors render in the configured language as well

### Fixed

- Oh My Pi: usage on provider cards works — cards read the usage script field the
  backend actually sends, native templates (balance / token plan / Copilot / official
  subscription) resolve on cards, and API keys in `$ENV` / secret-bridge form are
  resolved instead of leaking into request headers
- Official subscription quotas display without a manually configured usage script;
  turning the usage toggle off still opts a provider out
- Missing or unreadable CLI credentials show a hint naming the CLI to log in with,
  instead of rendering nothing
- Antigravity: the card no longer claims "session expired" while agy is signed in
  (the nested token shape agy stores is now read), and quota queries succeed with
  real numbers where they previously returned empty
- Switching back to an official provider restores the CLI login that switching away
  deleted (Grok Build, Codex) — no re-login needed
- Oh My Pi "Upgrade" works when the app process does not inherit the user's PATH
  (typically right after an MSI self-update), and PowerShell upgrade failures show
  readable text instead of CLIXML dumps
- Quota percentages are labeled: cards show "used 42%", the expanded view
  "used 42% · remaining 58%"
- The "Environment variable backups" section in Settings → Advanced is translated
  into all four languages

## [1.1.2] - 2026-09-23

### Added

- Settings → Advanced → "Environment variable backups": list the backups that are
  created when environment variables are deleted, compare each variable's backed-up
  value against the value currently on the machine, and restore them. This used to be
  a backend call with no UI at all, so a backup could be written but never used

### Fixed

- Environment variable conflict check no longer flags the variables OGG Switch itself
  manages. `GEMINI_API_KEY` / `GOOGLE_GEMINI_BASE_URL` are written by OGG when
  switching Antigravity providers — they are how agy authenticates — so listing them
  as "conflicts" with a delete button was inviting users to delete their own provider
  credentials. They are now excluded from the scan (including OGG's managed block in
  shell rc files) and rejected by the delete path itself; the banner explains what is
  managed and what is genuinely leftover
- Environment variable deletion now warns before it happens when a selected variable
  is the credential of a provider configured in OGG (it would silently stop working),
  and a delete that includes a managed variable is refused outright instead of
  partially succeeding. The backup file is only written after validation passes
- Antigravity CLI: "Upgrade" actually updates the CLI now. The official install script
  exits successfully without touching an existing binary, and agy's own `update`
  subcommand was not being used; the update flow now runs `agy update` first and, if
  the version still trails the official release manifest, downloads and verifies the
  latest binary (sha512) and replaces it directly
- Antigravity CLI: the provider form's default model is now honored. It used to be
  written to a `GEMINI_MODEL` environment variable that agy never reads; it is stored
  in agy's own config (`settings.json` → `model`) and applied when switching to the
  provider. Leaving it empty keeps whatever model is currently selected in agy. The
  candidate list comes from `agy models` (display names, as agy stores them), and the
  obsolete environment variable is cleaned up on the next provider switch

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