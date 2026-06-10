use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use crate::colors::*;
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::pricing::get_pricing;
use crate::viz::{TokenStats, print_cost_bar, print_token_bar};

#[derive(Default, Clone)]
pub struct ZedStats {
    pub daily_costs: BTreeMap<String, f64>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub total_cost: f64,
    pub daily_stats: BTreeMap<String, TokenStats>,
    pub monthly_stats: BTreeMap<String, TokenStats>,
    pub model_stats: HashMap<String, TokenStats>,
    pub threads_found: usize,
    pub languages: crate::languages::LanguageAnalyzer,
}

pub fn get_zed_db_path() -> Option<PathBuf> {
    let base = dirs::home_dir()?;

    #[cfg(target_os = "macos")]
    let paths = vec![
        base.join("Library/Application Support/Zed/threads/threads.db"),
        base.join("Library/Application Support/Zed/db/threads.db"),
    ];

    #[cfg(not(target_os = "macos"))]
    let paths = vec![
        base.join(".local/share/zed/threads/threads.db"),
        base.join(".local/share/zed/db/threads.db"),
        base.join(".config/zed/threads/threads.db"),
    ];

    paths.into_iter().find(|path| path.exists())
}

pub fn parse_zed_db() -> Option<ZedStats> {
    let db_path = get_zed_db_path()?;
    let conn = Connection::open(db_path).ok()?;

    let mut stmt = conn
        .prepare("SELECT data_type, data, updated_at FROM threads")
        .ok()?;

    let mut stats = ZedStats::default();

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .ok()?;

    for row in rows.flatten() {
        let (data_type, data, updated_at) = row;

        let json_data = if data_type == "zstd" {
            match zstd::decode_all(&data[..]) {
                Ok(d) => d,
                Err(_) => continue,
            }
        } else if data_type == "json" {
            data
        } else {
            continue;
        };

        let payload: serde_json::Value = match serde_json::from_slice(&json_data) {
            Ok(p) => p,
            Err(_) => continue,
        };

        stats.threads_found += 1;

        // Extract model dynamically
        let model_name = payload
            .get("model")
            .and_then(|m| {
                // Handle both object format and plain string format
                if let Some(obj) = m.as_object() {
                    obj.get("model")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string())
                } else {
                    m.as_str().map(|s| s.to_string())
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        let dt = updated_at
            .and_then(|ts| {
                DateTime::parse_from_rfc3339(&ts)
                    .or_else(|_| DateTime::parse_from_rfc3339(&ts.replace(' ', "T")))
                    .or_else(|_| {
                        let clean = ts.split('.').next().unwrap_or(&ts);
                        DateTime::parse_from_str(
                            &(clean.to_string() + "+0000"),
                            "%Y-%m-%dT%H:%M:%S%z",
                        )
                    })
                    .ok()
            })
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        let date_key = dt.format("%Y-%m-%d").to_string();
        let month_key = dt.format("%Y-%m").to_string();

        let mut processed_usage = false;

        // Dynamically process request_token_usage
        if let Some(usages) = payload
            .get("request_token_usage")
            .and_then(|u| u.as_object())
        {
            for usage in usages.values() {
                let in_tokens = usage
                    .get("input_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let out_tokens = usage
                    .get("output_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let cache_read = usage
                    .get("cache_read_input_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);

                if in_tokens == 0 && out_tokens == 0 && cache_read == 0 {
                    continue;
                }
                processed_usage = true;
                add_usage_to_stats(
                    &mut stats,
                    &model_name,
                    in_tokens,
                    out_tokens,
                    cache_read,
                    &date_key,
                    &month_key,
                );
            }
        }

        // Dynamically process cumulative_token_usage as fallback
        if !processed_usage {
            if let Some(usage) = payload.get("cumulative_token_usage") {
                let in_tokens = usage
                    .get("input_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let out_tokens = usage
                    .get("output_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let cache_read = usage
                    .get("cache_read_input_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);

                if in_tokens > 0 || out_tokens > 0 || cache_read > 0 {
                    add_usage_to_stats(
                        &mut stats,
                        &model_name,
                        in_tokens,
                        out_tokens,
                        cache_read,
                        &date_key,
                        &month_key,
                    );
                }
            }
        }
    }

    if stats.threads_found == 0 {
        return None;
    }

    Some(stats)
}

fn add_usage_to_stats(
    stats: &mut ZedStats,
    model_name: &str,
    in_tokens: i64,
    out_tokens: i64,
    cache_read: i64,
    date_key: &str,
    month_key: &str,
) {
    let pricing = get_pricing(model_name, in_tokens);
    let cost = (in_tokens as f64 / 1_000_000.0 * pricing.input)
        + (out_tokens as f64 / 1_000_000.0 * pricing.output)
        + (cache_read as f64 / 1_000_000.0 * pricing.cache_read);

    let entry = TokenStats {
        in_tokens,
        out_tokens,
        cache_read_tokens: cache_read,
        cache_create_tokens: 0,
    };

    *stats.daily_costs.entry(date_key.to_string()).or_insert(0.0) += cost;
    *stats
        .monthly_costs
        .entry(month_key.to_string())
        .or_insert(0.0) += cost;
    stats.total_cost += cost;

    stats
        .daily_stats
        .entry(date_key.to_string())
        .or_default()
        .add(&entry);
    stats
        .monthly_stats
        .entry(month_key.to_string())
        .or_default()
        .add(&entry);
    stats
        .model_stats
        .entry(model_name.to_string())
        .or_default()
        .add(&entry);
}

pub fn run_zed_report() -> Option<(ZedStats, f64)> {
    let start = Instant::now();
    let stats = parse_zed_db()?;
    let duration = start.elapsed().as_secs_f64();
    Some((stats, duration))
}

pub fn print_zed_report(stats: &ZedStats, duration: f64, daily_days: usize) {
    println!("\n{}", "=".repeat(95));
    println!("{}📊 ZED USAGE & COST ESTIMATE{}", TERM_HEADER, TERM_RESET);
    println!("{}", "=".repeat(95));

    println!(
        "\n{}=== TOKEN USAGE (STACKED) ==={}",
        TERM_HEADER, TERM_RESET
    );
    println!(
        "Legend: {}█ Input{} | {}█ Output{} | {}▒ Cache Read{}",
        TERM_BLUE, TERM_RESET, TERM_GREEN, TERM_RESET, TERM_YELLOW, TERM_RESET
    );

    println!("\n{}--- Monthly Token Usage ---{}", TERM_BOLD, TERM_RESET);
    let max_monthly = stats
        .monthly_stats
        .values()
        .map(|s| s.total())
        .max()
        .unwrap_or(0);
    for (month, s) in &stats.monthly_stats {
        print_token_bar(&format!("{:^10}", month), s, max_monthly, 35, false);
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
            false,
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
    println!("{}GRAND TOTALS (ZED){}", TERM_HEADER, TERM_RESET);
    println!("{}", "-".repeat(50));
    println!("{}Tokens:{}", TERM_BOLD, TERM_RESET);
    let in_sum: i64 = stats.monthly_stats.values().map(|s| s.in_tokens).sum();
    let out_sum: i64 = stats.monthly_stats.values().map(|s| s.out_tokens).sum();
    let cache_sum: i64 = stats
        .monthly_stats
        .values()
        .map(|s| s.cache_read_tokens)
        .sum();
    println!("  Input:       {:>12}", format_int_with_commas(in_sum));
    println!("  Output:      {:>12}", format_int_with_commas(out_sum));
    println!("  Cache Read:  {:>12}", format_int_with_commas(cache_sum));
    println!("{}Cost:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {} ${}{}",
        TERM_GREEN,
        format_float_with_commas(stats.total_cost),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Performance:{}", TERM_BOLD, TERM_RESET);
    println!("  Threads Parsed: {}", stats.threads_found);
    println!("  Parse Time:     {:.4} seconds", duration);
    println!("{}", "=".repeat(50));
}

#[cfg(test)]
mod tests {
    // Helper function to extract data the same way parse_zed_db will
    fn extract_from_json(json_str: &str) -> (String, i64, i64, i64) {
        let payload: serde_json::Value = serde_json::from_str(json_str).unwrap();

        let model_name = payload
            .get("model")
            .and_then(|m| {
                if let Some(obj) = m.as_object() {
                    obj.get("model")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string())
                } else {
                    m.as_str().map(|s| s.to_string())
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        let mut in_tokens = 0;
        let mut out_tokens = 0;
        let mut cache_read = 0;

        if let Some(usages) = payload
            .get("request_token_usage")
            .and_then(|u| u.as_object())
        {
            for usage in usages.values() {
                in_tokens += usage
                    .get("input_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                out_tokens += usage
                    .get("output_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                cache_read += usage
                    .get("cache_read_input_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
            }
        }

        (model_name, in_tokens, out_tokens, cache_read)
    }

    #[test]
    fn test_zed_dynamic_parsing_modern_format() {
        // This is what the code currently expects
        let json = r#"{
            "title": "Modern Thread",
            "model": { "provider": "anthropic", "model": "claude-3-5-sonnet-20241022" },
            "request_token_usage": {
                "msg1": { "input_tokens": 100, "output_tokens": 50, "cache_read_input_tokens": 20 }
            }
        }"#;

        let (model, input, output, cache) = extract_from_json(json);
        assert_eq!(model, "claude-3-5-sonnet-20241022");
        assert_eq!(input, 100);
        assert_eq!(output, 50);
        assert_eq!(cache, 20);
    }

    #[test]
    fn test_zed_dynamic_parsing_legacy_string_model() {
        // This is the broken format that was crashing the strict deserializer
        let json = r#"{
            "title": "Legacy Thread",
            "model": "gpt-4o",
            "request_token_usage": {
                "msg1": { "input_tokens": 500, "output_tokens": 25 }
            }
        }"#;

        let (model, input, output, cache) = extract_from_json(json);
        assert_eq!(model, "gpt-4o"); // We successfully extract it instead of panicking
        assert_eq!(input, 500);
        assert_eq!(output, 25);
        assert_eq!(cache, 0); // Handles missing cache keys safely
    }

    #[test]
    fn test_zed_dynamic_parsing_missing_model() {
        // Failsafe for entirely unknown structures
        let json = r#"{
            "title": "Unknown Model Thread",
            "request_token_usage": {
                "msg1": { "input_tokens": 10 }
            }
        }"#;

        let (model, input, output, _) = extract_from_json(json);
        assert_eq!(model, "unknown"); // Falls back safely
        assert_eq!(input, 10);
        assert_eq!(output, 0);
    }
}
