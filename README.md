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

Codex, Claude Code, Claude Desktop, OpenCode, Pi, DeepSeek Harness and Cursor each keep accounts, models, sessions, MCP servers and instructions in their own places. AgentHarbor manages them from one desktop app: switch accounts, wire in third-party relays, combine models and clean up sessions without hand-editing config files. Every change to a client's configuration is backed up first and can be restored at any time.

## Features

### Models and routing
- **Relays**: manage many model providers over Responses, Chat Completions and Anthropic Messages, with connectivity tests, balance checks, import/export and a model capability catalog (context, prices and more)
- **Model inject**: connect relay and account models to Codex, Claude Code, Claude Desktop, OpenCode, Pi, DeepSeek Harness and Cursor in one click, with preview, apply and restore
- **Local router**: automatic translation between the three protocols, plus model aggregation with priority-based failover
- **Gateway**: one local address and an API key give any agent with a custom base URL every model you have
- **Cross-protocol web search**: each client's built-in web search keeps working on models of another protocol
- **Cursor**: use your own models for Cursor's Agent, Tab completion and commit messages
- **Remote hosts**: edit Codex, Claude Code, OpenCode and Pi configuration on servers over SSH (hosts resolved as `ssh` resolves them; ssh-agent keys, key files or a password), direct to a relay or through this computer's router over an SSH tunnel; config directories follow the server's `CODEX_HOME` and `CLAUDE_CONFIG_DIR`, Codex can be a profile beside its official sign-in, and a web search MCP server (DeepSeek's search API or MiMo's own search) deploys in one click, with preview and restore
- **One-click launch**: start or restart a client right after changing its configuration

### Accounts
- **Codex accounts**: ChatGPT sign-in, quota windows, manual or threshold-based automatic switching
- **Account sources**: sign in to subscriptions such as ChatGPT, WorkBuddy, GitHub Copilot, Gemini CLI, Antigravity, Kiro, Grok, Cursor, Qoder, Trae, MiniMax Code, ZCode, OpenCode Zen / Go and Command Code, and use them as model providers
- **Daily check-in**: automatic check-in for sources that reward it

### Sessions and configuration
- **Usage**: one ledger of every client's tokens and cost, split by client, channel, model and project
- **Sessions**: browse, search, export and delete Codex and Claude Code conversations, and recover lost threads
- **MCP**: one list of MCP servers for every agent, switched per agent
- **Skills**: one copy of each Skill, shared with the agents you choose
- **Instructions**: manage `AGENTS.md` / `CLAUDE.md`

### Image workbench
- Text-to-image, or image-to-image from up to 16 reference images
- A local gallery that keeps each image's prompt and settings
- Prompt templates, a materials book (outfits, poses, angles and more) and a reference image library
- Polish prompts with any of your models

### And more
- Dashboard with router status, today's usage and recent sessions
- Themes, language, custom wallpaper, proxy settings, start at sign-in and memory optimization
- Tray menu shortcuts
- In-app updates

## Platforms

| Platform | Notes |
| --- | --- |
| Windows | x64, the primary platform |
| macOS | 12+ |
| Linux | Some features |

Download the installer from [Releases](https://github.com/liangyu-love/AgentHarbor/releases/latest).

## Run from source

Requires Node.js, pnpm, Rust and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```bash
pnpm install --frozen-lockfile
pnpm tauri dev
```

Issues and pull requests are welcome; please read [CONTRIBUTING.md](CONTRIBUTING.md) first.

## License

[Apache License 2.0](LICENSE). Third-party components and data, including the models.dev snapshot and code derived from [cursor-byok](https://github.com/leookun/cursor-byok), are listed in [LICENSES/THIRD_PARTY.md](LICENSES/THIRD_PARTY.md).

## Disclaimer

AgentHarbor is an independent tool, not affiliated with, endorsed by or sponsored by OpenAI, Anthropic or any other vendor. Using a subscription outside its vendor's own app may break the vendor's terms; use third-party relays and account sources at your own risk.
