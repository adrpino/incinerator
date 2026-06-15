use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use crate::colors::*;
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::viz::{TokenStats, print_cost_bar, print_token_bar};

#[derive(Default, Clone)]
pub struct OpencodeStats {
    pub daily_costs: BTreeMap<String, f64>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub total_cost: f64,
    pub daily_stats: BTreeMap<String, TokenStats>,
    pub monthly_stats: BTreeMap<String, TokenStats>,
    pub model_stats: HashMap<String, TokenStats>,
    pub sessions_found: usize,
    pub languages: crate::languages::LanguageAnalyzer,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct OpencodeModelJson {
    pub id: String,
    #[serde(rename = "providerID")]
    pub provider_id: Option<String>,
}

pub fn get_opencode_db_path() -> Option<PathBuf> {
    let base = dirs::home_dir()?;
    let path = base.join(".local/share/opencode/opencode.db");
    if path.exists() { Some(path) } else { None }
}

pub fn parse_opencode_db() -> Option<OpencodeStats> {
    let db_path = get_opencode_db_path()?;
    parse_opencode_db_at(db_path)
}

pub fn parse_opencode_db_at(db_path: PathBuf) -> Option<OpencodeStats> {
    let conn = Connection::open(db_path).ok()?;

    let mut stmt = conn
        .prepare(
            "SELECT model, cost, tokens_input, tokens_output, tokens_reasoning, \
             tokens_cache_read, tokens_cache_write, time_created FROM session",
        )
        .ok()?;

    let mut stats = OpencodeStats::default();

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?, // model (JSON string)
                row.get::<_, f64>(1)?,    // cost
                row.get::<_, i64>(2)?,    // tokens_input
                row.get::<_, i64>(3)?,    // tokens_output
                row.get::<_, i64>(4)?,    // tokens_reasoning
                row.get::<_, i64>(5)?,    // tokens_cache_read
                row.get::<_, i64>(6)?,    // tokens_cache_write
                row.get::<_, i64>(7)?,    // time_created (epoch ms)
            ))
        })
        .ok()?;

    for row in rows.flatten() {
        let (
            model_json,
            cost,
            tokens_in,
            tokens_out,
            _tokens_reasoning,
            cache_read,
            cache_write,
            time_created,
        ) = row;

        stats.sessions_found += 1;

        // 1. Safely extract model name from serialized JSON or fallback to raw column string
        let model_name = serde_json::from_str::<OpencodeModelJson>(&model_json)
            .map(|m| m.id)
            .unwrap_or_else(|_| {
                // Failsafe fallback
                if model_json.contains("\"id\":\"") {
                    model_json
                        .split("\"id\":\"")
                        .nth(1)
                        .and_then(|s| s.split('"').next())
                        .unwrap_or("unknown")
                        .to_string()
                } else {
                    "unknown".to_string()
                }
            });

        // 2. Format Dates
        let dt = DateTime::from_timestamp(
            time_created / 1000,
            ((time_created % 1000) * 1_000_000) as u32,
        )
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

        let date_key = dt.format("%Y-%m-%d").to_string();
        let month_key = dt.format("%Y-%m").to_string();

        // 3. Accumulate Statistics
        *stats.daily_costs.entry(date_key.clone()).or_insert(0.0) += cost;
        *stats.monthly_costs.entry(month_key.clone()).or_insert(0.0) += cost;
        stats.total_cost += cost;

        let entry = TokenStats {
            in_tokens: tokens_in,
            out_tokens: tokens_out,
            cache_read_tokens: cache_read,
            cache_create_tokens: cache_write,
        };

        stats
            .daily_stats
            .entry(date_key.clone())
            .or_default()
            .add(&entry);
        stats
            .monthly_stats
            .entry(month_key.clone())
            .or_default()
            .add(&entry);
        stats.model_stats.entry(model_name).or_default().add(&entry);
    }

    Some(stats)
}

pub fn run_opencode_report() -> Option<(OpencodeStats, f64)> {
    let start = Instant::now();
    let stats = parse_opencode_db()?;
    let elapsed = start.elapsed().as_secs_f64();
    Some((stats, elapsed))
}

pub fn print_opencode_report(stats: &OpencodeStats, elapsed: f64, daily_days: usize) {
    println!("\n{}", "=".repeat(95));
    println!(
        "{}📊 OPENCODE USAGE & COST REPORT{}",
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
    println!("{}GRAND TOTALS (OPENCODE){}", TERM_HEADER, TERM_RESET);
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

    #[test]
    fn test_opencode_model_json_parsing() {
        // Test that our Model deserialization handles default formats correctly
        let raw_json = r#"{"id":"gemini-3.5-flash","providerID":"google","variant":"default"}"#;
        let parsed: Result<OpencodeModelJson, _> = serde_json::from_str(raw_json);

        assert!(parsed.is_ok());
        let model = parsed.unwrap();
        assert_eq!(model.id, "gemini-3.5-flash");
    }

    #[test]
    fn test_opencode_timestamp_arithmetic() {
        use chrono::{DateTime, Utc};
        // Verify epoch millisecond to UTC standard conversion logic
        let time_created: i64 = 1781514498227; // June 15, 2026

        let dt = DateTime::from_timestamp(
            time_created / 1000,
            ((time_created % 1000) * 1_000_000) as u32,
        )
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap();

        let date_key = dt.format("%Y-%m-%d").to_string();
        let month_key = dt.format("%Y-%m").to_string();

        assert_eq!(date_key, "2026-06-15");
        assert_eq!(month_key, "2026-06");
    }
}
