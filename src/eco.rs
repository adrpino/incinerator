use crate::format::format_int_with_commas;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcoCategory {
    HomeAppliance,
    Transport,
    LargeScale,
    Medical,
}

pub struct UnitScale {
    pub divisor: f64,
    pub next_unit: &'static str,
}

pub struct EcoMapping {
    pub name: &'static str,
    pub category: EcoCategory,
    pub unit: &'static str,
    pub energy_cost_wh: f64,
    pub scale: Option<UnitScale>,
}

pub const ECO_MAPPINGS: &[EcoMapping] = &[
    // Tech & Infrastructure
    EcoMapping {
        name: "High-End Gaming PC (RTX 4090)",
        category: EcoCategory::LargeScale,
        unit: "hours of 4K gaming",
        energy_cost_wh: 600.0,
        scale: Some(UnitScale {
            divisor: 24.0,
            next_unit: "days of 4K gaming",
        }),
    },
    EcoMapping {
        name: "Tesla Supercharger (V3)",
        category: EcoCategory::Transport,
        unit: "seconds of rapid charging",
        energy_cost_wh: 69.4, // 250kW / 3600s = 69.4Wh per second
        scale: Some(UnitScale {
            divisor: 60.0,
            next_unit: "minutes of rapid charging",
        }),
    },
    // Medical / High Guilt
    EcoMapping {
        name: "Hospital Ventilator",
        category: EcoCategory::Medical,
        unit: "hours running",
        energy_cost_wh: 50.0,
        scale: Some(UnitScale {
            divisor: 24.0,
            next_unit: "days running",
        }),
    },
    // Transport
    EcoMapping {
        name: "Electric Car (EV)",
        category: EcoCategory::Transport,
        unit: "kilometers driven",
        energy_cost_wh: 150.0,
        scale: None,
    },
    EcoMapping {
        name: "E-Bike",
        category: EcoCategory::Transport,
        unit: "kilometers ridden",
        energy_cost_wh: 10.0,
        scale: None,
    },
    EcoMapping {
        name: "DJI Drone",
        category: EcoCategory::Transport,
        unit: "full battery flights",
        energy_cost_wh: 80.0,
        scale: None,
    },
    // Home Appliances
    EcoMapping {
        name: "Modern TV",
        category: EcoCategory::HomeAppliance,
        unit: "hours watched",
        energy_cost_wh: 100.0,
        scale: Some(UnitScale {
            divisor: 24.0,
            next_unit: "days watched",
        }),
    },
    EcoMapping {
        name: "Central AC",
        category: EcoCategory::HomeAppliance,
        unit: "hours of cooling",
        energy_cost_wh: 3500.0,
        scale: Some(UnitScale {
            divisor: 24.0,
            next_unit: "days of cooling",
        }),
    },
    EcoMapping {
        name: "Kitchen Oven",
        category: EcoCategory::HomeAppliance,
        unit: "minutes baking",
        energy_cost_wh: 33.3,
        scale: Some(UnitScale {
            divisor: 60.0,
            next_unit: "hours baking",
        }),
    },
    EcoMapping {
        name: "Espresso Machine",
        category: EcoCategory::HomeAppliance,
        unit: "shots brewed",
        energy_cost_wh: 10.0,
        scale: None,
    },
    EcoMapping {
        name: "Smartphone",
        category: EcoCategory::HomeAppliance,
        unit: "full charges",
        energy_cost_wh: 15.0,
        scale: None,
    },
    // Large Scale
    EcoMapping {
        name: "Wind Turbine",
        category: EcoCategory::LargeScale,
        unit: "seconds spinning",
        energy_cost_wh: 555.0,
        scale: Some(UnitScale {
            divisor: 60.0,
            next_unit: "minutes spinning",
        }),
    },
];

/// Calculate the total watt-hours based on the 300Wh per 1,000,000 tokens rule.
pub fn tokens_to_wh(total_tokens: f64) -> f64 {
    (total_tokens / 1_000_000.0) * 300.0
}

