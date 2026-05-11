use crossterm::event::{self, Event, KeyCode};
use notify::{RecursiveMode, Watcher};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Bar, BarChart, BarGroup, Block, Borders, Paragraph, Tabs},
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime};

use crate::claude::{ClaudeStats, get_claude_storage_path};
use crate::cline::{ClineStats, get_cline_storage_path};
use crate::format::{format_currency, format_int_with_commas, format_tokens};
use crate::gemini::{GeminiStats, get_gemini_storage_path};
use crate::unified::UnifiedStats;
use std::path::PathBuf;

pub fn run_tui() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let mut app = App::new();
    let _ = app.boot_with_splash(&mut terminal);
    let app_result = app.run(&mut terminal);
    ratatui::restore();
    app_result
}

#[derive(Default)]
struct ScanProgress {
    total: AtomicU32,
    done: AtomicU32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Summary,
    Providers,
    DailyCosts,
    DailyTokens,
    Settings,
}

impl Tab {
    fn next(self) -> Self {
        match self {
            Tab::Summary => Tab::Providers,
            Tab::Providers => Tab::DailyCosts,
            Tab::DailyCosts => Tab::DailyTokens,
            Tab::DailyTokens => Tab::Settings,
            Tab::Settings => Tab::Summary,
        }
    }

