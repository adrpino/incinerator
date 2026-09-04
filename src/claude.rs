use chrono::DateTime;
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::time::Instant;
use walkdir::WalkDir;

use crate::colors::*;
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::pricing::get_pricing;
use crate::viz::{TokenStats, print_cost_bar, print_token_bar};

#[derive(Deserialize)]
struct ClaudeUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct ClaudeAssistantMessage {
    model: Option<String>,
    usage: Option<ClaudeUsage>,
    content: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ClaudeLogEntry {
    #[serde(rename = "type")]
    msg_type: Option<String>,
    timestamp: Option<String>,
    message: Option<ClaudeAssistantMessage>,
}

#[derive(Default, Clone)]
pub struct ClaudeStats {
    pub daily_stats: BTreeMap<String, TokenStats>,
    pub daily_costs: BTreeMap<String, f64>,
    pub monthly_stats: BTreeMap<String, TokenStats>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub model_stats: HashMap<String, TokenStats>,
    pub monthly_model_usage: BTreeMap<String, HashMap<String, TokenStats>>,
    pub total_messages: usize,
    pub sessions_found: usize,
    pub languages: crate::languages::LanguageAnalyzer,
}

fn merge_claude_stats(mut a: ClaudeStats, b: ClaudeStats) -> ClaudeStats {
    a.total_messages += b.total_messages;
    a.sessions_found += b.sessions_found;
    a.languages.merge(&b.languages);

    for (k, v) in b.daily_stats {
        a.daily_stats.entry(k).or_default().add(&v);
    }
    for (k, v) in b.daily_costs {
        *a.daily_costs.entry(k).or_insert(0.0) += v;
    }
    for (k, v) in b.monthly_stats {
        a.monthly_stats.entry(k).or_default().add(&v);
    }
    for (k, v) in b.monthly_costs {
        *a.monthly_costs.entry(k).or_insert(0.0) += v;
    }
    for (model, v) in b.model_stats {
        a.model_stats.entry(model).or_default().add(&v);
    }
    for (month, models) in b.monthly_model_usage {
        let a_models = a.monthly_model_usage.entry(month).or_default();
        for (model, v) in models {
            a_models.entry(model).or_default().add(&v);
        }
    }
    a
}

pub fn get_claude_storage_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".claude/projects"))
}

/// Inspect the `content` array of an assistant message and record any file
/// edits performed by Claude Code's built-in tools (`Write`, `Edit`,
/// `MultiEdit`, `NotebookEdit`). The byte count attributed to each edit is the
/// length of the freshly written text, never the file as a whole.
fn extract_file_edits(
    content: &serde_json::Value,
    languages: &mut crate::languages::LanguageAnalyzer,
) {
    let Some(arr) = content.as_array() else {
        return;
    };

    for item in arr {
        if item.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
            continue;
        }
        let Some(name) = item.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let Some(input) = item.get("input") else {
            continue;
        };

        match name {
            "Write" => {
                if let Some(path) = input.get("file_path").and_then(|p| p.as_str()) {
                    let len = input
                        .get("content")
                        .and_then(|c| c.as_str())
                        .map_or(0, |c| c.len());
                    languages.record_file_edit(path, len);
                }
            }
            "Edit" => {
                if let Some(path) = input.get("file_path").and_then(|p| p.as_str()) {
                    let len = input
                        .get("new_string")
                        .and_then(|c| c.as_str())
                        .map_or(0, |c| c.len());
                    languages.record_file_edit(path, len);
                }
            }
            "MultiEdit" => {
                let Some(path) = input.get("file_path").and_then(|p| p.as_str()) else {
                    continue;
                };
                let Some(edits) = input.get("edits").and_then(|e| e.as_array()) else {
                    continue;
                };
                for edit in edits {
                    let len = edit
                        .get("new_string")
                        .and_then(|c| c.as_str())
                        .map_or(0, |c| c.len());
                    languages.record_file_edit(path, len);
                }
            }
            "NotebookEdit" => {
                if let Some(path) = input.get("notebook_path").and_then(|p| p.as_str()) {
                    let len = input
                        .get("new_source")
                        .and_then(|c| c.as_str())
                        .map_or(0, |c| c.len());
                    languages.record_file_edit(path, len);
                }
            }
            _ => {}
        }
    }
}

