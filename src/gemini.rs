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
struct GeminiTokens {
    input: Option<i64>,
    output: Option<i64>,
    cached: Option<i64>,
}

#[derive(Deserialize)]
struct GeminiMessage {
    #[serde(rename = "type")]
    msg_type: Option<String>,
    timestamp: Option<String>,
    model: Option<String>,
    tokens: Option<GeminiTokens>,
    content: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct GeminiSession {
    messages: Option<Vec<GeminiMessage>>,
}

#[derive(Default, Clone)]
pub struct GeminiStats {
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

fn estimate_tokens(content: &Option<serde_json::Value>) -> i64 {
    let mut text = String::new();
    if let Some(val) = content {
        if let Some(s) = val.as_str() {
            text = s.to_string();
        } else if let Some(arr) = val.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    text.push_str(s);
                } else if let Some(obj) = item.as_object() {
                    if let Some(s) = obj.get("text").and_then(|v| v.as_str()) {
                        text.push_str(s);
                    }
                }
            }
        }
    }
    if text.is_empty() {
        return 0;
    }
    (text.split_whitespace().count() as f64 * 1.33) as i64
}

fn merge_gemini_stats(mut a: GeminiStats, b: GeminiStats) -> GeminiStats {
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

pub fn get_gemini_storage_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".gemini/tmp"))
}

/// Parse the first-line preamble of a Gemini `tool-outputs/write_file_N.txt`
/// or `replace_N.txt` file and return `(absolute_path, body_byte_len)`.
///
/// Gemini's tool-output files are not JSON; they begin with a fixed sentence
/// describing what was written, followed by the file contents. We use the
/// declared path for language attribution and the length of the trailing body
/// as a byte count. Error outputs (which start with `{`) and any output we
/// cannot recognise return `None`.
pub fn parse_gemini_tool_output(text: &str) -> Option<(&str, usize)> {
    if let Some(rest) = text.strip_prefix("Successfully created and wrote to new file: ") {
        let suffix = ". Here is the updated code:";
        let end = rest.find(suffix)?;
        let path = &rest[..end];
        let body = rest[end + suffix.len()..].trim_start_matches('\n');
        return Some((path, body.len()));
    }
    if let Some(rest) = text.strip_prefix("Successfully modified file: ") {
        let count_marker = rest.find(" (");
        let here_marker = rest.find(". Here is the updated code:");
        let path_end = match (count_marker, here_marker) {
            (Some(c), Some(h)) => c.min(h),
            (Some(c), None) => c,
            (None, Some(h)) => h,
            (None, None) => return None,
        };
        let path = &rest[..path_end];
        let body = match here_marker {
            Some(h) => {
                let body_start = h + ". Here is the updated code:".len();
                rest[body_start..].trim_start_matches('\n')
            }
            None => "",
        };
        return Some((path, body.len()));
    }
    None
}

/// True if the basename of the file matches `<tool>_<N>.txt` for tools that
/// modify files (`write_file`, `replace`).
fn is_gemini_edit_tool_output(name: &str) -> bool {
    let stem = name.strip_suffix(".txt").unwrap_or(name);
    let (tool, num) = match stem.rsplit_once('_') {
        Some(pair) => pair,
        None => return false,
    };
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    matches!(tool, "write_file" | "replace")
}

pub fn parse_gemini_tool_output_file(
    file_path: &std::path::Path,
) -> crate::languages::LanguageAnalyzer {
    let mut analyzer = crate::languages::LanguageAnalyzer::new();
    let text = match fs::read_to_string(file_path) {
        Ok(t) => t,
        Err(_) => return analyzer,
    };
    if let Some((path, len)) = parse_gemini_tool_output(&text) {
        analyzer.record_file_edit(path, len);
    }
    analyzer
}

