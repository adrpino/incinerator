use chrono::Datelike;
use std::sync::atomic::{AtomicBool, Ordering};

static TUI_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_tui_mode(enabled: bool) {
    TUI_MODE.store(enabled, Ordering::Relaxed);
}

pub fn is_tui_mode() -> bool {
    TUI_MODE.load(Ordering::Relaxed)
}

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
    if m.contains("sonnet") {
        ModelPricing {
            input: 3.00,
            output: 15.00,
            cache_write: 3.75,
            cache_read: 0.30,
        }
    } else if m.contains("fable-5.1") || m.contains("fable-5-1") || m.contains("fable-5_1") {
        ModelPricing {
            input: 10.00,
            output: 50.00,
            cache_write: 12.50,
            cache_read: 0.25,
        }
    } else if m.contains("fable") {
        ModelPricing {
            input: 10.00,
            output: 50.00,
            cache_write: 12.50,
            cache_read: 1.00,
        }
    } else if m.contains("opus-3") || m.contains("3-opus") {
        ModelPricing {
            input: 15.00,
            output: 75.00,
            cache_write: 18.75,
            cache_read: 1.50,
        }
    } else if m.contains("opus") {
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
    } else if m.contains("synthetic") {
        ModelPricing::default()
    } else {
        if !is_tui_mode() {
            eprintln!(
                "Warning: Unknown Claude model '{}', defaulting to Sonnet pricing.",
                model
            );
        }
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
    let (input, output, cache) = if m.contains("gemini-3.7-flash") {
        if chrono::Local::now().year() <= 2026 {
            (0.75, 3.75, 0.075)
        } else {
            (1.50, 7.50, 0.15)
        }
    } else if m.contains("gemini-3.1-flash-lite") {
        (0.25, 1.50, 0.025)
    } else if m.contains("gemini-3-pro") || m.contains("gemini-3.1-pro") {
        if input_count <= 200_000 {
            (2.00, 12.00, 0.20)
        } else {
            (4.00, 18.00, 0.40)
        }
    } else if m.contains("gemini-3-flash") {
        (0.50, 3.00, 0.05)
    } else if m.contains("gemini-3.6-flash") {
        (1.50, 7.50, 0.15)
    } else if m.contains("gemini-3.5-flash") {
        (1.50, 9.00, 0.15)
    } else if m.contains("gemini-2.5-pro") {
        if input_count <= 200_000 {
            (1.25, 10.00, 0.125)
        } else {
            (2.50, 15.00, 0.25)
        }
    } else if m.contains("gemini-2.5-flash") {
        (0.30, 2.50, 0.03)
    } else if m.contains("gemini-2.0-flash") {
        (0.10, 0.40, 0.025)
    } else if m.contains("gemini-1.5-pro") {
        if input_count <= 128_000 {
            (1.25, 5.00, 0.3125)
        } else {
            (2.50, 10.00, 0.625)
        }
    } else if m.contains("gemini-1.5-flash") {
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
        cache_write: 0.0,
        cache_read: cache,
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

pub fn get_openai_pricing(model: &str) -> ModelPricing {
    let m = model.to_lowercase();
    if m.contains("gpt-5.6-sol") {
        ModelPricing {
            input: 5.00,
            output: 30.00,
            cache_write: 6.25,
            cache_read: 0.50,
        }
    } else if m.contains("gpt-5.6-terra") {
        ModelPricing {
            input: 2.00,
            output: 12.00,
            cache_write: 2.50,
            cache_read: 0.20,
        }
    } else if m.contains("gpt-5.6-luna") {
        ModelPricing {
            input: 0.20,
            output: 1.20,
            cache_write: 0.25,
            cache_read: 0.02,
        }
    } else if m.contains("gpt-5.5-pro") {
        ModelPricing {
            input: 30.00,
            output: 180.00,
            cache_write: 0.0,
            cache_read: 15.00,
        }
    } else if m.contains("gpt-5.5") {
        ModelPricing {
            input: 5.00,
            output: 30.00,
            cache_write: 0.0,
            cache_read: 0.50,
        }
    } else if m.contains("gpt-5.4-mini") {
        ModelPricing {
            input: 0.75,
            output: 4.50,
            cache_write: 0.0,
            cache_read: 0.075,
        }
    } else if m.contains("gpt-5.4") {
        ModelPricing {
            input: 2.50,
            output: 15.00,
            cache_write: 0.0,
            cache_read: 0.25,
        }
    } else if m.contains("gpt-5-mini") {
        ModelPricing {
            input: 0.15,
            output: 0.60,
            cache_write: 0.0,
            cache_read: 0.015,
        }
    } else if m.contains("gpt-5-nano") {
        ModelPricing {
            input: 0.05,
            output: 0.20,
            cache_write: 0.0,
            cache_read: 0.005,
        }
    } else if m.contains("gpt-5") {
        ModelPricing {
            input: 1.25,
            output: 10.00,
            cache_write: 0.0,
            cache_read: 0.125,
        }
    } else if m.contains("o1-preview") {
        ModelPricing {
            input: 15.00,
            output: 60.00,
            cache_write: 0.0,
            cache_read: 7.50,
        }
    } else if m.contains("o1-mini") {
        ModelPricing {
            input: 3.00,
            output: 12.00,
            cache_write: 0.0,
            cache_read: 1.50,
        }
    } else if m.contains("gpt-4o-mini") {
        ModelPricing {
            input: 0.15,
            output: 0.60,
            cache_write: 0.0,
            cache_read: 0.075,
        }
    } else if m.contains("gpt-4o") {
        ModelPricing {
            input: 2.50,
            output: 10.00,
            cache_write: 0.0,
            cache_read: 1.25,
        }
    } else {
        // Fallback for unknown OpenAI
        ModelPricing {
            input: 2.50,
            output: 10.00,
            cache_write: 0.0,
            cache_read: 1.25,
        }
    }
}

pub fn get_glm_pricing(model: &str) -> ModelPricing {
    let m = model.to_lowercase();
    if m.contains("glm-5.2") {
        ModelPricing {
            input: 1.40,
            output: 4.40,
            cache_write: 0.0,
            cache_read: 0.26,
        }
    } else {
        ModelPricing::default()
    }
}

pub fn get_kimi_pricing(model: &str) -> ModelPricing {
    let m = model.to_lowercase();
    if m.contains("kimi-k3") || m.contains("kimi_k3") {
        ModelPricing {
            input: 3.00,
            output: 15.00,
            cache_write: 0.0,
            cache_read: 0.30,
        }
    } else if m.contains("kimi-k2.7")
        || m.contains("kimi_k2.7")
        || m.contains("kimi-k2")
        || m.contains("kimi_k2")
    {
        ModelPricing {
            input: 0.95,
            output: 4.00,
            cache_write: 0.0,
            cache_read: 0.19,
        }
    } else if m.contains("kimi") {
        ModelPricing {
            input: 3.00,
            output: 15.00,
            cache_write: 0.0,
            cache_read: 0.30,
        }
    } else {
        ModelPricing::default()
    }
}

pub fn get_pricing(model: &str, input_count: i64) -> ModelPricing {
    let m = model.to_lowercase();
    if m.contains("claude") || m.contains("anthropic") || m.contains("fable") {
        get_claude_pricing(model)
    } else if m.contains("gemini") {
        get_gemini_pricing(model, input_count)
    } else if m.contains("deepseek") {
        get_deepseek_pricing(model)
    } else if m.contains("gpt-") || m.contains("o1-") {
        get_openai_pricing(model)
    } else if m.contains("glm-") {
        get_glm_pricing(model)
    } else if m.contains("kimi") {
        get_kimi_pricing(model)
    } else {
        // Try GLM then kimi then deepseek then gemini then openai then claude
        let p = get_glm_pricing(model);
        if p.input > 0.0 {
            return p;
        }
        let p = get_kimi_pricing(model);
        if p.input > 0.0 {
            return p;
        }
        let p = get_deepseek_pricing(model);
        if p.input > 0.0 {
            return p;
        }
        let p = get_gemini_pricing(model, input_count);
        if p.input != 1.0 || m.contains("gemini") {
            return p;
        }
        let p = get_openai_pricing(model);
        if p.input != 2.5 || m.contains("gpt") {
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

        let p = get_pricing("claude-opus-5", 0);
        assert_eq!(p.input, 5.00);
        assert_eq!(p.output, 25.00);
        assert_eq!(p.cache_write, 6.25);
        assert_eq!(p.cache_read, 0.50);

        let p = get_pricing("claude-fable-5.1", 0);
        assert_eq!(p.input, 10.00);
        assert_eq!(p.output, 50.00);
        assert_eq!(p.cache_write, 12.50);
        assert_eq!(p.cache_read, 0.25);

        let p = get_pricing("fable-5.1", 0);
        assert_eq!(p.input, 10.00);
        assert_eq!(p.output, 50.00);
        assert_eq!(p.cache_write, 12.50);
        assert_eq!(p.cache_read, 0.25);

        let p = get_pricing("gemini-3.1-pro", 0);
        assert_eq!(p.input, 2.0);

        let p = get_pricing("deepseek/deepseek-v4-flash", 0);
        assert_eq!(p.input, 0.14);

        let p = get_pricing("gpt-4o", 0);
        assert_eq!(p.input, 2.50);

        let p = get_pricing("gpt-5", 0);
        assert_eq!(p.input, 1.25);

        let p = get_pricing("z-ai/glm-5.2", 0);
        assert_eq!(p.input, 1.40);
        assert_eq!(p.output, 4.40);
        assert_eq!(p.cache_read, 0.26);

        let p = get_pricing("copilot/kimi-k3", 0);
        assert_eq!(p.input, 3.00);
        assert_eq!(p.output, 15.00);
        assert_eq!(p.cache_read, 0.30);

        let p = get_pricing("kimi-k3", 0);
        assert_eq!(p.input, 3.00);
        assert_eq!(p.output, 15.00);
        assert_eq!(p.cache_read, 0.30);

        let p = get_pricing("moonshot-ai/kimi-k3-preview", 0);
        assert_eq!(p.input, 3.00);
        assert_eq!(p.output, 15.00);
        assert_eq!(p.cache_read, 0.30);

        let p = get_pricing("copilot/kimi-k2.7", 0);
        assert_eq!(p.input, 0.95);
        assert_eq!(p.output, 4.00);
        assert_eq!(p.cache_read, 0.19);
    }

    #[test]
    fn test_kimi_pricing_logic() {
        for model in &[
            "kimi-k3",
            "copilot/kimi-k3",
            "moonshot-ai/kimi-k3",
            "kimi_k3",
            "custom-kimi-k3-long-context",
        ] {
            let p = get_kimi_pricing(model);
            assert_eq!(p.input, 3.00, "Failed for {}", model);
            assert_eq!(p.output, 15.00, "Failed for {}", model);
            assert_eq!(p.cache_read, 0.30, "Failed for {}", model);
            assert_eq!(p.cache_write, 0.0, "Failed for {}", model);
        }

        for model in &[
            "kimi-k2.7",
            "copilot/kimi-k2.7",
            "kimi-k2",
            "kimi_k2.7",
            "moonshot-ai/kimi-k2.7-code",
        ] {
            let p = get_kimi_pricing(model);
            assert_eq!(p.input, 0.95, "Failed for {}", model);
            assert_eq!(p.output, 4.00, "Failed for {}", model);
            assert_eq!(p.cache_read, 0.19, "Failed for {}", model);
            assert_eq!(p.cache_write, 0.0, "Failed for {}", model);
        }

        let p = get_kimi_pricing("unknown-model");
        assert_eq!(p.input, 0.0);
    }

    #[test]
    fn test_glm_pricing_logic() {
        let p = get_glm_pricing("z-ai/glm-5.2");
        assert_eq!(p.input, 1.40);
        assert_eq!(p.output, 4.40);
        assert_eq!(p.cache_read, 0.26);
        assert_eq!(p.cache_write, 0.0);

        let p = get_glm_pricing("unknown");
        assert_eq!(p.input, 0.0);
    }

    #[test]
    fn test_openai_pricing_logic() {
        let p = get_openai_pricing("gpt-4o");
        assert_eq!(p.input, 2.50);
        assert_eq!(p.output, 10.00);
        assert_eq!(p.cache_read, 1.25);

        let p = get_openai_pricing("gpt-5");
        assert_eq!(p.input, 1.25);
        assert_eq!(p.output, 10.00);

        let p = get_openai_pricing("gpt-5.5");
        assert_eq!(p.input, 5.00);
        assert_eq!(p.output, 30.00);
        assert_eq!(p.cache_read, 0.50);

        let p = get_openai_pricing("gpt-5.4");
        assert_eq!(p.input, 2.50);
        assert_eq!(p.output, 15.00);
        assert_eq!(p.cache_read, 0.25);

        let p = get_openai_pricing("gpt-5.4-mini");
        assert_eq!(p.input, 0.75);
        assert_eq!(p.output, 4.50);
        assert_eq!(p.cache_read, 0.075);

        let p = get_openai_pricing("gpt-5.5-pro");
        assert_eq!(p.input, 30.00);
        assert_eq!(p.output, 180.00);

        let p = get_openai_pricing("gpt-5.6-sol");
        assert_eq!(p.input, 5.00);
        assert_eq!(p.output, 30.00);
        assert_eq!(p.cache_read, 0.50);
        assert_eq!(p.cache_write, 6.25);

        let p = get_openai_pricing("gpt-5.6-terra");
        assert_eq!(p.input, 2.00);
        assert_eq!(p.output, 12.00);
        assert_eq!(p.cache_read, 0.20);
        assert_eq!(p.cache_write, 2.50);

        let p = get_openai_pricing("gpt-5.6-luna");
        assert_eq!(p.input, 0.20);
        assert_eq!(p.output, 1.20);
        assert_eq!(p.cache_read, 0.02);
        assert_eq!(p.cache_write, 0.25);
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
        let p = get_claude_pricing("claude-sonnet-5");
        assert_eq!(p.input, 3.00);
        assert_eq!(p.output, 15.00);
        assert_eq!(p.cache_write, 3.75);
        assert_eq!(p.cache_read, 0.30);

        let p = get_claude_pricing("claude-sonnet-4-6-20251001");
        assert_eq!(p.input, 3.00);
        assert_eq!(p.output, 15.00);
        assert_eq!(p.cache_write, 3.75);
        assert_eq!(p.cache_read, 0.30);

        let p = get_claude_pricing("claude-4.5-sonnet");
        assert_eq!(p.input, 3.00);
        assert_eq!(p.output, 15.00);
        assert_eq!(p.cache_write, 3.75);
        assert_eq!(p.cache_read, 0.30);

        let p = get_claude_pricing("claude-3-5-sonnet-20241022");
        assert_eq!(p.input, 3.00);
        assert_eq!(p.output, 15.00);
        assert_eq!(p.cache_write, 3.75);
        assert_eq!(p.cache_read, 0.30);

        let p = get_claude_pricing("claude-opus-4-7");
        assert_eq!(p.input, 5.00);
        assert_eq!(p.output, 25.00);

        let p = get_claude_pricing("claude-opus-4-8");
        assert_eq!(p.input, 5.00);
        assert_eq!(p.output, 25.00);
        assert_eq!(p.cache_write, 6.25);
        assert_eq!(p.cache_read, 0.50);

        let p = get_claude_pricing("claude-opus-5");
        assert_eq!(p.input, 5.00);
        assert_eq!(p.output, 25.00);
        assert_eq!(p.cache_write, 6.25);
        assert_eq!(p.cache_read, 0.50);

        let p = get_claude_pricing("claude-5-opus");
        assert_eq!(p.input, 5.00);
        assert_eq!(p.output, 25.00);
        assert_eq!(p.cache_write, 6.25);
        assert_eq!(p.cache_read, 0.50);

        let p = get_claude_pricing("anthropic.claude-opus-5");
        assert_eq!(p.input, 5.00);
        assert_eq!(p.output, 25.00);
        assert_eq!(p.cache_write, 6.25);
        assert_eq!(p.cache_read, 0.50);

        let p = get_claude_pricing("claude-4.6-opus");
        assert_eq!(p.input, 5.00);
        assert_eq!(p.output, 25.00);
        assert_eq!(p.cache_write, 6.25);
        assert_eq!(p.cache_read, 0.50);

        let p = get_claude_pricing("claude-fable-5");
        assert_eq!(p.input, 10.00);
        assert_eq!(p.output, 50.00);
        assert_eq!(p.cache_write, 12.50);
        assert_eq!(p.cache_read, 1.00);

        let p = get_claude_pricing("claude-fable-5.1");
        assert_eq!(p.input, 10.00);
        assert_eq!(p.output, 50.00);
        assert_eq!(p.cache_write, 12.50);
        assert_eq!(p.cache_read, 0.25);

        let p = get_claude_pricing("claude-fable-5-1");
        assert_eq!(p.input, 10.00);
        assert_eq!(p.output, 50.00);
        assert_eq!(p.cache_write, 12.50);
        assert_eq!(p.cache_read, 0.25);

        let p = get_claude_pricing("claude-opus-3");
        assert_eq!(p.input, 15.00);
        assert_eq!(p.output, 75.00);
        assert_eq!(p.cache_write, 18.75);
        assert_eq!(p.cache_read, 1.50);

        let p = get_claude_pricing("claude-3-opus-20240229");
        assert_eq!(p.input, 15.00);
        assert_eq!(p.output, 75.00);
        assert_eq!(p.cache_write, 18.75);
        assert_eq!(p.cache_read, 1.50);

        let p = get_claude_pricing("claude-haiku-4-5");
        assert_eq!(p.input, 1.00);
        assert_eq!(p.output, 5.00);

        let p = get_claude_pricing("<synthetic>");
        assert_eq!(p.input, 0.0);
        assert_eq!(p.output, 0.0);

        let p = get_claude_pricing("synthetic");
        assert_eq!(p.input, 0.0);
        assert_eq!(p.output, 0.0);

        let p = get_claude_pricing("unknown");
        assert_eq!(p.input, 3.00); // default
    }

    #[test]
    fn test_unknown_model_tui_mode_suppression() {
        if std::env::var("RUN_SUBPROCESS_TEST").is_ok() {
            set_tui_mode(true);
            get_claude_pricing("some-completely-unknown-model");
            return;
        }

        let current_exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(current_exe)
            .arg("test_unknown_model_tui_mode_suppression")
            .arg("--nocapture")
            .env("RUN_SUBPROCESS_TEST", "1")
            .output()
            .expect("failed to execute subprocess");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
        assert!(
            !combined.contains("Warning: Unknown Claude model"),
            "Output contains TUI polluting warning:\n{}",
            combined
        );
    }

    #[test]
    fn test_unknown_model_warning_when_not_tui_mode() {
        if std::env::var("RUN_SUBPROCESS_TEST").is_ok() {
            set_tui_mode(false);
            get_claude_pricing("some-completely-unknown-model-to-trigger-warning");
            return;
        }

        let current_exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(current_exe)
            .arg("test_unknown_model_warning_when_not_tui_mode")
            .arg("--nocapture")
            .env("RUN_SUBPROCESS_TEST", "1")
            .output()
            .expect("failed to execute subprocess");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
        assert!(
            combined.contains(
                "Warning: Unknown Claude model 'some-completely-unknown-model-to-trigger-warning'"
            ),
            "Output did not contain warning:\n{}",
            combined
        );
    }

    #[test]
    fn test_synthetic_model_never_warns() {
        if std::env::var("RUN_SUBPROCESS_TEST").is_ok() {
            set_tui_mode(false);
            get_claude_pricing("<synthetic>");
            get_claude_pricing("synthetic-something");
            return;
        }

        let current_exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(current_exe)
            .arg("test_synthetic_model_never_warns")
            .arg("--nocapture")
            .env("RUN_SUBPROCESS_TEST", "1")
            .output()
            .expect("failed to execute subprocess");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
        assert!(
            !combined.contains("Warning: Unknown Claude model"),
            "Synthetic model should never trigger warnings:\n{}",
            combined
        );
    }

    #[test]
    fn test_kimi_model_never_warns() {
        if std::env::var("RUN_SUBPROCESS_TEST").is_ok() {
            set_tui_mode(false);
            get_pricing("copilot/kimi-k3", 0);
            get_pricing("kimi-k3", 0);
            get_pricing("moonshot/kimi-k3-preview", 0);
            return;
        }

        let current_exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(current_exe)
            .arg("test_kimi_model_never_warns")
            .arg("--nocapture")
            .env("RUN_SUBPROCESS_TEST", "1")
            .output()
            .expect("failed to execute subprocess");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
        assert!(
            !combined.contains("Warning: Unknown Claude model"),
            "Kimi model should never trigger unknown Claude model warnings:\n{}",
            combined
        );
    }

    #[test]
    fn test_opus_and_fable_model_never_warns() {
        if std::env::var("RUN_SUBPROCESS_TEST").is_ok() {
            set_tui_mode(false);
            get_pricing("claude-opus-5", 0);
            get_pricing("claude-opus-4-8", 0);
            get_pricing("anthropic.claude-opus-5", 0);
            get_pricing("claude-3-opus", 0);
            get_pricing("claude-fable-5.1", 0);
            get_pricing("fable-5.1", 0);
            return;
        }

        let current_exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(current_exe)
            .arg("test_opus_and_fable_model_never_warns")
            .arg("--nocapture")
            .env("RUN_SUBPROCESS_TEST", "1")
            .output()
            .expect("failed to execute subprocess");

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr);
        assert!(
            !combined.contains("Warning: Unknown Claude model"),
            "Opus/Fable model should never trigger unknown Claude model warnings:\n{}",
            combined
        );
    }

    #[test]
    fn test_gemini_pricing_logic() {
        // Flash Lite
        let p = get_gemini_pricing("gemini-3.1-flash-lite-preview", 0);
        assert_eq!(p.input, 0.25);
        assert_eq!(p.output, 1.50);
        assert_eq!(p.cache_read, 0.025);
        assert_eq!(p.cache_write, 0.0);

        // 3.7 Flash
        let p = get_gemini_pricing("gemini-3.7-flash", 0);
        if chrono::Local::now().year() <= 2026 {
            assert_eq!(p.input, 0.75);
            assert_eq!(p.output, 3.75);
            assert_eq!(p.cache_read, 0.075);
        } else {
            assert_eq!(p.input, 1.50);
            assert_eq!(p.output, 7.50);
            assert_eq!(p.cache_read, 0.15);
        }
        assert_eq!(p.cache_write, 0.0);

        // 3.6 Flash
        let p = get_gemini_pricing("gemini-3.6-flash", 0);
        assert_eq!(p.input, 1.50);
        assert_eq!(p.output, 7.50);
        assert_eq!(p.cache_read, 0.15);
        assert_eq!(p.cache_write, 0.0);

        // 3.5 Flash
        let p = get_gemini_pricing("gemini-3.5-flash", 0);
        assert_eq!(p.input, 1.50);
        assert_eq!(p.output, 9.00);
        assert_eq!(p.cache_read, 0.15);
        assert_eq!(p.cache_write, 0.0);

        // 3.1 Pro (low context)
        let p = get_gemini_pricing("gemini-3.1-pro", 100_000);
        assert_eq!(p.input, 2.00);
        assert_eq!(p.output, 12.00);
        assert_eq!(p.cache_read, 0.20);
        assert_eq!(p.cache_write, 0.0);

        // 3.1 Pro (high context)
        let p = get_gemini_pricing("gemini-3.1-pro", 300_000);
        assert_eq!(p.input, 4.00);
        assert_eq!(p.output, 18.00);
        assert_eq!(p.cache_read, 0.40);
        assert_eq!(p.cache_write, 0.0);

        // 2.5 Pro (low context)
        let p = get_gemini_pricing("gemini-2.5-pro", 150_000);
        assert_eq!(p.input, 1.25);
        assert_eq!(p.output, 10.00);
        assert_eq!(p.cache_read, 0.125);
        assert_eq!(p.cache_write, 0.0);

        // 2.5 Pro (high context)
        let p = get_gemini_pricing("gemini-2.5-pro", 250_000);
        assert_eq!(p.input, 2.50);
        assert_eq!(p.output, 15.00);
        assert_eq!(p.cache_read, 0.25);
        assert_eq!(p.cache_write, 0.0);

        // 2.5 Flash
        let p = get_gemini_pricing("gemini-2.5-flash", 0);
        assert_eq!(p.input, 0.30);
        assert_eq!(p.output, 2.50);
        assert_eq!(p.cache_read, 0.03);
        assert_eq!(p.cache_write, 0.0);

        // 2.0 Flash
        let p = get_gemini_pricing("gemini-2.0-flash", 0);
        assert_eq!(p.input, 0.10);
        assert_eq!(p.output, 0.40);
        assert_eq!(p.cache_read, 0.025);
        assert_eq!(p.cache_write, 0.0);

        // 1.5 Pro (low context)
        let p = get_gemini_pricing("gemini-1.5-pro", 50_000);
        assert_eq!(p.input, 1.25);
        assert_eq!(p.output, 5.00);
        assert_eq!(p.cache_read, 0.3125);
        assert_eq!(p.cache_write, 0.0);

        // 1.5 Pro (high context)
        let p = get_gemini_pricing("gemini-1.5-pro", 200_000);
        assert_eq!(p.input, 2.50);
        assert_eq!(p.output, 10.00);
        assert_eq!(p.cache_read, 0.625);
        assert_eq!(p.cache_write, 0.0);

        // 1.5 Flash (low context)
        let p = get_gemini_pricing("gemini-1.5-flash", 50_000);
        assert_eq!(p.input, 0.075);
        assert_eq!(p.output, 0.30);
        assert_eq!(p.cache_read, 0.01875);
        assert_eq!(p.cache_write, 0.0);

        // 1.5 Flash (high context)
        let p = get_gemini_pricing("gemini-1.5-flash", 150_000);
        assert_eq!(p.input, 0.15);
        assert_eq!(p.output, 0.60);
        assert_eq!(p.cache_read, 0.0375);
        assert_eq!(p.cache_write, 0.0);
    }
}
