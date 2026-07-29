use chrono::{DateTime, Local};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use crate::colors::*;
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::pricing::get_pricing;
use crate::viz::{TokenStats, print_cost_bar, print_token_bar};

#[derive(Default, Clone)]
pub struct AntigravityStats {
    pub daily_costs: BTreeMap<String, f64>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub total_cost: f64,
    pub daily_stats: BTreeMap<String, TokenStats>,
    pub monthly_stats: BTreeMap<String, TokenStats>,
    pub model_stats: HashMap<String, TokenStats>,
    pub sessions_found: usize,
    pub languages: crate::languages::LanguageAnalyzer,
}

#[derive(Debug, Default, Clone)]
pub struct AntigravityGenTokenUsage {
    pub model_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub reasoning_tokens: i64,
}

pub fn get_antigravity_storage_path() -> Option<PathBuf> {
    let base = dirs::home_dir()?;
    let path = base.join(".gemini/antigravity-cli");
    if path.exists() { Some(path) } else { None }
}

pub fn parse_cortex_generator_metadata(blob: &[u8]) -> Option<AntigravityGenTokenUsage> {
    let mut usage = AntigravityGenTokenUsage::default();
    let mut pos = 0;

    while pos < blob.len() {
        let (tag, n) = read_varint(&blob[pos..])?;
        pos += n;
        let field_num = tag >> 3;
        let wire_type = tag & 0x07;

        match wire_type {
            0 => {
                // Varint
                let (_, n) = read_varint(&blob[pos..])?;
                pos += n;
            }
            2 => {
                // Length-delimited
                let (len, n) = read_varint(&blob[pos..])?;
                pos += n;
                let data = blob.get(pos..pos + len as usize)?;
                pos += len as usize;

                if field_num == 1 {
                    // SubMessage: Generation Details
                    parse_field1_details(data, &mut usage);
                }
            }
            1 => pos += 8, // 64-bit
            5 => pos += 4, // 32-bit
            _ => break,
        }
    }

    if usage.model_id.is_empty() {
        usage.model_id = "gemini-3.6-flash".to_string();
    }

    Some(usage)
}

fn parse_field1_details(blob: &[u8], usage: &mut AntigravityGenTokenUsage) {
    let mut pos = 0;
    while pos < blob.len() {
        let Some((tag, n)) = read_varint(&blob[pos..]) else {
            break;
        };
        pos += n;
        let field_num = tag >> 3;
        let wire_type = tag & 0x07;

        match wire_type {
            0 => {
                let Some((_, n)) = read_varint(&blob[pos..]) else {
                    break;
                };
                pos += n;
            }
            2 => {
                let Some((len, n)) = read_varint(&blob[pos..]) else {
                    break;
                };
                pos += n;
                let Some(data) = blob.get(pos..pos + len as usize) else {
                    break;
                };
                pos += len as usize;

                if field_num == 4 {
                    // Usage Breakdown SubMessage
                    parse_token_breakdown(data, usage);
                } else if field_num == 19 || field_num == 21 {
                    if let Ok(s) = std::str::from_utf8(data) {
                        if field_num == 19 || usage.model_id.is_empty() {
                            usage.model_id = s.to_string();
                        }
                    }
                }
            }
            1 => pos += 8,
            5 => pos += 4,
            _ => break,
        }
    }
}

fn parse_token_breakdown(blob: &[u8], usage: &mut AntigravityGenTokenUsage) {
    let mut pos = 0;
    while pos < blob.len() {
        let Some((tag, n)) = read_varint(&blob[pos..]) else {
            break;
        };
        pos += n;
        let field_num = tag >> 3;
        let wire_type = tag & 0x07;

        if wire_type == 0 {
            let Some((val, n)) = read_varint(&blob[pos..]) else {
                break;
            };
            pos += n;
            match field_num {
                1 => usage.input_tokens = val as i64,
                2 => usage.output_tokens = val as i64,
                3 => usage.cached_tokens = val as i64,
                9 => usage.reasoning_tokens = val as i64,
                _ => {}
            }
        } else if wire_type == 2 {
            let Some((len, n)) = read_varint(&blob[pos..]) else {
                break;
            };
            pos += n + len as usize;
        } else if wire_type == 1 {
            pos += 8;
        } else if wire_type == 5 {
            pos += 4;
        } else {
            break;
        }
    }
}