pub fn parse_claude_file(file_path: &std::path::Path) -> ClaudeStats {
    let mut local = ClaudeStats::default();
    let file = match fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return local,
    };
    local.sessions_found = 1;

    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let entry = match serde_json::from_str::<ClaudeLogEntry>(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.msg_type.as_deref() != Some("assistant") {
            continue;
        }
        let assistant_msg = match entry.message {
            Some(m) => m,
            None => continue,
        };

        if let Some(content) = &assistant_msg.content {
            extract_file_edits(content, &mut local.languages);
        }

        let usage = match assistant_msg.usage {
            Some(u) => u,
            None => continue,
        };
        let model = assistant_msg.model.unwrap_or_else(|| "unknown".to_string());

        let (date_str, month_str) = match entry.timestamp {
            Some(ts) => match DateTime::parse_from_rfc3339(&ts.replace('Z', "+00:00")) {
                Ok(dt) => (
                    dt.format("%Y-%m-%d").to_string(),
                    dt.format("%Y-%m").to_string(),
                ),
                Err(_) => ("Unknown".to_string(), "Unknown".to_string()),
            },
            None => ("Unknown".to_string(), "Unknown".to_string()),
        };

        let in_tokens = usage.input_tokens.unwrap_or(0);
        let out_tokens = usage.output_tokens.unwrap_or(0);
        let cache_create = usage.cache_creation_input_tokens.unwrap_or(0);
        let cache_read = usage.cache_read_input_tokens.unwrap_or(0);

        if in_tokens + out_tokens + cache_create + cache_read == 0 {
            continue;
        }

        let pricing = get_pricing(&model, 0);
        let turn_cost = (in_tokens as f64 / 1_000_000.0 * pricing.input)
            + (out_tokens as f64 / 1_000_000.0 * pricing.output)
            + (cache_create as f64 / 1_000_000.0 * pricing.cache_write)
            + (cache_read as f64 / 1_000_000.0 * pricing.cache_read);

        let entry = TokenStats {
            in_tokens,
            out_tokens,
            cache_read_tokens: cache_read,
            cache_create_tokens: cache_create,
        };

        local
            .daily_stats
            .entry(date_str.clone())
            .or_default()
            .add(&entry);
        *local.daily_costs.entry(date_str).or_insert(0.0) += turn_cost;

        local
            .monthly_stats
            .entry(month_str.clone())
            .or_default()
            .add(&entry);
        *local.monthly_costs.entry(month_str.clone()).or_insert(0.0) += turn_cost;

        local
            .model_stats
            .entry(model.clone())
            .or_default()
            .add(&entry);
        local
            .monthly_model_usage
            .entry(month_str)
            .or_default()
            .entry(model)
            .or_default()
            .add(&entry);

        local.total_messages += 1;
    }
    local
}

