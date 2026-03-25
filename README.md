<div align="center">

<img src="https://capsule-render.vercel.app/api?type=waving&color=0:1B5E20,50:00E676,100:FFB300&height=200&text=PULSE&fontSize=80&fontColor=E8F1F2&fontAlignY=35&desc=htop%20for%20AI%20coding%20sessions.&descAlignY=55&descSize=22&descAlign=50&animation=fadeIn" width="100%" alt="Pulse" />

[![Rust](https://img.shields.io/badge/Rust-2E7D32?style=for-the-badge&logo=rust&logoColor=E8F1F2&labelColor=1C1C1C)](https://rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-1B5E20?style=for-the-badge&labelColor=1C1C1C)](LICENSE)
[![CI](https://img.shields.io/github/actions/workflow/status/OriginalMHV/Pulse/ci.yml?style=for-the-badge&label=CI&labelColor=1C1C1C&color=2E7D32)](https://github.com/OriginalMHV/Pulse/actions)

[![Lines of Code](https://img.shields.io/badge/Lines_of_Code-2.5k-E8F1F2?style=for-the-badge&labelColor=1C1C1C)](https://github.com/OriginalMHV/Pulse)
[![Providers](https://img.shields.io/badge/Providers-3-FFB300?style=for-the-badge&labelColor=1C1C1C)](https://github.com/OriginalMHV/Pulse)
[![GitHub Release](https://img.shields.io/github/v/release/OriginalMHV/Pulse?style=for-the-badge&labelColor=1C1C1C&color=1B5E20)](https://github.com/OriginalMHV/Pulse/releases/latest)

[Install](#install) · [Usage](#usage) · [Providers](#supported-providers) · [Keybindings](#keybindings)

</div>

---

## What is Pulse?

A real-time TUI dashboard for your AI coding sessions. Like htop shows CPU, memory, and processes, Pulse shows tokens, cost, tools, and files for Copilot CLI, Claude Code, and Codex CLI.

No API keys. No proxies. No hooks. It reads the session files your tools already write to disk.

### Features

| Feature | Description |
|---------|-------------|
| **Live monitoring** | Watches session files in real-time via filesystem events |
| **Multi-provider** | Supports Copilot CLI, Claude Code, and Codex CLI |
| **Multiple modes** | Full dashboard, focus view, macOS menu bar, tmux side pane |
| **Token tracking** | Input, output, and cache token counts per session |
| **Cost estimation** | Per-model pricing for Claude, GPT, o-series, and Gemini models |
| **Tool breakdown** | Call counts, success rates, and durations per tool |
| **File audit** | Which files were read, written, and how many times |
| **Active detection** | Green dot for sessions currently running |
| **Event feed** | Chronological stream of tool calls, messages, and warnings |
| **CLI mode** | `--list` and `--stats` flags for scripting and quick checks |

### Supported providers

| Provider | Status | Session location |
|----------|--------|------------------|
| GitHub Copilot CLI | Supported | `~/.copilot/session-state/` |
| Claude Code | Supported | `~/.claude/projects/` |
| OpenAI Codex CLI | Supported | `~/.codex/sessions/` |

## Install

```bash
# Homebrew (macOS/Linux)
brew tap OriginalMHV/tap
brew install pulse

# Cargo
cargo install pulse-cli

# Shell (macOS/Linux)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/OriginalMHV/Pulse/releases/latest/download/pulse-cli-installer.sh | sh

# From source
git clone https://github.com/OriginalMHV/Pulse.git
cd Pulse && cargo install --path .
```

Requires Rust >= 1.85 for building from source.

## Getting started

```bash
pulse --init
```

The setup wizard detects your installed AI tools, lets you pick a default mode, and optionally installs the macOS menu bar widget with auto-start on login.

## Usage

```bash
# interactive setup
pulse --init

# launch the full TUI dashboard
pulse

# focus mode: compact single-session view (great for side panes)
pulse --focus

# macOS menu bar widget (detaches to background)
pulse --menubar

# install menu bar as a LaunchAgent (auto-starts on login)
pulse --install

# remove LaunchAgent and stop daemon
pulse --uninstall

# list sessions as plain text
pulse --list

# show aggregate statistics
pulse --stats

# filter by provider
pulse --provider claude

# attach as a tmux side pane (run this inside tmux)
pulse --attach

# attach filtered to a specific provider
pulse --attach copilot
```

### Modes

| Mode | Flag | Best for |
|------|------|----------|
| Dashboard | `pulse` | Full overview of all sessions |
| Focus | `--focus` | Compact single-session view, tmux side panes |
| Menu bar | `--menubar` | Always-visible macOS widget, zero terminal overhead |
| tmux attach | `--attach` | Auto-splits a focus pane alongside your current terminal |
| CLI | `--list` / `--stats` | Scripting and quick checks |

### Keybindings (Dashboard)

| Key | Action |
|-----|--------|
| `j` `k` / `↑` `↓` | Navigate sessions |
| `Enter` | Expand session detail |
| `Tab` | Cycle detail view: Feed / Tools / Files |
| `f` | Cycle provider filter |
| `/` | Search sessions |
| `r` | Force refresh |
| `←` `→` | Scroll event feed |
| `g` / `G` | Jump to first / last session |
| `q` / `Esc` | Quit |

### Keybindings (Focus)

| Key | Action |
|-----|--------|
| `j` `k` / `↑` `↓` | Scroll event feed (disables auto-scroll) |
| `s` | Toggle auto-scroll |
| `f` | Cycle provider filter |
| `r` | Force refresh |
| `q` / `Esc` | Quit |

### Search mode

| Key | Action |
|-----|--------|
| Type | Filter sessions |
| `Enter` / `Esc` | Exit search |
| `Ctrl+U` | Clear search |

## How it works

Pulse uses a provider-based architecture to discover and monitor sessions from multiple AI coding tools:

- **Copilot CLI**: Watches `events.jsonl` in each session directory. Parses event types (user messages, tool executions, assistant responses) and detects active sessions via lock files.
- **Claude Code**: Watches JSONL session files under `~/.claude/projects/`. Extracts tool use from assistant message content arrays and full token breakdowns including cache metrics.
- **Codex CLI**: Walks nested date directories under `~/.codex/sessions/` for rollout JSONL files. Parses session metadata and user messages.

All monitoring is read-only. Pulse never modifies your session data.

The file watcher uses OS-native filesystem events (FSEvents on macOS, inotify on Linux) for minimal overhead. JSONL files are read incrementally by tracking byte offsets, so only new lines are parsed on each update.

### Cost estimation

Pulse estimates session cost using published API pricing:

| Model | Input | Output |
|-------|-------|--------|
| Claude Opus 4/4.5/4.6 | $15/M tokens | $75/M tokens |
| Claude Sonnet 4/4.5/4.6 | $3/M tokens | $15/M tokens |
| Claude Haiku 4.5 | $0.80/M tokens | $4/M tokens |
| GPT-5 | $10/M tokens | $30/M tokens |
| GPT-5 mini | $1.50/M tokens | $6/M tokens |
| GPT-4.1 | $2/M tokens | $8/M tokens |
| GPT-4.1 mini | $0.40/M tokens | $1.60/M tokens |
| GPT-4.1 nano | $0.10/M tokens | $0.40/M tokens |
| o3 | $2/M tokens | $8/M tokens |
| o3-pro | $20/M tokens | $80/M tokens |
| o4-mini | $1.10/M tokens | $4.40/M tokens |
| Gemini 2.5 Pro | $1.25/M tokens | $10/M tokens |
| Gemini 2.5 Flash | $0.15/M tokens | $0.60/M tokens |

Cache read and write tokens are priced at their respective discounted rates. Copilot CLI sessions use Claude models under the hood.

## License

MIT. See [LICENSE](LICENSE).

<img src="https://capsule-render.vercel.app/api?type=waving&color=0:FFB300,50:00E676,100:1B5E20&height=120&section=footer&reversal=true" width="100%" alt="" />
