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

pub fn format_metric(n: f64, width: usize) -> String {
    let s = if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else {
        format!("{}", n as i64)
    };
    format!("{:<width$}", s, width = width)
}
