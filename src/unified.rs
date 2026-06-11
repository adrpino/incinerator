use std::collections::{BTreeMap, HashMap};

use crate::claude::{ClaudeStats, run_claude_report};
use crate::cline::{ClineStats, run_cline_report};
use crate::colors::*;
use crate::copilot::{CopilotStats, run_copilot_report};
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::gemini::{GeminiStats, run_gemini_report};
use crate::viz::TokenStats;
use crate::zed::{ZedStats, run_zed_report};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    Cline,
    ClaudeCode,
    GeminiCLI,
    Zed,
    Copilot,
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::Cline => write!(f, "Cline"),
            Provider::ClaudeCode => write!(f, "Claude Code"),
            Provider::GeminiCLI => write!(f, "Gemini CLI"),
            Provider::Zed => write!(f, "Zed"),
            Provider::Copilot => write!(f, "Copilot"),
        }
    }
}

#[derive(Default, Clone)]
pub struct UnifiedStats {
    pub daily_costs: BTreeMap<String, f64>,
    pub daily_costs_cline: BTreeMap<String, f64>,
    pub daily_costs_claude: BTreeMap<String, f64>,
    pub daily_costs_gemini: BTreeMap<String, f64>,
    pub daily_costs_zed: BTreeMap<String, f64>,
    pub daily_costs_copilot: BTreeMap<String, f64>,
    pub daily_tokens: BTreeMap<String, TokenStats>,
    pub daily_tokens_cline: BTreeMap<String, TokenStats>,
    pub daily_tokens_claude: BTreeMap<String, TokenStats>,
    pub daily_tokens_gemini: BTreeMap<String, TokenStats>,
    pub daily_tokens_zed: BTreeMap<String, TokenStats>,
    pub daily_tokens_copilot: BTreeMap<String, TokenStats>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub monthly_costs_cline: BTreeMap<String, f64>,
    pub monthly_costs_claude: BTreeMap<String, f64>,
    pub monthly_costs_gemini: BTreeMap<String, f64>,
    pub monthly_costs_zed: BTreeMap<String, f64>,
    pub monthly_costs_copilot: BTreeMap<String, f64>,
    pub monthly_tokens: BTreeMap<String, TokenStats>,
    pub model_stats: HashMap<String, TokenStats>,
    pub model_stats_cline: HashMap<String, TokenStats>,
    pub model_stats_claude: HashMap<String, TokenStats>,
    pub model_stats_gemini: HashMap<String, TokenStats>,
    pub model_stats_zed: HashMap<String, TokenStats>,
    pub model_stats_copilot: HashMap<String, TokenStats>,
    pub provider_costs: HashMap<Provider, f64>,
    pub total_tokens: TokenStats,
    pub total_cost: f64,
    pub parse_time: f64,
    pub files_parsed: u32,
    pub files_last_parsed: u32,
    pub show_cache_create: bool,
    pub languages: crate::languages::LanguageAnalyzer,
    pub languages_cline: crate::languages::LanguageAnalyzer,
    pub languages_claude: crate::languages::LanguageAnalyzer,
    pub languages_gemini: crate::languages::LanguageAnalyzer,
    pub languages_zed: crate::languages::LanguageAnalyzer,
    pub languages_copilot: crate::languages::LanguageAnalyzer,
}