fn read_varint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0;
    for (i, &byte) in buf.iter().enumerate() {
        result |= ((byte & 0x7f) as u64) << shift;
        if (byte & 0x80) == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

pub fn parse_antigravity_db() -> Option<AntigravityStats> {
    let base_path = get_antigravity_storage_path()?;
    let summaries_db = base_path.join("conversation_summaries.db");
    if !summaries_db.exists() {
        return None;
    }

    let conn = Connection::open(&summaries_db).ok()?;
    let mut stmt = conn
        .prepare("SELECT conversation_id, last_modified_time FROM conversation_summaries")
        .ok()?;

    let mut stats = AntigravityStats::default();

    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let mod_time: String = row.get(1)?;
            Ok((id, mod_time))
        })
        .ok()?;

    for row in rows.flatten() {
        let (conv_id, time_str) = row;
        let session_db_path = base_path.join(format!("conversations/{}.db", conv_id));
        if !session_db_path.exists() {
            continue;
        }

        stats.sessions_found += 1;

        let date_key = DateTime::parse_from_rfc3339(&time_str)
            .map(|dt| dt.with_timezone(&Local).format("%Y-%m-%d").to_string())
            .unwrap_or_else(|_| Local::now().format("%Y-%m-%d").to_string());

        let month_key = if date_key.len() >= 7 {
            date_key[..7].to_string()
        } else {
            "unknown".to_string()
        };

        if let Ok(session_conn) = Connection::open(&session_db_path) {
            if let Ok(mut gen_stmt) = session_conn.prepare("SELECT data FROM gen_metadata") {
                if let Ok(blobs) = gen_stmt.query_map([], |r| r.get::<_, Vec<u8>>(0)) {
                    for blob_res in blobs.flatten() {
                        if let Some(usage) = parse_cortex_generator_metadata(&blob_res) {
                            let prices = get_pricing(&usage.model_id, usage.input_tokens);

                            let cost = (usage.input_tokens as f64 / 1_000_000.0) * prices.input
                                + (usage.output_tokens as f64 / 1_000_000.0) * prices.output
                                + (usage.cached_tokens as f64 / 1_000_000.0) * prices.cache_read;

                            stats.total_cost += cost;

                            *stats.daily_costs.entry(date_key.clone()).or_insert(0.0) += cost;
                            *stats.monthly_costs.entry(month_key.clone()).or_insert(0.0) += cost;

                            let token_stat = TokenStats {
                                in_tokens: usage.input_tokens,
                                out_tokens: usage.output_tokens,
                                cache_read_tokens: usage.cached_tokens,
                                cache_create_tokens: 0,
                            };

                            stats
                                .daily_stats
                                .entry(date_key.clone())
                                .or_default()
                                .add(&token_stat);
                            stats
                                .monthly_stats
                                .entry(month_key.clone())
                                .or_default()
                                .add(&token_stat);
                            stats
                                .model_stats
                                .entry(usage.model_id)
                                .or_default()
                                .add(&token_stat);
                        }
                    }
                }
            }
        }
    }

    Some(stats)
}

pub fn run_antigravity_report() -> Option<(AntigravityStats, f64)> {
    let start_time = Instant::now();
    let stats = parse_antigravity_db()?;
    let parsing_time = start_time.elapsed().as_secs_f64();
    Some((stats, parsing_time))
}

pub fn print_antigravity_report(
    global_stats: &AntigravityStats,
    parsing_time: f64,
    daily_days: usize,
) {
    println!("\n{}", "=".repeat(105));
    println!(
        "{}📊 ANTIGRAVITY CLI USAGE & COST ESTIMATE{}",
        TERM_HEADER, TERM_RESET
    );
    println!("{}", "=".repeat(105));
    println!(
        "{}Sessions Scanned:{} {}",
        TERM_BOLD, TERM_RESET, global_stats.sessions_found
    );
    println!("{}", "-".repeat(105));

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
            if month == "Unknown" || month == "unknown" {
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
            if day == "Unknown" || day == "unknown" {
                continue;
            }
            print_cost_bar(&format!("{:<12}", day), *cost, max_day_cost, 35);
        }
    }

    let total_cost = global_stats.total_cost;
    let mut total_tokens = TokenStats::default();
    for s in global_stats.model_stats.values() {
        total_tokens.add(s);
    }

    println!("\n{}", "=".repeat(50));
    println!(
        "{}GRAND TOTALS (ANTIGRAVITY CLI){}",
        TERM_HEADER, TERM_RESET
    );
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_varint() {
        assert_eq!(read_varint(&[0x00]), Some((0, 1)));
        assert_eq!(read_varint(&[0x01]), Some((1, 1)));
        assert_eq!(read_varint(&[0xaf, 0x08]), Some((1071, 2)));
        assert_eq!(read_varint(&[0x80, 0x01]), Some((128, 2)));
    }

    #[test]
    fn test_parse_metadata() {
        // Construct standard CortexGeneratorMetadata blob
        // Tag 1 (Field 1, len 45 bytes):
        // 0x0a, 0x2d
        // Inside Field 1:
        // Tag 3 (Varint, 1071): 0x18, 0xaf, 0x08
        // Tag 4 (Field 4 SubMessage, len 20 bytes): 0x22, 0x14
        // Inside Field 4:
        // Tag 1 (Input tokens, 1071): 0x08, 0xaf, 0x08
        // Tag 2 (Output tokens, 2080): 0x10, 0xa0, 0x10
        // Tag 3 (Cached tokens, 708): 0x18, 0xc4, 0x05
        // Tag 9 (Reasoning tokens, 473): 0x48, 0xd9, 0x03
        // Back to Field 1:
        // Tag 19 (Model id, string "gemini-3.6-flash"):
        // Tag is 19 << 3 | 2 = 154 = 0x9a, 0x01
        // Length: 16 = 0x10
        // Bytes: "gemini-3.6-flash"

        let mut blob = vec![
            0x0a, 0x24, // Field 1 tag & len (36 bytes)
            0x18, 0xaf, 0x08, // Field 3 varint 1071
            0x22, 0x0c, // Field 4 submessage tag & len (12 bytes)
            0x08, 0xaf, 0x08, // input 1071
            0x10, 0xa0, 0x10, // output 2080
            0x18, 0xc4, 0x05, // cached 708
            0x48, 0xd9, 0x03, // reasoning 473
            0x9a, 0x01, 0x10, // Field 19 tag & len
        ];
        blob.extend_from_slice(b"gemini-3.6-flash");

        let res = parse_cortex_generator_metadata(&blob).unwrap();
        assert_eq!(res.model_id, "gemini-3.6-flash");
        assert_eq!(res.input_tokens, 1071);
        assert_eq!(res.output_tokens, 2080);
        assert_eq!(res.cached_tokens, 708);
        assert_eq!(res.reasoning_tokens, 473);
    }
}
