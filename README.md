<p align="center">
  <img src="assets/app-icon.png" alt="AgentHarbor" width="128" height="128" />
</p>

<h1 align="center">AgentHarbor</h1>

<p align="center">
  <strong>One desktop home for your coding agents: accounts, relays, a local model router, sessions and local configuration.</strong>
</p>

<p align="center">
  English · <a href="./README-cn.md">简体中文</a>
</p>

---

## What it is

Codex, Claude Code, Claude Desktop, OpenCode and Pi each keep accounts, model settings, sessions, MCP servers and instructions in their own files. Switching accounts when a quota runs out, wiring in third-party relays, or cleaning up sessions usually means hand-editing TOML, JSON and SQLite.

AgentHarbor is a Tauri 2 + React + Rust desktop app that does this for you. It runs a local model router (`127.0.0.1:43127`), writes each client's configuration as a reversible, field-level change with a backup, and shows what is actually happening on one dashboard. Codex is the primary integration; the other clients are supported through the same relays and router.

It is an independent rebuild with its own app identity, data directory, single-instance lock and router port, so it can run next to an installed AiMaMi.

## Features

| Area | What you can do |
| --- | --- |
| **Dashboard** | Routing topology of every client, today's token pulse, recent sessions, and a foldable map of each client's data directory and key files |
| **Relays** | Manage many providers; per-model protocol (Responses, Chat Completions, Anthropic Messages), connectivity tests, New API balance, import/export, and a capability catalog (context, output limit, prices) from a bundled [models.dev](https://github.com/anomalyco/models.dev) snapshot |
| **Client access** | Codex (local Router or direct Native, accountless mode, subagent defaults), Claude Code, Claude Desktop, OpenCode and Pi — preview, apply and restore, each with backups |
| **Local router** | Protocol translation between the three wire formats, model aggregation with priority tiers, failover, first-token timeout and live member state, plus request-level route diagnostics |
| **Accounts & usage** | ChatGPT OAuth login, quota windows, manual or threshold-based automatic switching, and a token monitor across Codex, Claude Code and relay traffic with recorded prices |
| **Sessions** | Browse, search, export and delete Codex and Claude Code conversations; recover Codex threads its index lost |
| **MCP / Skills / Instructions** | Manage MCP servers, Skills (with backups) and the AgentHarbor-managed block of `AGENTS.md` / `CLAUDE.md` for Codex and Claude Code |
| **Image workbench** | Generate or edit images through a configured provider and keep them in a local gallery |
| **Settings & maintenance** | Theme, language, custom wallpaper with automatic dark-mode contrast, app avatar, API proxy, Windows terminal proxy, diagnostics, backup cleanup and Codex restart |

Not available yet: account import/export, in-app plugin management, quota heatmaps and the macOS notch display. Update checks stay off until a signed release endpoint is configured.

## How Codex connects

The access mode is independent of the upstream protocol and the network route.

| Mode | How requests flow | Keep AgentHarbor running? |
| --- | --- | --- |
| **Router** | Codex talks to the local Responses gateway, which forwards each model with its own protocol and translates Chat Completions / Anthropic Messages. Official and relay models can share one menu, and the official account stays signed in unless accountless mode is on. | Yes, while using routed models |
| **Native** | Codex connects straight to the selected provider. Only that provider's Responses models are listed, under their real IDs; an incompatible default is rejected rather than swapped. | No |

- Enabling access edits only the fields AgentHarbor manages in `config.toml`; disabling it restores what was there before. Restart Codex after changing the mode, models or proxy (the app has a restart button).
- Relay models inherit Codex's client behaviour from the official model of the same name, so multi-agent V2 and the **Ultra** effort work through a relay too. Ultra is a Codex-side setting: upstream still receives a concrete effort such as `xhigh` or `max`.
- Native authenticates with the provider key: it is stored in the Codex `.env` under a random AgentHarbor variable and referenced via `env_key`, never written to `auth.json` or shown in previews. Treat `.env` and its backups as secrets — they are not encrypted.

## Data and safety

- **Codex home**: `AGENT_HARBOR_CODEX_HOME`, else `CODEX_HOME`, else `~/.codex`.
- **AgentHarbor data** (relays, routing state, usage ledger, caches): the platform's local data directory under `AgentHarbor` (for example `%LOCALAPPDATA%\AgentHarbor`), or `AGENT_HARBOR_DATA_DIR`. The dashboard's data map shows the exact paths.
- **Provider credentials** live in `newapi-providers.dat`, encrypted with DPAPI for the current user on Windows and stored owner-only (`0600`, not encrypted) elsewhere. The UI never reads saved secrets back; leaving a field empty keeps the stored value.
- **The local router** requires a local token. Requests to relays never carry the official account's credentials.
- **Client configuration** is applied field by field with a backup. Fields edited by you or other tools are reported as conflicts instead of being overwritten silently.
- **Proxy probes** only check network reachability, not authentication or quota. Codex's own login and Native requests leave from the Codex process, so restart Codex after changing its proxy.

## Platform support

| Platform | Notes |
| --- | --- |
| Windows | x64, NSIS installer (Chinese and English); the primary development and test platform |
| macOS | 12+, app bundle |
| Linux | Best effort for some workflows |

The terminal proxy is Windows-only.

## Development

**Requirements:** Node.js (24 is used locally), pnpm 11.22, Rust stable and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). On Windows that means the MSVC toolchain, Visual Studio 2022 Build Tools ("Desktop development with C++" plus the Windows SDK) and WebView2.

