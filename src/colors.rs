use ratatui::style::Color;

// --- Terminal ANSI Colors (for src/viz.rs and direct terminal output) ---
pub const TERM_HEADER: &str = "\x1b[95m";
pub const TERM_BLUE: &str = "\x1b[94m";
pub const TERM_GREEN: &str = "\x1b[92m";
pub const TERM_YELLOW: &str = "\x1b[93m";
pub const TERM_ORANGE: &str = "\x1b[38;5;208m";
pub const TERM_RED: &str = "\x1b[91m";
pub const TERM_CYAN: &str = "\x1b[96m";
pub const TERM_BOLD: &str = "\x1b[1m";
pub const TERM_RESET: &str = "\x1b[0m";

// --- TUI Colors (ratatui::style::Color constants for src/tui.rs) ---
pub const TUI_WHITE: Color = Color::White;
pub const TUI_YELLOW: Color = Color::Yellow;
pub const TUI_RED: Color = Color::Red;
pub const TUI_CYAN: Color = Color::Cyan;
pub const TUI_BLUE: Color = Color::Blue;
pub const TUI_GREEN: Color = Color::Green;
pub const TUI_MAGENTA: Color = Color::Magenta;
pub const TUI_DARK_GRAY: Color = Color::DarkGray;

// Custom RGB colors used in TUI
pub const TUI_ORANGE_601: Color = Color::Rgb(255, 140, 0);
pub const TUI_FLAME_ORANGE_1: Color = Color::Rgb(255, 165, 0);
pub const TUI_FLAME_RED_1: Color = Color::Rgb(180, 30, 0);
pub const TUI_FLAME_YELLOW_2: Color = Color::Rgb(255, 230, 80);
pub const TUI_FLAME_ORANGE_2: Color = Color::Rgb(255, 100, 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeType {
    #[default]
    Classic,
    Vivid,
}

impl ThemeType {
    pub fn name(&self) -> &'static str {
        match self {
            ThemeType::Classic => "Classic",
            ThemeType::Vivid => "Vivid",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ThemeType::Classic => "The standard incinerator look. Simple and familiar.",
            ThemeType::Vivid => {
                "High-contrast, modern palette optimized for clarity and visibility."
            }
        }
    }

    pub fn next(self) -> Self {
        match self {
            ThemeType::Classic => ThemeType::Vivid,
            ThemeType::Vivid => ThemeType::Classic,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub input: Color,
    pub output: Color,
    pub cache_read: Color,
    pub cache_create: Color,
    pub cost: Color,
    pub secondary: Color,
}

impl ThemeType {
    pub fn palette(&self) -> Palette {
        match self {
            ThemeType::Classic => Palette {
                input: TUI_BLUE,
                output: TUI_GREEN,
                cache_read: TUI_YELLOW,
                cache_create: TUI_MAGENTA,
                cost: TUI_RED,
                secondary: TUI_CYAN,
            },
            ThemeType::Vivid => Palette {
                input: Color::Rgb(0, 114, 178),          // Blue
                output: Color::Rgb(0, 158, 115),         // Bluish Green
                cache_read: Color::Rgb(240, 228, 66),    // Yellow
                cache_create: Color::Rgb(204, 121, 167), // Reddish Purple
                cost: Color::Rgb(213, 94, 0),            // Vermillion
                secondary: Color::Rgb(86, 180, 233),     // Sky Blue
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_cycle() {
        let t = ThemeType::Classic;
        assert_eq!(t.next(), ThemeType::Vivid);
        assert_eq!(t.next().next(), ThemeType::Classic);
    }

    #[test]
    fn test_palettes_are_distinct() {
        let classic = ThemeType::Classic.palette();
        let vivid = ThemeType::Vivid.palette();
        assert_ne!(classic.input, vivid.input);
        assert_ne!(classic.output, vivid.output);
    }
}