pub fn get_claude_files() -> Vec<PathBuf> {
    let target_path = match get_claude_storage_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    if !target_path.is_dir() {
        return Vec::new();
    }
    WalkDir::new(&target_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
        .map(|e| e.path().to_path_buf())
        .collect()
}

pub fn run_claude_report() -> Option<(ClaudeStats, f64)> {
    let start_time = Instant::now();
    let session_files = get_claude_files();
    if session_files.is_empty() {
        return None;
    }

    let global_stats = session_files
        .par_iter()
        .map(|file_path| parse_claude_file(file_path))
        .reduce(ClaudeStats::default, merge_claude_stats);

    let parsing_time = start_time.elapsed().as_secs_f64();
    if global_stats.total_messages == 0 {
        return None;
    }
    Some((global_stats, parsing_time))
}

pub fn print_claude_report(global_stats: &ClaudeStats, parsing_time: f64, daily_days: usize) {
    println!("\n{}", "=".repeat(105));
    println!(
        "{}📊 CLAUDE CLI USAGE & COST ESTIMATE{}",
        TERM_HEADER, TERM_RESET
    );
    println!("{}", "=".repeat(105));
    println!(
        "{}Sessions Scanned:{} {}",
        TERM_BOLD, TERM_RESET, global_stats.sessions_found
    );
    println!(
        "{}Total Messages:{}   {}",
        TERM_BOLD,
        TERM_RESET,
        format_int_with_commas(global_stats.total_messages as i64)
    );
    println!("{}", "-".repeat(105));

    if !global_stats.model_stats.is_empty() {
        println!(
            "\n{}=== TOKEN USAGE (STACKED) ==={}",
            TERM_HEADER, TERM_RESET
        );
        println!(
            "Legend: {}█ Input{} | {}█ Output{} | {}▒ Cache Read{} | {}░ Cache Create{}",
            TERM_BLUE,
            TERM_RESET,
            TERM_GREEN,
            TERM_RESET,
            TERM_YELLOW,
            TERM_RESET,
            TERM_ORANGE,
            TERM_RESET
        );

        let max_model_len = global_stats
            .model_stats
            .keys()
            .map(|m| m.len())
            .max()
            .unwrap_or(20)
            .min(30);

        println!(
            "\n{}--- Overall Usage by Model ---{}",
            TERM_BOLD, TERM_RESET
        );
        let all_max_tokens = global_stats
            .model_stats
            .values()
            .map(|s| s.total())
            .max()
            .unwrap_or(0);
        let mut sorted_models: Vec<_> = global_stats.model_stats.iter().collect();
        sorted_models.sort_by_key(|b| std::cmp::Reverse(b.1.total()));
        for (model, stats) in sorted_models {
            print_token_bar(
                &format!(
                    "{:<width$}",
                    model.get(..30).unwrap_or(model),
                    width = max_model_len
                ),
                stats,
                all_max_tokens,
                35,
                true,
            );
        }

        println!(
            "\n{}--- Monthly Breakdown by Model ---{}",
            TERM_BOLD, TERM_RESET
        );
        for (month, models) in global_stats.monthly_model_usage.iter().rev() {
            if month == "Unknown" {
                continue;
            }
            println!("\n{}{}{}", TERM_CYAN, month, TERM_RESET);
            let month_max = models.values().map(|s| s.total()).max().unwrap_or(1);
            let mut sorted_m: Vec<_> = models.iter().collect();
            sorted_m.sort_by_key(|b| std::cmp::Reverse(b.1.total()));
            for (model, stats) in sorted_m {
                print_token_bar(
                    &format!(
                        "  {:<width$}",
                        model.get(..30).unwrap_or(model),
                        width = max_model_len
                    ),
                    stats,
                    month_max,
                    35,
                    true,
                );
            }
        }
    }

    println!("\n{}=== FINANCIAL COSTS ==={}", TERM_HEADER, TERM_RESET);

    if !global_stats.monthly_costs.is_empty() {
        println!("\n{}--- Monthly Costs ---{}", TERM_BOLD, TERM_RESET);
        let max_month_cost = global_stats
            .monthly_costs
            .values()
            .copied()
            .fold(0.0_f64, |a, b| a.max(b));
        for (month, cost) in global_stats.monthly_costs.iter().rev() {
            if month == "Unknown" {
                continue;
            }
            print_cost_bar(&format!("{:^12}", month), *cost, max_month_cost, 35);
        }
    }

    if !global_stats.daily_costs.is_empty() {
        println!(
            "\n{}--- Daily Costs (Last {} days) ---{}",
            TERM_BOLD, daily_days, TERM_RESET
        );
        let max_day_cost = global_stats
            .daily_costs
            .values()
            .copied()
            .fold(0.0_f64, |a, b| a.max(b));
        let mut sorted_days: Vec<_> = global_stats.daily_costs.iter().collect();
        sorted_days.sort_by_key(|a| a.0);
        for (day, cost) in sorted_days.into_iter().rev().take(daily_days) {
            if day == "Unknown" {
                continue;
            }
            print_cost_bar(&format!("{:<12}", day), *cost, max_day_cost, 35);
        }
    }

    let total_cost: f64 = global_stats.monthly_costs.values().sum();
    let mut total_tokens = TokenStats::default();
    for s in global_stats.model_stats.values() {
        total_tokens.add(s);
    }

    println!("\n{}", "=".repeat(50));
    println!("{}GRAND TOTALS (CLAUDE CLI){}", TERM_HEADER, TERM_RESET);
    println!("{}", "-".repeat(50));
    println!("{}Tokens:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {}Input:        {:>12}{}",
        TERM_BLUE,
        format_int_with_commas(total_tokens.in_tokens),
        TERM_RESET
    );
    println!(
        "  {}Output:       {:>12}{}",
        TERM_GREEN,
        format_int_with_commas(total_tokens.out_tokens),
        TERM_RESET
    );
    println!(
        "  {}Cache Read:   {:>12}{}",
        TERM_YELLOW,
        format_int_with_commas(total_tokens.cache_read_tokens),
        TERM_RESET
    );
    println!(
        "  {}Cache Create: {:>12}{}",
        TERM_ORANGE,
        format_int_with_commas(total_tokens.cache_create_tokens),
        TERM_RESET
    );
    println!(
        "  {}Total:        {:>12}{}",
        TERM_BOLD,
        format_int_with_commas(total_tokens.total()),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Cost:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {}${}{}",
        TERM_RED,
        format_float_with_commas(total_cost),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Performance:{}", TERM_BOLD, TERM_RESET);
    println!("  Sessions Parsed: {}", global_stats.sessions_found);
    println!("  Parse Time:      {:.2} seconds", parsing_time);
    println!("{}", "=".repeat(50));

    if !global_stats.languages.stats.is_empty() {
        println!("\n{}=== LANGUAGES ==={}", TERM_HEADER, TERM_RESET);
        let mut sorted_langs: Vec<_> = global_stats.languages.stats.iter().collect();
        sorted_langs.sort_by_key(|b| std::cmp::Reverse(b.1.bytes));
        for (lang, stat) in sorted_langs {
            println!(
                "  {:<14} {:>5} occurrences ({:>8} bytes)",
                lang, stat.occurrences, stat.bytes
            );
        }
        println!("{}", "=".repeat(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::LanguageAnalyzer;
    use crate::pricing::get_claude_pricing;
    use serde_json::json;

    #[test]
    fn test_claude_pricing() {
        let pricing = get_claude_pricing("claude-sonnet-4-6-20251001");
        assert_eq!(pricing.input, 3.00);
        assert_eq!(pricing.output, 15.00);
        assert_eq!(pricing.cache_write, 3.75);
        assert_eq!(pricing.cache_read, 0.30);

        let pricing = get_claude_pricing("claude-opus-4-7");
        assert_eq!(pricing.input, 5.00);
        assert_eq!(pricing.output, 25.00);

        let pricing = get_claude_pricing("claude-opus-5");
        assert_eq!(pricing.input, 5.00);
        assert_eq!(pricing.output, 25.00);
        assert_eq!(pricing.cache_write, 6.25);
        assert_eq!(pricing.cache_read, 0.50);

        let pricing = get_claude_pricing("claude-fable-5.1");
        assert_eq!(pricing.input, 10.00);
        assert_eq!(pricing.output, 50.00);
        assert_eq!(pricing.cache_write, 12.50);
        assert_eq!(pricing.cache_read, 0.25);

        let pricing = get_claude_pricing("claude-haiku-4-5-20251001");
        assert_eq!(pricing.input, 1.00);
        assert_eq!(pricing.output, 5.00);

        // fallback
        let pricing = get_claude_pricing("some-unknown-model");
        assert_eq!(pricing.input, 3.00);
    }

    fn run_extract(content: serde_json::Value) -> LanguageAnalyzer {
        let mut analyzer = LanguageAnalyzer::new();
        extract_file_edits(&content, &mut analyzer);
        analyzer
    }

    #[test]
    fn extract_write_tool_records_content_length() {
        let content = json!([
            {
                "type": "tool_use",
                "name": "Write",
                "input": {
                    "file_path": "src/lib.rs",
                    "content": "fn hello() {}\n"
                }
            }
        ]);
        let analyzer = run_extract(content);
        let rust = analyzer.stats.get("Rust").expect("rust recorded");
        assert_eq!(rust.occurrences, 1);
        assert_eq!(rust.bytes, "fn hello() {}\n".len());
    }

    #[test]
    fn extract_edit_tool_uses_new_string_for_bytes() {
        let new_string = "replacement";
        let content = json!([
            {
                "type": "tool_use",
                "name": "Edit",
                "input": {
                    "file_path": "scripts/run.sh",
                    "old_string": "echo old",
                    "new_string": new_string,
                    "replace_all": false
                }
            }
        ]);
        let analyzer = run_extract(content);
        let shell = analyzer.stats.get("Shell").expect("shell recorded");
        assert_eq!(shell.occurrences, 1);
        assert_eq!(shell.bytes, new_string.len());
    }

    #[test]
    fn extract_multi_edit_records_each_edit() {
        let content = json!([
            {
                "type": "tool_use",
                "name": "MultiEdit",
                "input": {
                    "file_path": "src/app.py",
                    "edits": [
                        {"old_string": "a", "new_string": "xx"},
                        {"old_string": "b", "new_string": "yyyy"},
                        {"old_string": "c", "new_string": ""}
                    ]
                }
            }
        ]);
        let analyzer = run_extract(content);
        let py = analyzer.stats.get("Python").expect("python recorded");
        assert_eq!(py.occurrences, 3);
        assert_eq!(py.bytes, 2 + 4);
    }

    #[test]
    fn extract_notebook_edit_uses_new_source() {
        let content = json!([
            {
                "type": "tool_use",
                "name": "NotebookEdit",
                "input": {
                    "notebook_path": "analysis.ipynb",
                    "new_source": "print('hi')"
                }
            }
        ]);
        let analyzer = run_extract(content);
        // .ipynb is not in the extension map → ignored (unknown language)
        assert!(analyzer.stats.is_empty());
    }

    #[test]
    fn extract_skips_non_tool_use_items() {
        let content = json!([
            {"type": "text", "text": "some assistant prose"},
            {
                "type": "tool_use",
                "name": "Bash",
                "input": {"command": "ls"}
            },
            {
                "type": "tool_use",
                "name": "Read",
                "input": {"file_path": "src/main.rs"}
            }
        ]);
        let analyzer = run_extract(content);
        assert!(
            analyzer.stats.is_empty(),
            "Read/Bash/text should not record file edits"
        );
    }

    #[test]
    fn extract_handles_missing_fields() {
        // Tool calls in flight (no file_path / no content yet) must not panic
        // and must not record bogus edits.
        let content = json!([
            {"type": "tool_use", "name": "Write", "input": {}},
            {"type": "tool_use", "name": "Edit", "input": {"file_path": "x.rs"}},
            {"type": "tool_use", "name": "MultiEdit", "input": {"file_path": "x.rs"}},
            {"type": "tool_use", "name": "Write", "input": {"file_path": "x.rs"}}
        ]);
        let analyzer = run_extract(content);
        // The last Write has a path but no content → record as 0 bytes occurrence.
        // Edit with no new_string → record as 0 bytes occurrence.
        let rust = analyzer.stats.get("Rust").expect("rust recorded");
        assert_eq!(rust.occurrences, 2);
        assert_eq!(rust.bytes, 0);
    }

    #[test]
    fn extract_handles_non_array_content() {
        // Some assistant messages carry content as a plain string. Should not
        // panic and should record nothing.
        let analyzer = run_extract(json!("just a string"));
        assert!(analyzer.stats.is_empty());

        let analyzer = run_extract(json!(null));
        assert!(analyzer.stats.is_empty());
    }

    #[test]
    fn extract_handles_multiple_languages_in_one_message() {
        let content = json!([
            {"type": "tool_use", "name": "Write", "input": {"file_path": "a.py", "content": "x"}},
            {"type": "tool_use", "name": "Edit", "input": {"file_path": "b.ts", "new_string": "yy"}},
            {"type": "tool_use", "name": "Write", "input": {"file_path": "Dockerfile", "content": "FROM scratch"}}
        ]);
        let analyzer = run_extract(content);
        assert_eq!(analyzer.stats.get("Python").unwrap().occurrences, 1);
        assert_eq!(analyzer.stats.get("TypeScript").unwrap().occurrences, 1);
        assert_eq!(analyzer.stats.get("Dockerfile").unwrap().occurrences, 1);
    }

    #[test]
    fn parse_claude_file_end_to_end() {
        use std::io::Write;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");
        let mut f = std::fs::File::create(&path).expect("create");

        // assistant message with a Write tool call and a usage block
        let line1 = json!({
            "type": "assistant",
            "timestamp": "2026-05-19T10:00:00Z",
            "message": {
                "model": "claude-sonnet-4-6-20251001",
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 50,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 10
                },
                "content": [
                    {"type": "tool_use", "name": "Write", "input": {
                        "file_path": "src/main.rs",
                        "content": "fn main() {}\n"
                    }}
                ]
            }
        });
        // user message — must be skipped
        let line2 = json!({"type": "user", "message": {"content": "hi"}});
        // malformed line — must be skipped without panicking
        let line3 = "{not json";
        // assistant with usage but no tool calls — should still record tokens
        let line4 = json!({
            "type": "assistant",
            "timestamp": "2026-05-19T11:00:00Z",
            "message": {
                "model": "claude-sonnet-4-6-20251001",
                "usage": {"input_tokens": 5, "output_tokens": 5},
                "content": [{"type": "text", "text": "ok"}]
            }
        });

        for line in [
            line1.to_string(),
            line2.to_string(),
            line3.to_string(),
            line4.to_string(),
        ] {
            writeln!(f, "{}", line).unwrap();
        }
        drop(f);

        let stats = parse_claude_file(&path);
        assert_eq!(stats.sessions_found, 1);
        assert_eq!(stats.total_messages, 2);

        let rust = stats.languages.stats.get("Rust").expect("rust recorded");
        assert_eq!(rust.occurrences, 1);
        assert_eq!(rust.bytes, "fn main() {}\n".len());

        // Tokens & costs were aggregated under the right day/month buckets.
        assert!(stats.daily_stats.contains_key("2026-05-19"));
        assert!(stats.monthly_stats.contains_key("2026-05"));
        let day = stats.daily_stats.get("2026-05-19").unwrap();
        assert_eq!(day.in_tokens, 105);
        assert_eq!(day.out_tokens, 55);
    }
}
