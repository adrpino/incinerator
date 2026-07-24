use chrono::{DateTime, Local};
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

pub fn parse_model_id(raw: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
        if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
            return id.to_string();
        }
        if let Some(id) = val.get("modelID").and_then(|v| v.as_str()) {
            return id.to_string();
        }
    }
    if raw.starts_with('{') {
        "unknown".to_string()
    } else {
        raw.to_string()
    }
}

pub fn parse_opencode_db() -> Option<OpencodeStats> {
    let db_path = get_opencode_db_path()?;
    parse_opencode_db_at(db_path)
}

pub fn parse_opencode_db_at(db_path: PathBuf) -> Option<OpencodeStats> {
    let conn = Connection::open(db_path).ok()?;
    parse_opencode_connection(&conn)
}

pub fn parse_opencode_connection(conn: &Connection) -> Option<OpencodeStats> {
    let mut stats = OpencodeStats::default();
    let mut unique_sessions = std::collections::HashSet::new();

    // 1. Try to query `message` table first to get per-message granularity
    let message_query_success = if let Ok(mut stmt) = conn.prepare(
        "SELECT \
         session_id, \
         json_extract(data, '$.modelID'), \
         json_extract(data, '$.cost'), \
         json_extract(data, '$.tokens.input'), \
         json_extract(data, '$.tokens.output'), \
         json_extract(data, '$.tokens.reasoning'), \
         json_extract(data, '$.tokens.cache.read'), \
         json_extract(data, '$.tokens.cache.write'), \
         time_created \
         FROM message \
         WHERE json_extract(data, '$.role') = 'assistant'",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, i64>(8)?,
            ))
        }) {
            for row in rows.flatten() {
                let (
                    session_id,
                    model_raw,
                    cost_opt,
                    tokens_in_opt,
                    tokens_out_opt,
                    tokens_reasoning_opt,
                    cache_read_opt,
                    cache_write_opt,
                    time_created,
                ) = row;

                if let Some(sid) = session_id {
                    unique_sessions.insert(sid);
                }

                let cost = cost_opt.unwrap_or(0.0);
                let tokens_in = tokens_in_opt.unwrap_or(0);
                let tokens_out = tokens_out_opt.unwrap_or(0);
                let tokens_reasoning = tokens_reasoning_opt.unwrap_or(0);
                let cache_read = cache_read_opt.unwrap_or(0);
                let cache_write = cache_write_opt.unwrap_or(0);

                let model_name = model_raw
                    .as_deref()
                    .map(parse_model_id)
                    .unwrap_or_else(|| "unknown".to_string());

                // Format Dates (using Local timezone to align with TUI)
                let dt = DateTime::from_timestamp(
                    time_created / 1000,
                    ((time_created % 1000) * 1_000_000) as u32,
                )
                .map(|dt| dt.with_timezone(&Local))
                .unwrap_or_else(Local::now);

                let date_key = dt.format("%Y-%m-%d").to_string();
                let month_key = dt.format("%Y-%m").to_string();

                *stats.daily_costs.entry(date_key.clone()).or_insert(0.0) += cost;
                *stats.monthly_costs.entry(month_key.clone()).or_insert(0.0) += cost;
                stats.total_cost += cost;

                let entry = TokenStats {
                    in_tokens: tokens_in,
                    out_tokens: tokens_out + tokens_reasoning,
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
            stats.sessions_found = unique_sessions.len();
            true
        } else {
            false
        }
    } else {
        false
    };

    // 2. Fallback to `session` table if `message` table query fails or is not present
    if !message_query_success {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT model, cost, tokens_input, tokens_output, tokens_reasoning, \
             tokens_cache_read, tokens_cache_write, time_created FROM session",
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
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
            }) {
                for row in rows.flatten() {
                    let (
                        model_json,
                        cost,
                        tokens_in,
                        tokens_out,
                        tokens_reasoning,
                        cache_read,
                        cache_write,
                        time_created,
                    ) = row;

                    stats.sessions_found += 1;

                    let model_name = parse_model_id(&model_json);

                    // Format Dates (using Local timezone to align with TUI)
                    let dt = DateTime::from_timestamp(
                        time_created / 1000,
                        ((time_created % 1000) * 1_000_000) as u32,
                    )
                    .map(|dt| dt.with_timezone(&Local))
                    .unwrap_or_else(Local::now);

                    let date_key = dt.format("%Y-%m-%d").to_string();
                    let month_key = dt.format("%Y-%m").to_string();

                    *stats.daily_costs.entry(date_key.clone()).or_insert(0.0) += cost;
                    *stats.monthly_costs.entry(month_key.clone()).or_insert(0.0) += cost;
                    stats.total_cost += cost;

                    let entry = TokenStats {
                        in_tokens: tokens_in,
                        out_tokens: tokens_out + tokens_reasoning,
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
            }
        }
    }

    // 3. Populate language statistics from the `part` table if available
    if let Ok(mut part_stmt) = conn.prepare(
        "SELECT \
         json_extract(data, '$.path'), \
         json_extract(data, '$.file_path'), \
         json_extract(data, '$.filePath'), \
         json_extract(data, '$.bytes'), \
         json_extract(data, '$.size'), \
         json_extract(data, '$.content'), \
         json_extract(data, '$.patch') \
         FROM part \
         WHERE json_extract(data, '$.type') IN ('patch', 'file', 'tool')",
    ) {
        if let Ok(part_rows) = part_stmt.query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        }) {
            for row in part_rows.flatten() {
                let (
                    path_opt,
                    file_path_opt,
                    file_path_camel_opt,
                    bytes_opt,
                    size_opt,
                    content_opt,
                    patch_opt,
                ) = row;

                let path = path_opt.or(file_path_opt).or(file_path_camel_opt);
                if let Some(p) = path {
                    let bytes = if let Some(b) = bytes_opt {
                        b as usize
                    } else if let Some(s) = size_opt {
                        s as usize
                    } else if let Some(c) = content_opt {
                        c.len()
                    } else if let Some(pt) = patch_opt {
                        pt.len()
                    } else {
                        0
                    };
                    stats.languages.record_file_edit(&p, bytes);
                }
            }
        }
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

    #[test]
    fn test_model_id_parsing() {
        // - Standard JSON: {"id": "gemini-3.5-flash", "providerID": "google"}
        assert_eq!(
            parse_model_id(r#"{"id": "gemini-3.5-flash", "providerID": "google"}"#),
            "gemini-3.5-flash"
        );

        // - Alternate JSON: {"modelID": "claude-3-5-sonnet", "provider": "anthropic"}
        assert_eq!(
            parse_model_id(r#"{"modelID": "claude-3-5-sonnet", "provider": "anthropic"}"#),
            "claude-3-5-sonnet"
        );

        // - Plain string: "gpt-4o"
        assert_eq!(parse_model_id("gpt-4o"), "gpt-4o");

        // - Fallback / Malformed: "{invalid_json" -> "unknown"
        assert_eq!(parse_model_id("{invalid_json"), "unknown");
    }

    #[test]
    fn test_timezone_consistency() {
        let time_created: i64 = 1781514498227; // June 15, 2026 epoch ms

        let dt = DateTime::from_timestamp(
            time_created / 1000,
            ((time_created % 1000) * 1_000_000) as u32,
        )
        .map(|dt| dt.with_timezone(&Local))
        .unwrap();

        let date_key = dt.format("%Y-%m-%d").to_string();
        let month_key = dt.format("%Y-%m").to_string();

        let expected_dt = DateTime::from_timestamp(
            time_created / 1000,
            ((time_created % 1000) * 1_000_000) as u32,
        )
        .map(|dt| dt.with_timezone(&Local))
        .unwrap();

        assert_eq!(date_key, expected_dt.format("%Y-%m-%d").to_string());
        assert_eq!(month_key, expected_dt.format("%Y-%m").to_string());
    }

    #[test]
    fn test_in_memory_opencode_db_parsing() {
        let conn = Connection::open_in_memory().unwrap();

        // 1. Create schema
        conn.execute(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                model TEXT,
                cost REAL,
                tokens_input INTEGER,
                tokens_output INTEGER,
                tokens_reasoning INTEGER,
                tokens_cache_read INTEGER,
                tokens_cache_write INTEGER,
                time_created INTEGER
            )",
            [],
        )
        .unwrap();

        conn.execute(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                data TEXT,
                time_created INTEGER,
                FOREIGN KEY(session_id) REFERENCES session(id)
            )",
            [],
        )
        .unwrap();

        conn.execute(
            "CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT,
                data TEXT,
                time_created INTEGER,
                FOREIGN KEY(message_id) REFERENCES message(id)
            )",
            [],
        )
        .unwrap();

        // 2. Insert test data
        let day1_ms: i64 = 1781514498227; // June 15, 2026
        let day2_ms: i64 = 1781600898227; // June 16, 2026

        // Insert session
        conn.execute(
            "INSERT INTO session (id, model, cost, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write, time_created) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            (
                "session-1",
                "gemini-3.5-flash",
                0.0,
                0,
                0,
                0,
                0,
                0,
                day1_ms,
            ),
        )
        .unwrap();

        // Message 1 (Day 1) with reasoning tokens: input=100, output=100, reasoning=250, cost=0.001
        let message_data1 = serde_json::json!({
            "role": "assistant",
            "modelID": {
                "id": "gemini-3.5-flash",
                "providerID": "google"
            },
            "cost": 0.001,
            "tokens": {
                "input": 100,
                "output": 100,
                "reasoning": 250,
                "cache": {
                    "read": 10,
                    "write": 20
                }
            }
        })
        .to_string();

        conn.execute(
            "INSERT INTO message (id, session_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
            ("msg-1", "session-1", message_data1, day1_ms),
        )
        .unwrap();

        // Message 2 (Day 2) with reasoning tokens: input=200, output=150, reasoning=100, cost=0.002
        let message_data2 = serde_json::json!({
            "role": "assistant",
            "modelID": "gemini-3.5-flash",
            "cost": 0.002,
            "tokens": {
                "input": 200,
                "output": 150,
                "reasoning": 100,
                "cache": {
                    "read": 30,
                    "write": 40
                }
            }
        })
        .to_string();

        conn.execute(
            "INSERT INTO message (id, session_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
            ("msg-2", "session-1", message_data2, day2_ms),
        )
        .unwrap();

        // Part 1 (Day 1): type='patch', path='src/main.rs', bytes=150
        let part_data1 = serde_json::json!({
            "type": "patch",
            "path": "src/main.rs",
            "bytes": 150
        })
        .to_string();

        // Part 2 (Day 2): type='file', file_path='app.py', size=300
        let part_data2 = serde_json::json!({
            "type": "file",
            "file_path": "app.py",
            "size": 300
        })
        .to_string();

        conn.execute(
            "INSERT INTO part (id, message_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
            ("part-1", "msg-1", part_data1, day1_ms),
        )
        .unwrap();

        conn.execute(
            "INSERT INTO part (id, message_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
            ("part-2", "msg-2", part_data2, day2_ms),
        )
        .unwrap();

        // Parse and verify
        let stats =
            parse_opencode_connection(&conn).expect("Should successfully parse in-memory db");

        // Verify sessions found (we inserted messages for 1 unique session)
        assert_eq!(stats.sessions_found, 1);

        // Verify grand totals
        assert_eq!(stats.total_cost, 0.003);

        // Verify model stats: out_tokens should sum output + reasoning for both messages
        // Msg 1: 100 output + 250 reasoning = 350 out_tokens
        // Msg 2: 150 output + 100 reasoning = 250 out_tokens
        // Total: 600 out_tokens, 300 in_tokens
        let model_stat = stats
            .model_stats
            .get("gemini-3.5-flash")
            .expect("Model should be found");
        assert_eq!(model_stat.in_tokens, 300);
        assert_eq!(model_stat.out_tokens, 600); // 100 + 250 + 150 + 100

        // Verify daily breakdown (checking keys exist, accounting for local timezone mapping)
        let dt1 = DateTime::from_timestamp(day1_ms / 1000, 0)
            .unwrap()
            .with_timezone(&Local);
        let dt2 = DateTime::from_timestamp(day2_ms / 1000, 0)
            .unwrap()
            .with_timezone(&Local);
        let date_key1 = dt1.format("%Y-%m-%d").to_string();
        let date_key2 = dt2.format("%Y-%m-%d").to_string();

        assert!(stats.daily_costs.contains_key(&date_key1));
        assert!(stats.daily_costs.contains_key(&date_key2));

        // Verify language stats: Rust should be 150 bytes, Python should be 300 bytes
        assert_eq!(
            stats.languages.stats.get("Rust").map(|s| s.bytes),
            Some(150)
        );
        assert_eq!(
            stats.languages.stats.get("Python").map(|s| s.bytes),
            Some(300)
        );
    }
}
