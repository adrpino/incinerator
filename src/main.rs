mod claude;
mod cline;
mod colors;
mod format;
mod gemini;
mod unified;
mod viz;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "incinerator")]
#[command(
    about = "🔥 Track how much money you're burning through tokens across Cline, Claude Code, and Gemini CLI",
    long_about = "Incinerator parses your local logs from various AI CLI tools to provide a unified view of token usage and financial costs."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Number of days to show in the daily costs chart (default: 14)
    #[arg(long)]
    daily: Option<usize>,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze Cline conversation logs
    Cline {
        /// Exclude Anthropic Claude data
        #[arg(long)]
        exclude_claude: bool,
        /// Exclude Google Gemini data
        #[arg(long)]
        exclude_gemini: bool,
        /// Number of days to show in the daily costs chart
        #[arg(long)]
        daily: Option<usize>,
    },
    /// Analyze Claude Code session logs (~/.claude/projects)
    Claude {
        /// Number of days to show in the daily costs chart
        #[arg(long)]
        daily: Option<usize>,
    },
    /// Analyze Gemini CLI usage logs
    Gemini {
        /// Number of days to show in the daily costs chart
        #[arg(long)]
        daily: Option<usize>,
    },
}

fn main() {
    let cli = Cli::parse();
    let default_daily = cli.daily.unwrap_or(14);

    match cli.command {
        Some(Commands::Cline { exclude_claude, exclude_gemini, daily }) => {
            let days = daily.unwrap_or(default_daily);
            if let Some((stats, time)) = cline::run_cline_report(exclude_claude, exclude_gemini) {
                cline::print_cline_report(&stats, time, days);
            }
        }
        Some(Commands::Claude { daily }) => {
            let days = daily.unwrap_or(default_daily);
            if let Some((stats, time)) = claude::run_claude_report() {
                claude::print_claude_report(&stats, time, days);
            }
        }
        Some(Commands::Gemini { daily }) => {
            let days = daily.unwrap_or(default_daily);
            if let Some((stats, time)) = gemini::run_gemini_report() {
                gemini::print_gemini_report(&stats, time, days);
            }
        }
        None => {
            unified::run_unified_report(default_daily);
        }
    }
}
