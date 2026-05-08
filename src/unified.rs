use std::collections::{BTreeMap, HashMap};

use crate::claude::{run_claude_report, ClaudeStats};
use crate::cline::{run_cline_report, ClineStats};
use crate::colors::*;
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::gemini::{run_gemini_report, GeminiStats};
use crate::viz::{TokenStats};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Cline,
    ClaudeCode,
    GeminiCLI,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::Cline => write!(f, "Cline"),
            Provider::ClaudeCode => write!(f, "Claude Code"),
            Provider::GeminiCLI => write!(f, "Gemini CLI"),
        }
    }
}

#[derive(Default, Clone)]
pub struct UnifiedStats {
    pub daily_costs: BTreeMap<String, f64>,
    pub daily_tokens: BTreeMap<String, TokenStats>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub monthly_tokens: BTreeMap<String, TokenStats>,
    pub model_stats: HashMap<String, TokenStats>,
    pub provider_costs: HashMap<Provider, f64>,
    pub total_tokens: TokenStats,
    pub total_cost: f64,
    pub parse_time: f64,
    pub files_parsed: u32,
    pub show_cache_create: bool,
}

impl UnifiedStats {
    pub fn collect() -> Option<Self> {
        let cline_res = run_cline_report(false, false);
        let gemini_res = run_gemini_report();
        let claude_res = run_claude_report();

        if cline_res.is_none() && gemini_res.is_none() && claude_res.is_none() {
            return None;
        }

        let mut unified = UnifiedStats::default();
        if let Some((s, t)) = cline_res {
            unified.add_cline(s, t);
        }
        if let Some((s, t)) = gemini_res {
            unified.add_gemini(s, t);
        }
        if let Some((s, t)) = claude_res {
            unified.add_claude(s, t);
        }
        Some(unified)
    }

    fn merge_costs(&mut self, daily: &BTreeMap<String, f64>, monthly: &BTreeMap<String, f64>) {
        for (k, v) in daily {
            *self.daily_costs.entry(k.clone()).or_insert(0.0) += v;
        }
        for (k, v) in monthly {
            *self.monthly_costs.entry(k.clone()).or_insert(0.0) += v;
        }
    }

    fn merge_monthly_tokens(&mut self, monthly: &BTreeMap<String, TokenStats>) {
        for (k, v) in monthly {
            self.monthly_tokens.entry(k.clone()).or_default().add(v);
        }
    }

    fn merge_daily_tokens(&mut self, daily: &BTreeMap<String, TokenStats>) {
        for (k, v) in daily {
            self.daily_tokens.entry(k.clone()).or_default().add(v);
        }
    }

    fn merge_model_stats(&mut self, models: impl IntoIterator<Item = (String, TokenStats)>) {
        for (model, v) in models {
            self.model_stats.entry(model).or_default().add(&v);
        }
    }

    pub fn add_cline(&mut self, s: ClineStats, parse_time: f64) {
        self.merge_costs(&s.daily_costs, &s.monthly_costs);
        self.merge_daily_tokens(&s.daily_tokens);
        *self.provider_costs.entry(Provider::Cline).or_insert(0.0) += s.total_cost;
        self.merge_monthly_tokens(&s.monthly_tokens);
        let flat_models: Vec<(String, TokenStats)> = s
            .monthly_model_tokens
            .into_iter()
            .flat_map(|(_, models)| models.into_iter())
            .collect();
        self.merge_model_stats(flat_models);
        self.total_tokens.add(&s.total_tokens);
        self.total_cost += s.total_cost;
        self.parse_time += parse_time;
        self.files_parsed += s.files_found;
    }

    pub fn add_gemini(&mut self, s: GeminiStats, parse_time: f64) {
        self.merge_costs(&s.daily_costs, &s.monthly_costs);
        self.merge_daily_tokens(&s.daily_stats);
        let cost = s.monthly_costs.values().sum::<f64>();
        *self.provider_costs.entry(Provider::GeminiCLI).or_insert(0.0) += cost;
        self.merge_monthly_tokens(&s.monthly_stats);
        self.merge_model_stats(s.model_stats.clone());
        let mut total = TokenStats::default();
        for v in s.model_stats.values() {
            total.add(v);
        }
        self.total_tokens.add(&total);
        self.total_cost += cost;
        self.parse_time += parse_time;
        self.files_parsed += s.sessions_found as u32;
    }

    pub fn add_claude(&mut self, s: ClaudeStats, parse_time: f64) {
        self.merge_costs(&s.daily_costs, &s.monthly_costs);
        self.merge_daily_tokens(&s.daily_stats);
        let cost = s.monthly_costs.values().sum::<f64>();
        *self.provider_costs.entry(Provider::ClaudeCode).or_insert(0.0) += cost;
        self.merge_monthly_tokens(&s.monthly_stats);
        self.merge_model_stats(s.model_stats.clone());
        let mut total = TokenStats::default();
        for v in s.model_stats.values() {
            total.add(v);
        }
        self.total_tokens.add(&total);
        self.total_cost += cost;
        self.parse_time += parse_time;
        self.files_parsed += s.sessions_found as u32;
        if total.cache_create_tokens > 0 {
            self.show_cache_create = true;
        }
    }
}

