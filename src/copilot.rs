use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::colors::*;
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::viz::{TokenStats, print_cost_bar, print_token_bar};

#[derive(Default, Clone, Debug)]
pub struct CopilotStats {
    pub daily_costs: BTreeMap<String, f64>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub total_cost: f64,
    pub daily_stats: BTreeMap<String, TokenStats>,
    pub monthly_stats: BTreeMap<String, TokenStats>,
    pub model_stats: HashMap<String, TokenStats>,
    pub threads_found: usize,
    pub languages: crate::languages::LanguageAnalyzer,
}

#[derive(Deserialize, Debug)]
struct CopilotModelMetadata {
    #[allow(dead_code)]
    name: Option<String>,
    #[serde(rename = "inputCost")]
    input_cost: Option<f64>,
    #[serde(rename = "outputCost")]
    output_cost: Option<f64>,
    #[serde(rename = "cacheCost")]
    #[allow(dead_code)]
    cache_cost: Option<f64>,
}

#[derive(Deserialize, Debug)]
struct CopilotSelectedModel {
    identifier: Option<String>,
    metadata: Option<CopilotModelMetadata>,
}

#[derive(Deserialize, Debug)]
struct CopilotInputState {
    #[serde(rename = "selectedModel")]
    selected_model: Option<CopilotSelectedModel>,
}

#[derive(Deserialize, Debug)]
struct CopilotResultMetadata {
    #[serde(rename = "promptTokens")]
    prompt_tokens: Option<i64>,
    #[serde(rename = "outputTokens")]
    output_tokens: Option<i64>,
    #[allow(dead_code)]
    details: Option<String>,
}

#[derive(Deserialize, Debug)]
struct CopilotResult {
    metadata: Option<CopilotResultMetadata>,
}

#[derive(Deserialize, Debug)]
struct CopilotRequest {
    #[serde(rename = "requestId")]
    #[allow(dead_code)]
    request_id: String,
    result: Option<CopilotResult>,
    #[serde(rename = "inputState")]
    input_state: Option<CopilotInputState>,
}

#[derive(Deserialize, Debug)]
struct CopilotSessionData {
    #[serde(rename = "sessionId")]
    #[allow(dead_code)]
    session_id: Option<String>,
    #[serde(default)]
    requests: Vec<CopilotRequest>,
}

#[derive(Deserialize, Debug)]
struct CopilotLine {
    v: CopilotSessionData,
}

pub fn get_copilot_storage_path() -> Option<PathBuf> {
    let base = dirs::home_dir()?;
    let ext_path = "Code/User/workspaceStorage";

    #[cfg(target_os = "macos")]
    let path = base.join("Library/Application Support").join(ext_path);

    #[cfg(target_os = "linux")]
    let path = base.join(".config").join(ext_path);

    #[cfg(target_os = "windows")]
    let path = {
        let appdata = std::env::var_os("APPDATA")?;
        PathBuf::from(appdata).join(ext_path)
    };

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let path = return None;

    Some(path)
}

pub fn get_copilot_files() -> Vec<PathBuf> {
    let base_dir = match get_copilot_storage_path() {
        Some(d) => d,
        None => return Vec::new(),
    };

    if !base_dir.is_dir() {
        return Vec::new();
    }

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&base_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let chat_sessions_dir = entry.path().join("chatSessions");
                if chat_sessions_dir.is_dir() {
                    if let Ok(session_files) = std::fs::read_dir(chat_sessions_dir) {
                        for s_file in session_files.flatten() {
                            if s_file.path().extension().and_then(|e| e.to_str()) == Some("jsonl") {
                                files.push(s_file.path());
                            }
                        }
                    }
                }
            }
        }
    }
    files
}

pub fn parse_copilot_file(file_path: &Path) -> CopilotStats {
    let mut local = CopilotStats::default();

    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return local,
    };

    let mut parsed_line: Option<CopilotLine> = None;
    for line in content.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(p) = serde_json::from_str::<CopilotLine>(line) {
            parsed_line = Some(p);
            break;
        }
    }

    let Some(parsed_line) = parsed_line else {
        return local;
    };

    local.threads_found = 1;

    let mtime = std::fs::metadata(file_path)
        .and_then(|m| m.modified())
        .unwrap_or_else(|_| SystemTime::now());
    let datetime: chrono::DateTime<chrono::Utc> = mtime.into();
    let date_key = datetime.format("%Y-%m-%d").to_string();
    let month_key = datetime.format("%Y-%m").to_string();

    for request in parsed_line.v.requests {
        let mut in_tokens = 0;
        let mut out_tokens = 0;

        if let Some(res) = &request.result {
            if let Some(meta) = &res.metadata {
                in_tokens = meta.prompt_tokens.unwrap_or(0);
                out_tokens = meta.output_tokens.unwrap_or(0);
            }
        }

        if in_tokens == 0 && out_tokens == 0 {
            continue;
        }

        let (model_name, input_cost, output_cost) = if let Some(input_state) = &request.input_state
        {
            if let Some(selected_model) = &input_state.selected_model {
                let id = selected_model
                    .identifier
                    .clone()
                    .unwrap_or_else(|| "copilot/unknown".to_string());
                let (in_c, out_c) = if let Some(meta) = &selected_model.metadata {
                    (meta.input_cost, meta.output_cost)
                } else {
                    (None, None)
                };
                (id, in_c, out_c)
            } else {
                ("copilot/unknown".to_string(), None, None)
            }
        } else {
            ("copilot/unknown".to_string(), None, None)
        };

        let cost = match (input_cost, output_cost) {
            (Some(in_c), Some(out_c)) => {
                let usd_in_price = in_c * 0.01;
                let usd_out_price = out_c * 0.01;
                (in_tokens as f64 / 1_000_000.0 * usd_in_price)
                    + (out_tokens as f64 / 1_000_000.0 * usd_out_price)
            }
            _ => {
                let pricing = crate::pricing::get_pricing(&model_name, in_tokens);
                (in_tokens as f64 / 1_000_000.0 * pricing.input)
                    + (out_tokens as f64 / 1_000_000.0 * pricing.output)
            }
        };

        let entry = TokenStats {
            in_tokens,
            out_tokens,
            cache_read_tokens: 0,
            cache_create_tokens: 0,
        };

        *local.daily_costs.entry(date_key.clone()).or_insert(0.0) += cost;
        *local.monthly_costs.entry(month_key.clone()).or_insert(0.0) += cost;
        local.total_cost += cost;

        local
            .daily_stats
            .entry(date_key.clone())
            .or_default()
            .add(&entry);
        local
            .monthly_stats
            .entry(month_key.clone())
            .or_default()
            .add(&entry);
        local.model_stats.entry(model_name).or_default().add(&entry);
    }

    local
}

