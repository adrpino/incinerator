use crate::colors::*;
use crate::format::{format_float_with_commas, format_metric};

#[derive(Default, Clone, Debug)]
pub struct TokenStats {
    pub in_tokens: i64,
    pub out_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_create_tokens: i64,
}

impl TokenStats {
    pub fn total(&self) -> i64 {
        self.in_tokens + self.out_tokens + self.cache_read_tokens + self.cache_create_tokens
    }

    pub fn add(&mut self, other: &TokenStats) {
        self.in_tokens += other.in_tokens;
        self.out_tokens += other.out_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.cache_create_tokens += other.cache_create_tokens;
    }
}

pub fn print_token_bar(
    label: &str,
    stats: &TokenStats,
    max_val: i64,
    bar_width: usize,
    show_cache_create: bool,
) {
    if max_val == 0 {
        return;
    }

    let scale = |v: i64| -> usize {
        let w = ((v as f64 / max_val as f64) * bar_width as f64) as usize;
        if v > 0 && w == 0 { 1 } else { w }
    };

    let w_in = scale(stats.in_tokens);
    let w_out = scale(stats.out_tokens);
    let w_cache_read = scale(stats.cache_read_tokens);
    let w_cache_create = if show_cache_create { scale(stats.cache_create_tokens) } else { 0 };
    let w_total_visible = w_in + w_out + w_cache_read + w_cache_create;

    let bar_str = if show_cache_create {
        format!(
            "{}{}{}{}{}{}{}{}{}",
            BLUE, "█".repeat(w_in),
            GREEN, "█".repeat(w_out),
            YELLOW, "▒".repeat(w_cache_read),
            ORANGE, "░".repeat(w_cache_create),
            RESET
        )
    } else {
        format!(
            "{}{}{}{}{}{}{}",
            BLUE, "█".repeat(w_in),
            GREEN, "█".repeat(w_out),
            YELLOW, "▒".repeat(w_cache_read),
            RESET
        )
    };

    let padding_needed = if bar_width >= w_total_visible {
        bar_width - w_total_visible + 1
    } else {
        1
    };
    let padding = " ".repeat(padding_needed);

    let stats_str = if show_cache_create {
        format!(
            "{}In:{} {}Out:{} {}C_Rd:{} {}C_Cr:{}{}",
            BLUE, format_metric(stats.in_tokens as f64, 6),
            GREEN, format_metric(stats.out_tokens as f64, 6),
            YELLOW, format_metric(stats.cache_read_tokens as f64, 6),
            ORANGE, format_metric(stats.cache_create_tokens as f64, 6),
            RESET
        )
    } else {
        format!(
            "{}In:{} {}Out:{} {}Cache:{}{}",
            BLUE, format_metric(stats.in_tokens as f64, 7),
            GREEN, format_metric(stats.out_tokens as f64, 7),
            YELLOW, format_metric(stats.cache_read_tokens as f64, 7),
            RESET
        )
    };

    println!("{} | {}{}| {}", label, bar_str, padding, stats_str);
}

pub fn print_cost_bar(label: &str, cost: f64, max_cost: f64, bar_width: usize) {
    let bar_length = if max_cost > 0.0 {
        ((cost / max_cost) * bar_width as f64) as usize
    } else {
        0
    };

    let bar_str = format!("{}{}{}", GREEN, "█".repeat(bar_length), RESET);
    let padding_needed = if bar_width >= bar_length {
        bar_width - bar_length + 1
    } else {
        1
    };
    let padding = " ".repeat(padding_needed);

    println!("{} | {}{}| ${}", label, bar_str, padding, format_float_with_commas(cost));
}