```bash
pnpm install --frozen-lockfile
pnpm tauri dev            # desktop app; `pnpm dev` is a browser-only preview without Rust commands
```

`scripts/tauri.mjs` selects `stable-x86_64-pc-windows-msvc` on Windows unless `RUSTUP_TOOLCHAIN` is set; your global Rust default is left alone.

**Isolated runs.** Point AgentHarbor at a throwaway Codex home without touching your real one:

```powershell
$env:AGENT_HARBOR_CODEX_HOME = Join-Path (Get-Location) '.runtime-test\codex'
pnpm tauri dev
```

This also separates the single-instance lock and WebView data. `AGENT_HARBOR_CLIENT_HOME` does the same for Claude Code, Claude Desktop, OpenCode and Pi paths.

**Checks.**

```bash
pnpm build           # TypeScript + Vite production build
pnpm test:rust       # Rust unit and workflow tests
pnpm test:frontend   # Playwright regressions against real components and mocked commands (run `pnpm build` first)
pnpm tauri build     # release build; Windows installer in src-tauri/target/release/bundle/nsis/
```

- The frontend tests need Playwright and Chromium. Use `PLAYWRIGHT_MODULE_PATH` and `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` if they are not on the default search path. Set `FRONTEND_QA_SCOPE` to run a single group.
- Tests against a real Codex binary are skipped by default. Set `AGENTHARBOR_TEST_CODEX_BIN` and run `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_codex`. They use temporary homes, synthetic credentials and loopback upstreams only.
- To refresh the model catalog snapshot for a release, run `node scripts/build-models-dev-snapshot.mjs`. At runtime a downloaded cache replaces the bundled snapshot whenever it is complete and valid.

## Project structure

```text
src/                 React UI (pages, components, hooks, locales en / zh)
src-tauri/src/core/  Rust domain logic: router and proxy, aggregation, client configs, accounts, sessions, MCP, Skills
src-tauri/src/       Tauri commands and platform code
scripts/             Build helpers and the frontend regression suite
assets/, LICENSES/   Branding and third-party notices
```

## Contributing

Issues and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) first: this app writes to real local Codex and client data, so changes must stay safe and reversible. For larger changes, open an issue first.

## License

[Apache License 2.0](LICENSE). Third-party components and data, including the models.dev snapshot, are listed in [LICENSES/THIRD_PARTY.md](LICENSES/THIRD_PARTY.md).

## Disclaimer

AgentHarbor is an independent tool. It is not affiliated with, endorsed by or sponsored by OpenAI or Anthropic. Use third-party relay services at your own risk and follow their terms of service.
