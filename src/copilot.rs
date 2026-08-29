use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use chrono::{DateTime, Local};

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

#[derive(Default, Clone, Debug)]
struct RequestState {
    model_id: Option<String>,
    input_cost: Option<f64>,
    output_cost: Option<f64>,
    prompt_tokens: i64,
    output_tokens: i64,
    timestamp: Option<i64>,
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
    let mut files = Vec::new();
    let base_dir = match get_copilot_storage_path() {
        Some(d) => d,
        None => return files,
    };

    // 1. Scan workspaceStorage/*/chatSessions/*.jsonl
    if base_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&base_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let chat_sessions_dir = entry.path().join("chatSessions");
                    if chat_sessions_dir.is_dir() {
                        if let Ok(session_files) = std::fs::read_dir(chat_sessions_dir) {
                            for s_file in session_files.flatten() {
                                if s_file.path().extension().and_then(|e| e.to_str())
                                    == Some("jsonl")
                                {
                                    files.push(s_file.path());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Scan globalStorage/emptyWindowChatSessions/*.jsonl
    if let Some(user_dir) = base_dir.parent() {
        let empty_window_dir = user_dir
            .join("globalStorage")
            .join("emptyWindowChatSessions");
        if empty_window_dir.is_dir() {
            if let Ok(session_files) = std::fs::read_dir(empty_window_dir) {
                for s_file in session_files.flatten() {
                    if s_file.path().extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        files.push(s_file.path());
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

    let mut requests: BTreeMap<usize, RequestState> = BTreeMap::new();
    let mut default_model: Option<String> = None;
    let mut default_input_cost: Option<f64> = None;
    let mut default_output_cost: Option<f64> = None;
    let mut session_creation_date: Option<i64> = None;
    let mut valid_file_parsed = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let val: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        valid_file_parsed = true;

        let kind = val.get("kind").and_then(|k| k.as_i64());
        let k_arr = val.get("k").and_then(|k| k.as_array());
        let v_val = val.get("v");

        if kind == Some(0) || kind.is_none() {
            // Process initial state snapshot
            let target_obj = if kind.is_none() {
                &val
            } else {
                v_val.unwrap_or(&val)
            };
            let session_data = target_obj.get("v").unwrap_or(target_obj);

            if let Some(cdate) = session_data
                .get("creationDate")
                .or_else(|| target_obj.get("creationDate"))
                .and_then(|d| d.as_i64())
            {
                session_creation_date = Some(cdate);
            }

            if let Some(reqs) = session_data.get("requests").and_then(|r| r.as_array()) {
                for (idx, r_val) in reqs.iter().enumerate() {
                    let mut r_state = RequestState::default();
                    if let Some(ts) = r_val.get("timestamp").and_then(|t| t.as_i64()) {
                        r_state.timestamp = Some(ts);
                    } else if let Some(ts) = r_val
                        .get("modelState")
                        .and_then(|m| m.get("completedAt"))
                        .and_then(|t| t.as_i64())
                    {
                        r_state.timestamp = Some(ts);
                    }
                    if let Some(m_id) = r_val.get("modelId").and_then(|m| m.as_str()) {
                        r_state.model_id = Some(m_id.to_string());
                    }
                    if let Some(res) = r_val.get("result") {
                        if let Some(meta) = res.get("metadata") {
                            r_state.prompt_tokens = meta
                                .get("promptTokens")
                                .and_then(|t| t.as_i64())
                                .unwrap_or(0);
                            r_state.output_tokens = meta
                                .get("completionTokens")
                                .or_else(|| meta.get("outputTokens"))
                                .and_then(|t| t.as_i64())
                                .unwrap_or(0);
                        }
                    }
                    if let Some(in_state) = r_val.get("inputState") {
                        if let Some(sel_model) = in_state.get("selectedModel") {
                            if r_state.model_id.is_none() {
                                if let Some(m_id) =
                                    sel_model.get("identifier").and_then(|i| i.as_str())
                                {
                                    r_state.model_id = Some(m_id.to_string());
                                }
                            }
                            if let Some(meta) = sel_model.get("metadata") {
                                r_state.input_cost = meta.get("inputCost").and_then(|c| c.as_f64());
                                r_state.output_cost =
                                    meta.get("outputCost").and_then(|c| c.as_f64());
                            }
                        }
                    }
                    requests.insert(idx, r_state);
                }
            }
        } else if let Some(k) = k_arr {
            if k.len() == 1 && k[0] == "creationDate" {
                if let Some(v_i64) = v_val.and_then(|v| v.as_i64()) {
                    session_creation_date = Some(v_i64);
                }
            } else if k.len() == 1 && k[0] == "requests" {
                if let Some(reqs) = v_val.and_then(|v| v.as_array()) {
                    for (idx, r_val) in reqs.iter().enumerate() {
                        let r_entry = requests.entry(idx).or_default();
                        if let Some(ts) = r_val.get("timestamp").and_then(|t| t.as_i64()) {
                            r_entry.timestamp = Some(ts);
                        } else if let Some(ts) = r_val
                            .get("modelState")
                            .and_then(|m| m.get("completedAt"))
                            .and_then(|t| t.as_i64())
                        {
                            if r_entry.timestamp.is_none() {
                                r_entry.timestamp = Some(ts);
                            }
                        }
                        if let Some(m_id) = r_val.get("modelId").and_then(|m| m.as_str()) {
                            r_entry.model_id = Some(m_id.to_string());
                        }
                        if let Some(res) = r_val.get("result") {
                            if let Some(meta) = res.get("metadata") {
                                if let Some(prompt) =
                                    meta.get("promptTokens").and_then(|p| p.as_i64())
                                {
                                    r_entry.prompt_tokens = prompt;
                                }
                                if let Some(output) = meta
                                    .get("outputTokens")
                                    .or_else(|| meta.get("completionTokens"))
                                    .and_then(|o| o.as_i64())
                                {
                                    r_entry.output_tokens = output;
                                }
                            }
                        }
                        if let Some(in_state) = r_val.get("inputState") {
                            if let Some(sel_model) = in_state.get("selectedModel") {
                                if r_entry.model_id.is_none() {
                                    if let Some(m_id) =
                                        sel_model.get("identifier").and_then(|i| i.as_str())
                                    {
                                        r_entry.model_id = Some(m_id.to_string());
                                    }
                                }
                                if let Some(meta) = sel_model.get("metadata") {
                                    if r_entry.input_cost.is_none() {
                                        r_entry.input_cost =
                                            meta.get("inputCost").and_then(|c| c.as_f64());
                                    }
                                    if r_entry.output_cost.is_none() {
                                        r_entry.output_cost =
                                            meta.get("outputCost").and_then(|c| c.as_f64());
                                    }
                                }
                            }
                        }
                    }
                }
            } else if k.len() >= 3 && k[0] == "requests" {
                if let Some(idx) = k[1].as_u64().map(|u| u as usize) {
                    let r_entry = requests.entry(idx).or_default();
                    if k.len() == 3 {
                        if let Some(prop) = k[2].as_str() {
                            match prop {
                                "timestamp" => {
                                    if let Some(v_i64) = v_val.and_then(|v| v.as_i64()) {
                                        r_entry.timestamp = Some(v_i64);
                                    }
                                }
                                "promptTokens" => {
                                    if let Some(v_i64) = v_val.and_then(|v| v.as_i64()) {
                                        r_entry.prompt_tokens = v_i64;
                                    }
                                }
                                "completionTokens" | "outputTokens" => {
                                    if let Some(v_i64) = v_val.and_then(|v| v.as_i64()) {
                                        r_entry.output_tokens = v_i64;
                                    }
                                }
                                "modelId" => {
                                    if let Some(v_str) = v_val.and_then(|v| v.as_str()) {
                                        r_entry.model_id = Some(v_str.to_string());
                                    }
                                }
                                "result" => {
                                    if let Some(v_obj) = v_val {
                                        if let Some(meta) = v_obj.get("metadata") {
                                            if let Some(prompt) =
                                                meta.get("promptTokens").and_then(|p| p.as_i64())
                                            {
                                                r_entry.prompt_tokens = prompt;
                                            }
                                            if let Some(output) = meta
                                                .get("outputTokens")
                                                .or_else(|| meta.get("completionTokens"))
                                                .and_then(|o| o.as_i64())
                                            {
                                                r_entry.output_tokens = output;
                                            }
                                        }
                                    }
                                }
                                "inputState" => {
                                    if let Some(v_obj) = v_val {
                                        if let Some(sel_model) = v_obj.get("selectedModel") {
                                            if r_entry.model_id.is_none() {
                                                if let Some(m_id) = sel_model
                                                    .get("identifier")
                                                    .and_then(|i| i.as_str())
                                                {
                                                    r_entry.model_id = Some(m_id.to_string());
                                                }
                                            }
                                            if let Some(meta) = sel_model.get("metadata") {
                                                r_entry.input_cost =
                                                    meta.get("inputCost").and_then(|c| c.as_f64());
                                                r_entry.output_cost =
                                                    meta.get("outputCost").and_then(|c| c.as_f64());
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else if k.len() == 4 {
                        if k[2] == "result" && k[3] == "metadata" {
                            if let Some(v_obj) = v_val {
                                if let Some(prompt) =
                                    v_obj.get("promptTokens").and_then(|p| p.as_i64())
                                {
                                    r_entry.prompt_tokens = prompt;
                                }
                                if let Some(output) = v_obj
                                    .get("outputTokens")
                                    .or_else(|| v_obj.get("completionTokens"))
                                    .and_then(|o| o.as_i64())
                                {
                                    r_entry.output_tokens = output;
                                }
                            }
                        } else if k[2] == "modelState" && k[3] == "completedAt" {
                            if let Some(v_i64) = v_val.and_then(|v| v.as_i64()) {
                                if r_entry.timestamp.is_none() {
                                    r_entry.timestamp = Some(v_i64);
                                }
                            }
                        }
                    } else if k.len() == 5 {
                        if k[2] == "result" && k[3] == "metadata" {
                            if let Some(prop) = k[4].as_str() {
                                if prop == "promptTokens" {
                                    if let Some(v_i64) = v_val.and_then(|v| v.as_i64()) {
                                        r_entry.prompt_tokens = v_i64;
                                    }
                                } else if prop == "completionTokens" || prop == "outputTokens" {
                                    if let Some(v_i64) = v_val.and_then(|v| v.as_i64()) {
                                        r_entry.output_tokens = v_i64;
                                    }
                                }
                            }
                        }
                    }
                }
            } else if k.len() == 2 && k[0] == "inputState" && k[1] == "selectedModel" {
                if let Some(v_obj) = v_val {
                    if let Some(ident) = v_obj.get("identifier").and_then(|i| i.as_str()) {
                        default_model = Some(ident.to_string());
                    }
                    if let Some(meta) = v_obj.get("metadata") {
                        default_input_cost = meta.get("inputCost").and_then(|c| c.as_f64());
                        default_output_cost = meta.get("outputCost").and_then(|c| c.as_f64());
                    }
                }
            }
        }
    }

    if !valid_file_parsed {
        return local;
    }

    local.threads_found = 1;

    let mtime = std::fs::metadata(file_path)
        .and_then(|m| m.modified())
        .unwrap_or_else(|_| SystemTime::now());
    let fallback_dt: DateTime<Local> = mtime.into();
    let fallback_date_key = fallback_dt.format("%Y-%m-%d").to_string();
    let fallback_month_key = fallback_dt.format("%Y-%m").to_string();

    for (_idx, state) in requests {
        let in_tokens = state.prompt_tokens;
        let out_tokens = state.output_tokens;

        if in_tokens == 0 && out_tokens == 0 {
            continue;
        }

        let req_timestamp = state.timestamp.or(session_creation_date);
        let (date_key, month_key) = if let Some(ts_ms) = req_timestamp {
            if let Some(dt) = DateTime::from_timestamp(
                ts_ms / 1000,
                ((ts_ms % 1000) * 1_000_000) as u32,
            ) {
                let dt_local: DateTime<Local> = dt.with_timezone(&Local);
                (
                    dt_local.format("%Y-%m-%d").to_string(),
                    dt_local.format("%Y-%m").to_string(),
                )
            } else {
                (fallback_date_key.clone(), fallback_month_key.clone())
            }
        } else {
            (fallback_date_key.clone(), fallback_month_key.clone())
        };

        let model_name = state.model_id.or_else(|| default_model.clone());
        let (input_cost, output_cost) = if state.input_cost.is_some() || state.output_cost.is_some()
        {
            (state.input_cost, state.output_cost)
        } else {
            (default_input_cost, default_output_cost)
        };

        let model_name = model_name.unwrap_or_else(|| "copilot/unknown".to_string());

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
        let payload = r#"{"kind":0,"v":{"version":3,"sessionId":"test-session-123","requests":[{"requestId":"req1","result":{"metadata":{"promptTokens":100,"outputTokens":50,"details":"Gemini 3.5 Flash"}},"inputState":{"selectedModel":{"identifier":"copilot/gemini-3.5-flash","metadata":{"name":"Gemini 3.5 Flash","inputCost":150,"outputCost":900,"cacheCost":15}}}}]}}"#;
        writeln!(file, "{}", payload).unwrap();
        file.flush().unwrap();

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
        let payload = r#"{"kind":0,"v":{"version":3,"sessionId":"test-session-456","requests":[{"requestId":"req1","result":{"metadata":{"promptTokens":1000,"outputTokens":500}},"inputState":{"selectedModel":{"identifier":"gpt-4o"}}}]}}"#;
        writeln!(file, "{}", payload).unwrap();
        file.flush().unwrap();

        let stats = parse_copilot_file(file.path());
        assert_eq!(stats.threads_found, 1);
        assert!(stats.total_cost > 0.0);
    }

    #[test]
    fn test_copilot_parser_request_timestamps_multi_day() {
        let mut file = NamedTempFile::new().unwrap();

        // 1787184000000 ms (2026-08-20 00:00:00 UTC)
        // 1787616000000 ms (2026-08-25 00:00:00 UTC)
        let ts1: i64 = 1787184000000;
        let ts2: i64 = 1787616000000;

        let dt1 = DateTime::from_timestamp(ts1 / 1000, 0)
            .unwrap()
            .with_timezone(&Local);
        let dt2 = DateTime::from_timestamp(ts2 / 1000, 0)
            .unwrap()
            .with_timezone(&Local);

        let date_key1 = dt1.format("%Y-%m-%d").to_string();
        let date_key2 = dt2.format("%Y-%m-%d").to_string();

        let payload = serde_json::json!({
            "kind": 0,
            "v": {
                "version": 3,
                "creationDate": ts1,
                "requests": [
                    {
                        "requestId": "req1",
                        "timestamp": ts1,
                        "result": {
                            "metadata": {
                                "promptTokens": 1000,
                                "outputTokens": 500
                            }
                        },
                        "inputState": {
                            "selectedModel": {
                                "identifier": "copilot/gpt-5.6-sol",
                                "metadata": {
                                    "inputCost": 500,
                                    "outputCost": 3000
                                }
                            }
                        }
                    },
                    {
                        "requestId": "req2",
                        "timestamp": ts2,
                        "result": {
                            "metadata": {
                                "promptTokens": 2000,
                                "outputTokens": 1000
                            }
                        },
                        "inputState": {
                            "selectedModel": {
                                "identifier": "copilot/gpt-5.6-sol",
                                "metadata": {
                                    "inputCost": 500,
                                    "outputCost": 3000
                                }
                            }
                        }
                    }
                ]
            }
        })
        .to_string();
        writeln!(file, "{}", payload).unwrap();
        file.flush().unwrap();

        let stats = parse_copilot_file(file.path());
        assert_eq!(stats.threads_found, 1);

        // Verify request 1 is attributed to date_key1
        let day1_tokens = stats.daily_stats.get(&date_key1).expect("day 1 present");
        assert_eq!(day1_tokens.in_tokens, 1000);
        assert_eq!(day1_tokens.out_tokens, 500);
        assert!(stats.daily_costs.get(&date_key1).unwrap() > &0.0);

        // Verify request 2 is attributed to date_key2
        let day2_tokens = stats.daily_stats.get(&date_key2).expect("day 2 present");
        assert_eq!(day2_tokens.in_tokens, 2000);
        assert_eq!(day2_tokens.out_tokens, 1000);
        assert!(stats.daily_costs.get(&date_key2).unwrap() > &0.0);

        // Grand total should be sum of both
        assert_eq!(
            stats.model_stats.get("copilot/gpt-5.6-sol").unwrap().in_tokens,
            3000
        );
        assert_eq!(
            stats.model_stats.get("copilot/gpt-5.6-sol").unwrap().out_tokens,
            1500
        );
    }

    #[test]
    fn test_copilot_parser_creation_date_fallback() {
        let mut file = NamedTempFile::new().unwrap();
        let ts: i64 = 1787184000000; // Aug 20, 2026 UTC
        let dt = DateTime::from_timestamp(ts / 1000, 0)
            .unwrap()
            .with_timezone(&Local);
        let date_key = dt.format("%Y-%m-%d").to_string();

        // Request has NO timestamp, but root session has creationDate
        let payload = serde_json::json!({
            "kind": 0,
            "v": {
                "version": 3,
                "creationDate": ts,
                "requests": [
                    {
                        "requestId": "req1",
                        "result": {
                            "metadata": {
                                "promptTokens": 100,
                                "outputTokens": 50
                            }
                        },
                        "inputState": {
                            "selectedModel": {
                                "identifier": "copilot/gemini-3.5-flash",
                                "metadata": {
                                    "inputCost": 150,
                                    "outputCost": 900
                                }
                            }
                        }
                    }
                ]
            }
        })
        .to_string();
        writeln!(file, "{}", payload).unwrap();
        file.flush().unwrap();

        let stats = parse_copilot_file(file.path());
        assert_eq!(stats.threads_found, 1);
        assert!(stats.daily_stats.contains_key(&date_key));
        assert_eq!(stats.daily_stats.get(&date_key).unwrap().in_tokens, 100);
    }

    #[test]
    fn test_copilot_parser_delta_stream_timestamps() {
        let mut file = NamedTempFile::new().unwrap();
        let ts: i64 = 1787184000000; // Aug 20, 2026 UTC
        let dt = DateTime::from_timestamp(ts / 1000, 0)
            .unwrap()
            .with_timezone(&Local);
        let date_key = dt.format("%Y-%m-%d").to_string();

        // Line 1: empty requests snapshot
        writeln!(
            file,
            r#"{{"kind":0,"v":{{"version":3,"creationDate":{ts},"requests":[]}}}}"#
        )
        .unwrap();

        // Line 2: stream delta appending request with timestamp
        writeln!(
            file,
            r#"{{"kind":2,"k":["requests"],"v":[{{"requestId":"req_delta","timestamp":{ts},"modelId":"copilot/kimi-k3","result":{{"metadata":{{"promptTokens":300,"outputTokens":150}}}},"inputState":{{"selectedModel":{{"identifier":"copilot/kimi-k3","metadata":{{"inputCost":300,"outputCost":1500}}}}}}}}]}}"#
        )
        .unwrap();
        file.flush().unwrap();

        let stats = parse_copilot_file(file.path());
        assert_eq!(stats.threads_found, 1);
        assert!(stats.daily_stats.contains_key(&date_key));
        assert_eq!(stats.daily_stats.get(&date_key).unwrap().in_tokens, 300);
        assert_eq!(stats.daily_stats.get(&date_key).unwrap().out_tokens, 150);
        assert!(stats.model_stats.contains_key("copilot/kimi-k3"));
    }
}