    fn prev(self) -> Self {
        match self {
            Tab::Summary => Tab::Settings,
            Tab::Providers => Tab::Summary,
            Tab::DailyCosts => Tab::Providers,
            Tab::DailyTokens => Tab::DailyCosts,
            Tab::Settings => Tab::DailyTokens,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

enum AppEvent {
    Terminal(Event),
    FileChanged,
    Tick,
}

struct AppSettings {
    heat_effects: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { heat_effects: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DailyFilter {
    All,
    Cline,
    Claude,
    Gemini,
}

impl DailyFilter {
    fn label(self) -> &'static str {
        match self {
            DailyFilter::All => "All",
            DailyFilter::Cline => "Cline",
            DailyFilter::Claude => "Claude",
            DailyFilter::Gemini => "Gemini",
        }
    }
}

#[derive(Default)]
struct ValueTracker {
    values: HashMap<String, (f64, Instant)>,
}

impl ValueTracker {
    fn has_active_animations(&self) -> bool {
        self.values
            .values()
            .any(|(_, ts)| ts.elapsed().as_millis() < 1201)
    }

    fn get_style(&mut self, id: &str, current: f64, base_color: Color, enabled: bool) -> Style {
        if !enabled {
            return Style::default().fg(base_color);
        }

        let entry = self
            .values
            .entry(id.to_string())
            .or_insert((current, Instant::now()));

        if (current - entry.0).abs() > f64::EPSILON {
            entry.0 = current;
            entry.1 = Instant::now();
        }

        let elapsed = entry.1.elapsed().as_millis();
        match elapsed {
            0..=200 => Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            201..=600 => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            601..=1200 => Style::default().fg(Color::Rgb(255, 140, 0)), // Orange
            _ => Style::default().fg(base_color),
        }
    }
}

#[derive(Clone)]
enum FileStats {
    Cline(ClineStats),
    Claude(ClaudeStats),
    Gemini(GeminiStats),
}

struct App {
    stats: UnifiedStats,
    tab: Tab,
    should_quit: bool,
    last_refresh: Instant,
    settings: AppSettings,
    daily_filter: DailyFilter,
    tracker: RefCell<ValueTracker>,
    // Per-file results cache
    file_cache: HashMap<PathBuf, (SystemTime, FileStats)>,
}

impl App {
    fn new() -> Self {
        Self {
            stats: UnifiedStats::default(),
            tab: Tab::Summary,
            should_quit: false,
            last_refresh: Instant::now(),
            settings: AppSettings::default(),
            daily_filter: DailyFilter::All,
            tracker: RefCell::new(ValueTracker::default()),
            file_cache: HashMap::new(),
        }
    }

    fn smart_scan(&mut self) {
        let cache = std::mem::take(&mut self.file_cache);
        let progress = Arc::new(ScanProgress::default());
        let (cache, stats) = run_scan_pass(cache, progress);
        self.file_cache = cache;
        self.stats = stats;
        self.last_refresh = Instant::now();
    }

    fn boot_with_splash(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let cache = std::mem::take(&mut self.file_cache);
        let progress = Arc::new(ScanProgress::default());
        let progress_worker = progress.clone();

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = run_scan_pass(cache, progress_worker);
            let _ = tx.send(result);
        });

        let start = Instant::now();
        loop {
            terminal.draw(|f| draw_splash(f, &progress, start.elapsed()))?;
            match rx.recv_timeout(Duration::from_millis(60)) {
                Ok((cache, stats)) => {
                    self.file_cache = cache;
                    self.stats = stats;
                    self.last_refresh = Instant::now();
                    // One last frame at 100% so the user sees the bar fill
                    terminal.draw(|f| draw_splash(f, &progress, start.elapsed()))?;
                    return Ok(());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Ok(()),
            }
        }
    }

    fn run(mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let (tx, rx) = mpsc::channel();

        // 1. Terminal Event Thread
        let tx_term = tx.clone();
        std::thread::spawn(move || {
            loop {
                if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                    if let Ok(ev) = event::read() {
                        if tx_term.send(AppEvent::Terminal(ev)).is_err() {
                            break;
                        }
                    }
                }
                if tx_term.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
        });

        // 2. File Watcher
        let tx_file = tx.clone();
        let mut watcher = notify::recommended_watcher(move |res| {
            if let Ok(_) = res {
                let _ = tx_file.send(AppEvent::FileChanged);
            }
        })
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        if let Some(path) = get_cline_storage_path() {
            if path.exists() {
                let _ = watcher.watch(&path, RecursiveMode::Recursive);
            }
        }
        if let Some(path) = get_claude_storage_path() {
            if path.exists() {
                let _ = watcher.watch(&path, RecursiveMode::Recursive);
            }
        }
        if let Some(path) = get_gemini_storage_path() {
            if path.exists() {
                let _ = watcher.watch(&path, RecursiveMode::Recursive);
            }
        }

        let mut needs_refresh = false;
        let mut last_tab = self.tab;
        let mut was_animating = false;
        let debounce_duration = Duration::from_millis(500);

        // Initial draw
        terminal.draw(|f| self.draw(f))?;

        while !self.should_quit {
            let is_animating = self.tracker.borrow().has_active_animations();

            if is_animating || was_animating || self.tab != last_tab {
                terminal.draw(|f| self.draw(f))?;
                last_tab = self.tab;
            }
            was_animating = is_animating;

            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(AppEvent::Terminal(ev)) => match ev {
                    Event::Key(key) if key.kind == event::KeyEventKind::Press => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                        KeyCode::Tab | KeyCode::Right => self.tab = self.tab.next(),
                        KeyCode::BackTab | KeyCode::Left => self.tab = self.tab.prev(),
                        KeyCode::Char(' ') if self.tab == Tab::Settings => {
                            self.settings.heat_effects = !self.settings.heat_effects;
                            terminal.draw(|f| self.draw(f))?;
                        }
                        KeyCode::Char(c @ ('1' | '2' | '3' | '4'))
                            if matches!(self.tab, Tab::DailyCosts | Tab::DailyTokens) =>
                        {
                            self.daily_filter = match c {
                                '1' => DailyFilter::All,
                                '2' => DailyFilter::Cline,
                                '3' => DailyFilter::Claude,
                                '4' => DailyFilter::Gemini,
                                _ => self.daily_filter,
                            };
                            terminal.draw(|f| self.draw(f))?;
                        }
                        KeyCode::Char('r') => {
                            self.smart_scan();
                            terminal.draw(|f| self.draw(f))?;
                        }
                        _ => {}
                    },
                    Event::Resize(_, _) => {
                        terminal.draw(|f| self.draw(f))?;
                    }
                    _ => {}
                },
                Ok(AppEvent::FileChanged) => {
                    needs_refresh = true;
                }
                Ok(AppEvent::Tick) | Err(mpsc::RecvTimeoutError::Timeout) => {
                    if needs_refresh && self.last_refresh.elapsed() > debounce_duration {
                        self.smart_scan();
                        needs_refresh = false;
                        terminal.draw(|f| self.draw(f))?;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(())
    }

    fn draw(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(f.area());

        // Header
        let titles = vec![
            " Summary ",
            " Providers ",
            " Daily Costs ",
            " Daily Tokens ",
            " Settings ",
        ];
        let tabs = Tabs::new(titles)
            .block(
                Block::default()
                    .title(" 🔥 INCINERATOR ")
                    .borders(Borders::ALL),
            )
            .select(self.tab.index())
            .highlight_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
        f.render_widget(tabs, chunks[0]);

        match self.tab {
            Tab::Summary => self.draw_summary(f, chunks[1]),
            Tab::Providers => self.draw_providers(f, chunks[1]),
            Tab::DailyCosts => self.draw_daily_costs(f, chunks[1]),
            Tab::DailyTokens => self.draw_daily_tokens(f, chunks[1]),
            Tab::Settings => self.draw_settings(f, chunks[1]),
        }

        let footer_spans = vec![
            Span::styled(" [q] Quit ", Style::default().dim()),
            Span::styled(" [Tab] Switch View ", Style::default().dim()),
            Span::styled(" [r] Refresh ", Style::default().dim()),
            Span::styled(
                format!(
                    " | Parsed: {} files ({:.2}s)",
                    self.stats.files_last_parsed, self.stats.parse_time
                ),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!(
                    " | Last update: {}s ago ",
                    self.last_refresh.elapsed().as_secs()
                ),
                Style::default().dim(),
            ),
        ];

        if self.tab == Tab::Settings {
            // Add space for toggle but keep it clean
            let mut parts = footer_spans;
            parts.insert(
                3,
                Span::styled(" [Space] Toggle ", Style::default().yellow()),
            );
            let footer = Paragraph::new(Line::from(parts));
            f.render_widget(footer, chunks[2]);
        } else {
            let footer = Paragraph::new(Line::from(footer_spans));
            f.render_widget(footer, chunks[2]);
        }
    }

    fn draw_summary(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Left: Totals
        let total_tokens = self.stats.total_tokens.total();
        let mut tracker = self.tracker.borrow_mut();

        let cost_style = tracker.get_style(
            "total_cost",
            self.stats.total_cost,
            Color::Red,
            self.settings.heat_effects,
        );
        let token_style = tracker.get_style(
            "total_tokens",
            total_tokens as f64,
            Color::White,
            self.settings.heat_effects,
        );
        let in_style = tracker.get_style(
            "in_tokens",
            self.stats.total_tokens.in_tokens as f64,
            Color::Blue,
            self.settings.heat_effects,
        );
        let out_style = tracker.get_style(
            "out_tokens",
            self.stats.total_tokens.out_tokens as f64,
            Color::Green,
            self.settings.heat_effects,
        );
        let cache_style = tracker.get_style(
            "cache_tokens",
            self.stats.total_tokens.cache_read_tokens as f64,
            Color::Yellow,
            self.settings.heat_effects,
        );

        let text = vec![
            Line::from(vec![
                Span::raw("Total Cost:   "),
                Span::styled(
                    format_currency(self.stats.total_cost),
                    cost_style.add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::raw("Total Tokens: "),
                Span::styled(
                    format_tokens(total_tokens),
                    token_style.add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" ({})", format_int_with_commas(total_tokens)),
                    Style::default().dim(),
                ),
            ]),
            Line::from(vec![
                Span::raw("  Input:      "),
                Span::styled(format_tokens(self.stats.total_tokens.in_tokens), in_style),
            ]),
            Line::from(vec![
                Span::raw("  Output:     "),
                Span::styled(format_tokens(self.stats.total_tokens.out_tokens), out_style),
            ]),
            Line::from(vec![
                Span::raw("  Cache Read: "),
                Span::styled(
                    format_tokens(self.stats.total_tokens.cache_read_tokens),
                    cache_style,
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::raw("Files Scanned: "),
                Span::styled(
                    format_int_with_commas(self.stats.files_parsed as i64),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
        ];

        let totals = Paragraph::new(text).block(
            Block::default()
                .title(" Grand Totals ")
                .borders(Borders::ALL),
        );
        f.render_widget(totals, chunks[0]);

        // Right: Model Stats (Top 10)
        let mut sorted_models: Vec<_> = self.stats.model_stats.iter().collect();
        sorted_models.sort_by(|a, b| b.1.total().cmp(&a.1.total()));

        let model_bars: Vec<Bar> = sorted_models
            .iter()
            .take(10)
            .map(|(name, stats)| {
                Bar::default()
                    .value(stats.total() as u64)
                    .label(Line::from((*name).clone()))
                    .text_value(format_tokens(stats.total()))
                    .style(Style::default().fg(Color::Cyan))
            })
            .collect();

        let barchart = BarChart::default()
            .block(
                Block::default()
                    .title(" Top Models (Tokens) ")
                    .borders(Borders::ALL),
            )
            .data(BarGroup::default().bars(&model_bars))
            .bar_width(12);
        f.render_widget(barchart, chunks[1]);
    }

    fn draw_providers(&self, f: &mut Frame, area: Rect) {
        let mut sorted_providers: Vec<_> = self.stats.provider_costs.iter().collect();
        sorted_providers.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());

        let mut tracker = self.tracker.borrow_mut();
        let provider_bars: Vec<Bar> = sorted_providers
            .iter()
            .map(|(provider, cost)| {
                let id = format!("provider_{:?}", provider);
                let style =
                    tracker.get_style(&id, **cost, Color::Yellow, self.settings.heat_effects);

                Bar::default()
                    .value((**cost * 100.0) as u64)
                    .label(Line::from(provider.to_string()))
                    .text_value(format_currency(**cost))
                    .style(style)
            })
            .collect();

        let barchart = BarChart::default()
            .block(
                Block::default()
                    .title(" Cost by Provider ")
                    .borders(Borders::ALL),
            )
            .data(BarGroup::default().bars(&provider_bars))
            .bar_width(15)
            .bar_gap(3);
        f.render_widget(barchart, area);
    }

    fn render_filter_chips(&self, f: &mut Frame, area: Rect) {
        let chips = [
            (DailyFilter::All, "1"),
            (DailyFilter::Cline, "2"),
            (DailyFilter::Claude, "3"),
            (DailyFilter::Gemini, "4"),
        ];
        let mut chip_spans: Vec<Span> = vec![Span::styled(" Filter: ", Style::default().dim())];
        for (filter, key) in chips {
            let active = filter == self.daily_filter;
            let label = if active {
                format!(" [{}] ", filter.label())
            } else {
                format!("  {}  ", filter.label())
            };
            let style = if active {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default().dim()
            };
            chip_spans.push(Span::styled(label, style));
            chip_spans.push(Span::styled(format!("({}) ", key), Style::default().dim()));
        }
        f.render_widget(Paragraph::new(Line::from(chip_spans)), area);
    }

    fn draw_daily_costs(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);

        self.render_filter_chips(f, chunks[0]);

        // Pick filtered source map
        let source = match self.daily_filter {
            DailyFilter::All => &self.stats.daily_costs,
            DailyFilter::Cline => &self.stats.daily_costs_cline,
            DailyFilter::Claude => &self.stats.daily_costs_claude,
            DailyFilter::Gemini => &self.stats.daily_costs_gemini,
        };

        let mut sorted_days: Vec<_> = source.iter().collect();
        sorted_days.sort_by(|a, b| a.0.cmp(b.0));

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut tracker = self.tracker.borrow_mut();

        let title = format!(" Daily Burn (USD) — {} ", self.daily_filter.label());
        let chart_area = chunks[1];

        let daily_bars: Vec<Bar> = sorted_days
            .iter()
            .rev()
            .take(chart_area.width as usize / 10)
            .rev()
            .map(|(day, cost)| {
                let id = format!("daily_cost_{:?}_{}", self.daily_filter, day);
                let is_today = **day == today;
                let style = tracker.get_style(
                    &id,
                    **cost,
                    Color::Red,
                    self.settings.heat_effects && is_today,
                );

                Bar::default()
                    .value((**cost * 100.0) as u64)
                    .label(Line::from(day.split('-').last().unwrap_or("").to_string()))
                    .text_value(format_currency(**cost))
                    .style(style)
            })
            .collect();

        let barchart = BarChart::default()
            .block(Block::default().title(title).borders(Borders::ALL))
            .data(BarGroup::default().bars(&daily_bars))
            .bar_width(8)
            .bar_gap(1);
        f.render_widget(barchart, chart_area);
    }

    fn draw_daily_tokens(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);

        self.render_filter_chips(f, chunks[0]);

        let area = chunks[1];

        let source = match self.daily_filter {
            DailyFilter::All => &self.stats.daily_tokens,
            DailyFilter::Cline => &self.stats.daily_tokens_cline,
            DailyFilter::Claude => &self.stats.daily_tokens_claude,
            DailyFilter::Gemini => &self.stats.daily_tokens_gemini,
        };

        let mut sorted_days: Vec<_> = source.iter().collect();
        sorted_days.sort_by(|a, b| a.0.cmp(b.0));

        // How many days fit? One line per day + block padding
        let num_days = (area.height as usize).saturating_sub(4).max(1);
        let last_days: Vec<_> = sorted_days.iter().rev().take(num_days).collect();

        if last_days.is_empty() {
            f.render_widget(Paragraph::new("No daily token data available."), area);
            return;
        }

        let max_total = last_days.iter().map(|(_, s)| s.total()).max().unwrap_or(1) as f64;
        let chart_width = (area.width as usize).saturating_sub(45).max(10); // Leave space for labels and borders

        let mut lines = Vec::new();

        // Header/Legend
        lines.push(Line::from(vec![
            Span::styled("Legend: ", Style::default().dim()),
            Span::styled("█ Input ", Style::default().fg(Color::Blue)),
            Span::styled("█ Output ", Style::default().fg(Color::Green)),
            Span::styled("▒ Cache Rd ", Style::default().fg(Color::Yellow)),
            Span::styled("░ Cache Cr ", Style::default().fg(Color::Magenta)),
        ]));
        lines.push(Line::from(""));

        let mut tracker = self.tracker.borrow_mut();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        for (day, stats) in last_days {
            let label = format!("{:<12} ", day);
            let total = stats.total();

            let id = format!("daily_total_tokens_{:?}_{}", self.daily_filter, day);
            let is_today = **day == today;
            let total_style = tracker.get_style(
                &id,
                total as f64,
                Color::White,
                self.settings.heat_effects && is_today,
            );

            let scale = |v: i64| -> usize {
                ((v as f64 / max_total) * chart_width as f64).round() as usize
            };

            let w_in = scale(stats.in_tokens);
            let w_out = scale(stats.out_tokens);
            let w_c_rd = scale(stats.cache_read_tokens);
            let w_c_cr = scale(stats.cache_create_tokens);

            let row = vec![
                Span::raw(label),
                Span::styled("█".repeat(w_in), Style::default().fg(Color::Blue)),
                Span::styled("█".repeat(w_out), Style::default().fg(Color::Green)),
                Span::styled("▒".repeat(w_c_rd), Style::default().fg(Color::Yellow)),
                Span::styled("░".repeat(w_c_cr), Style::default().fg(Color::Magenta)),
                Span::raw(" "),
                Span::styled(
                    format_tokens(total),
                    total_style.add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("({})", format_int_with_commas(total)),
                    Style::default().dim(),
                ),
            ];

            lines.push(Line::from(row));
        }

        let title = format!(
            " Daily Token Breakdown (Stacked) — {} ",
            self.daily_filter.label()
        );
        let paragraph =
            Paragraph::new(lines).block(Block::default().title(title).borders(Borders::ALL));

        f.render_widget(paragraph, area);
    }

    fn draw_settings(&self, f: &mut Frame, area: Rect) {
        let check = if self.settings.heat_effects {
            "[X]"
        } else {
            "[ ]"
        };
        let text = vec![
            Line::from(""),
            Line::from(vec![
                Span::raw(format!("  {} ", check)),
                Span::styled(
                    "Enable Heat Decay Effects",
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Description: ", Style::default().dim()),
                Span::raw(
                    "When values change, they flash 'white hot' and cool down to their base color.",
                ),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Controls: ", Style::default().dim()),
                Span::raw("Press [Space] to toggle."),
            ]),
        ];

        let p = Paragraph::new(text).block(
            Block::default()
                .title(" TUI Settings ")
                .borders(Borders::ALL),
        );
        f.render_widget(p, area);
    }
}

fn run_scan_pass(
    mut cache: HashMap<PathBuf, (SystemTime, FileStats)>,
    progress: Arc<ScanProgress>,
) -> (HashMap<PathBuf, (SystemTime, FileStats)>, UnifiedStats) {
    use crate::claude::{get_claude_files, parse_claude_file};
    use crate::cline::{get_cline_files, parse_cline_file};
    use crate::gemini::{get_gemini_files, parse_gemini_file};
    use rayon::prelude::*;
    use std::collections::HashSet;
    use std::fs;

    let start = Instant::now();

    let t = Instant::now();
    let cline_files = get_cline_files();
    let t_walk_cline = t.elapsed();
    let n_cline = cline_files.len();

    let t = Instant::now();
    let claude_files = get_claude_files();
    let t_walk_claude = t.elapsed();
    let n_claude = claude_files.len();

    let t = Instant::now();
    let gemini_files = get_gemini_files();
    let t_walk_gemini = t.elapsed();
    let n_gemini = gemini_files.len();

    progress
        .total
        .store((n_cline + n_claude + n_gemini) as u32, Ordering::Relaxed);

    let all_paths: HashSet<PathBuf> = cline_files
        .iter()
        .chain(claude_files.iter())
        .chain(gemini_files.iter())
        .cloned()
        .collect();
    cache.retain(|p, _| all_paths.contains(p));

    enum PType {
        Cline,
        Claude,
        Gemini,
    }
    let mut tasks = Vec::new();
    for p in cline_files {
        tasks.push((p, PType::Cline));
    }
    for p in claude_files {
        tasks.push((p, PType::Claude));
    }
    for p in gemini_files {
        tasks.push((p, PType::Gemini));
    }

    let t_parse_start = Instant::now();
    let cache_ref = &cache;
    let progress_ref = &progress;
    let results: Vec<(PathBuf, SystemTime, FileStats, bool, u64)> = tasks
        .into_par_iter()
        .map(|(path, p_type)| {
            let mtime = fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::now());
            if let Some((cached_time, cached_stats)) = cache_ref.get(&path) {
                if *cached_time == mtime {
                    progress_ref.done.fetch_add(1, Ordering::Relaxed);
                    return (path, mtime, cached_stats.clone(), false, 0);
                }
            }
            let parse_start = Instant::now();
            let stats = match p_type {
                PType::Cline => FileStats::Cline(parse_cline_file(&path, false, false)),
                PType::Claude => FileStats::Claude(parse_claude_file(&path)),
                PType::Gemini => FileStats::Gemini(parse_gemini_file(&path)),
            };
            let parse_us = parse_start.elapsed().as_micros() as u64;
            progress_ref.done.fetch_add(1, Ordering::Relaxed);
            (path, mtime, stats, true, parse_us)
        })
        .collect();

    let mut files_parsed = 0u32;
    let mut cline_n = 0u32;
    let mut cline_us_sum: u64 = 0;
    let mut cline_us_max: u64 = 0;
    let mut claude_n = 0u32;
    let mut claude_us_sum: u64 = 0;
    let mut claude_us_max: u64 = 0;
    let mut gemini_n = 0u32;
    let mut gemini_us_sum: u64 = 0;
    let mut gemini_us_max: u64 = 0;
    for (path, mtime, stats, was_parsed, parse_us) in results {
        if was_parsed {
            files_parsed += 1;
            match &stats {
                FileStats::Cline(_) => {
                    cline_n += 1;
                    cline_us_sum += parse_us;
                    if parse_us > cline_us_max {
                        cline_us_max = parse_us;
                    }
                }
                FileStats::Claude(_) => {
                    claude_n += 1;
                    claude_us_sum += parse_us;
                    if parse_us > claude_us_max {
                        claude_us_max = parse_us;
                    }
                }
                FileStats::Gemini(_) => {
                    gemini_n += 1;
                    gemini_us_sum += parse_us;
                    if parse_us > gemini_us_max {
                        gemini_us_max = parse_us;
                    }
                }
            }
        }
        cache.insert(path, (mtime, stats));
    }
    let t_parse = t_parse_start.elapsed();

    let t_agg_start = Instant::now();
    let mut new_stats = UnifiedStats::default();
    let mut paths: Vec<_> = cache.keys().collect();
    paths.sort();
    for path in paths {
        if let Some((_, stats)) = cache.get(path) {
            match stats {
                FileStats::Cline(s) => new_stats.add_cline(s.clone(), 0.0),
                FileStats::Claude(s) => new_stats.add_claude(s.clone(), 0.0),
                FileStats::Gemini(s) => new_stats.add_gemini(s.clone(), 0.0),
            }
        }
    }
    let t_agg = t_agg_start.elapsed();

    new_stats.parse_time = start.elapsed().as_secs_f64();
    new_stats.files_last_parsed = files_parsed;

    if std::env::var_os("INCINERATOR_DEBUG").is_some() {
        let log_path = std::env::temp_dir().join("incinerator-timings.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
            let cache_size = cache.len();
            let cache_hits = cache_size.saturating_sub(files_parsed as usize);
            let _ = writeln!(
                f,
                "[{}] mode={} walk_cline={}ms (n={}) walk_claude={}ms (n={}) walk_gemini={}ms (n={}) parse={}ms [cline n={} sum={}ms max={}ms | claude n={} sum={}ms max={}ms | gemini n={} sum={}ms max={}ms] agg={}ms total={}ms parsed={} cache_hits={} cache_size={}",
                now,
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                },
                t_walk_cline.as_millis(),
                n_cline,
                t_walk_claude.as_millis(),
                n_claude,
                t_walk_gemini.as_millis(),
                n_gemini,
                t_parse.as_millis(),
                cline_n,
                cline_us_sum / 1000,
                cline_us_max / 1000,
                claude_n,
                claude_us_sum / 1000,
                claude_us_max / 1000,
                gemini_n,
                gemini_us_sum / 1000,
                gemini_us_max / 1000,
                t_agg.as_millis(),
                start.elapsed().as_millis(),
                files_parsed,
                cache_hits,
                cache_size,
            );
        }
    }

    (cache, new_stats)
}

fn draw_splash(f: &mut Frame, progress: &ScanProgress, elapsed: Duration) {
    let area = f.area();
    let total = progress.total.load(Ordering::Relaxed);
    let done = progress.done.load(Ordering::Relaxed);

    // Two-frame flame flicker (~150ms each)
    let frame = (elapsed.as_millis() / 150) % 2;
    let (c_top, c_mid, c_low, c_base, c_char) = if frame == 0 {
        (
            Color::Yellow,
            Color::Rgb(255, 165, 0),
            Color::Red,
            Color::Rgb(180, 30, 0),
            Color::DarkGray,
        )
    } else {
        (
            Color::Rgb(255, 230, 80),
            Color::Yellow,
            Color::Rgb(255, 100, 0),
            Color::Red,
            Color::DarkGray,
        )
    };

    let flame: Vec<Line> = vec![
        Line::from(Span::styled(
            "       )         (       ",
            Style::default().fg(c_top),
        )),
        Line::from(Span::styled(
            "      ( )       ( )      ",
            Style::default().fg(c_top),
        )),
        Line::from(Span::styled(
            "       )(  /\\  )(        ",
            Style::default().fg(c_mid),
        )),
        Line::from(Span::styled(
            "      ( )(_||_)( )       ",
            Style::default().fg(c_low),
        )),
        Line::from(Span::styled(
            "       )/______\\(        ",
            Style::default().fg(c_base),
        )),
        Line::from(Span::styled(
            "        '------'         ",
            Style::default().fg(c_char),
        )),
    ];

    // Animated dots: . / .. / ... / ....
    let dot_count = ((elapsed.as_millis() / 350) % 4) as usize;
    let dots: String = ".".repeat(dot_count);
    let tabulating = format!("Tabulating financial damage{}", dots);

    // Progress bar
    let bar_width = 22usize;
    let pct = if total > 0 {
        done as f32 / total as f32
    } else {
        0.0
    };
    let pct = pct.clamp(0.0, 1.0);
    let filled = (pct * bar_width as f32).round() as usize;
    let filled = filled.min(bar_width);
    let bar_fill: String = "█".repeat(filled);
    let bar_empty: String = "░".repeat(bar_width - filled);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for l in flame {
        lines.push(l);
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "🔥 INCINERATOR 🔥",
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "═══════════════════",
        Style::default().fg(Color::Rgb(180, 30, 0)),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        tabulating,
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(bar_fill, Style::default().fg(Color::Red)),
        Span::styled(bar_empty, Style::default().fg(Color::DarkGray)),
        Span::raw(format!("  {} / {} files", done, total)),
    ]));

    let height = lines.len() as u16;
    let vpad = area.height.saturating_sub(height) / 2;
    let inner = Rect {
        x: area.x,
        y: area.y + vpad,
        width: area.width,
        height: height.min(area.height.saturating_sub(vpad)),
    };

    let p = Paragraph::new(lines).alignment(Alignment::Center);
    f.render_widget(p, inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_next_cycles_through_all_variants() {
        let order = [
            Tab::Summary,
            Tab::Providers,
            Tab::DailyCosts,
            Tab::DailyTokens,
            Tab::Settings,
        ];
        for i in 0..order.len() {
            let expected = order[(i + 1) % order.len()];
            assert_eq!(order[i].next(), expected);
        }
    }

    #[test]
    fn tab_prev_cycles_through_all_variants() {
        let order = [
            Tab::Summary,
            Tab::Providers,
            Tab::DailyCosts,
            Tab::DailyTokens,
            Tab::Settings,
        ];
        for i in 0..order.len() {
            let expected = order[(i + order.len() - 1) % order.len()];
            assert_eq!(order[i].prev(), expected);
        }
    }

    #[test]
    fn tab_next_and_prev_are_inverses() {
        for t in [
            Tab::Summary,
            Tab::Providers,
            Tab::DailyCosts,
            Tab::DailyTokens,
            Tab::Settings,
        ] {
            assert_eq!(t.next().prev(), t);
            assert_eq!(t.prev().next(), t);
        }
    }

    #[test]
    fn tab_index_is_stable_and_distinct() {
        assert_eq!(Tab::Summary.index(), 0);
        assert_eq!(Tab::Providers.index(), 1);
        assert_eq!(Tab::DailyCosts.index(), 2);
        assert_eq!(Tab::DailyTokens.index(), 3);
        assert_eq!(Tab::Settings.index(), 4);
    }

    #[test]
    fn daily_filter_labels() {
        assert_eq!(DailyFilter::All.label(), "All");
        assert_eq!(DailyFilter::Cline.label(), "Cline");
        assert_eq!(DailyFilter::Claude.label(), "Claude");
        assert_eq!(DailyFilter::Gemini.label(), "Gemini");
    }
}
