use std::collections::{BTreeMap, HashMap};

use crate::claude::{ClaudeStats, run_claude_report};
use crate::cline::{ClineStats, run_cline_report};
use crate::colors::*;
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::gemini::{GeminiStats, run_gemini_report};
use crate::viz::TokenStats;

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
    pub daily_costs_cline: BTreeMap<String, f64>,
    pub daily_costs_claude: BTreeMap<String, f64>,
    pub daily_costs_gemini: BTreeMap<String, f64>,
    pub daily_tokens: BTreeMap<String, TokenStats>,
    pub daily_tokens_cline: BTreeMap<String, TokenStats>,
    pub daily_tokens_claude: BTreeMap<String, TokenStats>,
    pub daily_tokens_gemini: BTreeMap<String, TokenStats>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub monthly_tokens: BTreeMap<String, TokenStats>,
    pub model_stats: HashMap<String, TokenStats>,
    pub provider_costs: HashMap<Provider, f64>,
    pub total_tokens: TokenStats,
    pub total_cost: f64,
    pub parse_time: f64,
    pub files_parsed: u32,
    pub files_last_parsed: u32,
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
        for (k, v) in &s.daily_costs {
            *self.daily_costs_cline.entry(k.clone()).or_insert(0.0) += v;
        }
        self.merge_daily_tokens(&s.daily_tokens);
        for (k, v) in &s.daily_tokens {
            self.daily_tokens_cline.entry(k.clone()).or_default().add(v);
        }
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
        for (k, v) in &s.daily_costs {
            *self.daily_costs_gemini.entry(k.clone()).or_insert(0.0) += v;
        }
        self.merge_daily_tokens(&s.daily_stats);
        for (k, v) in &s.daily_stats {
            self.daily_tokens_gemini
                .entry(k.clone())
                .or_default()
                .add(v);
        }
        let cost = s.monthly_costs.values().sum::<f64>();
        *self
            .provider_costs
            .entry(Provider::GeminiCLI)
            .or_insert(0.0) += cost;
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
        for (k, v) in &s.daily_costs {
            *self.daily_costs_claude.entry(k.clone()).or_insert(0.0) += v;
        }
        self.merge_daily_tokens(&s.daily_stats);
        for (k, v) in &s.daily_stats {
            self.daily_tokens_claude
                .entry(k.clone())
                .or_default()
                .add(v);
        }
        let cost = s.monthly_costs.values().sum::<f64>();
        *self
            .provider_costs
            .entry(Provider::ClaudeCode)
            .or_insert(0.0) += cost;
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
    println!(
        "{}🔥 INCINERATOR: AI USAGE & COST ESTIMATE{}",
        HEADER, RESET
    );
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
    let max_monthly = unified
        .monthly_tokens
        .values()
        .map(|s| s.total())
        .max()
        .unwrap_or(0);
    for (month, stats) in &unified.monthly_tokens {
        // Note: print_token_bar is in viz.rs
        crate::viz::print_token_bar(
            &format!("{:^10}", month),
            stats,
            max_monthly,
            35,
            unified.show_cache_create,
        );
    }

    if !unified.model_stats.is_empty() {
        println!("\n{}--- Overall Usage by Model ---{}", BOLD, RESET);
        let max_model_tokens = unified
            .model_stats
            .values()
            .map(|s| s.total())
            .max()
            .unwrap_or(0);
        let max_model_len = unified
            .model_stats
            .keys()
            .map(|m| m.len())
            .max()
            .unwrap_or(20)
            .min(30);
        let mut sorted_models: Vec<_> = unified.model_stats.iter().collect();
        sorted_models.sort_by(|a, b| b.1.total().cmp(&a.1.total()));
        for (model, stats) in sorted_models {
            crate::viz::print_token_bar(
                &format!(
                    "{:<width$}",
                    model.get(..30).unwrap_or(model),
                    width = max_model_len
                ),
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
        let max_month_cost = unified
            .monthly_costs
            .values()
            .copied()
            .fold(0.0_f64, |a, b| a.max(b));
        for (month, cost) in unified.monthly_costs.iter().rev() {
            if month == "Unknown" {
                continue;
            }
            crate::viz::print_cost_bar(&format!("{:^12}", month), *cost, max_month_cost, 35);
        }
    }

    if !unified.daily_costs.is_empty() {
        println!(
            "\n{}--- Daily Costs (Last {} days) ---{}",
            BOLD, daily_days, RESET
        );
        let max_day_cost = unified
            .daily_costs
            .values()
            .copied()
            .fold(0.0_f64, |a, b| a.max(b));
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
    println!(
        "  {}Input:        {:>12}{}",
        BLUE,
        format_int_with_commas(unified.total_tokens.in_tokens),
        RESET
    );
    println!(
        "  {}Output:       {:>12}{}",
        GREEN,
        format_int_with_commas(unified.total_tokens.out_tokens),
        RESET
    );
    println!(
        "  {}Cache Read:   {:>12}{}",
        YELLOW,
        format_int_with_commas(unified.total_tokens.cache_read_tokens),
        RESET
    );
    if unified.show_cache_create {
        println!(
            "  {}Cache Create: {:>12}{}",
            ORANGE,
            format_int_with_commas(unified.total_tokens.cache_create_tokens),
            RESET
        );
    }
    println!(
        "  {}Total:        {:>12}{}",
        BOLD,
        format_int_with_commas(unified.total_tokens.total()),
        RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Cost:{}", BOLD, RESET);
    println!(
        "  {} ${}{}",
        RED,
        format_float_with_commas(unified.total_cost),
        RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Performance:{}", BOLD, RESET);
    println!("  Files Parsed: {}", unified.files_parsed);
    println!("  Parse Time:   {:.2} seconds", unified.parse_time);
    println!("{}", "=".repeat(50));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude::ClaudeStats;
    use crate::cline::ClineStats;
    use crate::gemini::GeminiStats;

    fn ts(in_t: i64, out_t: i64, c_read: i64, c_create: i64) -> TokenStats {
        TokenStats {
            in_tokens: in_t,
            out_tokens: out_t,
            cache_read_tokens: c_read,
            cache_create_tokens: c_create,
        }
    }

    fn cline_fixture() -> ClineStats {
        let mut s = ClineStats::default();
        s.daily_costs.insert("2026-05-10".into(), 1.50);
        s.daily_costs.insert("2026-05-11".into(), 2.00);
        s.monthly_costs.insert("2026-05".into(), 3.50);
        s.daily_tokens
            .insert("2026-05-10".into(), ts(100, 50, 25, 0));
        s.daily_tokens
            .insert("2026-05-11".into(), ts(200, 100, 50, 0));
        s.monthly_tokens
            .insert("2026-05".into(), ts(300, 150, 75, 0));
        s.total_cost = 3.50;
        s.total_tokens = ts(300, 150, 75, 0);
        s.files_found = 4;
        s
    }

    fn claude_fixture_with_cache_create() -> ClaudeStats {
        let mut s = ClaudeStats::default();
        s.daily_costs.insert("2026-05-11".into(), 5.25);
        s.monthly_costs.insert("2026-05".into(), 5.25);
        s.daily_stats
            .insert("2026-05-11".into(), ts(500, 250, 100, 80));
        s.monthly_stats
            .insert("2026-05".into(), ts(500, 250, 100, 80));
        s.model_stats
            .insert("claude-sonnet-4-6".into(), ts(500, 250, 100, 80));
        s.sessions_found = 2;
        s
    }

    fn claude_fixture_no_cache_create() -> ClaudeStats {
        let mut s = ClaudeStats::default();
        s.daily_costs.insert("2026-05-11".into(), 1.00);
        s.monthly_costs.insert("2026-05".into(), 1.00);
        s.daily_stats.insert("2026-05-11".into(), ts(50, 25, 10, 0));
        s.monthly_stats.insert("2026-05".into(), ts(50, 25, 10, 0));
        s.model_stats
            .insert("claude-haiku-4-5".into(), ts(50, 25, 10, 0));
        s.sessions_found = 1;
        s
    }

    fn gemini_fixture() -> GeminiStats {
        let mut s = GeminiStats::default();
        s.daily_costs.insert("2026-05-11".into(), 0.75);
        s.monthly_costs.insert("2026-05".into(), 0.75);
        s.daily_stats.insert("2026-05-11".into(), ts(80, 40, 0, 0));
        s.monthly_stats.insert("2026-05".into(), ts(80, 40, 0, 0));
        s.model_stats
            .insert("gemini-3-pro".into(), ts(80, 40, 0, 0));
        s.sessions_found = 3;
        s
    }

    #[test]
    fn add_cline_populates_provider_specific_and_global_maps() {
        let mut u = UnifiedStats::default();
        u.add_cline(cline_fixture(), 0.12);

        // Per-provider daily costs match the input
        assert_eq!(u.daily_costs_cline.get("2026-05-10"), Some(&1.50));
        assert_eq!(u.daily_costs_cline.get("2026-05-11"), Some(&2.00));
        // Global daily costs match (only one provider added)
        assert_eq!(u.daily_costs.get("2026-05-10"), Some(&1.50));
        assert_eq!(u.daily_costs.get("2026-05-11"), Some(&2.00));
        // Other providers untouched
        assert!(u.daily_costs_claude.is_empty());
        assert!(u.daily_costs_gemini.is_empty());

        assert_eq!(u.provider_costs.get(&Provider::Cline), Some(&3.50));
        assert!(!u.provider_costs.contains_key(&Provider::ClaudeCode));
        assert_eq!(u.total_cost, 3.50);
        assert_eq!(u.total_tokens.total(), 525);
        assert_eq!(u.files_parsed, 4);
        assert!((u.parse_time - 0.12).abs() < f64::EPSILON);
        assert!(!u.show_cache_create);
    }

    #[test]
    fn add_claude_with_cache_create_flips_flag() {
        let mut u = UnifiedStats::default();
        u.add_claude(claude_fixture_with_cache_create(), 0.0);
        assert!(u.show_cache_create);
        assert_eq!(u.provider_costs.get(&Provider::ClaudeCode), Some(&5.25));
        assert_eq!(u.files_parsed, 2);
    }

    #[test]
    fn add_claude_without_cache_create_leaves_flag_off() {
        let mut u = UnifiedStats::default();
        u.add_claude(claude_fixture_no_cache_create(), 0.0);
        assert!(!u.show_cache_create);
    }

    #[test]
    fn merging_multiple_providers_sums_global_but_keeps_per_provider_separate() {
        let mut u = UnifiedStats::default();
        u.add_cline(cline_fixture(), 0.10);
        u.add_claude(claude_fixture_with_cache_create(), 0.20);
        u.add_gemini(gemini_fixture(), 0.05);

        // 2026-05-11 is shared across all three providers
        let expected_05_11 = 2.00 + 5.25 + 0.75;
        assert!((u.daily_costs.get("2026-05-11").unwrap() - expected_05_11).abs() < 1e-9);
        // 2026-05-10 only has cline
        assert_eq!(u.daily_costs.get("2026-05-10"), Some(&1.50));

        // Per-provider maps stay isolated
        assert_eq!(u.daily_costs_cline.get("2026-05-11"), Some(&2.00));
        assert_eq!(u.daily_costs_claude.get("2026-05-11"), Some(&5.25));
        assert_eq!(u.daily_costs_gemini.get("2026-05-11"), Some(&0.75));

        // Per-provider costs reflect each provider individually
        assert_eq!(u.provider_costs.get(&Provider::Cline), Some(&3.50));
        assert_eq!(u.provider_costs.get(&Provider::ClaudeCode), Some(&5.25));
        assert_eq!(u.provider_costs.get(&Provider::GeminiCLI), Some(&0.75));

        // Grand total is the sum
        assert!((u.total_cost - (3.50 + 5.25 + 0.75)).abs() < 1e-9);

        // files_parsed accumulates across cline (files_found) + claude/gemini (sessions_found)
        assert_eq!(u.files_parsed, 4 + 2 + 3);

        // parse_time accumulates
        assert!((u.parse_time - 0.35).abs() < 1e-9);

        // show_cache_create stays sticky once flipped
        assert!(u.show_cache_create);
    }

    #[test]
    fn same_day_across_providers_sums_daily_tokens() {
        let mut u = UnifiedStats::default();
        u.add_cline(cline_fixture(), 0.0);
        u.add_claude(claude_fixture_with_cache_create(), 0.0);

        // 2026-05-11: cline contributes 200+100+50 = 350, claude contributes 500+250+100+80 = 930
        let day = u.daily_tokens.get("2026-05-11").expect("day present");
        assert_eq!(day.in_tokens, 200 + 500);
        assert_eq!(day.out_tokens, 100 + 250);
        assert_eq!(day.cache_read_tokens, 50 + 100);
        assert_eq!(day.cache_create_tokens, 80);
    }

    #[test]
    fn model_stats_track_each_provider() {
        let mut u = UnifiedStats::default();
        u.add_claude(claude_fixture_with_cache_create(), 0.0);
        u.add_gemini(gemini_fixture(), 0.0);

        assert!(u.model_stats.contains_key("claude-sonnet-4-6"));
        assert!(u.model_stats.contains_key("gemini-3-pro"));
        assert_eq!(u.model_stats.len(), 2);
    }
}