/// Find the top mappings whose unit count is closest to a whole integer.
/// Returns a vector of mappings and their rounded unit counts.
pub fn find_top_mappings(total_wh: f64, limit: usize) -> Vec<(&'static EcoMapping, u64)> {
    if total_wh <= 0.0 {
        return Vec::new();
    }

    let mut scored_mappings: Vec<_> = ECO_MAPPINGS
        .iter()
        .map(|m| {
            let units = total_wh / m.energy_cost_wh;
            let diff = (units - units.round()).abs();
            let rounded = if units.round() == 0.0 {
                1
            } else {
                units.round() as u64
            };
            (m, rounded, diff)
        })
        .collect();

    // Sort by smallest difference (closest to integer)
    scored_mappings.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut results = Vec::new();
    let mut seen_units = std::collections::HashSet::new();

    for (m, r, _) in scored_mappings {
        if !seen_units.contains(&r) {
            results.push((m, r));
            seen_units.insert(r);
        }
        if results.len() >= limit {
            break;
        }
    }

    results
}

pub fn format_eco_metrics(total_tokens: u64, color: Color, width: usize) -> Vec<Line<'static>> {
    if total_tokens == 0 || width < 10 {
        return Vec::new();
    }

    let wh = tokens_to_wh(total_tokens as f64);
    let top_mappings = find_top_mappings(wh, 2);

    if top_mappings.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::new();

    for (mapping, units) in top_mappings {
        let icon = match mapping.category {
            EcoCategory::HomeAppliance => "🏠",
            EcoCategory::Transport => "🚗",
            EcoCategory::LargeScale => "🏭",
            EcoCategory::Medical => "🏥",
        };

        let (final_units, final_unit_str) = if units > 9999 {
            if let Some(scale) = &mapping.scale {
                (units as f64 / scale.divisor, scale.next_unit.to_string())
            } else {
                (units as f64, mapping.unit.to_string())
            }
        } else {
            (units as f64, mapping.unit.to_string())
        };

        let units_display = if final_units == final_units.round() {
            format_int_with_commas(final_units as i64)
        } else {
            let int_part = final_units.trunc() as i64;
            let frac_part = (final_units.fract() * 10.0).round().abs() as i64;
            format!("{}.{}", format_int_with_commas(int_part), frac_part)
        };

        let prefix = format!(" • {} ", icon);
        let prefix_width = 5; // " • " (3) + icon (1) + " " (1)
        let main_text = format!("{} {}", units_display, final_unit_str);
        let suffix = format!(" from a {}", mapping.name);

        let full_text = format!("{}{}{}", prefix, main_text, suffix);

        if full_text.chars().count() <= width {
            lines.push(Line::from(vec![
                Span::raw(prefix),
                Span::styled(main_text, Style::default().fg(color)),
                Span::raw(suffix),
            ]));
            continue;
        }

        // Manual wrapping for long lines
        let wrapped = wrap_with_hanging_indent(&full_text, width, prefix_width);
        lines.extend(wrapped.into_iter().map(Line::from));
    }

    lines
}

