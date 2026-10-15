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

Codex, Claude Code, Claude Desktop, OpenCode, Pi and Cursor each keep accounts, model settings, sessions, MCP servers and instructions in their own files. Switching accounts when a quota runs out, wiring in third-party relays, or cleaning up sessions usually means hand-editing TOML, JSON and SQLite.

AgentHarbor is a Tauri 2 + React + Rust desktop app that does this for you. It runs a local model router (`127.0.0.1:43127`), writes each client's configuration as a reversible, field-level change with a backup, and shows what is actually happening on one dashboard. Codex is the primary integration; the other clients are supported through the same relays and router.

## Features

| Area | What you can do |
| --- | --- |
| **Dashboard** | Routing topology of every client, today's token pulse, recent sessions, and a foldable map of each client's data directory and key files |
| **Relays** | Manage many providers; per-model protocol (Responses, Chat Completions, Anthropic Messages), connectivity tests, New API balance, import/export, and a capability catalog with per-relay match preview (context, output limit, prices) from a bundled [models.dev](https://github.com/anomalyco/models.dev) snapshot |
| **Model inject** | Connect relay models and official-account models to Codex (local Router or direct Native, accountless mode, subagent defaults), Claude Code, Claude Desktop, OpenCode and Pi — preview, apply and restore, each with backups. Through the router, relays, account sources and aggregates combine freely. Cursor IDE and cursor-agent use them for the Agent, Tab and commit messages (see below) |
| **Local router** | Protocol translation between the three wire formats, model aggregation with priority tiers, failover, first-token timeout and live member state, plus request-level route diagnostics |
| **Gateway** | One local address and named keys give any agent that takes a base URL and an API key every enabled model (see below) |
| **Accounts** | Codex: ChatGPT OAuth login, quota windows, manual or threshold-based automatic switching (a running Codex keeps its task until the old account is used up). Account sources: ChatGPT (Sign in with ChatGPT), WorkBuddy, WorkBuddy AI, GitHub Copilot, Gemini CLI, Antigravity, Kiro, Grok and Cursor, which can be shared with model inject (see below) |
| **Usage** | A token monitor across Codex, Claude Code and relay traffic with recorded prices |
| **Sessions** | Browse, search, export and delete Codex and Claude Code conversations; recover Codex threads its index lost |
| **MCP / Skills / Instructions** | Manage MCP servers, Skills (with backups) and the AgentHarbor-managed block of `AGENTS.md` / `CLAUDE.md` for Codex and Claude Code |
| **Image workbench** | Generate or edit images through a configured provider and keep them in a local gallery |
| **Settings & maintenance** | Theme, language, custom wallpaper with automatic dark-mode contrast, app avatar, API proxy, Windows terminal proxy, start at sign-in (into the tray), diagnostics, backup cleanup and Codex restart |

Not available yet: account import/export, in-app plugin management, quota heatmaps and the macOS notch display. Updates come from GitHub Releases and are installed in the app once their signature checks out.

## How Codex connects

The access mode is independent of the upstream protocol and the network route.

| Mode | How requests flow | Keep AgentHarbor running? |
| --- | --- | --- |
| **Router** | Codex talks to the local Responses gateway, which forwards each model with its own protocol and translates Chat Completions / Anthropic Messages. Official and relay models can share one menu, and the official account stays signed in unless accountless mode is on. | Yes, while using routed models |
| **Native** | Codex connects straight to the selected provider. Only that provider's Responses models are listed, under their real IDs; an incompatible default is rejected rather than swapped. | No |

- Enabling access edits only the fields AgentHarbor manages in `config.toml`; disabling it restores what was there before. Restart Codex after changing the mode, models or proxy (the app has a restart button).
- **Model sources.** The Codex card's relay / account source / aggregate switches combine freely: the menu holds every kind turned on, and a kind turned off can no longer be called. With aggregates on, one can be Codex's startup model. ChatGPT's own models still follow accountless mode.
- **Thinking levels.** A relay model's levels come from what models.dev declares, whenever it declares any. Sources that disagree give the levels they all list; a model described by a thinking budget only (Claude, for example) shows low / medium / high / xhigh, which the router turns into budgets; a speed build (`-fast`) thinks like its base model; a model its publisher does not list at all uses other providers' entries for the same id. models.dev is refreshed daily, so new models bring their levels without manual upkeep. The default is medium, or high without a medium. A model whose thinking is only a switch (MiMo, for example) shows on and off; a model with levels that can also stop thinking gets an extra off. Off reaches the upstream as its own field: `thinking.type`, OpenRouter's `reasoning.enabled` or DashScope's `enable_thinking`. Levels typed under the relay model's field overrides are shown as typed.
- Relay models inherit Codex's client behaviour from the official model of the same name, so multi-agent V2 and the **Ultra** effort work through a relay too. Ultra is a Codex-side setting: upstream still receives a concrete effort such as `xhigh` or `max`.
- Native authenticates with the provider key: it is stored in the Codex `.env` under a random AgentHarbor variable and referenced via `env_key`, never written to `auth.json` or shown in previews. Treat `.env` and its backups as secrets — they are not encrypted.

## Combining model sources

The Claude Code, OpenCode and Pi inject dialogs have the same three switches in router mode: turning a kind off drops its chosen relays or aggregates and hides them. Claude Code with one relay, or with aggregates alone, works as before; with several relays, or relays and aggregates, it reaches them through the local router under the gateway's `short name/model` ids, and its `/model` menu lists every chosen model.

## How Cursor connects

Cursor decides where its models and Agent run and offers no address to fill in. AgentHarbor serves a local port that answers, in Cursor's servers' place, the calls made for an AgentHarbor model:

```text
Cursor IDE ── http.proxy ──┐
cursor-agent ── CURSOR_API_ENDPOINT ─┤──▶ AgentHarbor's Cursor port (127.0.0.1)
                                     ├ model list, Agent, Tab, commit messages → handled here, models called through the local router
                                     └ everything else and Cursor's own models → sent to Cursor unchanged
```

- **Cursor IDE**: turning it on in the Cursor panel of the inject page writes four settings, `http.proxy` among them, into Cursor's `settings.json`, keeping its comments and layout. Turning it off, or quitting AgentHarbor, puts back only what AgentHarbor wrote; a value you changed since is left as it is. Restart Cursor to apply it; the panel has a restart button.
- **Local certificate**: Cursor reaches its servers over HTTPS, so the local proxy needs a certificate Cursor trusts. It is made on this machine and name-constrained to `cursor.sh` and its subdomains, so it cannot stand in for any other site. On Windows, *Install and trust* adds it to your user's root store after Windows asks you, with no administrator rights; it can be removed the same way. On macOS and Linux the panel shows the command. Only traffic to `*.cursor.sh` is decrypted; every other site passes through a plain tunnel, through Cursor's previous proxy if it had one.
- **cursor-agent CLI**: turning it on sets `CURSOR_API_ENDPOINT` for your user (Windows), so `cursor-agent` in every terminal opened afterwards is connected; *Open a connected terminal* does it for one. The CLI needs no certificate. Elsewhere the panel shows the command to start it with.
- **Account**: sign in to Cursor with your own account. AgentHarbor writes no token and leaves the account, plan and usage alone; Cursor's own models keep going to Cursor. Whether a plan may use custom models is Cursor's rule.
- **Agent**: with a model AgentHarbor offers, AgentHarbor runs the Agent loop locally. Cursor runs the tools (terminal, reading and editing files, search, MCP, subagents) and the results go back to the model; conversation checkpoints are kept in `cursor/conversations.db`. WebSearch uses the search account chosen under the shared Web search strip of the client console, and WebFetch pages are fetched by AgentHarbor. The prompts follow Cursor's tool conventions, so models call Cursor's tools as Cursor expects.
- **Model sources**: the panel has the same relay / account / aggregate switches, combined freely. Thinking levels follow models.dev; models of the Cursor account source are not offered back to Cursor.
- **Tab and commit messages**: by default both stay with Cursor, untouched. With a model chosen, Tab suggestions are made locally and your code is not sent to Cursor's Tab service; Git commit messages can use a model too, in English or Chinese.

The Agent protocol adapter is derived from [cursor-byok](https://github.com/leookun/cursor-byok) (MIT); see `LICENSES/THIRD_PARTY.md`.

## Cross-protocol web search

Codex, Claude Code and Claude Desktop keep their own WebSearch. The router translates the search declaration, results, citations and history to the target model's protocol.

- **Native first**: Codex's `web_search` runs on Anthropic Messages models and Claude's search runs on Responses models. A Chat model gets search only through a confirmed format, currently OpenAI Chat search and MiMo search: official endpoints are recognised automatically, and for a relay you pick the format under the provider's Advanced settings → cross-protocol search. Codex's default cached search mode runs as live search on other providers.
- **Check search capability**: the provider's Advanced settings can send one or two small billable requests with the saved key to see whether the model really searches. If a Chat format works, the check suggests it; the suggestion only changes the draft and is not saved automatically.
- **Fallbacks**: when the upstream explicitly refuses an optional search declaration, only the declaration is removed and the message goes through. That does not count as a successful search, and native search is not retried on that target for 30 minutes. You can also select a Responses / Messages search model under Web search at the bottom of the inject page's client console (off by default, shared by every routed client): it runs the searches and the original model writes the answer.
- **Diagnostics**: per-request route diagnostics show what each request actually used: native adaptation, the selected search account, a separately run search, or an omitted declaration.

## Gateway

Instead of injecting a client, hand an agent the gateway from the model inject page:

- **Address.** OpenAI compatible (Responses, Chat Completions): `http://127.0.0.1:43127/gateway/v1`. Anthropic Messages: `http://127.0.0.1:43127/gateway`. `/gateway/v1/models` lists every model the key can use.
- **Keys.** Create a named key per agent; copy, regenerate or delete each on its own. Gateway keys are separate from the token injected client configs carry, and neither works in the other's place.
- **Model names.**
  - Relay and account-source models are `short name/model`, for example `deepseek/deepseek-v4-pro` or `workbuddy/glm-5.3`. Aggregates are `prefix/public id`, `pool/gpt-5.5` by default, so they never read as a relay's model of the same name; the shared prefix is set in the gateway panel and one aggregate can name its own. A request that still sends an aggregate's bare public id keeps working.
  - The short name is derived from the relay's name and can be edited in the relay editor; renaming the relay later does not change it.
- **What it serves.** Click relay models, account sources or aggregates on the right of the gateway diagram to turn that kind off: it leaves the model list and its requests are refused.
- **Protocol translation** works as for injected clients, for example calling a Chat Completions-only relay over Responses.
- **Keep running.** Turn on "Keep the gateway running" to keep the local router up even with no client injected.
- **Connection settings** can be copied as a generic, Anthropic SDK or OpenCode template. Previews mask the key; copying writes the real one.

## Account sources

An account source is a subscription whose own sign-in serves models. Sign in on the Accounts page and turn on "Share with model inject": the source then appears as a special provider on the model inject page and in aggregates.

- **Pick the models.** A source shares all its models by default; click a model on the Accounts page to leave it out of model inject and Codex's menu (the Codex desktop app's menu shows the first 100 models, and the Codex card names any it leaves out). Leaving every model out stops the share.
- **Router only.** For each request the router takes the active account's token and refreshes it when it is about to expire. No account token is ever written to a client config, so direct mode and Codex Native cannot select an account source.
- **ChatGPT** uses OpenAI's Sign in with ChatGPT for apps on the user's machine: requests go to the standard `api.openai.com/v1/responses` with the client's own prompt. It is separate from the Codex accounts and needs its own sign-in. The limit is shared with ChatGPT and shown only at chatgpt.com; some plans are not eligible, and signing in says so. A Team, Business or Enterprise workspace must allow Sign in with ChatGPT apps first (its admin decides), or pick a personal workspace on the sign-in page.
- **WorkBuddy / WorkBuddy AI** (Tencent's CodeBuddy plan, China and international builds) sign in on their own; the WorkBuddy desktop app's session is never read or changed. They refuse chats from Codex and Claude Code because of their system prompts, so use them from OpenCode or Pi; in an aggregate, a refused request moves to the next member.
- **GitHub Copilot** signs in with a GitHub device code (the code is shown on the Accounts page). Each model is asked on the API Copilot serves it on: Claude models on Anthropic Messages, newer GPT models on Responses, the rest on Chat Completions. Only models the account may call now are listed; a model whose terms are not accepted yet is enabled in GitHub's Copilot settings, never by AgentHarbor. The card shows this month's premium requests.
- **Gemini CLI / Antigravity** sign in with each app's Google sign-in and are served by Google Code Assist. Gemini CLI's sign-in is served only to Gemini Code Assist Standard and Enterprise, which need a Google Cloud project (entered on the account card). Antigravity serves personal accounts, but Google may suspend an account it sees used outside Antigravity, so the Accounts page asks first. Antigravity answers the system prompt of Claude Code and the Claude Agent SDK with a 429, so use it from OpenCode or Pi, or in an aggregate.
- **Kiro** signs in on Kiro's own page (Google, GitHub, AWS Builder ID or IAM Identity Center); each account is AgentHarbor's own, and the Kiro IDE's and kiro-cli's sign-ins are left alone. The card shows this month's credits.
- **Grok** (SuperGrok, X Premium+) signs in with xAI's Grok CLI (Grok Build), which must be installed: AgentHarbor runs `grok login` in a Grok home of its own, so the grok in your terminal stays signed in as it is, and the CLI keeps and renews the token. Requests go to the Responses API the CLI uses. Codex's grouped and freeform tools are turned into what Grok takes and restored in the reply, and whole numbers Grok writes as `2500.0` in tool arguments go back as `2500`.
- **Cursor** signs in with Cursor's CLI (`cursor-agent`), which must be installed and keeps one sign-in per machine; removing the account here does not sign the CLI out. Each request is a Cursor agent run over HTTP/2: the conversation goes in privacy mode, the client's tools are offered as tools Cursor calls back, and Cursor's own commands (shell, file edits) are never run by AgentHarbor.
- Google's, Kiro's and Cursor's own APIs are spoken through Chat Completions inside the router, so every client protocol reaches them with the usual translation.
- **Claude Desktop** does not support account sources yet.

## Data and safety

- **Codex home**: `AGENT_HARBOR_CODEX_HOME`, else `CODEX_HOME`, else `~/.codex`.
- **AgentHarbor data** (relays, routing state, usage ledger, caches): the platform's local data directory under `AgentHarbor` (for example `%LOCALAPPDATA%\AgentHarbor`), or `AGENT_HARBOR_DATA_DIR`. The dashboard's data map shows the exact paths.
- **Provider credentials** live in `newapi-providers.dat`, encrypted with DPAPI for the current user on Windows and stored owner-only (`0600`, not encrypted) elsewhere. The UI never reads saved secrets back; leaving a field empty keeps the stored value.
- **Account source credentials** live in `account-sources.dat` with the same protection. Their tokens are used only when the router forwards a request and are never shown in the UI.
- **Cursor**: the local certificate and its key live in the `cursor` folder of the AgentHarbor data directory, the key readable by its owner only. The certificate is name-constrained to `cursor.sh`. Quitting AgentHarbor restores Cursor's `settings.json` and `CURSOR_API_ENDPOINT`, so Cursor never points at a port nothing listens on; the next start connects it again.
- **Gateway keys** are stored encrypted with the provider credentials. The gateway listens on `127.0.0.1` only and accepts gateway keys only; upstream requests carry each relay's own credentials, never a gateway key.
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

This also separates the single-instance lock and WebView data. `AGENT_HARBOR_CLIENT_HOME` does the same for Claude Code, Claude Desktop, OpenCode, Pi and Cursor IDE paths (and leaves `CURSOR_API_ENDPOINT` alone).

**Checks.**

```bash
pnpm build           # TypeScript + Vite production build
pnpm test:rust       # Rust unit and workflow tests
pnpm test:frontend   # Playwright regressions against real components and mocked commands (run `pnpm build` first)
pnpm release         # signed release build: installer, updater signature, latest.json and SHA256SUMS.txt in src-tauri/target/release/bundle/nsis/
```

- The frontend tests need Playwright and Chromium. Use `PLAYWRIGHT_MODULE_PATH` and `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` if they are not on the default search path. Set `FRONTEND_QA_SCOPE` to run a single group.
- Tests against a real Codex binary are skipped by default. Set `AGENTHARBOR_TEST_CODEX_BIN` and run `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_codex`. They use temporary homes, synthetic credentials and loopback upstreams only.
- Release builds sign their installer for the in-app updater. The private key is read from `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PATH` or `~/.tauri/agentharbor-updater.key` and never enters the repository; keep a backup, since installs only accept updates signed with it.
- To release, bump the version (`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`), commit, and push an annotated tag whose message is the release notes: `git tag -a v1.0.2 -F notes.md && git push origin v1.0.2`. GitHub Actions runs the tests, builds and signs with the `TAURI_SIGNING_PRIVATE_KEY` repository secret, and drafts the release with the four files above. Publish the draft as the latest release (not a pre-release): the updater reads `releases/latest/download/latest.json`. Running the Release workflow by hand builds the files as artifacts without a release.
- CI runs the type check, the Rust tests and the frontend regressions on every push to `main` and every pull request.
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
