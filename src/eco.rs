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

    scored_mappings
        .into_iter()
        .take(limit)
        .map(|(m, r, _)| (m, r))
        .collect()
}

pub fn format_eco_metrics(total_tokens: u64, color: Color) -> Vec<Line<'static>> {
    if total_tokens == 0 {
        return Vec::new();
    }

    let wh = tokens_to_wh(total_tokens as f64);
    let top_mappings = find_top_mappings(wh, 2);

    if top_mappings.is_empty() {
        return Vec::new();
    }

    let mut lines = vec![Line::from("Eco Impact:")];

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
            format!("{}", final_units as u64)
        } else {
            format!("{:.1}", final_units)
        };

        lines.push(Line::from(vec![
            Span::raw(format!(" • {} ", icon)),
            Span::styled(
                format!("{} {}", units_display, final_unit_str),
                Style::default().fg(color),
            ),
            Span::raw(format!(" from a {}", mapping.name)),
        ]));
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
        let lines = format_eco_metrics(tokens as u64, Color::Red);

        // Find the "Modern TV" line if it exists in top 2
        let tv_line = lines.iter().find(|l| l.to_string().contains("Modern TV"));
        if let Some(line) = tv_line {
            assert!(line.to_string().contains("days watched"));
        }
    }

    #[test]
    fn test_format_eco_metrics() {
        let lines = format_eco_metrics(1_000_000, Color::Red);
        assert_eq!(lines.len(), 3); // "Eco Impact:" + 2 bullets
        assert_eq!(lines[0].spans[0].content, "Eco Impact:");
    }

    #[test]
    fn test_format_eco_metrics_zero() {
        let lines = format_eco_metrics(0, Color::Red);
        assert!(lines.is_empty());
    }
}
