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
use crate::viz::{print_cost_bar, print_token_bar, TokenStats};

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

#[derive(Default)]
pub struct GeminiStats {
    pub daily_stats: BTreeMap<String, TokenStats>,
    pub daily_costs: BTreeMap<String, f64>,
    pub monthly_stats: BTreeMap<String, TokenStats>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub model_stats: HashMap<String, TokenStats>,
    pub monthly_model_usage: BTreeMap<String, HashMap<String, TokenStats>>,
    pub total_messages: usize,
    pub sessions_found: usize,
}

pub fn get_gemini_pricing(model: &str, input_count: i64) -> (f64, f64, f64) {
    let m = model.to_lowercase();
    if m.contains("gemini-3.1-flash-lite") {
        (0.25, 1.50, 0.025)
    } else if m.contains("gemini-3-pro") || m.contains("gemini-3.1-pro") {
        if input_count <= 200_000 { (2.00, 12.00, 0.20) } else { (4.00, 18.00, 0.40) }
    } else if m.contains("gemini-3-flash") {
        (0.50, 3.00, 0.05)
    } else if m.contains("gemini-1.5-pro") || m.contains("gemini-2.5-pro") {
        if input_count <= 128_000 { (1.25, 5.00, 0.3125) } else { (2.50, 10.00, 0.625) }
    } else if m.contains("gemini-1.5-flash") || m.contains("gemini-2.0-flash") || m.contains("gemini-2.5-flash") {
        if input_count <= 128_000 { (0.075, 0.30, 0.01875) } else { (0.15, 0.60, 0.0375) }
    } else {
        (1.00, 4.00, 0.10)
    }
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

pub fn run_gemini_report() -> Option<(GeminiStats, f64)> {
    let home = match dirs::home_dir() {
        Some(p) => p,
        None => {
            println!("Error: Could not determine home directory.");
            return None;
        }
    };
    let target_path = home.join(".gemini/tmp");
    if !target_path.exists() {
        println!("{}Error: Could not find storage at {}{}", RED, target_path.display(), RESET);
        return None;
    }

    let session_files: Vec<PathBuf> = WalkDir::new(&target_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("session-") && e.file_name().to_string_lossy().contains(".json"))
        .map(|e| e.path().to_path_buf())
        .collect();

    println!("{}Scanning Gemini session files in:{}\n{}\n", BOLD, RESET, target_path.display());

    let start_time = Instant::now();

    let global_stats = session_files
        .par_iter()
        .map(|file_path| {
            let mut local = GeminiStats::default();
            let mut messages = Vec::new();

            if file_path.to_string_lossy().ends_with(".jsonl") {
                if let Ok(file) = fs::File::open(file_path) {
                    let reader = BufReader::new(file);
                    for line in reader.lines() {
                        if let Ok(l) = line {
                            if let Ok(msg) = serde_json::from_str::<GeminiMessage>(&l) {
                                if msg.msg_type.is_some() && msg.timestamp.is_some() {
                                    messages.push(msg);
                                }
                            }
                        }
                    }
                }
            } else if let Ok(content) = fs::read(file_path) {
                if let Ok(session) = serde_json::from_slice::<GeminiSession>(&content) {
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
                let msg_type = msg.msg_type.as_deref().unwrap_or("unknown");
                let msg_model = msg.model.clone().unwrap_or_else(|| session_model.clone());

                let (date_str, month_str) = if let Some(ts_str) = &msg.timestamp {
                    if let Ok(dt) = DateTime::parse_from_rfc3339(&ts_str.replace('Z', "+00:00")) {
                        (dt.format("%Y-%m-%d").to_string(), dt.format("%Y-%m").to_string())
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
                let (price_in, price_out, price_cache) = get_gemini_pricing(&msg_model, total_context);
                let turn_cost = (in_tokens as f64 / 1_000_000.0 * price_in)
                    + (out_tokens as f64 / 1_000_000.0 * price_out)
                    + (cache_tokens as f64 / 1_000_000.0 * price_cache);

                let entry = TokenStats {
                    in_tokens,
                    out_tokens,
                    cache_read_tokens: cache_tokens,
                    cache_create_tokens: 0,
                };

                local.daily_stats.entry(date_str.clone()).or_default().add(&entry);
                *local.daily_costs.entry(date_str).or_insert(0.0) += turn_cost;

                local.monthly_stats.entry(month_str.clone()).or_default().add(&entry);
                *local.monthly_costs.entry(month_str.clone()).or_insert(0.0) += turn_cost;

                local.model_stats.entry(msg_model.clone()).or_default().add(&entry);
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
        })
        .reduce(GeminiStats::default, merge_gemini_stats);

    let parsing_time = start_time.elapsed().as_secs_f64();
    if global_stats.total_messages == 0 {
        return None;
    }
    Some((global_stats, parsing_time))
}

pub fn print_gemini_report(global_stats: &GeminiStats, parsing_time: f64, daily_days: usize) {
    println!("\n{}", "=".repeat(95));
    println!("{}📊 GEMINI CLI USAGE & COST ESTIMATE{}", HEADER, RESET);
    println!("{}", "=".repeat(95));
    println!("{}Sessions Scanned:{} {}", BOLD, RESET, global_stats.sessions_found);
    println!("{}Total Messages:{}   {}", BOLD, RESET, format_int_with_commas(global_stats.total_messages as i64));
    println!("{}", "-".repeat(95));

    if !global_stats.model_stats.is_empty() {
        println!("\n{}=== TOKEN USAGE (STACKED) ==={}", HEADER, RESET);
        println!(
            "Legend: {}█ Input{} | {}█ Output{} | {}▒ Cache Read{}",
            BLUE, RESET, GREEN, RESET, YELLOW, RESET
        );

        let max_model_len = global_stats.model_stats.keys().map(|m| m.len()).max().unwrap_or(20).min(30);
        println!("\n{}--- Overall Usage by Model ---{}", BOLD, RESET);
        let all_max_tokens = global_stats.model_stats.values().map(|s| s.total()).max().unwrap_or(0);
        let mut sorted_models: Vec<_> = global_stats.model_stats.iter().collect();
        sorted_models.sort_by(|a, b| b.1.total().cmp(&a.1.total()));
        for (model, stats) in sorted_models {
            print_token_bar(
                &format!("{:<width$}", model.get(..30).unwrap_or(model), width = max_model_len),
                stats,
                all_max_tokens,
                35,
                false,
            );
        }

        println!("\n{}--- Monthly Breakdown by Model ---{}", BOLD, RESET);
        for (month, models) in global_stats.monthly_model_usage.iter().rev() {
            if month == "Unknown" {
                continue;
            }
            println!("\n{}{}{}", CYAN, month, RESET);
            let month_max = models.values().map(|s| s.total()).max().unwrap_or(1);
            let mut sorted_m: Vec<_> = models.iter().collect();
            sorted_m.sort_by(|a, b| b.1.total().cmp(&a.1.total()));
            for (model, stats) in sorted_m {
                print_token_bar(
                    &format!("  {:<width$}", model.get(..30).unwrap_or(model), width = max_model_len),
                    stats,
                    month_max,
                    35,
                    false,
                );
            }
        }
    }

    println!("\n{}=== FINANCIAL COSTS ==={}", HEADER, RESET);
    if !global_stats.monthly_costs.is_empty() {
        println!("\n{}--- Monthly Costs ---{}", BOLD, RESET);
        let max_month_cost = global_stats.monthly_costs.values().copied().fold(0.0_f64, |a, b| a.max(b));
        for (month, cost) in global_stats.monthly_costs.iter().rev() {
            if month == "Unknown" {
                continue;
            }
            print_cost_bar(&format!("{:^12}", month), *cost, max_month_cost, 35);
        }
    }

    if !global_stats.daily_costs.is_empty() {
        println!("\n{}--- Daily Costs (Last {} days) ---{}", BOLD, daily_days, RESET);
        let max_day_cost = global_stats.daily_costs.values().copied().fold(0.0_f64, |a, b| a.max(b));
        let mut sorted_days: Vec<_> = global_stats.daily_costs.iter().collect();
        sorted_days.sort_by(|a, b| a.0.cmp(b.0));
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
    println!("{}GRAND TOTALS (GEMINI CLI){}", HEADER, RESET);
    println!("{}", "-".repeat(50));
    println!("{}Tokens:{}", BOLD, RESET);
    println!("  {}Input:       {:>12}{}", BLUE, format_int_with_commas(total_tokens.in_tokens), RESET);
    println!("  {}Output:      {:>12}{}", GREEN, format_int_with_commas(total_tokens.out_tokens), RESET);
    println!("  {}Cache:       {:>12}{}", YELLOW, format_int_with_commas(total_tokens.cache_read_tokens), RESET);
    println!("  {}Total:       {:>12}{}", BOLD, format_int_with_commas(total_tokens.total()), RESET);
    println!("{}", "-".repeat(50));
    println!("{}Cost:{}", BOLD, RESET);
    println!("  {} ${}{}", RED, format_float_with_commas(total_cost), RESET);
    println!("{}", "-".repeat(50));
    println!("{}Performance:{}", BOLD, RESET);
    println!("  Sessions Parsed: {}", global_stats.sessions_found);
    println!("  Parse Time:      {:.2} seconds", parsing_time);
    println!("{}", "=".repeat(50));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gemini_pricing() {
        let (in_p, out_p, cache_p) = get_gemini_pricing("gemini-3.1-flash-lite-preview", 0);
        assert_eq!(in_p, 0.25);
        assert_eq!(out_p, 1.50);
        assert_eq!(cache_p, 0.025);

        let (in_p, out_p, cache_p) = get_gemini_pricing("gemini-3.1-pro", 100_000);
        assert_eq!(in_p, 2.00);
        assert_eq!(out_p, 12.00);
        assert_eq!(cache_p, 0.20);

        let (in_p, out_p, cache_p) = get_gemini_pricing("gemini-3.1-pro", 300_000);
        assert_eq!(in_p, 4.00);
        assert_eq!(out_p, 18.00);
        assert_eq!(cache_p, 0.40);

        let (in_p, out_p, cache_p) = get_gemini_pricing("gemini-3-flash", 0);
        assert_eq!(in_p, 0.50);
        assert_eq!(out_p, 3.00);
        assert_eq!(cache_p, 0.05);

        let (in_p, out_p, cache_p) = get_gemini_pricing("unknown-model", 0);
        assert_eq!(in_p, 1.00);
        assert_eq!(out_p, 4.00);
        assert_eq!(cache_p, 0.10);
    }
}