pub fn merge_copilot_stats(mut a: CopilotStats, b: CopilotStats) -> CopilotStats {
    a.threads_found += b.threads_found;
    a.total_cost += b.total_cost;
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

    a
}

pub fn run_copilot_report() -> Option<(CopilotStats, f64)> {
    use rayon::prelude::*;
    use std::time::Instant;

    let start_time = Instant::now();
    let session_files = get_copilot_files();
    if session_files.is_empty() {
        return None;
    }

    let global_stats = session_files
        .par_iter()
        .map(|file_path| parse_copilot_file(file_path))
        .reduce(CopilotStats::default, merge_copilot_stats);

    let parsing_time = start_time.elapsed().as_secs_f64();
    if global_stats.threads_found == 0 {
        return None;
    }

    Some((global_stats, parsing_time))
}

pub fn print_copilot_report(global_stats: &CopilotStats, parsing_time: f64, daily_days: usize) {
    println!("\n{}", "=".repeat(105));
    println!(
        "{}📊 COPILOT CHAT USAGE & COST ESTIMATE{}",
        TERM_HEADER, TERM_RESET
    );
    println!("{}", "=".repeat(105));
    println!(
        "{}Sessions Scanned:{} {}",
        TERM_BOLD, TERM_RESET, global_stats.threads_found
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
        let max_model_tokens = global_stats
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
                max_model_tokens,
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
            print_cost_bar(&format!("{:<12}", day), *cost, max_day_cost, 35);
        }
    }

    let mut total_tokens = TokenStats::default();
    for v in global_stats.model_stats.values() {
        total_tokens.add(v);
    }

    println!("\n{}", "=".repeat(50));
    println!("{}GRAND TOTALS (COPILOT CHAT){}", TERM_HEADER, TERM_RESET);
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
        "  {}Total:        {:>12}{}",
        TERM_BOLD,
        format_int_with_commas(total_tokens.total()),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Cost:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {} ${}{}",
        TERM_RED,
        format_float_with_commas(global_stats.total_cost),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Performance:{}", TERM_BOLD, TERM_RESET);
    println!("  Sessions Scanned: {}", global_stats.threads_found);
    println!("  Parse Time:       {:.6} seconds", parsing_time);
    println!("{}", "=".repeat(50));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_copilot_parser_basic() {
        let mut file = NamedTempFile::new().unwrap();
        let payload = r#"{"v":{"version":3,"sessionId":"test-session-123","requests":[{"requestId":"req1","result":{"metadata":{"promptTokens":100,"outputTokens":50,"details":"Gemini 3.5 Flash"}},"inputState":{"selectedModel":{"identifier":"copilot/gemini-3.5-flash","metadata":{"name":"Gemini 3.5 Flash","inputCost":150,"outputCost":900,"cacheCost":15}}}}]}}"#;
        writeln!(file, "{}", payload).unwrap();

        let stats = parse_copilot_file(file.path());
        assert_eq!(stats.threads_found, 1);
        assert_eq!(
            stats
                .model_stats
                .get("copilot/gemini-3.5-flash")
                .unwrap()
                .in_tokens,
            100
        );
        assert_eq!(
            stats
                .model_stats
                .get("copilot/gemini-3.5-flash")
                .unwrap()
                .out_tokens,
            50
        );

        // inputCost 150 -> 1.50 per 1M. outputCost 900 -> 9.00 per 1M.
        // 100 in_tokens -> 100 / 1_000_000 * 1.50 = 0.00015
        // 50 out_tokens -> 50 / 1_000_000 * 9.00 = 0.00045
        // Total cost: 0.00060
        assert!((stats.total_cost - 0.00060).abs() < 1e-9);
    }

    #[test]
    fn test_copilot_parser_fallback_pricing() {
        let mut file = NamedTempFile::new().unwrap();
        // Here, metadata doesn't contain inputCost/outputCost, so it should fallback to pricing.rs (which defaults to gpt-4o pricing or Sonnet pricing, or we get Copilot/unknown)
        let payload = r#"{"v":{"version":3,"sessionId":"test-session-456","requests":[{"requestId":"req1","result":{"metadata":{"promptTokens":1000,"outputTokens":500}},"inputState":{"selectedModel":{"identifier":"gpt-4o"}}}]}}"#;
        writeln!(file, "{}", payload).unwrap();

        let stats = parse_copilot_file(file.path());
        assert_eq!(stats.threads_found, 1);
        assert!(stats.total_cost > 0.0);
    }
}
