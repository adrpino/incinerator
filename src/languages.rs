use phf::phf_map;
use std::collections::HashMap;

static EXTENSION_MAP: phf::Map<&'static str, &'static str> = phf_map! {
    "rs" => "Rust",
    "py" => "Python",
    "js" => "JavaScript",
    "ts" => "TypeScript",
    "jsx" => "JavaScript",
    "tsx" => "TypeScript",
    "go" => "Go",
    "c" => "C",
    "cpp" => "C++",
    "h" => "C/C++ Header",
    "hpp" => "C++ Header",
    "cs" => "C#",
    "java" => "Java",
    "php" => "PHP",
    "rb" => "Ruby",
    "sql" => "SQL",
    "html" => "HTML",
    "css" => "CSS",
    "json" => "JSON",
    "yaml" => "YAML",
    "yml" => "YAML",
    "xml" => "XML",
    "sh" => "Shell",
    "bash" => "Shell",
    "md" => "Markdown",
    "ex" => "Elixir",
    "exs" => "Elixir",
    "heex" => "Elixir (HEEx)",
    "erl" => "Erlang",
    "swift" => "Swift",
    "kt" => "Kotlin",
    "toml" => "TOML",
    "dockerfile" => "Dockerfile",
};

#[derive(Default, Clone, Debug)]
pub struct LanguageStats {
    pub occurrences: u32,
    pub bytes: usize,
}

#[derive(Default, Clone, Debug)]
pub struct LanguageAnalyzer {
    pub stats: HashMap<String, LanguageStats>,
}

impl LanguageAnalyzer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_file_edit(&mut self, file_path: &str, byte_count: usize) {
        if let Some(ext) = Self::extract_extension(file_path) {
            if let Some(lang) = EXTENSION_MAP.get(ext) {
                let entry = self.stats.entry(lang.to_string()).or_default();
                entry.occurrences += 1;
                entry.bytes += byte_count;
            } else {
                // Ignore unknown extensions completely
            }
        } else if file_path.to_lowercase().ends_with("dockerfile") {
            let entry = self.stats.entry("Dockerfile".to_string()).or_default();
            entry.occurrences += 1;
            entry.bytes += byte_count;
        }
    }

    fn extract_extension(path: &str) -> Option<&str> {
        let path = path.trim();
        if path.is_empty() {
            return None;
        }

        let last_slash = path.rfind('/').unwrap_or(0);
        let file_name = &path[last_slash..];

        if let Some(dot_idx) = file_name.rfind('.') {
            if dot_idx > 0 && dot_idx < file_name.len() - 1 {
                return Some(&file_name[dot_idx + 1..]);
            }
        }
        None
    }

    pub fn merge(&mut self, other: &LanguageAnalyzer) {
        for (lang, stats) in &other.stats {
            let entry = self.stats.entry(lang.clone()).or_insert(LanguageStats {
                occurrences: 0,
                bytes: 0,
            });
            entry.occurrences += stats.occurrences;
            entry.bytes += stats.bytes;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_file_edit() {
        let mut analyzer = LanguageAnalyzer::new();

        analyzer.record_file_edit("src/main.rs", 100);
        analyzer.record_file_edit("scripts/build.sh", 50);
        analyzer.record_file_edit("Dockerfile", 200);
        analyzer.record_file_edit("Makefile", 500); // Should be ignored
        analyzer.record_file_edit("unknown.txt", 10); // Should be ignored

        assert_eq!(analyzer.stats.get("Rust").map(|s| s.occurrences), Some(1));
        assert_eq!(analyzer.stats.get("Rust").map(|s| s.bytes), Some(100));
        assert_eq!(analyzer.stats.get("Shell").map(|s| s.occurrences), Some(1));
        assert_eq!(
            analyzer.stats.get("Dockerfile").map(|s| s.occurrences),
            Some(1)
        );
        assert!(analyzer.stats.get("Unknown").is_none());
        assert!(analyzer.stats.get("Makefile").is_none());
    }
}
