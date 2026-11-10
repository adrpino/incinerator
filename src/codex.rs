use chrono::{DateTime, Local};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;
use walkdir::WalkDir;

use crate::colors::*;
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::languages::LanguageAnalyzer;
use crate::pricing::get_pricing;
use crate::viz::{TokenStats, print_cost_bar, print_token_bar};

#[derive(Default, Clone)]
pub struct CodexStats {
    pub daily_costs: BTreeMap<String, f64>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub total_cost: f64,
    pub daily_stats: BTreeMap<String, TokenStats>,
    pub monthly_stats: BTreeMap<String, TokenStats>,
    pub model_stats: HashMap<String, TokenStats>,
    pub total_messages: usize,
    pub sessions_found: usize,
    pub languages: LanguageAnalyzer,
}

#[derive(Default, Clone)]
pub struct CodexCacheState {
    pub offset: u64,
    pub current_model: String,
    pub last_thread_total_tokens: i64,
}

impl CodexStats {
    pub fn merge(&mut self, other: &CodexStats) {
        self.total_messages += other.total_messages;
        self.sessions_found += other.sessions_found;
        self.total_cost += other.total_cost;
        for (k, v) in &other.daily_costs {
            *self.daily_costs.entry(k.clone()).or_insert(0.0) += v;
        }
        for (k, v) in &other.monthly_costs {
            *self.monthly_costs.entry(k.clone()).or_insert(0.0) += v;
        }
        for (k, v) in &other.daily_stats {
            self.daily_stats.entry(k.clone()).or_default().add(v);
        }
        for (k, v) in &other.monthly_stats {
            self.monthly_stats.entry(k.clone()).or_default().add(v);
        }
        for (k, v) in &other.model_stats {
            self.model_stats.entry(k.clone()).or_default().add(v);
        }
        self.languages.merge(&other.languages);
    }
}

pub fn get_codex_storage_path() -> Option<PathBuf> {
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        let path = PathBuf::from(codex_home).join("sessions");
        if path.exists() {
            return Some(path);
        }
    }
    let home = dirs::home_dir()?;
    let path = home.join(".codex/sessions");
    if path.exists() { Some(path) } else { None }
}

