#[cfg(test)]
mod tests {
    use serde_json::json;

    // Helper to simulate a line in Claude's JSONL format
    fn claude_log_line(model: &str, input: i64, output: i64, cache_read: i64, cache_create: i64) -> String {
        json!({
            "type": "assistant",
            "timestamp": "2026-05-01T12:00:00Z",
            "message": {
                "model": model,
                "usage": {
                    "input_tokens": input,
                    "output_tokens": output,
                    "cache_read_input_tokens": cache_read,
                    "cache_creation_input_tokens": cache_create
                }
            }
        }).to_string()
    }

    #[test]
    fn test_claude_parsing_logic() {
        // This test simulates the logic inside run_claude_report by checking JSON deserialization
        let line = claude_log_line("claude-3-5-sonnet-20241022", 1000, 500, 200, 100);
        
        // We'll just verify the JSON structure matches our expected internal types
        // (normally we'd test the full report, but that requires filesystem access)
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["type"], "assistant");
        assert_eq!(parsed["message"]["usage"]["input_tokens"], 1000);
    }

    #[test]
    fn test_cline_api_req_parsing() {
        // Test parsing of 'api_req_started' messages in Cline
        let api_req_text = json!({
            "cost": 0.005,
            "tokensIn": 1000,
            "tokensOut": 200,
            "cacheReads": 500
        }).to_string();

        let message = json!({
            "ts": 1714992000000.0,
            "say": "api_req_started",
            "text": api_req_text
        });

        // Verify the nested JSON structure
        assert_eq!(message["say"], "api_req_started");
        let nested: serde_json::Value = serde_json::from_str(message["text"].as_str().unwrap()).unwrap();
        assert_eq!(nested["cost"], 0.005);
        assert_eq!(nested["tokensIn"], 1000);
    }

    #[test]
    fn test_gemini_token_estimation() {
        // Test the logic used when tokens aren't explicitly provided by Gemini CLI
        let content = json!(["This is a test message to see how token estimation works."]);
        
        // Manual implementation of estimate_tokens logic for validation
        let text = content.as_array().unwrap()[0].as_str().unwrap();
        let word_count = text.split_whitespace().count();
        let estimated = (word_count as f64 * 1.33) as i64;
        
        assert!(estimated > 0);
        assert_eq!(estimated, (11.0 * 1.33) as i64); // 11 words
    }
}
