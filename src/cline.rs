use chrono::{TimeZone, Utc};
use rayon::prelude::*;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;
use walkdir::WalkDir;

use crate::colors::*;
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::viz::{TokenStats, print_cost_bar, print_token_bar};

#[derive(Deserialize)]
struct ClineModelUsage {
    ts: Option<f64>,
    model_id: Option<String>,
}

#[derive(Deserialize)]
struct ClineTaskMetadata {
    model_usage: Option<Vec<ClineModelUsage>>,
}

#[derive(Deserialize)]
struct ClineMessage {
    ts: Option<f64>,
    say: Option<String>,
    text: Option<String>,
    #[serde(rename = "tokensIn")]
    tokens_in: Option<i64>,
    #[serde(rename = "tokensOut")]
    tokens_out: Option<i64>,
    #[serde(rename = "cacheReads")]
    cache_reads: Option<i64>,
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct ClineApiReqData {
    cost: Option<f64>,
    #[serde(rename = "tokensIn")]
    tokens_in: Option<i64>,
    #[serde(rename = "tokensOut")]
    tokens_out: Option<i64>,
    #[serde(rename = "cacheReads")]
    cache_reads: Option<i64>,
}

#[derive(Default, Clone)]
pub struct ClineStats {
    pub daily_costs: BTreeMap<String, f64>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub monthly_model_costs: BTreeMap<String, HashMap<String, f64>>,
    pub total_cost: f64,
    pub daily_tokens: BTreeMap<String, TokenStats>,
    pub monthly_tokens: BTreeMap<String, TokenStats>,
    pub monthly_model_tokens: BTreeMap<String, HashMap<String, TokenStats>>,
    pub total_tokens: TokenStats,
    pub files_found: u32,
}

fn merge_cline_stats(mut a: ClineStats, b: ClineStats) -> ClineStats {
    a.total_cost += b.total_cost;
    a.total_tokens.add(&b.total_tokens);
    a.files_found += b.files_found;

    for (k, v) in b.daily_costs {
        *a.daily_costs.entry(k).or_insert(0.0) += v;
    }
    for (k, v) in b.monthly_costs {
        *a.monthly_costs.entry(k).or_insert(0.0) += v;
    }
    for (month, models) in b.monthly_model_costs {
        let a_models = a.monthly_model_costs.entry(month).or_default();
        for (model, cost) in models {
            *a_models.entry(model).or_insert(0.0) += cost;
        }
    }
    for (k, v) in b.daily_tokens {
        a.daily_tokens.entry(k).or_default().add(&v);
    }
    for (k, v) in b.monthly_tokens {
        a.monthly_tokens.entry(k).or_default().add(&v);
    }
    for (month, models) in b.monthly_model_tokens {
        let a_models = a.monthly_model_tokens.entry(month).or_default();
        for (model, v) in models {
            a_models.entry(model).or_default().add(&v);
        }
    }
    a
}

pub fn get_cline_storage_path() -> Option<PathBuf> {
    let base = dirs::home_dir()?;
    let ext_path = "globalStorage/saoudrizwan.claude-dev/tasks";

    #[cfg(target_os = "macos")]
    let path = base
        .join("Library/Application Support/Code/User")
        .join(ext_path);

    #[cfg(target_os = "linux")]
    let path = base.join(".config/Code/User").join(ext_path);

    #[cfg(target_os = "windows")]
    let path = {
        let appdata = std::env::var_os("APPDATA")?;
        PathBuf::from(appdata).join("Code/User").join(ext_path)
    };

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let path = return None;

    Some(path)
}

pub fn parse_cline_file(
    path: &std::path::Path,
    exclude_claude: bool,
    exclude_gemini: bool,
) -> ClineStats {
    let mut local = ClineStats {
        files_found: 1,
        ..ClineStats::default()
    };

    let task_dir = path.parent().unwrap();
    let metadata_path = task_dir.join("task_metadata.json");

    let mut model_usages = Vec::new();
    if metadata_path.is_file() {
        if let Ok(file) = fs::File::open(&metadata_path) {
            let reader = std::io::BufReader::new(file);
            if let Ok(metadata) = serde_json::from_reader::<_, ClineTaskMetadata>(reader) {
                if let Some(usages) = metadata.model_usage {
                    for u in usages {
                        if let (Some(ts), Some(id)) = (u.ts, u.model_id) {
                            model_usages.push((ts as i64, id));
                        }
                    }
                }
            }
        }
    }

    if let Ok(file) = fs::File::open(path) {
        let reader = std::io::BufReader::new(file);
        if let Ok(messages) = serde_json::from_reader::<_, Vec<ClineMessage>>(reader) {
            for message in messages {
                let mut cost = 0.0;
                let mut t_in = 0;
                let mut t_out = 0;
                let mut c_read = 0;

                let timestamp_ms = match message.ts {
                    Some(t) => t as i64,
                    None => continue,
                };

                if message.say.as_deref() == Some("api_req_started") {
                    if let Some(text) = message.text {
                        if let Ok(data) = serde_json::from_str::<ClineApiReqData>(&text) {
                            cost = data.cost.unwrap_or(0.0);
                            t_in = data.tokens_in.unwrap_or(0);
                            t_out = data.tokens_out.unwrap_or(0);
                            c_read = data.cache_reads.unwrap_or(0);
                        }
                    }
                } else if message.tokens_in.is_some()
                    || message.tokens_out.is_some()
                    || message.cost.is_some()
                {
                    t_in = message.tokens_in.unwrap_or(0);
                    t_out = message.tokens_out.unwrap_or(0);
                    c_read = message.cache_reads.unwrap_or(0);
                    cost = message.cost.unwrap_or(0.0);
                }

                if (cost > 0.0 || t_in + t_out + c_read > 0) && timestamp_ms > 0 {
                    let mut best_model = "unknown-model".to_string();
                    let mut min_time_diff = i64::MAX;

                    for (u_ts, u_id) in &model_usages {
                        let time_diff = timestamp_ms - u_ts;
                        if time_diff >= 0 && time_diff < min_time_diff {
                            min_time_diff = time_diff;
                            best_model = u_id.clone();
                        }
                    }

                    let model_check = best_model.to_lowercase();
                    if exclude_claude && model_check.contains("claude") {
                        continue;
                    }
                    if exclude_gemini && model_check.contains("gemini") {
                        continue;
                    }

                    let dt = Utc.timestamp_opt(timestamp_ms / 1000, 0).unwrap();
                    let date_key = dt.format("%Y-%m-%d").to_string();
                    let month_key = dt.format("%Y-%m").to_string();

                    let entry = TokenStats {
                        in_tokens: t_in,
                        out_tokens: t_out,
                        cache_read_tokens: c_read,
                        cache_create_tokens: 0,
                    };

                    *local.daily_costs.entry(date_key.clone()).or_insert(0.0) += cost;
                    *local.monthly_costs.entry(month_key.clone()).or_insert(0.0) += cost;
                    *local
                        .monthly_model_costs
                        .entry(month_key.clone())
                        .or_default()
                        .entry(best_model.clone())
                        .or_insert(0.0) += cost;
                    local.total_cost += cost;

                    local.daily_tokens.entry(date_key).or_default().add(&entry);
                    local
                        .monthly_tokens
                        .entry(month_key.clone())
                        .or_default()
                        .add(&entry);
                    local
                        .monthly_model_tokens
                        .entry(month_key)
                        .or_default()
                        .entry(best_model)
                        .or_default()
                        .add(&entry);
                    local.total_tokens.add(&entry);
                }
            }
        }
    }
    local
}

pub fn get_cline_files() -> Vec<PathBuf> {
    let tasks_path = match get_cline_storage_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    if !tasks_path.is_dir() {
        return Vec::new();
    }
    WalkDir::new(&tasks_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() == "ui_messages.json")
        .map(|e| e.path().to_path_buf())
        .collect()
}

pub fn run_cline_report(exclude_claude: bool, exclude_gemini: bool) -> Option<(ClineStats, f64)> {
    let start_time = Instant::now();
    let paths = get_cline_files();
    if paths.is_empty() {
        return None;
    }

    let global_stats = paths
        .par_iter()
        .map(|path| parse_cline_file(path, exclude_claude, exclude_gemini))
        .reduce(ClineStats::default, merge_cline_stats);

    let parsing_time = start_time.elapsed().as_secs_f64();

    if global_stats.daily_costs.is_empty() && global_stats.total_tokens.in_tokens == 0 {
        return None;
    }

    Some((global_stats, parsing_time))
}

pub fn print_cline_report(global_stats: &ClineStats, parsing_time: f64, daily_days: usize) {
    println!("\n{}", "=".repeat(95));
    println!(
        "{}📊 CLINE USAGE & COST ESTIMATE{}",
        TERM_HEADER, TERM_RESET
    );
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
    let max_monthly_tokens = global_stats
        .monthly_tokens
        .values()
        .map(|s| s.total())
        .max()
        .unwrap_or(0);
    for (month, stats) in &global_stats.monthly_tokens {
        print_token_bar(
            &format!("{:^10}", month),
            stats,
            max_monthly_tokens,
            35,
            false,
        );
    }

    if !global_stats.monthly_model_tokens.is_empty() {
        println!(
            "\n{}--- Monthly Token Usage by Model ---{}",
            TERM_BOLD, TERM_RESET
        );
        let mut global_max_model_tokens = 0;
        for models in global_stats.monthly_model_tokens.values() {
            for stats in models.values() {
                if stats.total() > global_max_model_tokens {
                    global_max_model_tokens = stats.total();
                }
            }
        }

        for (month, models) in &global_stats.monthly_model_tokens {
            println!("{}{}{}:", TERM_CYAN, month, TERM_RESET);
            let mut sorted_models: Vec<_> = models.iter().collect();
            sorted_models.sort_by(|a, b| b.1.total().cmp(&a.1.total()));
            for (model, stats) in sorted_models {
                print_token_bar(
                    &format!("  {:<35}", model),
                    stats,
                    global_max_model_tokens,
                    35,
                    false,
                );
            }
        }
    }

    println!(
        "\n{}--- Daily Token Usage (Last {} Days) ---{}",
        TERM_BOLD, daily_days, TERM_RESET
    );
    let mut sorted_days: Vec<_> = global_stats.daily_tokens.iter().collect();
    sorted_days.sort_by(|a, b| a.0.cmp(b.0));
    let last_n_days: Vec<_> = sorted_days.into_iter().rev().take(daily_days).collect();
    let last_n_days: Vec<_> = last_n_days.into_iter().rev().collect();

    let max_daily_tokens = last_n_days
        .iter()
        .map(|(_, s)| s.total())
        .max()
        .unwrap_or(0);
    for (day, stats) in last_n_days {
        print_token_bar(&format!("{:<10}", day), stats, max_daily_tokens, 35, false);
    }

    println!("\n\n{}=== FINANCIAL COSTS ==={}", TERM_HEADER, TERM_RESET);

    println!(
        "\n{}--- Daily Costs (Last {} Days) ---{}",
        TERM_BOLD, daily_days, TERM_RESET
    );
    let mut sorted_days_cost: Vec<_> = global_stats.daily_costs.iter().collect();
    sorted_days_cost.sort_by(|a, b| a.0.cmp(b.0));
    let last_n_days_cost: Vec<_> = sorted_days_cost
        .into_iter()
        .rev()
        .take(daily_days)
        .collect();
    let last_n_days_cost: Vec<_> = last_n_days_cost.into_iter().rev().collect();

    let max_daily_cost = last_n_days_cost
        .iter()
        .map(|(_, c)| *c)
        .fold(0.0_f64, |a, b| a.max(*b));
    for (day, &cost) in last_n_days_cost {
        print_cost_bar(&format!("{:<10}", day), cost, max_daily_cost, 35);
    }

    println!("\n{}--- Monthly Costs ---{}", TERM_BOLD, TERM_RESET);
    let max_monthly_cost = global_stats
        .monthly_costs
        .values()
        .copied()
        .fold(0.0_f64, |a, b| a.max(b));
    for (month, &cost) in &global_stats.monthly_costs {
        print_cost_bar(&format!("{:^10}", month), cost, max_monthly_cost, 35);
    }

    println!("\n{}", "=".repeat(50));
    println!("{}GRAND TOTALS (CLINE){}", TERM_HEADER, TERM_RESET);
    println!("{}", "-".repeat(50));

    println!("{}Tokens:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  Input:       {:>12}",
        format_int_with_commas(global_stats.total_tokens.in_tokens)
    );
    println!(
        "  Output:      {:>12}",
        format_int_with_commas(global_stats.total_tokens.out_tokens)
    );
    println!(
        "  Cache Reads: {:>12}",
        format_int_with_commas(global_stats.total_tokens.cache_read_tokens)
    );
    println!(
        "  {}Total:       {:>12}{}",
        TERM_BOLD,
        format_int_with_commas(global_stats.total_tokens.total()),
        TERM_RESET
    );

    println!("{}", "-".repeat(50));
    println!("{}Cost:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {}${}{}",
        TERM_GREEN,
        format_float_with_commas(global_stats.total_cost),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Performance:{}", TERM_BOLD, TERM_RESET);
    println!("  Files Parsed: {}", global_stats.files_found);
    println!("  Parse Time:   {:.2} seconds", parsing_time);
    println!("{}", "=".repeat(50));
}
