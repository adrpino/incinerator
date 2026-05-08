pub fn format_int_with_commas(mut n: i64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut s = String::new();
    let is_negative = n < 0;
    if is_negative {
        n = -n;
    }
    let n_str = n.to_string();
    let chars: Vec<char> = n_str.chars().rev().collect();
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && i % 3 == 0 {
            s.push(',');
        }
        s.push(*c);
    }
    if is_negative {
        s.push('-');
    }
    s.chars().rev().collect()
}

pub fn format_float_with_commas(n: f64) -> String {
    let int_part = n.trunc() as i64;
    let frac_part = (n.fract() * 100.0).round().abs() as i64;
    format!("{}.{:02}", format_int_with_commas(int_part), frac_part)
}

/// Formats a token count into a human-readable string (e.g., 1.25M, 450.0K, 123)
pub fn format_tokens(n: i64) -> String {
    let n_f = n as f64;
    if n_f.abs() >= 1_000_000.0 {
        format!("{:.2}M", n_f / 1_000_000.0)
    } else if n_f.abs() >= 1_000.0 {
        format!("{:.1}K", n_f / 1_000.0)
    } else {
        format_int_with_commas(n)
    }
}

/// Formats a currency amount with a dollar sign and commas
pub fn format_currency(amount: f64) -> String {
    format!("${}", format_float_with_commas(amount))
}

/// Formats a number into a fixed-width metric string for alignment in bars/tables
pub fn format_metric(n: f64, width: usize) -> String {
    let s = if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}K", n / 1_000.0)
    } else {
        format_int_with_commas(n as i64)
    };
    format!("{:<width$}", s, width = width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(1_250_000), "1.25M");
        assert_eq!(format_tokens(450_000), "450.0K");
        assert_eq!(format_tokens(1_500), "1.5K");
        assert_eq!(format_tokens(123), "123");
    }

    #[test]
    fn test_format_currency() {
        assert_eq!(format_currency(1234.56), "$1,234.56");
        assert_eq!(format_currency(0.05), "$0.05");
    }
}