pub fn get_gemini_tool_output_files() -> Vec<PathBuf> {
    let base = match get_gemini_storage_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    if !base.exists() {
        return Vec::new();
    }
    WalkDir::new(&base)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            // require parent dir to be named "tool-outputs"
            let in_tool_outputs = e
                .path()
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == "tool-outputs");
            in_tool_outputs && is_gemini_edit_tool_output(&e.file_name().to_string_lossy())
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

pub fn parse_gemini_file(file_path: &std::path::Path) -> GeminiStats {
    let mut local = GeminiStats::default();
    let mut messages = Vec::new();

    if file_path.to_string_lossy().ends_with(".jsonl") {
        if let Ok(file) = fs::File::open(file_path) {
            let reader = BufReader::new(file);
            for l in reader.lines().map_while(Result::ok) {
                if let Ok(msg) = serde_json::from_str::<GeminiMessage>(&l) {
                    if msg.msg_type.is_some() && msg.timestamp.is_some() {
                        messages.push(msg);
                    }
                }
            }
        }
    } else if let Ok(file) = fs::File::open(file_path) {
        let reader = BufReader::new(file);
        if let Ok(session) = serde_json::from_reader::<_, GeminiSession>(reader) {
            if let Some(msgs) = session.messages {
                messages = msgs;
            }
        }
    }

    if messages.is_empty() {
        return local;
    }
    local.sessions_found = 1;

    let mut session_model = "unknown".to_string();
    for m in &messages {
        if let Some(mod_id) = &m.model {
            session_model = mod_id.clone();
            break;
        }
    }

    for msg in messages {
        // File edits are not recorded in session JSONL files — Gemini stores
        // tool invocations as plain-text files under a sibling `tool-outputs/`
        // directory. Those are walked separately in `run_gemini_report`.
        let msg_type = msg.msg_type.as_deref().unwrap_or("unknown");
        let msg_model = msg.model.clone().unwrap_or_else(|| session_model.clone());

        let (date_str, month_str) = if let Some(ts_str) = &msg.timestamp {
            if let Ok(dt) = DateTime::parse_from_rfc3339(&ts_str.replace('Z', "+00:00")) {
                (
                    dt.format("%Y-%m-%d").to_string(),
                    dt.format("%Y-%m").to_string(),
                )
            } else {
                ("Unknown".to_string(), "Unknown".to_string())
            }
        } else {
            ("Unknown".to_string(), "Unknown".to_string())
        };

        let mut in_tokens = 0;
        let mut out_tokens = 0;
        let mut cache_tokens = 0;

        if let Some(t) = msg.tokens {
            in_tokens = t.input.unwrap_or(0);
            out_tokens = t.output.unwrap_or(0);
            cache_tokens = t.cached.unwrap_or(0);
        }

        if in_tokens == 0 && out_tokens == 0 && cache_tokens == 0 {
            let est = estimate_tokens(&msg.content);
            if msg_type == "user" {
                in_tokens = est;
            } else if msg_type == "gemini" || msg_type == "model" {
                out_tokens = est;
            }
        }

        let total_context = in_tokens + cache_tokens;
        let pricing = get_pricing(&msg_model, total_context);
        let turn_cost = (in_tokens as f64 / 1_000_000.0 * pricing.input)
            + (out_tokens as f64 / 1_000_000.0 * pricing.output)
            + (cache_tokens as f64 / 1_000_000.0 * pricing.cache_write);

        let entry = TokenStats {
            in_tokens,
            out_tokens,
            cache_read_tokens: cache_tokens,
            cache_create_tokens: 0,
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
            .entry(msg_model.clone())
            .or_default()
            .add(&entry);
        local
            .monthly_model_usage
            .entry(month_str)
            .or_default()
            .entry(msg_model)
            .or_default()
            .add(&entry);

        local.total_messages += 1;
    }
    local
}

pub fn get_gemini_files() -> Vec<PathBuf> {
    let target_path = match get_gemini_storage_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    if !target_path.exists() {
        return Vec::new();
    }
    WalkDir::new(&target_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name().to_string_lossy().starts_with("session-")
                && e.file_name().to_string_lossy().contains(".json")
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

pub fn run_gemini_report() -> Option<(GeminiStats, f64)> {
    let start_time = Instant::now();
    let session_files = get_gemini_files();
    if session_files.is_empty() {
        return None;
    }

    let mut global_stats = session_files
        .par_iter()
        .map(|file_path| parse_gemini_file(file_path))
        .reduce(GeminiStats::default, merge_gemini_stats);

    // Walk sibling tool-outputs/ files to recover language data, which the
    // session JSONL does not carry.
    let tool_output_files = get_gemini_tool_output_files();
    if !tool_output_files.is_empty() {
        let languages = tool_output_files
            .par_iter()
            .map(|p| parse_gemini_tool_output_file(p))
            .reduce(crate::languages::LanguageAnalyzer::new, |mut a, b| {
                a.merge(&b);
                a
            });
        global_stats.languages.merge(&languages);
    }

    let parsing_time = start_time.elapsed().as_secs_f64();
    if global_stats.total_messages == 0 {
        return None;
    }
    Some((global_stats, parsing_time))
}

pub fn print_gemini_report(global_stats: &GeminiStats, parsing_time: f64, daily_days: usize) {
    println!("\n{}", "=".repeat(95));
    println!(
        "{}📊 GEMINI CLI USAGE & COST ESTIMATE{}",
        TERM_HEADER, TERM_RESET
    );
    println!("{}", "=".repeat(95));
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
    println!("{}", "-".repeat(95));

    if !global_stats.model_stats.is_empty() {
        println!(
            "\n{}=== TOKEN USAGE (STACKED) ==={}",
            TERM_HEADER, TERM_RESET
        );
        println!(
            "Legend: {}█ Input{} | {}█ Output{} | {}▒ Cache Read{}",
            TERM_BLUE, TERM_RESET, TERM_GREEN, TERM_RESET, TERM_YELLOW, TERM_RESET
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
                false,
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
                    false,
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
    println!("{}GRAND TOTALS (GEMINI CLI){}", TERM_HEADER, TERM_RESET);
    println!("{}", "-".repeat(50));
    println!("{}Tokens:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {}Input:       {:>12}{}",
        TERM_BLUE,
        format_int_with_commas(total_tokens.in_tokens),
        TERM_RESET
    );
    println!(
        "  {}Output:      {:>12}{}",
        TERM_GREEN,
        format_int_with_commas(total_tokens.out_tokens),
        TERM_RESET
    );
    println!(
        "  {}Cache:       {:>12}{}",
        TERM_YELLOW,
        format_int_with_commas(total_tokens.cache_read_tokens),
        TERM_RESET
    );
    println!(
        "  {}Total:       {:>12}{}",
        TERM_BOLD,
        format_int_with_commas(total_tokens.total()),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Cost:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {} ${}{}",
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
    use crate::pricing::get_gemini_pricing;

    #[test]
    fn parses_write_file_preamble() {
        let text = "Successfully created and wrote to new file: /tmp/example.rs. Here is the updated code:\nfn main() {}\n";
        let (path, len) = parse_gemini_tool_output(text).expect("parsed");
        assert_eq!(path, "/tmp/example.rs");
        assert_eq!(len, "fn main() {}\n".len());
    }

    #[test]
    fn parses_replace_preamble_with_count() {
        let text = "Successfully modified file: /tmp/x.py (3 replacements). Here is the updated code:\nbody\n";
        let (path, len) = parse_gemini_tool_output(text).expect("parsed");
        assert_eq!(path, "/tmp/x.py");
        assert_eq!(len, "body\n".len());
    }

    #[test]
    fn parses_replace_preamble_without_here_block() {
        // Older / shorter form: no trailing "Here is the updated code:" body
        let text = "Successfully modified file: /tmp/y.ts (1 replacements).";
        let (path, len) = parse_gemini_tool_output(text).expect("parsed");
        assert_eq!(path, "/tmp/y.ts");
        assert_eq!(len, 0);
    }

    #[test]
    fn rejects_error_output() {
        let text = "{\n  \"error\": \"Failed to edit, 0 occurrences found for old_string in src/x.rs.\"\n}";
        assert!(parse_gemini_tool_output(text).is_none());
    }

    #[test]
    fn rejects_unknown_preamble() {
        assert!(parse_gemini_tool_output("nope").is_none());
        assert!(parse_gemini_tool_output("").is_none());
    }

    #[test]
    fn filename_matcher_only_accepts_edit_tools() {
        assert!(is_gemini_edit_tool_output("write_file_0.txt"));
        assert!(is_gemini_edit_tool_output("write_file_42.txt"));
        assert!(is_gemini_edit_tool_output("replace_3.txt"));
        assert!(!is_gemini_edit_tool_output("read_file_3.txt"));
        assert!(!is_gemini_edit_tool_output("google_web_search_2.txt"));
        assert!(!is_gemini_edit_tool_output("run_shell_command_1.txt"));
        assert!(!is_gemini_edit_tool_output("write_file.txt"));
        assert!(!is_gemini_edit_tool_output("write_file_.txt"));
        assert!(!is_gemini_edit_tool_output("replace_abc.txt"));
    }

    #[test]
    fn parse_file_records_language_and_bytes() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("write_file_0.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(
            f,
            "Successfully created and wrote to new file: /tmp/demo.py. Here is the updated code:\nprint('hi')\n"
        )
        .unwrap();
        drop(f);

        let analyzer = parse_gemini_tool_output_file(&path);
        let py = analyzer.stats.get("Python").expect("python recorded");
        assert_eq!(py.occurrences, 1);
        assert_eq!(py.bytes, "print('hi')\n".len());
    }

    #[test]
    fn parse_file_ignores_error_output() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replace_0.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{{\"error\": \"User denied execution.\"}}").unwrap();
        drop(f);

        let analyzer = parse_gemini_tool_output_file(&path);
        assert!(analyzer.stats.is_empty());
    }

    #[test]
    fn parse_file_handles_missing_file_gracefully() {
        let analyzer =
            parse_gemini_tool_output_file(std::path::Path::new("/does/not/exist/write_file_0.txt"));
        assert!(analyzer.stats.is_empty());
    }

    #[test]
    fn walk_picks_up_only_edit_tool_outputs_under_tool_outputs_dir() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("some-project");
        let tool_outputs = project.join("tool-outputs");
        std::fs::create_dir_all(&tool_outputs).unwrap();

        // Edit-tool outputs: should be picked up.
        let edit_paths = [
            (
                "write_file_0.txt",
                "Successfully created and wrote to new file: /tmp/a.rs. Here is the updated code:\nfn a() {}\n",
            ),
            (
                "replace_1.txt",
                "Successfully modified file: /tmp/b.js (1 replacements). Here is the updated code:\nconst b=1;\n",
            ),
        ];
        for (name, body) in &edit_paths {
            let mut f = std::fs::File::create(tool_outputs.join(name)).unwrap();
            write!(f, "{}", body).unwrap();
        }

        // Non-edit-tool outputs: should be ignored.
        let other_paths = [
            "read_file_0.txt",
            "google_web_search_2.txt",
            "run_shell_command_1.txt",
        ];
        for name in &other_paths {
            let mut f = std::fs::File::create(tool_outputs.join(name)).unwrap();
            write!(f, "irrelevant").unwrap();
        }

        // A write_file outside tool-outputs/ must NOT be picked up.
        let mut f = std::fs::File::create(project.join("write_file_0.txt")).unwrap();
        write!(f, "noise").unwrap();

        // Run the same filtering logic the production walker uses.
        let found: Vec<_> = walkdir::WalkDir::new(dir.path())
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let in_dir = e
                    .path()
                    .parent()
                    .and_then(|p| p.file_name())
                    .is_some_and(|n| n == "tool-outputs");
                in_dir && is_gemini_edit_tool_output(&e.file_name().to_string_lossy())
            })
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|f| f == "write_file_0.txt"));
        assert!(found.iter().any(|f| f == "replace_1.txt"));

        // Parsing the picked-up files yields the right languages.
        let analyzer = walkdir::WalkDir::new(dir.path())
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let in_dir = e
                    .path()
                    .parent()
                    .and_then(|p| p.file_name())
                    .is_some_and(|n| n == "tool-outputs");
                in_dir && is_gemini_edit_tool_output(&e.file_name().to_string_lossy())
            })
            .map(|e| parse_gemini_tool_output_file(e.path()))
            .fold(crate::languages::LanguageAnalyzer::new(), |mut a, b| {
                a.merge(&b);
                a
            });
        assert!(analyzer.stats.contains_key("Rust"));
        assert!(analyzer.stats.contains_key("JavaScript"));
    }

    #[test]
    fn parse_gemini_file_aggregates_session_jsonl() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session-2026-05-19T10-00-test.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "{{\"sessionId\":\"x\",\"projectHash\":\"y\",\"startTime\":\"2026-05-19T10:00:00Z\",\"lastUpdated\":\"2026-05-19T10:01:00Z\",\"kind\":\"test\"}}").unwrap();
        // Empty content on the user message so the token-estimation fallback
        // doesn't contribute — keeps this assertion focused on the explicit
        // `tokens` block.
        writeln!(
            f,
            "{{\"id\":0,\"type\":\"user\",\"timestamp\":\"2026-05-19T10:00:00Z\",\"content\":[]}}"
        )
        .unwrap();
        writeln!(f, "{{\"id\":1,\"type\":\"gemini\",\"model\":\"gemini-3-flash\",\"timestamp\":\"2026-05-19T10:00:01Z\",\"content\":[{{\"text\":\"hi back\"}}],\"tokens\":{{\"input\":5,\"output\":10,\"cached\":0}}}}").unwrap();
        drop(f);

        let stats = parse_gemini_file(&path);
        assert_eq!(stats.sessions_found, 1);
        assert_eq!(stats.total_messages, 2);
        // tokens recorded from explicit `tokens` block on the gemini message
        let day = stats.daily_stats.get("2026-05-19").expect("day bucket");
        assert_eq!(day.in_tokens, 5);
        assert_eq!(day.out_tokens, 10);
        // languages are NOT populated from session JSONL — that's tool-outputs' job.
        assert!(stats.languages.stats.is_empty());
    }

    #[test]
    fn test_gemini_pricing() {
        let pricing = get_gemini_pricing("gemini-3.1-flash-lite-preview", 0);
        assert_eq!(pricing.input, 0.25);
        assert_eq!(pricing.output, 1.50);
        assert_eq!(pricing.cache_write, 0.025);

        let pricing = get_gemini_pricing("gemini-3.1-pro", 100_000);
        assert_eq!(pricing.input, 2.00);
        assert_eq!(pricing.output, 12.00);
        assert_eq!(pricing.cache_write, 0.20);

        let pricing = get_gemini_pricing("gemini-3.1-pro", 300_000);
        assert_eq!(pricing.input, 4.00);
        assert_eq!(pricing.output, 18.00);
        assert_eq!(pricing.cache_write, 0.40);

        let pricing = get_gemini_pricing("gemini-3-flash", 0);
        assert_eq!(pricing.input, 0.50);
        assert_eq!(pricing.output, 3.00);
        assert_eq!(pricing.cache_write, 0.05);

        let pricing = get_gemini_pricing("unknown-model", 0);
        assert_eq!(pricing.input, 1.00);
        assert_eq!(pricing.output, 4.00);
        assert_eq!(pricing.cache_write, 0.10);
    }
}
