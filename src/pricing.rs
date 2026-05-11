pub struct ModelPricing {
    pub input: f64,       // Price per 1M tokens
    pub output: f64,      // Price per 1M tokens
    pub cache_write: f64, // Price per 1M tokens (Claude "cache_create", Gemini cache)
    pub cache_read: f64,  // Price per 1M tokens (Claude "cache_read")
}

impl Default for ModelPricing {
    fn default() -> Self {
        Self {
            input: 0.0,
            output: 0.0,
            cache_write: 0.0,
            cache_read: 0.0,
        }
    }
}

pub fn get_claude_pricing(model: &str) -> ModelPricing {
    let m = model.to_lowercase();
    if m.contains("sonnet-4-6") || m.contains("sonnet-3-5") {
        ModelPricing {
            input: 3.00,
            output: 15.00,
            cache_write: 3.75,
            cache_read: 0.30,
        }
    } else if m.contains("opus-4-7") || m.contains("opus-3") {
        ModelPricing {
            input: 5.00,
            output: 25.00,
            cache_write: 6.25,
            cache_read: 0.50,
        }
    } else if m.contains("haiku-4-5") || m.contains("haiku-3-5") {
        ModelPricing {
            input: 1.00,
            output: 5.00,
            cache_write: 1.25,
            cache_read: 0.10,
        }
    } else {
        // Default to Sonnet
        ModelPricing {
            input: 3.00,
            output: 15.00,
            cache_write: 3.75,
            cache_read: 0.30,
        }
    }
}

pub fn get_gemini_pricing(model: &str, input_count: i64) -> ModelPricing {
    let m = model.to_lowercase();
    let (input, output, cache) = if m.contains("gemini-3.1-flash-lite") {
        (0.25, 1.50, 0.025)
    } else if m.contains("gemini-3-pro") || m.contains("gemini-3.1-pro") {
        if input_count <= 200_000 {
            (2.00, 12.00, 0.20)
        } else {
            (4.00, 18.00, 0.40)
        }
    } else if m.contains("gemini-3-flash") {
        (0.50, 3.00, 0.05)
    } else if m.contains("gemini-1.5-pro") || m.contains("gemini-2.5-pro") {
        if input_count <= 128_000 {
            (1.25, 5.00, 0.3125)
        } else {
            (2.50, 10.00, 0.625)
        }
    } else if m.contains("gemini-1.5-flash")
        || m.contains("gemini-2.0-flash")
        || m.contains("gemini-2.5-flash")
    {
        if input_count <= 128_000 {
            (0.075, 0.30, 0.01875)
        } else {
            (0.15, 0.60, 0.0375)
        }
    } else {
        (1.00, 4.00, 0.10)
    };

    ModelPricing {
        input,
        output,
        cache_write: cache,
        cache_read: 0.0, // Gemini only has one cache price point in current logic
    }
}

pub fn get_deepseek_pricing(model: &str) -> ModelPricing {
    let m = model.to_lowercase();
    if m.contains("deepseek-v4-flash") {
        ModelPricing {
            input: 0.14,
            output: 0.28,
            cache_write: 0.0,
            cache_read: 0.0028,
        }
    } else if m.contains("deepseek-v4-pro") {
        ModelPricing {
            input: 0.435,
            output: 0.87,
            cache_write: 0.0,
            cache_read: 0.003625,
        }
    } else {
        ModelPricing::default()
    }
}

pub fn get_pricing(model: &str, input_count: i64) -> ModelPricing {
    let m = model.to_lowercase();
    if m.contains("claude") {
        get_claude_pricing(model)
    } else if m.contains("gemini") {
        get_gemini_pricing(model, input_count)
    } else if m.contains("deepseek") {
        get_deepseek_pricing(model)
    } else {
        // Try deepseek then gemini then claude (which has fallback)
        let p = get_deepseek_pricing(model);
        if p.input > 0.0 {
            return p;
        }
        let p = get_gemini_pricing(model, input_count);
        if p.input != 1.0 || m.contains("gemini") {
            return p;
        }
        get_claude_pricing(model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unified_pricing() {
        let p = get_pricing("claude-sonnet", 0);
        assert_eq!(p.input, 3.0);

        let p = get_pricing("gemini-3.1-pro", 0);
        assert_eq!(p.input, 2.0);

        let p = get_pricing("deepseek/deepseek-v4-flash", 0);
        assert_eq!(p.input, 0.14);
    }

    #[test]
    fn test_deepseek_pricing_logic() {
        let p = get_deepseek_pricing("deepseek/deepseek-v4-flash");
        assert_eq!(p.input, 0.14);
        assert_eq!(p.output, 0.28);
        assert_eq!(p.cache_read, 0.0028);

        let p = get_deepseek_pricing("deepseek/deepseek-v4-pro");
        assert_eq!(p.input, 0.435);
        assert_eq!(p.output, 0.87);
        assert_eq!(p.cache_read, 0.003625);

        let p = get_deepseek_pricing("unknown");
        assert_eq!(p.input, 0.0);
    }

    #[test]
    fn test_claude_pricing_logic() {
        let p = get_claude_pricing("claude-sonnet-4-6-20251001");
        assert_eq!(p.input, 3.00);
        assert_eq!(p.output, 15.00);
        assert_eq!(p.cache_write, 3.75);
        assert_eq!(p.cache_read, 0.30);

        let p = get_claude_pricing("claude-opus-4-7");
        assert_eq!(p.input, 5.00);
        assert_eq!(p.output, 25.00);

        let p = get_claude_pricing("claude-haiku-4-5");
        assert_eq!(p.input, 1.00);
        assert_eq!(p.output, 5.00);

        let p = get_claude_pricing("unknown");
        assert_eq!(p.input, 3.00); // default
    }

    #[test]
    fn test_gemini_pricing_logic() {
        // Flash Lite
        let p = get_gemini_pricing("gemini-3.1-flash-lite-preview", 0);
        assert_eq!(p.input, 0.25);
        assert_eq!(p.output, 1.50);
        assert_eq!(p.cache_write, 0.025);

        // 3.1 Pro (low context)
        let p = get_gemini_pricing("gemini-3.1-pro", 100_000);
        assert_eq!(p.input, 2.00);
        assert_eq!(p.output, 12.00);

        // 3.1 Pro (high context)
        let p = get_gemini_pricing("gemini-3.1-pro", 300_000);
        assert_eq!(p.input, 4.00);
        assert_eq!(p.output, 18.00);

        // 1.5 Pro (low context)
        let p = get_gemini_pricing("gemini-1.5-pro", 50_000);
        assert_eq!(p.input, 1.25);
        assert_eq!(p.output, 5.00);

        // 1.5 Pro (high context)
        let p = get_gemini_pricing("gemini-1.5-pro", 200_000);
        assert_eq!(p.input, 2.50);
        assert_eq!(p.output, 10.00);
    }
}