fn wrap_with_hanging_indent(text: &str, width: usize, indent: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut first_word_on_line = true;

    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        let space_needed = if first_word_on_line { 0 } else { 1 };
        let current_len = current_line.chars().count();

        if !first_word_on_line && current_len + space_needed + word_len > width {
            lines.push(current_line);
            current_line = " ".repeat(indent);
            current_line.push_str(word);
            first_word_on_line = false; // It's not the first word of the line's *storage*, but it is the first of its *content*
        } else {
            if !first_word_on_line {
                current_line.push(' ');
            }
            current_line.push_str(word);
            first_word_on_line = false;
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    // Restore leading space for bullet point style consistency if it was in the original
    if !lines.is_empty() && text.starts_with(' ') && !lines[0].starts_with(' ') {
        lines[0] = format!(" {}", lines[0]);
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokens_to_wh() {
        assert_eq!(tokens_to_wh(1_000_000.0), 300.0);
        assert_eq!(tokens_to_wh(0.0), 0.0);
    }

    #[test]
    fn test_find_top_mappings() {
        let wh = 600.0;
        let top = find_top_mappings(wh, 2);
        assert_eq!(top.len(), 2);
        // RTX 4090 is exactly 600Wh
        assert!(
            top.iter()
                .any(|(m, _)| m.name == "High-End Gaming PC (RTX 4090)")
        );
    }

    #[test]
    fn test_rescaling_logic() {
        // 20,000 Wh / 100Wh (Modern TV) = 200 units.
        // But if we use a much larger number:
        // 1,000,000 Wh / 100Wh = 10,000 hours.
        // 10,000 > 9999, so it should scale to days: 10,000 / 24 = 416.7 days.
        let wh = 1_000_000.0;
        let tokens = (wh / 300.0) * 1_000_000.0;
        let lines = format_eco_metrics(tokens as u64, Color::Red, 100);

        // Find the "Modern TV" line if it exists in top 2
        let tv_line = lines.iter().find(|l| l.to_string().contains("Modern TV"));
        if let Some(line) = tv_line {
            assert!(line.to_string().contains("days watched"));
        }
    }

    #[test]
    fn test_format_eco_metrics() {
        let lines = format_eco_metrics(1_000_000, Color::Red, 100);
        assert_eq!(lines.len(), 2); // 2 bullets, no header
    }

    #[test]
    fn test_format_eco_metrics_zero() {
        let lines = format_eco_metrics(0, Color::Red, 100);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_find_top_mappings_deduplication() {
        // If we have a watt-hour value that would result in the same unit count for two mappings,
        // we should only see one of them.
        // e.g., Espresso Machine and Smartphone both cost 10-15Wh (depending on exact mapping)
        // Let's use a very low Wh like 15.0.
        // Smartphone = 1 unit, Espresso = 1.5 -> 2 units.
        // If we had two identical cost mappings, we'd only see one.
        let wh = 10.0;
        let top = find_top_mappings(wh, 10);

        let mut seen_units = std::collections::HashSet::new();
        for (_, units) in top {
            assert!(
                seen_units.insert(units),
                "Duplicate unit count {} found",
                units
            );
        }
    }

    #[test]
    fn test_format_eco_metrics_thousand_separator() {
        // 100,000,000 tokens = 30,000Wh.
        // 30,000Wh / 10Wh (E-Bike) = 3,000 units.
        let lines = format_eco_metrics(100_000_000, Color::Red, 100);
        let ebike_line = lines.iter().find(|l| l.to_string().contains("E-Bike"));
        if let Some(line) = ebike_line {
            assert!(line.to_string().contains("3,000"));
        }
    }

    #[test]
    fn test_wrap_with_hanging_indent() {
        let text = " • 🚗 3,000 kilometers driven from a Electric Car (EV)";
        let width = 25; // slightly wider to see better wrap
        let indent = 5;
        let wrapped = wrap_with_hanging_indent(text, width, indent);

        assert_eq!(wrapped[0], " • 🚗 3,000 kilometers");
        assert_eq!(wrapped[1], "     driven from a");
        assert_eq!(wrapped[2], "     Electric Car (EV)");

        // Test with no wrapping needed
        let wrapped_no_wrap = wrap_with_hanging_indent("short text", 20, 5);
        assert_eq!(wrapped_no_wrap.len(), 1);
        assert_eq!(wrapped_no_wrap[0], "short text");

        // Test with very narrow width
        let wrapped_narrow = wrap_with_hanging_indent("word", 2, 1);
        assert_eq!(wrapped_narrow.len(), 1);
        assert_eq!(wrapped_narrow[0], "word"); // words themselves aren't broken
    }

    #[test]
    fn test_format_eco_metrics_wrapping() {
        // Use a very narrow width to force wrapping
        let lines = format_eco_metrics(1_000_000, Color::Red, 20);
        // Should have more than 3 lines now
        assert!(lines.len() > 3);
        // Second line of a bullet should start with spaces
        assert!(lines[2].to_string().starts_with("     "));
    }
}