pub fn get_codex_files() -> Vec<PathBuf> {
    let Some(target_path) = get_codex_storage_path() else {
        return Vec::new();
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

pub fn extract_apply_patch_edits(patch_text: &str, languages: &mut LanguageAnalyzer) {
    let mut current_file: Option<String> = None;
    let mut added_bytes: usize = 0;

    let flush = |file: &mut Option<String>, bytes: &mut usize, langs: &mut LanguageAnalyzer| {
        if let Some(path) = file.take() {
            langs.record_file_edit(&path, *bytes);
        }
        *bytes = 0;
    };

    for line in patch_text.lines() {
        if let Some(rest) = line
            .strip_prefix("*** Add File: ")
            .or_else(|| line.strip_prefix("*** Update File: "))
        {
            flush(&mut current_file, &mut added_bytes, languages);
            let path = rest.trim();
            if !path.is_empty() {
                current_file = Some(path.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("*** Move to: ") {
            let path = rest.trim();
            if !path.is_empty() {
                current_file = Some(path.to_string());
            }
        } else if line.starts_with("*** Delete File: ") || line.starts_with("*** End Patch") {
            flush(&mut current_file, &mut added_bytes, languages);
        } else if current_file.is_some() && line.starts_with('+') && !line.starts_with("+++") {
            added_bytes += line.len().saturating_sub(1);
        }
    }

    flush(&mut current_file, &mut added_bytes, languages);
}

fn extract_codex_tool_edits(payload: &serde_json::Value, languages: &mut LanguageAnalyzer) {
    let Some(item_type) = payload.get("type").and_then(|t| t.as_str()) else {
        return;
    };

    match item_type {
        "custom_tool_call" => {
            let name = payload.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name == "apply_patch" {
                if let Some(input) = payload.get("input").and_then(|i| i.as_str()) {
                    extract_apply_patch_edits(input, languages);
                }
            }
        }
        "function_call" => {
            let name = payload.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let Some(args_str) = payload.get("arguments").and_then(|a| a.as_str()) else {
                return;
            };
            if name == "apply_patch" {
                if args_str.contains("*** Begin Patch") {
                    if let Ok(args_val) = serde_json::from_str::<serde_json::Value>(args_str) {
                        if let Some(patch) = args_val
                            .get("patch")
                            .or_else(|| args_val.get("input"))
                            .or_else(|| args_val.get("cmd"))
                            .and_then(|v| v.as_str())
                        {
                            extract_apply_patch_edits(patch, languages);
                            return;
                        }
                    }
                    extract_apply_patch_edits(args_str, languages);
                }
            } else if matches!(name, "write_file" | "edit_file" | "create_file") {
                if let Ok(args_val) = serde_json::from_str::<serde_json::Value>(args_str) {
                    if let Some(path) = args_val
                        .get("file_path")
                        .or_else(|| args_val.get("path"))
                        .or_else(|| args_val.get("filePath"))
                        .and_then(|v| v.as_str())
                    {
                        let len = args_val
                            .get("content")
                            .or_else(|| args_val.get("new_string"))
                            .or_else(|| args_val.get("newString"))
                            .and_then(|v| v.as_str())
                            .map_or(0, str::len);
                        languages.record_file_edit(path, len);
                    }
                }
            }
        }
        _ => {}
    }
}

fn extract_model_from_entry(entry_type: &str, payload: &serde_json::Value) -> Option<String> {
    match entry_type {
        "session_meta" => payload
            .get("model")
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
        "turn_context" => payload
            .get("model")
            .and_then(|m| m.as_str())
            .or_else(|| {
                payload
                    .get("collaboration_mode")
                    .and_then(|cm| cm.get("settings"))
                    .and_then(|s| s.get("model"))
                    .and_then(|m| m.as_str())
            })
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
        "world_state" => payload
            .get("state")
            .and_then(|st| st.get("collaboration_mode"))
            .and_then(|cm| cm.get("model"))
            .and_then(|m| m.as_str())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
        "event_msg" => {
            if payload.get("type").and_then(|t| t.as_str()) == Some("thread_settings_applied") {
                let ts = payload.get("thread_settings")?;
                ts.get("model")
                    .and_then(|m| m.as_str())
                    .or_else(|| {
                        ts.get("collaboration_mode")
                            .and_then(|cm| cm.get("settings"))
                            .and_then(|s| s.get("model"))
                            .and_then(|m| m.as_str())
                    })
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn parse_codex_file(file_path: &Path) -> CodexStats {
    parse_codex_file_incremental(file_path, None).1
}

pub fn parse_codex_file_incremental(
    file_path: &Path,
    prev: Option<(&CodexCacheState, &CodexStats)>,
) -> (CodexCacheState, CodexStats) {
    let mut file = match fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return (CodexCacheState::default(), CodexStats::default()),
    };

    let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let (mut state, mut local) = match prev {
        Some((prev_state, prev_stats)) if file_len >= prev_state.offset => {
            (prev_state.clone(), prev_stats.clone())
        }
        _ => (
            CodexCacheState::default(),
            CodexStats {
                sessions_found: 1,
                ..Default::default()
            },
        ),
    };

    if state.offset > 0 && state.offset == file_len {
        return (state, local);
    }

    if state.offset > 0 {
        if file.seek(SeekFrom::Start(state.offset)).is_err() {
            state = CodexCacheState::default();
            local = CodexStats {
                sessions_found: 1,
                ..Default::default()
            };
            let _ = file.seek(SeekFrom::Start(0));
        }
    }

    let reader = BufReader::new(file);
    for line in reader.lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(entry_type) = val.get("type").and_then(|t| t.as_str()) else {
            continue;
        };
        let Some(payload) = val.get("payload") else {
            continue;
        };

        if let Some(model) = extract_model_from_entry(entry_type, payload) {
            state.current_model = model;
        }

        if entry_type == "response_item" {
            extract_codex_tool_edits(payload, &mut local.languages);
            continue;
        }

        let (usage_opt, thread_total_opt) = if entry_type == "token_usage_record" {
            let u = payload.get("usage");
            let tt = payload
                .get("thread_token_usage")
                .and_then(|t| t.get("total_tokens"))
                .and_then(|v| v.as_i64());
            (u, tt)
        } else if entry_type == "event_msg"
            && payload.get("type").and_then(|t| t.as_str()) == Some("token_count")
        {
            let info = payload.get("info");
            let u = info.and_then(|i| i.get("last_token_usage"));
            let tt = info
                .and_then(|i| i.get("total_token_usage"))
                .and_then(|t| t.get("total_tokens"))
                .and_then(|v| v.as_i64());
            (u, tt)
        } else {
            (None, None)
        };

        let Some(usage) = usage_opt else {
            continue;
        };

        if let Some(tt) = thread_total_opt {
            if tt > 0 {
                if tt == state.last_thread_total_tokens {
                    continue;
                }
                state.last_thread_total_tokens = tt;
            }
        }

        let raw_in = usage
            .get("input_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cache_read = usage
            .get("cached_input_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cache_write = usage
            .get("cache_write_input_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let uncached_in = raw_in
            .saturating_sub(cache_read)
            .saturating_sub(cache_write);

        let raw_out = usage
            .get("output_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let reasoning = usage
            .get("reasoning_output_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let total_tok = usage
            .get("total_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let out_tokens = if reasoning > 0 && total_tok == raw_in + raw_out + reasoning {
            raw_out + reasoning
        } else {
            raw_out
        };

        if uncached_in + out_tokens + cache_read + cache_write == 0 {
            continue;
        }

        let model_name = if state.current_model.is_empty() {
            "gpt-6.1-sol".to_string()
        } else {
            state.current_model.clone()
        };

        let (date_str, month_str) = match val.get("timestamp").and_then(|t| t.as_str()) {
            Some(ts) => match DateTime::parse_from_rfc3339(&ts.replace('Z', "+00:00")) {
                Ok(dt) => {
                    let local_dt = dt.with_timezone(&Local);
                    (
                        local_dt.format("%Y-%m-%d").to_string(),
                        local_dt.format("%Y-%m").to_string(),
                    )
                }
                Err(_) => ("Unknown".to_string(), "Unknown".to_string()),
            },
            None => ("Unknown".to_string(), "Unknown".to_string()),
        };

        let pricing = get_pricing(&model_name, raw_in);
        let turn_cost = (uncached_in as f64 * pricing.input
            + out_tokens as f64 * pricing.output
            + cache_read as f64 * pricing.cache_read
            + cache_write as f64 * pricing.cache_write)
            / 1_000_000.0;

        let entry = TokenStats {
            in_tokens: uncached_in,
            out_tokens,
            cache_read_tokens: cache_read,
            cache_create_tokens: cache_write,
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
        *local.monthly_costs.entry(month_str).or_insert(0.0) += turn_cost;

        local.model_stats.entry(model_name).or_default().add(&entry);
        local.total_cost += turn_cost;
        local.total_messages += 1;
    }

    state.offset = file_len;
    (state, local)
}

pub fn run_codex_report() -> Option<(CodexStats, f64)> {
    let start = Instant::now();
    let files = get_codex_files();
    if files.is_empty() {
        return None;
    }

    let stats =
        files
            .par_iter()
            .map(|f| parse_codex_file(f))
            .reduce(CodexStats::default, |mut a, b| {
                a.merge(&b);
                a
            });

    let elapsed = start.elapsed().as_secs_f64();
    if stats.sessions_found == 0 {
        return None;
    }
    Some((stats, elapsed))
}

pub fn print_codex_report(stats: &CodexStats, elapsed: f64, daily_days: usize) {
    println!("\n{}", "=".repeat(95));
    println!(
        "{}📊 CODEX CLI USAGE & COST REPORT{}",
        TERM_HEADER, TERM_RESET
    );
    println!("{}", "=".repeat(95));

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

    println!("\n{}--- Monthly Token Usage ---{}", TERM_BOLD, TERM_RESET);
    let max_monthly = stats
        .monthly_stats
        .values()
        .map(|s| s.total())
        .max()
        .unwrap_or(0);
    for (month, s) in &stats.monthly_stats {
        print_token_bar(&format!("{:^10}", month), s, max_monthly, 35, true);
    }

    println!("\n{}--- Usage by Model ---{}", TERM_BOLD, TERM_RESET);
    let max_model = stats
        .model_stats
        .values()
        .map(|s| s.total())
        .max()
        .unwrap_or(0);
    let mut sorted_models: Vec<_> = stats.model_stats.iter().collect();
    sorted_models.sort_by_key(|b| std::cmp::Reverse(b.1.total()));
    for (model, s) in sorted_models {
        print_token_bar(
            &format!("{:<30}", model.get(..30).unwrap_or(model)),
            s,
            max_model,
            35,
            true,
        );
    }

    println!("\n{}=== FINANCIAL COSTS ==={}", TERM_HEADER, TERM_RESET);

    println!(
        "\n{}--- Daily Costs (Last {} Days) ---{}",
        TERM_BOLD, daily_days, TERM_RESET
    );
    let max_daily_cost = stats.daily_costs.values().copied().fold(0.0, f64::max);
    for (day, cost) in stats.daily_costs.iter().rev().take(daily_days) {
        print_cost_bar(&format!("{:<12}", day), *cost, max_daily_cost, 35);
    }

    println!("\n{}", "=".repeat(50));
    println!("{}GRAND TOTALS (CODEX){}", TERM_HEADER, TERM_RESET);
    println!("{}", "-".repeat(50));
    println!("{}Tokens:{}", TERM_BOLD, TERM_RESET);
    let in_sum: i64 = stats.monthly_stats.values().map(|s| s.in_tokens).sum();
    let out_sum: i64 = stats.monthly_stats.values().map(|s| s.out_tokens).sum();
    let cache_sum: i64 = stats
        .monthly_stats
        .values()
        .map(|s| s.cache_read_tokens)
        .sum();
    let cache_create_sum: i64 = stats
        .monthly_stats
        .values()
        .map(|s| s.cache_create_tokens)
        .sum();
    println!("  Input:        {:>12}", format_int_with_commas(in_sum));
    println!("  Output:       {:>12}", format_int_with_commas(out_sum));
    println!("  Cache Read:   {:>12}", format_int_with_commas(cache_sum));
    if cache_create_sum > 0 {
        println!(
            "  Cache Create: {:>12}",
            format_int_with_commas(cache_create_sum)
        );
    }
    println!("{}Cost:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {} ${}{}",
        TERM_GREEN,
        format_float_with_commas(stats.total_cost),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Performance:{}", TERM_BOLD, TERM_RESET);
    println!("  Sessions Parsed: {}", stats.sessions_found);
    println!("  Parse Time:      {:.4} seconds", elapsed);
    println!("{}", "=".repeat(50));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_codex_rollout_parsing_and_deduplication() {
        let mut tmp = NamedTempFile::new().unwrap();
        let lines = [
            serde_json::json!({
                "timestamp": "2026-11-10T09:57:23.922Z",
                "ordinal": 7,
                "type": "turn_context",
                "payload": {
                    "model": "global.openai.gpt-6.1-sol"
                }
            }),
            serde_json::json!({
                "timestamp": "2026-11-10T09:57:25.566Z",
                "ordinal": 13,
                "type": "token_usage_record",
                "payload": {
                    "usage": {
                        "input_tokens": 9689,
                        "cached_input_tokens": 0,
                        "cache_write_input_tokens": 9687,
                        "output_tokens": 15,
                        "reasoning_output_tokens": 0,
                        "total_tokens": 9704
                    },
                    "thread_token_usage": {
                        "total_tokens": 9704
                    }
                }
            }),
            // Duplicate event_msg token_count for the same response must not double-count
            serde_json::json!({
                "timestamp": "2026-11-10T09:57:25.567Z",
                "ordinal": 14,
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "total_token_usage": {
                            "total_tokens": 9704
                        },
                        "last_token_usage": {
                            "input_tokens": 9689,
                            "cached_input_tokens": 0,
                            "cache_write_input_tokens": 9687,
                            "output_tokens": 15,
                            "reasoning_output_tokens": 0,
                            "total_tokens": 9704
                        }
                    }
                }
            }),
            // Model switch mid-session to gpt-6.1-terra
            serde_json::json!({
                "timestamp": "2026-11-10T09:58:39.819Z",
                "ordinal": 49,
                "type": "event_msg",
                "payload": {
                    "type": "thread_settings_applied",
                    "thread_settings": {
                        "model": "global.openai.gpt-6.1-terra"
                    }
                }
            }),
            // apply_patch custom tool call
            serde_json::json!({
                "timestamp": "2026-11-10T09:58:40.000Z",
                "ordinal": 50,
                "type": "response_item",
                "payload": {
                    "type": "custom_tool_call",
                    "name": "apply_patch",
                    "input": "*** Begin Patch\n*** Update File: src/main.rs\n@@\n+fn main() {}\n*** End Patch"
                }
            }),
            serde_json::json!({
                "timestamp": "2026-11-10T09:58:41.000Z",
                "ordinal": 51,
                "type": "token_usage_record",
                "payload": {
                    "usage": {
                        "input_tokens": 10000,
                        "cached_input_tokens": 9000,
                        "cache_write_input_tokens": 500,
                        "output_tokens": 100,
                        "reasoning_output_tokens": 20,
                        "total_tokens": 10100
                    },
                    "thread_token_usage": {
                        "total_tokens": 19804
                    }
                }
            }),
        ];

        for l in &lines {
            writeln!(tmp, "{}", l).unwrap();
        }

        let stats = parse_codex_file(tmp.path());
        assert_eq!(stats.sessions_found, 1);
        assert_eq!(stats.total_messages, 2);

        let sol = stats.model_stats.get("global.openai.gpt-6.1-sol").unwrap();
        assert_eq!(sol.in_tokens, 2);
        assert_eq!(sol.cache_read_tokens, 0);
        assert_eq!(sol.cache_create_tokens, 9687);
        assert_eq!(sol.out_tokens, 15);
        assert_eq!(sol.total(), 9704);

        let terra = stats
            .model_stats
            .get("global.openai.gpt-6.1-terra")
            .unwrap();
        assert_eq!(terra.in_tokens, 500);
        assert_eq!(terra.cache_read_tokens, 9000);
        assert_eq!(terra.cache_create_tokens, 500);
        assert_eq!(terra.out_tokens, 100);
        assert_eq!(terra.total(), 10100);

        let rust_lang = stats.languages.stats.get("Rust").unwrap();
        assert_eq!(rust_lang.occurrences, 1);
        assert_eq!(rust_lang.bytes, "fn main() {}".len());
    }

    #[test]
    fn test_codex_incremental_parsing() {
        let mut tmp = NamedTempFile::new().unwrap();
        let line1 = serde_json::json!({
            "timestamp": "2026-11-10T09:57:23.922Z",
            "ordinal": 1,
            "type": "turn_context",
            "payload": { "model": "global.openai.gpt-6.1-sol" }
        });
        let line2 = serde_json::json!({
            "timestamp": "2026-11-10T09:57:25.566Z",
            "ordinal": 2,
            "type": "token_usage_record",
            "payload": {
                "usage": {
                    "input_tokens": 1000,
                    "cached_input_tokens": 0,
                    "cache_write_input_tokens": 0,
                    "output_tokens": 100,
                    "reasoning_output_tokens": 0,
                    "total_tokens": 1100
                },
                "thread_token_usage": { "total_tokens": 1100 }
            }
        });
        writeln!(tmp, "{}", line1).unwrap();
        writeln!(tmp, "{}", line2).unwrap();

        let (state1, stats1) = parse_codex_file_incremental(tmp.path(), None);
        assert_eq!(stats1.total_messages, 1);

        let line3 = serde_json::json!({
            "timestamp": "2026-11-10T09:58:00.000Z",
            "ordinal": 3,
            "type": "token_usage_record",
            "payload": {
                "usage": {
                    "input_tokens": 2000,
                    "cached_input_tokens": 1000,
                    "cache_write_input_tokens": 0,
                    "output_tokens": 50,
                    "reasoning_output_tokens": 0,
                    "total_tokens": 2050
                },
                "thread_token_usage": { "total_tokens": 3150 }
            }
        });
        writeln!(tmp, "{}", line3).unwrap();

        let (_, stats2) = parse_codex_file_incremental(tmp.path(), Some((&state1, &stats1)));
        assert_eq!(stats2.total_messages, 2);
        let sol = stats2.model_stats.get("global.openai.gpt-6.1-sol").unwrap();
        assert_eq!(sol.in_tokens, 2000);
        assert_eq!(sol.cache_read_tokens, 1000);
        assert_eq!(sol.out_tokens, 150);
    }
}
