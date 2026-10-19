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

Codex, Claude Code, Claude Desktop, OpenCode, Pi, DeepSeek Harness and Cursor each keep accounts, model settings, sessions, MCP servers and instructions in their own files. Switching accounts when a quota runs out, wiring in third-party relays, or cleaning up sessions usually means hand-editing TOML, JSON and SQLite.

AgentHarbor is a Tauri 2 + React + Rust desktop app that does this for you. It runs a local model router (`127.0.0.1:43127` by default, changeable in Settings), writes each client's configuration as a reversible, field-level change with a backup, and shows what is actually happening on one dashboard. Codex is the primary integration; the other clients are supported through the same relays and router.

## Features

| Area | What you can do |
| --- | --- |
| **Dashboard** | Routing topology of every client, today's token pulse, recent sessions, and a foldable map of each client's data directory and key files |
| **Relays** | Manage many providers; per-model protocol (Responses, Chat Completions, Anthropic Messages), connectivity tests, New API balance, import/export, and a capability catalog with per-relay match preview (context, output limit, prices) from a bundled [models.dev](https://github.com/anomalyco/models.dev) snapshot |
| **Model inject** | Connect relay models and official-account models to Codex (local Router or direct Native, accountless mode, subagent defaults), Claude Code, Claude Desktop, OpenCode, Pi and DeepSeek Harness — preview, apply and restore, each with backups. Through the router, relays, account sources and aggregates combine freely. Cursor IDE and cursor-agent use them for the Agent, Tab and commit messages (see below). Each client panel also starts or restarts its client (see below) |
| **Local router** | Protocol translation between the three wire formats, model aggregation with priority tiers, failover, first-token timeout and live member state, plus request-level route diagnostics |
| **Gateway** | One local address and named keys give any agent that takes a base URL and an API key every enabled model (see below) |
| **Accounts** | Codex: ChatGPT OAuth login, quota windows, manual or threshold-based automatic switching (a running Codex keeps its task until the old account is used up). Account sources: ChatGPT (Sign in with ChatGPT), WorkBuddy, WorkBuddy AI, GitHub Copilot, Gemini CLI, Antigravity, Kiro, Grok, Cursor, Qoder, Qoder CN, Trae CN, Trae Global, MiniMax Code (China and Global), ZCode, OpenCode Zen, OpenCode Go and Command Code, which can be shared with model inject, plus a daily check-in for the sources that pay one (see below) |
| **Usage** | One ledger of every client's tokens and cost: local Codex and Claude Code sessions joined with routed requests, by client, channel, model and project; deleted sessions keep their usage |
| **Sessions** | Browse, search, export and delete Codex and Claude Code conversations; recover Codex threads its index lost |
| **MCP / Skills / Instructions** | Manage MCP servers, Skills (with backups) and the AgentHarbor-managed block of `AGENTS.md` / `CLAUDE.md` for Codex and Claude Code |
| **Image workbench** | Generate or edit images through a configured provider and keep them in a local gallery with the prompt and settings that made each one, browsed as a masonry of whole images or as a swipeable album slideshow; keep your own prompt templates in groups, with JSON import and export; polish a prompt with any model your relays, accounts or aggregates offer |
| **Settings & maintenance** | Theme, language, custom wallpaper with automatic dark-mode contrast, app avatar, API proxy, Windows terminal proxy, start at sign-in (into the tray), Windows memory optimization (working sets and standby cache), diagnostics, backup cleanup and Codex restart. The tray menu shows the local router and today's usage, opens any main page, and switches Cursor injection, start at sign-in, update checks and memory optimization |

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
- **Prompt cache across protocols.** Anthropic Messages caches nothing unmarked, so a request the router rebuilds as Messages (from Codex or any Chat client) carries the two marks Claude Code sends itself: the end of the system prompt and the last block of the conversation. The shared prefix is then read from cache each turn instead of billed in full. Codex's `prompt_cache_key` reaches the official Chat endpoint and Responses upstreams. The input a client is told about counts cache reads and writes, so Codex's context meter stays right.
- **Reasoning on Chat relays.** Codex replays reasoning items, but a relay never receives them. When a Chat relay sends its reasoning as `reasoning_content` (DeepSeek refuses a tool-enabled history without it), the router keeps that text in memory for the same relay and model and puts it back on the turn it came with. Relays answering in another field get nothing extra. After AgentHarbor restarts, earlier turns go without it.
- Native authenticates with the provider key: it is stored in the Codex `.env` under a random AgentHarbor variable and referenced via `env_key`, never written to `auth.json` or shown in previews. Treat `.env` and its backups as secrets — they are not encrypted.

## Starting clients

Every client panel on the inject page can start its client after a change:

- **Claude Code, OpenCode, Pi and DeepSeek Harness** open in a new terminal (Windows Terminal when installed, otherwise PowerShell) running the CLI in a working folder you pick once; the panel remembers it per client and keeps recent folders in the button's menu. Harness follows the injected profile: `desktop` starts or restarts Harness Desktop, `web` runs `dsh web`, and the one-shot `headless`, `sdk` and `acp` profiles offer their command to copy instead.
- **Codex, Claude Desktop and Cursor IDE** show *Start* while they are not running and *Restart* while they are; a stopped Codex starts without the restart prompt.
- Apps AgentHarbor starts no longer inherit its sockets, so quitting AgentHarbor while Cursor or Codex keeps running leaves the router's port free. If another program does hold that port while the router is down, the inject page names it (process and PID) and, after you confirm, ends it and starts the router again. A port Windows reserved for Hyper-V, WSL or Docker is reported as such. Either way another port can be chosen right there or in Settings: the routed clients and Codex are rewritten to it, and a port that cannot be taken changes nothing.
- **Routed clients come back.** Claude Code, OpenCode, Pi and Harness configurations in router mode are withdrawn when AgentHarbor quits, since the router stops with it, and applied again at the next start once the router holds its port, the way Cursor reconnects. While one waits (for example while another program holds the port) its panel says so and opens the configure dialog on it; restoring or releasing a client by hand, or choosing *Don't bring it back*, keeps it withdrawn.

## Combining model sources

The Claude Code, OpenCode, Pi and DeepSeek Harness inject dialogs have the same three switches in router mode: turning a kind off drops its chosen relays or aggregates and hides them. Claude Code with one relay, or with aggregates alone, works as before; with several relays, or relays and aggregates, it reaches them through the local router under the gateway's `short name/model` ids, and its `/model` menu lists every chosen model.

## How DeepSeek Harness connects

DeepSeek Harness (`dsh`, Harness Desktop) runs its custom models through pi-ai routes declared in a profile's `cordis.patch.yml`. Its inject dialog writes into one profile you choose (`desktop`, `web`, or any other under `$DSH_HOME/profiles`; `$DSH_HOME` defaults to `~/.dsh`):

- **Routes**: one `agentharbor-*` provider per relay and protocol (a route speaks one of Chat Completions, Responses or Anthropic Messages), plus one for the chosen aggregates, added to the profile's own `llm-pi-ai` entry next to your providers. A patch entry's `config` replaces the composed one as a whole, so AgentHarbor never adds a second `llm-pi-ai` entry.
- **Keys**: the routes name a credential reference; the value goes into Harness's `$DSH_HOME/.credentials.yaml` (`AGENTHARBOR_ROUTER_TOKEN` for the local router, one `AGENTHARBOR_RELAY_*` per relay in direct mode). Account sources and aggregates are reached through the router only, which keeps and refreshes their credentials. A headless profile (Harness 0.2.1 and later) leaves the credential store out, so for it the keys go into `$DSH_HOME/.env` instead: only AgentHarbor's own lines, withdrawn the same way on restore. A same-named variable exported when `dsh` starts, or one in the working folder's `.env`, takes precedence.
- **Default model**: optionally, `agent-default-model` starts new sessions on the chosen model. Picking another model in Harness afterwards is normal use, not a conflict, and a restore leaves your pick in place.
- Comments, `!!js` expressions and every other entry are kept as they are; each edit is read back and refused if the file would not read exactly as intended. Restoring puts the files back byte for byte when nothing else changed, otherwise field by field; an empty patch goes back as `[]`, which Harness needs to start. Quitting AgentHarbor withdraws router routes the same way, and the next start puts them back.
- Harness reads the profile patch again for the next request. A `$DSH_HOME/cordis.patch.yml` entry for the same id outranks the profile and is reported, never edited.

## How Cursor connects

Cursor decides where its models and Agent run and offers no address to fill in. AgentHarbor serves a local port that answers, in Cursor's servers' place, the calls made for an AgentHarbor model:

```text
Cursor IDE ── http.proxy ──┐
cursor-agent ── CURSOR_API_ENDPOINT ─┤──▶ AgentHarbor's Cursor port (127.0.0.1)
                                     ├ model list, account/plan checks, Agent, Tab, commit messages → handled here, models called through the local router
                                     └ everything else and Cursor's own models → sent to Cursor unchanged
```

- **Cursor IDE**: turning it on in the Cursor panel of the inject page writes four settings, `http.proxy` among them, into Cursor's `settings.json`, keeping its comments and layout. Turning it off, or quitting AgentHarbor, puts back only what AgentHarbor wrote; a value you changed since is left as it is. Restart Cursor to apply it; the panel has a restart button.
- **Local certificate**: Cursor reaches its servers over HTTPS, so the local proxy needs a certificate Cursor trusts. It is made on this machine and name-constrained to `cursor.sh`, `cursor.com` and their subdomains. On Windows, *Install and trust* adds it to your user's root store after Windows asks you, with no administrator rights; it can be removed the same way. On macOS and Linux the panel shows the command. Older certificates covering only `cursor.sh` are replaced and must be trusted again. Other sites pass through a plain tunnel, through Cursor's previous proxy if it had one.
- **cursor-agent CLI**: turning it on sets `CURSOR_API_ENDPOINT` for your user (Windows), so `cursor-agent` in every terminal opened afterwards is connected; *Open a connected terminal* does it for one. The CLI needs no certificate. Elsewhere the panel shows the command to start it with.
- **Account**: an IDE without an existing login gets a local identity in `globalStorage/state.vscdb`, restored when IDE access is turned off or AgentHarbor quits. Official login tokens stay intact. Local accounts and confirmed Free accounts receive a simulated active Ultra plan and usage response so the client can select injected models; paid accounts retain their official responses. This local state provides no official Cursor credits: injected models use your configured providers, and Cursor's own models still go to Cursor. Restart Cursor after enabling or disabling IDE access.
- **Agent**: with a model AgentHarbor offers, AgentHarbor runs the Agent loop locally. Cursor runs the tools (terminal, reading and editing files, search, MCP, subagents) and the results go back to the model; conversation checkpoints are kept in `cursor/conversations.db`. WebSearch uses the search account chosen under the shared Web search strip of the client console, and WebFetch pages are fetched by AgentHarbor. The prompts follow Cursor's tool conventions, so models call Cursor's tools as Cursor expects.
- **Model sources**: the panel has the same relay / account / aggregate switches, combined freely. Thinking levels follow models.dev; models of the Cursor account source are not offered back to Cursor.
- **Tab and commit messages**: by default both stay with Cursor, untouched. With a model chosen, Tab suggestions are made locally and your code is not sent to Cursor's Tab service; Git commit messages can use a model too, in English or Chinese.

The Agent protocol adapter is derived from [cursor-byok](https://github.com/leookun/cursor-byok) (MIT); see `LICENSES/THIRD_PARTY.md`.

## Cross-protocol web search

Codex, Claude Code and Claude Desktop keep their own WebSearch. The router translates the search declaration, results, citations and history to the target model's protocol.

- **Native first**: Codex's `web_search` runs on Anthropic Messages models and Claude's search runs on Responses models. A Chat model gets search only through a confirmed format, currently OpenAI Chat search and MiMo search: official endpoints are recognised automatically, and for a relay you pick the format under the provider's Advanced settings → cross-protocol search. Codex's default cached search mode runs as live search on other providers.
- **Check search capability**: the provider's Advanced settings can send one or two small billable requests with the saved key to see whether the model really searches. If a Chat format works, the check suggests it; the suggestion only changes the draft and is not saved automatically.
- **Fallbacks**: when the upstream explicitly refuses an optional search declaration, only the declaration is removed and the message goes through. That does not count as a successful search, and native search is not retried on that target for 30 minutes. You can also select a Responses / Messages search model under Web search at the bottom of the inject page's client console (off by default, shared by every routed client): it runs the searches and the original model writes the answer. A card on the official DeepSeek endpoint can instead offer DeepSeek's Search API (`/v1/tools/search`, billed per search from 1 November 2026) with the same key, so models without hosted search, such as DeepSeek Flash, still search; set such a model's cross-protocol search to "Always use selected search account".
- **Diagnostics**: per-request route diagnostics show what each request actually used: native adaptation, the selected search account, a separately run search, or an omitted declaration.

## Gateway

Instead of injecting a client, hand an agent the gateway from the model inject page:

- **Address** (with the default router port). OpenAI compatible (Responses, Chat Completions): `http://127.0.0.1:43127/gateway/v1`. Anthropic Messages: `http://127.0.0.1:43127/gateway`. `/gateway/v1/models` lists every model the key can use.
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
- **Ways to sign in.** A source that signs in more than one way (browser, the vendor's own app or CLI, a pasted key) offers them as a menu on its sign-in button; the first is the usual one. A pasted key or address is asked for in a dialog and kept like any other credential.
- **Daily check-in.** WorkBuddy, MiniMax Code, Qoder, Qoder CN and Trae CN pay credits for checking in each day. Turn on "Daily check-in" on the source's panel and AgentHarbor checks each of its accounts in once a Beijing day (two minutes after it starts, then every 30 minutes; a failure is tried again later); "Check in now" does it at once, and an account can be left out with the switch on its card. It is off by default. When a vendor pays the check-in only to its own app or asks for a captcha, the card says so and nothing is forced.
- **Prompts and client identity follow [magpie](https://github.com/yetone/magpie).** Where a vendor answers only its own client, AgentHarbor sends that client's headers, system prompt lines and request shape, and rewords the few phrases a vendor's check refuses, as magpie and its community plugins do. Vendor terms are never accepted for you. Using a subscription outside its own app may break its terms; where the vendor says so, the Accounts page shows the risk before signing in.
- **ChatGPT** uses OpenAI's Sign in with ChatGPT for apps on the user's machine: requests go to the standard `api.openai.com/v1/responses` with the client's own prompt. It is separate from the Codex accounts and needs its own sign-in. The limit is shared with ChatGPT and shown only at chatgpt.com; some plans are not eligible, and signing in says so. A Team, Business or Enterprise workspace must allow Sign in with ChatGPT apps first (its admin decides), or pick a personal workspace on the sign-in page.
- **WorkBuddy / WorkBuddy AI** (Tencent's CodeBuddy plan, China and international builds) sign in on their own; the WorkBuddy desktop app's session is never read or changed. The few phrases in Codex's and Claude Code's prompts that WorkBuddy's channel check refuses are reworded on the way, so every client works; wording it doesn't know yet may still be refused, and an aggregate moves such a request to its next member. Enterprise accounts get their enterprise's headers and usage.
- **GitHub Copilot** signs in with a GitHub device code (the code is shown on the Accounts page). Each model is asked on the API Copilot serves it on: Claude models on Anthropic Messages, newer GPT models on Responses, the rest on Chat Completions. Only models the account may call now are listed; a model whose terms are not accepted yet is enabled in GitHub's Copilot settings, never by AgentHarbor. The card shows this month's premium requests.
- **Gemini CLI / Antigravity** sign in with each app's Google sign-in and are served by Google Code Assist. Gemini CLI's sign-in is served only to Gemini Code Assist Standard and Enterprise, which need a Google Cloud project (entered on the account card). Antigravity serves personal accounts, but Google may suspend an account it sees used outside Antigravity, so the Accounts page asks first. Antigravity answers the system prompt of Claude Code and the Claude Agent SDK with a 429, so use it from OpenCode or Pi, or in an aggregate.
- **Kiro** signs in on Kiro's own page (Google, GitHub, AWS Builder ID or IAM Identity Center); each account is AgentHarbor's own, and the Kiro IDE's and kiro-cli's sign-ins are left alone. The card shows this month's credits.
- **Grok** (SuperGrok, X Premium+) signs in with xAI's Grok CLI (Grok Build), which must be installed: AgentHarbor runs `grok login` in a Grok home of its own, so the grok in your terminal stays signed in as it is, and the CLI keeps and renews the token. Requests go to the Responses API the CLI uses. Codex's grouped and freeform tools are turned into what Grok takes and restored in the reply, and whole numbers Grok writes as `2500.0` in tool arguments go back as `2500`.
- **Cursor** signs in with Cursor's CLI (`cursor-agent`), which must be installed and keeps one sign-in per machine; removing the account here does not sign the CLI out. Each request is a Cursor agent run over HTTP/2: the conversation goes in privacy mode, the client's tools are offered as tools Cursor calls back, and Cursor's own commands (shell, file edits) are never run by AgentHarbor.
- **Qoder / Qoder CN** sign in as Qoder's desktop client does (a device code; a personal access token on qoder.com; an enterprise VPC address on qoder.cn) and are served on the client's own chat API. Tool calls the model writes in its text are lifted out as real calls. Qoder's daily credits need Qoder desktop or Qoder CLI on this computer (its runtime-info helper reads the device identity), or `QODER_RUNTIME_INFO` set to its path; Qoder CN needs neither.
- **Trae CN / Trae Global** (experimental) sign in in the browser as the IDE does; the sign-in makes a device for this computer (ids and a P-256 key) and the token is renewed bound to it. Trae's chat takes text only, so tools are named in a system prompt and the calls the model writes (tags, GLM's template, DeepSeek's DSML) are lifted out as real calls. The international site has no check-in.
- **MiniMax Code** (China and Global) signs in with MiniMax Code's device code; `~/.minimax` is never read or changed. Models are served over Anthropic Messages. MiniMax gives its credits for MiniMax Code only and may limit an account used elsewhere, so the Accounts page asks first.
- **ZCode** signs in with Z.ai or BigModel (智谱), imports the ZCode app's sign-in, or takes a key. It uses the account's GLM Coding Plan, a team seat, or the free Start Plan (sent with ZCode's own prompt and headers); gift plans appear as `-Trial` models.
- **OpenCode Zen / OpenCode Go** import the key `opencode auth login` saved (auth.json is only read), or take a pasted key; each model is asked on the API OpenCode's docs name. Zen's free models need no account: requests go as OpenCode 1.19's own, always streamed, with OpenCode's core tools as stand-ins. Gemini models use Google's own API and are left out.
- **Command Code** signs in in the browser, from the CLI's `~/.commandcode/auth.json`, or with a key. Claude goes over Anthropic Messages and the rest over the API Command Code names; the Go plan has no Provider API, so it is asked on `/alpha/generate` as the CLI does.
- Google's, Kiro's, Cursor's, Qoder's, Trae's and Command Code Go's own APIs are spoken through Chat Completions inside the router, so every client protocol reaches them with the usual translation.
- **Claude Desktop** does not support account sources yet.

## Data and safety

- **Codex home**: `AGENT_HARBOR_CODEX_HOME`, else `CODEX_HOME`, else `~/.codex`.
- **AgentHarbor data** (relays, routing state, usage ledger, caches): the platform's local data directory under `AgentHarbor` (for example `%LOCALAPPDATA%\AgentHarbor`), or `AGENT_HARBOR_DATA_DIR`. The dashboard's data map shows the exact paths.
- **Provider credentials** live in `newapi-providers.dat`, encrypted with DPAPI for the current user on Windows and stored owner-only (`0600`, not encrypted) elsewhere. The UI never reads saved secrets back; leaving a field empty keeps the stored value.
- **Account source credentials** live in `account-sources.dat` with the same protection. Their tokens are used only when the router forwards a request and are never shown in the UI.
- **Cursor**: the local certificate and its key live in the `cursor` folder of the AgentHarbor data directory, the key readable by its owner only. The certificate is name-constrained to `cursor.sh` and `cursor.com`. Quitting AgentHarbor restores its local identity writes, Cursor's `settings.json` and `CURSOR_API_ENDPOINT`; the next start connects it again.
- **DeepSeek Harness** keeps the keys its routes name in its own `.credentials.yaml`, or in `$DSH_HOME/.env` for a headless profile (written owner-only, `0600`, on macOS and Linux, as Harness requires; not encrypted); the profile patch holds only the reference names.
- **Gateway keys** are stored encrypted with the provider credentials. The gateway listens on `127.0.0.1` only and accepts gateway keys only; upstream requests carry each relay's own credentials, never a gateway key.
- **The local router** requires a local token. Requests to relays never carry the official account's credentials.
- **Client configuration** is applied field by field with a backup. Fields edited by you or other tools are reported as conflicts instead of being overwritten silently.
- **Proxy probes** only check network reachability, not authentication or quota. Codex's own login and Native requests leave from the Codex process, so restart Codex after changing its proxy.
- **Direct cards** keep only their relay off the proxy. Through the router each card takes its own exit; Codex Native, direct client configurations and clients started from their panel keep the proxy for everything else (sign-in, MCP, plugins). The system proxy is read live, so starting, stopping or moving your proxy software needs no AgentHarbor restart. In TUN mode the proxy software's rules decide: no app can leave a TUN adapter.

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

This also separates the single-instance lock and WebView data. `AGENT_HARBOR_CLIENT_HOME` does the same for Claude Code, Claude Desktop, OpenCode, Pi, DeepSeek Harness (`.dsh` under it instead of `DSH_HOME`) and Cursor IDE paths (and leaves `CURSOR_API_ENDPOINT` alone).

**Checks.**

```bash
pnpm build           # TypeScript + Vite production build
pnpm test:rust       # Rust unit and workflow tests
pnpm test:frontend   # Playwright regressions against real components and mocked commands (run `pnpm build` first)
pnpm release         # signed release build: installer, updater signature, latest.json and SHA256SUMS.txt in src-tauri/target/release/bundle/nsis/
```

- The frontend tests need Playwright and Chromium. Use `PLAYWRIGHT_MODULE_PATH` and `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` if they are not on the default search path. Set `FRONTEND_QA_SCOPE` to run a single group.
- Tests against a real Codex binary are skipped by default. Set `AGENTHARBOR_TEST_CODEX_BIN` and run `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_codex`. They use temporary homes, synthetic credentials and loopback upstreams only.
- The real DeepSeek Harness test is skipped the same way: set `AGENTHARBOR_TEST_DSH_BIN` to an installed `dsh` (or its `lib/bin.js`, run with Node) and run `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_harness`. It initializes a profile in a temporary `DSH_HOME` and drives each route kind through the router to a loopback upstream.
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