impl UnifiedStats {
    pub fn collect() -> Option<Self> {
        let cline_res = run_cline_report(false, false);
        let gemini_res = run_gemini_report();
        let claude_res = run_claude_report();
        let zed_res = run_zed_report();
        let copilot_res = run_copilot_report();

        if cline_res.is_none()
            && gemini_res.is_none()
            && claude_res.is_none()
            && zed_res.is_none()
            && copilot_res.is_none()
        {
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
        if let Some((s, t)) = zed_res {
            unified.add_zed(s, t);
        }
        if let Some((s, t)) = copilot_res {
            unified.add_copilot(s, t);
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
        for (k, v) in &s.monthly_costs {
            *self.monthly_costs_cline.entry(k.clone()).or_insert(0.0) += v;
        }
        self.merge_daily_tokens(&s.daily_tokens);
        for (k, v) in &s.daily_tokens {
            self.daily_tokens_cline.entry(k.clone()).or_default().add(v);
        }
        *self.provider_costs.entry(Provider::Cline).or_insert(0.0) += s.total_cost;
        self.merge_monthly_tokens(&s.monthly_tokens);
        let flat_models: Vec<(String, TokenStats)> = s
            .monthly_model_tokens
            .into_values()
            .flat_map(|models| models.into_iter())
            .collect();
        for (model, v) in &flat_models {
            self.model_stats_cline
                .entry(model.clone())
                .or_default()
                .add(v);
        }
        self.merge_model_stats(flat_models);
        self.total_tokens.add(&s.total_tokens);
        self.total_cost += s.total_cost;
        self.parse_time += parse_time;
        self.files_parsed += s.files_found;
        self.languages.merge(&s.languages);
        self.languages_cline.merge(&s.languages);
    }

    pub fn add_gemini(&mut self, s: GeminiStats, parse_time: f64) {
        self.merge_costs(&s.daily_costs, &s.monthly_costs);
        for (k, v) in &s.daily_costs {
            *self.daily_costs_gemini.entry(k.clone()).or_insert(0.0) += v;
        }
        for (k, v) in &s.monthly_costs {
            *self.monthly_costs_gemini.entry(k.clone()).or_insert(0.0) += v;
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
        for (model, v) in &s.model_stats {
            self.model_stats_gemini
                .entry(model.clone())
                .or_default()
                .add(v);
        }
        self.merge_model_stats(s.model_stats.clone());
        let mut total = TokenStats::default();
        for v in s.model_stats.values() {
            total.add(v);
        }
        self.total_tokens.add(&total);
        self.total_cost += cost;
        self.parse_time += parse_time;
        self.files_parsed += s.sessions_found as u32;
        self.languages.merge(&s.languages);
        self.languages_gemini.merge(&s.languages);
    }

    pub fn add_claude(&mut self, s: ClaudeStats, parse_time: f64) {
        self.merge_costs(&s.daily_costs, &s.monthly_costs);
        for (k, v) in &s.daily_costs {
            *self.daily_costs_claude.entry(k.clone()).or_insert(0.0) += v;
        }
        for (k, v) in &s.monthly_costs {
            *self.monthly_costs_claude.entry(k.clone()).or_insert(0.0) += v;
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
        for (model, v) in &s.model_stats {
            self.model_stats_claude
                .entry(model.clone())
                .or_default()
                .add(v);
        }
        self.merge_model_stats(s.model_stats.clone());
        let mut total = TokenStats::default();
        for v in s.model_stats.values() {
            total.add(v);
        }
        self.total_tokens.add(&total);
        self.total_cost += cost;
        self.parse_time += parse_time;
        self.files_parsed += s.sessions_found as u32;
        self.languages.merge(&s.languages);
        self.languages_claude.merge(&s.languages);
        if total.cache_create_tokens > 0 {
            self.show_cache_create = true;
        }
    }

    /// Merge a standalone Gemini tool-output language analyzer (sourced from
    /// `~/.gemini/tmp/<project>/tool-outputs/`) into both the global and the
    /// per-provider Gemini analyzers. Used by the TUI scan path, which walks
    /// tool-output files on its own so they participate in the mtime cache.
    pub fn add_gemini_languages(&mut self, langs: &crate::languages::LanguageAnalyzer) {
        self.languages.merge(langs);
        self.languages_gemini.merge(langs);
    }

    pub fn add_zed(&mut self, s: ZedStats, parse_time: f64) {
        self.merge_costs(&s.daily_costs, &s.monthly_costs);
        for (k, v) in &s.daily_costs {
            *self.daily_costs_zed.entry(k.clone()).or_insert(0.0) += v;
        }
        for (k, v) in &s.monthly_costs {
            *self.monthly_costs_zed.entry(k.clone()).or_insert(0.0) += v;
        }
        self.merge_daily_tokens(&s.daily_stats);
        for (k, v) in &s.daily_stats {
            self.daily_tokens_zed.entry(k.clone()).or_default().add(v);
        }
        *self.provider_costs.entry(Provider::Zed).or_insert(0.0) += s.total_cost;
        self.merge_monthly_tokens(&s.monthly_stats);
        for (model, v) in &s.model_stats {
            self.model_stats_zed
                .entry(model.clone())
                .or_default()
                .add(v);
        }
        self.merge_model_stats(s.model_stats.clone());
        let mut total = TokenStats::default();
        for v in s.model_stats.values() {
            total.add(v);
        }
        self.total_tokens.add(&total);
        self.total_cost += s.total_cost;
        self.parse_time += parse_time;
        self.files_parsed += s.threads_found as u32;
        self.languages.merge(&s.languages);
        self.languages_zed.merge(&s.languages);
    }

    pub fn add_copilot(&mut self, s: CopilotStats, parse_time: f64) {
        self.merge_costs(&s.daily_costs, &s.monthly_costs);
        for (k, v) in &s.daily_costs {
            *self.daily_costs_copilot.entry(k.clone()).or_insert(0.0) += v;
        }
        for (k, v) in &s.monthly_costs {
            *self.monthly_costs_copilot.entry(k.clone()).or_insert(0.0) += v;
        }
        self.merge_daily_tokens(&s.daily_stats);
        for (k, v) in &s.daily_stats {
            self.daily_tokens_copilot
                .entry(k.clone())
                .or_default()
                .add(v);
        }
        *self.provider_costs.entry(Provider::Copilot).or_insert(0.0) += s.total_cost;
        self.merge_monthly_tokens(&s.monthly_stats);
        for (model, v) in &s.model_stats {
            self.model_stats_copilot
                .entry(model.clone())
                .or_default()
                .add(v);
        }
        self.merge_model_stats(s.model_stats.clone());
        let mut total = TokenStats::default();
        for v in s.model_stats.values() {
            total.add(v);
        }
        self.total_tokens.add(&total);
        self.total_cost += s.total_cost;
        self.parse_time += parse_time;
        self.files_parsed += s.threads_found as u32;
        self.languages.merge(&s.languages);
        self.languages_copilot.merge(&s.languages);
    }

    pub fn pad_missing_dates(&mut self) {
        use chrono::{Datelike, Duration, NaiveDate};

        // 1. Pad Daily Data
        let all_dates: Vec<NaiveDate> = self
            .daily_costs
            .keys()
            .filter_map(|k| NaiveDate::parse_from_str(k, "%Y-%m-%d").ok())
            .collect();

        if let (Some(min_date), Some(max_date)) = (all_dates.iter().min(), all_dates.iter().max()) {
            let mut curr = *min_date;
            while curr <= *max_date {
                let key = curr.format("%Y-%m-%d").to_string();
                self.daily_costs.entry(key.clone()).or_insert(0.0);
                self.daily_costs_cline.entry(key.clone()).or_insert(0.0);
                self.daily_costs_claude.entry(key.clone()).or_insert(0.0);
                self.daily_costs_gemini.entry(key.clone()).or_insert(0.0);
                self.daily_costs_zed.entry(key.clone()).or_insert(0.0);
                self.daily_costs_copilot.entry(key.clone()).or_insert(0.0);
                self.daily_tokens.entry(key.clone()).or_default();
                self.daily_tokens_cline.entry(key.clone()).or_default();
                self.daily_tokens_claude.entry(key.clone()).or_default();
                self.daily_tokens_gemini.entry(key.clone()).or_default();
                self.daily_tokens_zed.entry(key.clone()).or_default();
                self.daily_tokens_copilot.entry(key.clone()).or_default();
                if let Some(next) = curr.checked_add_signed(Duration::try_days(1).unwrap()) {
                    curr = next;
                } else {
                    break;
                }
            }
        }

        // 2. Pad Monthly Data
        // Monthly keys are %Y-%m. We'll parse them as %Y-%m-01
        let all_months: Vec<NaiveDate> = self
            .monthly_costs
            .keys()
            .filter_map(|k| NaiveDate::parse_from_str(&format!("{}-01", k), "%Y-%m-%d").ok())
            .collect();

        if let (Some(min_month), Some(max_month)) =
            (all_months.iter().min(), all_months.iter().max())
        {
            let mut curr = *min_month;
            while curr <= *max_month {
                let key = curr.format("%Y-%m").to_string();
                self.monthly_costs.entry(key.clone()).or_insert(0.0);
                self.monthly_costs_cline.entry(key.clone()).or_insert(0.0);
                self.monthly_costs_claude.entry(key.clone()).or_insert(0.0);
                self.monthly_costs_gemini.entry(key.clone()).or_insert(0.0);
                self.monthly_costs_zed.entry(key.clone()).or_insert(0.0);
                self.monthly_costs_copilot.entry(key.clone()).or_insert(0.0);
                self.monthly_tokens.entry(key.clone()).or_default();

                // Move to first day of next month
                if curr.month() == 12 {
                    if let Some(next) = NaiveDate::from_ymd_opt(curr.year() + 1, 1, 1) {
                        curr = next;
                    } else {
                        break;
                    }
                } else if let Some(next) = NaiveDate::from_ymd_opt(curr.year(), curr.month() + 1, 1)
                {
                    curr = next;
                } else {
                    break;
                }
            }
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
        TERM_HEADER, TERM_RESET
    );
    println!("{}", "=".repeat(95));

    println!(
        "\n{}=== TOKEN USAGE (STACKED) ==={}",
        TERM_HEADER, TERM_RESET
    );
    if unified.show_cache_create {
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
    } else {
        println!(
            "Legend: {}█ Input{} | {}█ Output{} | {}▒ Cache Read{}",
            TERM_BLUE, TERM_RESET, TERM_GREEN, TERM_RESET, TERM_YELLOW, TERM_RESET
        );
    }

    println!("\n{}--- Monthly Token Usage ---{}", TERM_BOLD, TERM_RESET);
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
        println!(
            "\n{}--- Overall Usage by Model ---{}",
            TERM_BOLD, TERM_RESET
        );
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
        sorted_models.sort_by_key(|b| std::cmp::Reverse(b.1.total()));
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

    println!("\n{}=== FINANCIAL COSTS ==={}", TERM_HEADER, TERM_RESET);

    if !unified.monthly_costs.is_empty() {
        println!("\n{}--- Monthly Costs ---{}", TERM_BOLD, TERM_RESET);
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
            TERM_BOLD, daily_days, TERM_RESET
        );
        let max_day_cost = unified
            .daily_costs
            .values()
            .copied()
            .fold(0.0_f64, |a, b| a.max(b));
        let mut sorted_days: Vec<_> = unified.daily_costs.iter().collect();
        sorted_days.sort_by_key(|a| a.0);
        for (day, cost) in sorted_days.into_iter().rev().take(daily_days) {
            if day == "Unknown" {
                continue;
            }
            crate::viz::print_cost_bar(&format!("{:<12}", day), *cost, max_day_cost, 35);
        }
    }

    println!("\n{}", "=".repeat(50));
    println!("{}GRAND TOTALS (UNIFIED){}", TERM_HEADER, TERM_RESET);
    println!("{}", "-".repeat(50));
    println!("{}Tokens:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {}Input:        {:>12}{}",
        TERM_BLUE,
        format_int_with_commas(unified.total_tokens.in_tokens),
        TERM_RESET
    );
    println!(
        "  {}Output:       {:>12}{}",
        TERM_GREEN,
        format_int_with_commas(unified.total_tokens.out_tokens),
        TERM_RESET
    );
    println!(
        "  {}Cache Read:   {:>12}{}",
        TERM_YELLOW,
        format_int_with_commas(unified.total_tokens.cache_read_tokens),
        TERM_RESET
    );
    if unified.show_cache_create {
        println!(
            "  {}Cache Create: {:>12}{}",
            TERM_ORANGE,
            format_int_with_commas(unified.total_tokens.cache_create_tokens),
            TERM_RESET
        );
    }
    println!(
        "  {}Total:        {:>12}{}",
        TERM_BOLD,
        format_int_with_commas(unified.total_tokens.total()),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Cost:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {} ${}{}",
        TERM_RED,
        format_float_with_commas(unified.total_cost),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Performance:{}", TERM_BOLD, TERM_RESET);
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

    fn copilot_fixture() -> CopilotStats {
        let mut s = CopilotStats::default();
        s.daily_costs.insert("2026-05-11".into(), 1.20);
        s.monthly_costs.insert("2026-05".into(), 1.20);
        s.daily_stats
            .insert("2026-05-11".into(), ts(1000, 500, 0, 0));
        s.monthly_stats
            .insert("2026-05".into(), ts(1000, 500, 0, 0));
        s.model_stats
            .insert("copilot/gemini-3.5-flash".into(), ts(1000, 500, 0, 0));
        s.threads_found = 1;
        s.total_cost = 1.20;
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
    fn add_copilot_populates_provider_specific_and_global_maps() {
        let mut u = UnifiedStats::default();
        u.add_copilot(copilot_fixture(), 0.15);

        // Per-provider daily costs match the input
        assert_eq!(u.daily_costs_copilot.get("2026-05-11"), Some(&1.20));
        // Global daily costs match (only one provider added)
        assert_eq!(u.daily_costs.get("2026-05-11"), Some(&1.20));
        // Other providers untouched
        assert!(u.daily_costs_cline.is_empty());
        assert!(u.daily_costs_claude.is_empty());
        assert!(u.daily_costs_gemini.is_empty());

        assert_eq!(u.provider_costs.get(&Provider::Copilot), Some(&1.20));
        assert!(!u.provider_costs.contains_key(&Provider::Cline));
        assert_eq!(u.total_cost, 1.20);
        assert_eq!(u.total_tokens.total(), 1500);
        assert_eq!(u.files_parsed, 1);
        assert!((u.parse_time - 0.15).abs() < f64::EPSILON);
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
        u.add_copilot(copilot_fixture(), 0.15);

        // 2026-05-11 is shared across all four providers
        let expected_05_11 = 2.00 + 5.25 + 0.75 + 1.20;
        assert!((u.daily_costs.get("2026-05-11").unwrap() - expected_05_11).abs() < 1e-9);
        // 2026-05-10 only has cline
        assert_eq!(u.daily_costs.get("2026-05-10"), Some(&1.50));

        // Per-provider maps stay isolated
        assert_eq!(u.daily_costs_cline.get("2026-05-11"), Some(&2.00));
        assert_eq!(u.daily_costs_claude.get("2026-05-11"), Some(&5.25));
        assert_eq!(u.daily_costs_gemini.get("2026-05-11"), Some(&0.75));
        assert_eq!(u.daily_costs_copilot.get("2026-05-11"), Some(&1.20));

        // Per-provider costs reflect each provider individually
        assert_eq!(u.provider_costs.get(&Provider::Cline), Some(&3.50));
        assert_eq!(u.provider_costs.get(&Provider::ClaudeCode), Some(&5.25));
        assert_eq!(u.provider_costs.get(&Provider::GeminiCLI), Some(&0.75));
        assert_eq!(u.provider_costs.get(&Provider::Copilot), Some(&1.20));

        // Grand total is the sum
        assert!((u.total_cost - (3.50 + 5.25 + 0.75 + 1.20)).abs() < 1e-9);

        // files_parsed accumulates across cline (files_found) + claude/gemini (sessions_found) + copilot (threads_found)
        assert_eq!(u.files_parsed, 4 + 2 + 3 + 1);

        // parse_time accumulates
        assert!((u.parse_time - 0.50).abs() < 1e-9);

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
    fn languages_merge_from_every_provider() {
        use crate::zed::ZedStats;

        let mut cline = cline_fixture();
        cline.languages.record_file_edit("a.py", 10);
        cline.languages.record_file_edit("b.py", 5);

        let mut claude = claude_fixture_with_cache_create();
        claude.languages.record_file_edit("c.rs", 20);

        let mut gemini = gemini_fixture();
        gemini.languages.record_file_edit("d.ts", 7);

        let mut zed = ZedStats::default();
        zed.languages.record_file_edit("e.go", 3);

        let mut u = UnifiedStats::default();
        u.add_cline(cline, 0.0);
        u.add_claude(claude, 0.0);
        u.add_gemini(gemini, 0.0);
        u.add_zed(zed, 0.0);

        // Global analyzer holds the union (regression: add_cline / add_gemini
        // previously dropped languages on the floor).
        let py = u.languages.stats.get("Python").expect("python present");
        assert_eq!(py.occurrences, 2);
        assert_eq!(py.bytes, 15);
        assert!(u.languages.stats.contains_key("Rust"));
        assert!(u.languages.stats.contains_key("TypeScript"));
        assert!(u.languages.stats.contains_key("Go"));

        // Per-provider analyzers each carry only their own data — this is what
        // the TUI's Languages filter switches between.
        let cline_py = u.languages_cline.stats.get("Python").expect("py in cline");
        assert_eq!(cline_py.occurrences, 2);
        assert_eq!(cline_py.bytes, 15);
        assert!(!u.languages_cline.stats.contains_key("Rust"));
        assert!(!u.languages_cline.stats.contains_key("TypeScript"));
        assert!(!u.languages_cline.stats.contains_key("Go"));

        assert!(u.languages_claude.stats.contains_key("Rust"));
        assert_eq!(u.languages_claude.stats.len(), 1);

        assert!(u.languages_gemini.stats.contains_key("TypeScript"));
        assert_eq!(u.languages_gemini.stats.len(), 1);

        // The TUI scan path uses add_gemini_languages to merge tool-output
        // files directly, bypassing GeminiStats. That data must also land in
        // both the global and per-provider Gemini analyzers.
        let mut extra = crate::languages::LanguageAnalyzer::new();
        extra.record_file_edit("f.js", 11);
        u.add_gemini_languages(&extra);
        let js_global = u.languages.stats.get("JavaScript").expect("js global");
        assert_eq!(js_global.occurrences, 1);
        assert_eq!(js_global.bytes, 11);
        let js_gemini = u
            .languages_gemini
            .stats
            .get("JavaScript")
            .expect("js gemini");
        assert_eq!(js_gemini.occurrences, 1);
        assert!(!u.languages_cline.stats.contains_key("JavaScript"));
        assert!(!u.languages_claude.stats.contains_key("JavaScript"));
        assert!(!u.languages_zed.stats.contains_key("JavaScript"));

        assert!(u.languages_zed.stats.contains_key("Go"));
        assert_eq!(u.languages_zed.stats.len(), 1);
    }

    #[test]
    fn model_stats_track_each_provider() {
        let mut u = UnifiedStats::default();
        u.add_claude(claude_fixture_with_cache_create(), 0.0);
        u.add_gemini(gemini_fixture(), 0.0);
        u.add_copilot(copilot_fixture(), 0.0);

        assert!(u.model_stats.contains_key("claude-sonnet-4-6"));
        assert!(u.model_stats.contains_key("gemini-3-pro"));
        assert!(u.model_stats.contains_key("copilot/gemini-3.5-flash"));
        assert_eq!(u.model_stats.len(), 3);

        assert!(u.model_stats_claude.contains_key("claude-sonnet-4-6"));
        assert_eq!(u.model_stats_claude.len(), 1);

        assert!(u.model_stats_gemini.contains_key("gemini-3-pro"));
        assert_eq!(u.model_stats_gemini.len(), 1);

        assert!(
            u.model_stats_copilot
                .contains_key("copilot/gemini-3.5-flash")
        );
        assert_eq!(u.model_stats_copilot.len(), 1);

        assert!(u.model_stats_cline.is_empty());
        assert!(u.model_stats_zed.is_empty());
    }

    #[test]
    fn pad_missing_dates_fills_gaps() {
        let mut u = UnifiedStats::default();
        u.daily_costs.insert("2026-05-10".into(), 1.0);
        u.daily_costs.insert("2026-05-12".into(), 2.0);
        u.monthly_costs.insert("2026-03".into(), 10.0);
        u.monthly_costs.insert("2026-05".into(), 20.0);

        u.pad_missing_dates();

        assert_eq!(u.daily_costs.get("2026-05-10"), Some(&1.0));
        assert_eq!(u.daily_costs.get("2026-05-11"), Some(&0.0));
        assert_eq!(u.daily_costs.get("2026-05-12"), Some(&2.0));
        assert_eq!(u.daily_costs.len(), 3);

        assert_eq!(u.monthly_costs.get("2026-03"), Some(&10.0));
        assert_eq!(u.monthly_costs.get("2026-04"), Some(&0.0));
        assert_eq!(u.monthly_costs.get("2026-05"), Some(&20.0));
        assert_eq!(u.monthly_costs.len(), 3);
    }
}
