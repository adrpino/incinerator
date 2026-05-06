# 🔥 Incinerator

A CLI tool to track how much money you're "burning" through tokens across **Cline**, **Claude Code**, and **Gemini CLI**.

Incinerator parses your local logs from various AI CLI tools to provide a unified, colorized view of your token usage and financial costs.

## Features

- **Unified Reporting**: Get a single view of your costs across multiple AI tools.
- **Stacked Token Visualizations**: See a breakdown of Input, Output, Cache Read, and Cache Create tokens.
- **Historical Analysis**: View daily costs for the last 14 days (configurable) and monthly summaries.
- **Model Breakdown**: Identify which models are consuming the most tokens.
- **Fast and Efficient**: Built in Rust with parallel processing for quick log parsing.

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

Run `incinerator` without any arguments for a unified report:

```bash
incinerator
```

### Subcommands

Analyze specific tool logs:

```bash
incinerator cline   # Analyze Cline logs
incinerator claude  # Analyze Claude Code logs
incinerator gemini  # Analyze Gemini CLI logs
```

### Options

- `--daily <N>`: Show the last N days in the daily costs chart (default: 14).

Example:
```bash
incinerator --daily 30
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