#[allow(dead_code)]
pub fn run_unified_report(daily_days: usize) {
    let Some(unified) = UnifiedStats::collect() else {
        println!("No usage data found from any source.");
        return;
    };

    println!("\n{}", "=".repeat(95));
    println!("{}🔥 INCINERATOR: AI USAGE & COST ESTIMATE{}", HEADER, RESET);
    println!("{}", "=".repeat(95));

    println!("\n{}=== TOKEN USAGE (STACKED) ==={}", HEADER, RESET);
    if unified.show_cache_create {
        println!(
            "Legend: {}█ Input{} | {}█ Output{} | {}▒ Cache Read{} | {}░ Cache Create{}",
            BLUE, RESET, GREEN, RESET, YELLOW, RESET, ORANGE, RESET
        );
    } else {
        println!(
            "Legend: {}█ Input{} | {}█ Output{} | {}▒ Cache Read{}",
            BLUE, RESET, GREEN, RESET, YELLOW, RESET
        );
    }

    println!("\n{}--- Monthly Token Usage ---{}", BOLD, RESET);
    let max_monthly = unified.monthly_tokens.values().map(|s| s.total()).max().unwrap_or(0);
    for (month, stats) in &unified.monthly_tokens {
        // Note: print_token_bar is in viz.rs
        crate::viz::print_token_bar(&format!("{:^10}", month), stats, max_monthly, 35, unified.show_cache_create);
    }

    if !unified.model_stats.is_empty() {
        println!("\n{}--- Overall Usage by Model ---{}", BOLD, RESET);
        let max_model_tokens = unified.model_stats.values().map(|s| s.total()).max().unwrap_or(0);
        let max_model_len = unified.model_stats.keys().map(|m| m.len()).max().unwrap_or(20).min(30);
        let mut sorted_models: Vec<_> = unified.model_stats.iter().collect();
        sorted_models.sort_by(|a, b| b.1.total().cmp(&a.1.total()));
        for (model, stats) in sorted_models {
            crate::viz::print_token_bar(
                &format!("{:<width$}", model.get(..30).unwrap_or(model), width = max_model_len),
                stats,
                max_model_tokens,
                35,
                unified.show_cache_create,
            );
        }
    }

    println!("\n{}=== FINANCIAL COSTS ==={}", HEADER, RESET);

    if !unified.monthly_costs.is_empty() {
        println!("\n{}--- Monthly Costs ---{}", BOLD, RESET);
        let max_month_cost = unified.monthly_costs.values().copied().fold(0.0_f64, |a, b| a.max(b));
        for (month, cost) in unified.monthly_costs.iter().rev() {
            if month == "Unknown" {
                continue;
            }
            crate::viz::print_cost_bar(&format!("{:^12}", month), *cost, max_month_cost, 35);
        }
    }

    if !unified.daily_costs.is_empty() {
        println!("\n{}--- Daily Costs (Last {} days) ---{}", BOLD, daily_days, RESET);
        let max_day_cost = unified.daily_costs.values().copied().fold(0.0_f64, |a, b| a.max(b));
        let mut sorted_days: Vec<_> = unified.daily_costs.iter().collect();
        sorted_days.sort_by(|a, b| a.0.cmp(b.0));
        for (day, cost) in sorted_days.into_iter().rev().take(daily_days) {
            if day == "Unknown" {
                continue;
            }
            crate::viz::print_cost_bar(&format!("{:<12}", day), *cost, max_day_cost, 35);
        }
    }

    println!("\n{}", "=".repeat(50));
    println!("{}GRAND TOTALS (UNIFIED){}", HEADER, RESET);
    println!("{}", "-".repeat(50));
    println!("{}Tokens:{}", BOLD, RESET);
    println!("  {}Input:        {:>12}{}", BLUE, format_int_with_commas(unified.total_tokens.in_tokens), RESET);
    println!("  {}Output:       {:>12}{}", GREEN, format_int_with_commas(unified.total_tokens.out_tokens), RESET);
    println!("  {}Cache Read:   {:>12}{}", YELLOW, format_int_with_commas(unified.total_tokens.cache_read_tokens), RESET);
    if unified.show_cache_create {
        println!("  {}Cache Create: {:>12}{}", ORANGE, format_int_with_commas(unified.total_tokens.cache_create_tokens), RESET);
    }
    println!("  {}Total:        {:>12}{}", BOLD, format_int_with_commas(unified.total_tokens.total()), RESET);
    println!("{}", "-".repeat(50));
    println!("{}Cost:{}", BOLD, RESET);
    println!("  {} ${}{}", RED, format_float_with_commas(unified.total_cost), RESET);
    println!("{}", "-".repeat(50));
    println!("{}Performance:{}", BOLD, RESET);
    println!("  Files Parsed: {}", unified.files_parsed);
    println!("  Parse Time:   {:.2} seconds", unified.parse_time);
    println!("{}", "=".repeat(50));
}
