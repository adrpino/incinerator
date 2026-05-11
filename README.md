# 🔥 Incinerator

A CLI tool to track how much money you're "burning" through tokens across **Cline**, **Claude Code**, and **Gemini CLI**.

Incinerator parses your local logs from various AI CLI tools to provide a unified, colorized view of your token usage and financial costs — either as an interactive live dashboard or a one-shot text report.

## Features

- **Interactive TUI**: Live dashboard with multiple views (totals, per-provider costs, daily costs, daily tokens). Auto-refreshes when your logs change.
- **Unified Reporting**: One view across multiple AI tools.
- **Stacked Token Visualizations**: Breakdown of Input, Output, Cache Read, and Cache Create tokens.
- **Historical Analysis**: Daily costs (last 14 days, configurable) and monthly summaries.
- **Model Breakdown**: See which models are consuming the most tokens.
- **Fast and Efficient**: Rust + Rayon for parallel parsing; per-file caching keeps refreshes under 100ms.

## Supported Tools

- **Cline**: Analyzes conversation logs.
- **Claude Code**: Analyzes session logs in `~/.claude/projects`.
- **Gemini CLI**: Analyzes usage logs.

## Installation

### From Source

Ensure you have Rust and Cargo installed. Then, clone the repository and build:

```bash
git clone https://github.com/adrianpino/incinerator
cd incinerator
cargo install --path .
```

## Usage

By default, `incinerator` launches the interactive TUI dashboard:

```bash
incinerator
```

You can also explicitly request the TUI:

```bash
incinerator tui
incinerator --tui
```

### TUI Views

The dashboard has five tabs, switchable with `Tab` / `Shift+Tab`:

- **Summary** — grand totals and top models
- **Providers** — cost broken down by tool (Cline / Claude Code / Gemini CLI)
- **Daily Costs** — bar chart of recent daily spend
- **Daily Tokens** — stacked token breakdown per day
- **Settings** — toggle visual effects

### TUI Keybindings

| Key | Action |
|---|---|
| `Tab` / `→` | Next view |
| `Shift+Tab` / `←` | Previous view |
| `r` | Manual refresh |
| `Space` | Toggle heat-decay effect (Settings tab) |
| `q` / `Esc` | Quit |

The TUI auto-refreshes when new log entries appear. On first boot, an animated splash shows parsing progress.

### One-shot Text Reports

For a static, scriptable text report from a single tool:

```bash
incinerator cline    # Cline only
incinerator claude   # Claude Code only
incinerator gemini   # Gemini CLI only
```

### Options

- `--daily <N>`: show the last N days in the daily costs chart (default: 14).
- `--exclude-claude` / `--exclude-gemini`: (on the `cline` subcommand) filter out specific model families.

Example:
```bash
incinerator cline --daily 30
```

### Debugging

Set `INCINERATOR_DEBUG=1` to append per-scan timing breakdowns to `/tmp/incinerator-timings.log` — useful for diagnosing slow boots:

```bash
INCINERATOR_DEBUG=1 incinerator
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
