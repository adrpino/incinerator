use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};
use std::fs;
use std::cell::RefCell;
use std::collections::HashMap;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs, BarChart, Bar, BarGroup},
    DefaultTerminal, Frame,
};
use notify::{Watcher, RecursiveMode};

use crate::unified::UnifiedStats;
use crate::format::{format_currency, format_tokens, format_int_with_commas};
use crate::cline::get_cline_storage_path;
use crate::claude::get_claude_storage_path;
use crate::gemini::get_gemini_storage_path;

pub fn run_tui(stats: UnifiedStats) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let app_result = App::new(stats).run(&mut terminal);
    ratatui::restore();
    app_result
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

#[derive(Default)]
struct ValueTracker {
    values: HashMap<String, (f64, Instant)>,
}

impl ValueTracker {
    fn has_active_animations(&self) -> bool {
        self.values.values().any(|(_, ts)| ts.elapsed().as_millis() < 1201)
    }

    fn get_style(&mut self, id: &str, current: f64, base_color: Color, enabled: bool) -> Style {
        if !enabled {
            return Style::default().fg(base_color);
        }

        let entry = self.values.entry(id.to_string()).or_insert((current, Instant::now()));
        
        if (current - entry.0).abs() > f64::EPSILON {
            entry.0 = current;
            entry.1 = Instant::now();
        }

        let elapsed = entry.1.elapsed().as_millis();
        match elapsed {
            0..=200 => Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            201..=600 => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            601..=1200 => Style::default().fg(Color::Rgb(255, 140, 0)), // Orange
            _ => Style::default().fg(base_color),
        }
    }
}

struct App {
    stats: UnifiedStats,
    tab: Tab,
    should_quit: bool,
    last_refresh: Instant,
    settings: AppSettings,
    tracker: RefCell<ValueTracker>,
}

impl App {
    fn new(stats: UnifiedStats) -> Self {
        Self {
            stats,
            tab: Tab::Summary,
            should_quit: false,
            last_refresh: Instant::now(),
            settings: AppSettings::default(),
            tracker: RefCell::new(ValueTracker::default()),
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
        }).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

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
            
            // Draw if:
            // 1. We are currently animating
            // 2. We JUST finished animating (cleanup draw)
            // 3. Tab changed
            // 4. File watcher flagged a refresh
            if is_animating || was_animating || self.tab != last_tab {
                terminal.draw(|f| self.draw(f))?;
                last_tab = self.tab;
            }
            was_animating = is_animating;

            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(AppEvent::Terminal(ev)) => {
                    match ev {
                        Event::Key(key) if key.kind == event::KeyEventKind::Press => {
                            match key.code {
                                KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                                KeyCode::Tab | KeyCode::Right => self.tab = self.tab.next(),
                                KeyCode::Left => self.tab = self.tab.prev(),
                                KeyCode::Char(' ') if self.tab == Tab::Settings => {
                                    self.settings.heat_effects = !self.settings.heat_effects;
                                }
                                KeyCode::Char('r') => {
                                    if let Some(new_stats) = UnifiedStats::collect() {
                                        self.stats = new_stats;
                                        self.last_refresh = Instant::now();
                                        terminal.draw(|f| self.draw(f))?;
                                    }
                                }
                                _ => {}
                            }
                        }
                        Event::Resize(_, _) => {
                            terminal.draw(|f| self.draw(f))?;
                        }
                        _ => {}
                    }
                }
                Ok(AppEvent::FileChanged) => {
                    needs_refresh = true;
                }
                Ok(AppEvent::Tick) | Err(mpsc::RecvTimeoutError::Timeout) => {
                    if needs_refresh && self.last_refresh.elapsed() > debounce_duration {
                        if let Some(new_stats) = UnifiedStats::collect() {
                            self.stats = new_stats;
                            self.last_refresh = Instant::now();
                            needs_refresh = false;
                            terminal.draw(|f| self.draw(f))?;
                        }
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
        let titles = vec![" Summary ", " Providers ", " Daily Costs ", " Daily Tokens ", " Settings "];
        let tabs = Tabs::new(titles)
            .block(Block::default().title(" 🔥 INCINERATOR ").borders(Borders::ALL))
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

        // Footer
        let mut footer_spans = vec![
            Span::styled(" [q] Quit ", Style::default().dim()),
            Span::styled(" [Tab] Switch View ", Style::default().dim()),
            Span::styled(" [r] Refresh ", Style::default().dim()),
        ];
        
        if self.tab == Tab::Settings {
            footer_spans.push(Span::styled(" [Space] Toggle ", Style::default().yellow()));
        }

        footer_spans.push(Span::styled(format!(" | Last update: {}s ago ", self.last_refresh.elapsed().as_secs()), Style::default().dim()));
        
        let footer = Paragraph::new(Line::from(footer_spans));
        f.render_widget(footer, chunks[2]);
    }

    fn draw_summary(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Left: Totals
        let total_tokens = self.stats.total_tokens.total();
        let mut tracker = self.tracker.borrow_mut();
        
        let cost_style = tracker.get_style("total_cost", self.stats.total_cost, Color::Red, self.settings.heat_effects);
        let token_style = tracker.get_style("total_tokens", total_tokens as f64, Color::White, self.settings.heat_effects);
        let in_style = tracker.get_style("in_tokens", self.stats.total_tokens.in_tokens as f64, Color::Blue, self.settings.heat_effects);
        let out_style = tracker.get_style("out_tokens", self.stats.total_tokens.out_tokens as f64, Color::Green, self.settings.heat_effects);
        let cache_style = tracker.get_style("cache_tokens", self.stats.total_tokens.cache_read_tokens as f64, Color::Yellow, self.settings.heat_effects);

        let text = vec![
            Line::from(vec![
                Span::raw("Total Cost:   "),
                Span::styled(format_currency(self.stats.total_cost), cost_style.add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::raw("Total Tokens: "),
                Span::styled(format_tokens(total_tokens), token_style.add_modifier(Modifier::BOLD)),
                Span::styled(format!(" ({})", format_int_with_commas(total_tokens)), Style::default().dim()),
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
                Span::styled(format_tokens(self.stats.total_tokens.cache_read_tokens), cache_style),
            ]),
        ];
        
        let totals = Paragraph::new(text)
            .block(Block::default().title(" Grand Totals ").borders(Borders::ALL));
        f.render_widget(totals, chunks[0]);

        // Right: Model Stats (Top 10)
        let mut sorted_models: Vec<_> = self.stats.model_stats.iter().collect();
        sorted_models.sort_by(|a, b| b.1.total().cmp(&a.1.total()));
        
        let model_bars: Vec<Bar> = sorted_models.iter().take(10).map(|(name, stats)| {
            Bar::default()
                .value(stats.total() as u64)
                .label(Line::from((*name).clone()))
                .text_value(format_tokens(stats.total()))
                .style(Style::default().fg(Color::Cyan))
        }).collect();

        let barchart = BarChart::default()
            .block(Block::default().title(" Top Models (Tokens) ").borders(Borders::ALL))
            .data(BarGroup::default().bars(&model_bars))
            .bar_width(12);
        f.render_widget(barchart, chunks[1]);
    }

    fn draw_providers(&self, f: &mut Frame, area: Rect) {
        let mut sorted_providers: Vec<_> = self.stats.provider_costs.iter().collect();
        sorted_providers.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());

        let mut tracker = self.tracker.borrow_mut();
        let provider_bars: Vec<Bar> = sorted_providers.iter().map(|(provider, cost)| {
            let id = format!("provider_{:?}", provider);
            let style = tracker.get_style(&id, **cost, Color::Yellow, self.settings.heat_effects);
            
            Bar::default()
                .value((**cost * 100.0) as u64)
                .label(Line::from(provider.to_string()))
                .text_value(format_currency(**cost))
                .style(style)
        }).collect();

        let barchart = BarChart::default()
            .block(Block::default().title(" Cost by Provider ").borders(Borders::ALL))
            .data(BarGroup::default().bars(&provider_bars))
            .bar_width(15)
            .bar_gap(3);
        f.render_widget(barchart, area);
    }

    fn draw_daily_costs(&self, f: &mut Frame, area: Rect) {
        let mut sorted_days: Vec<_> = self.stats.daily_costs.iter().collect();
        sorted_days.sort_by(|a, b| a.0.cmp(b.0));
        
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut tracker = self.tracker.borrow_mut();
        
        let daily_bars: Vec<Bar> = sorted_days.iter().rev().take(area.width as usize / 10).rev().map(|(day, cost)| {
            let id = format!("daily_cost_{}", day);
            let is_today = **day == today;
            let style = tracker.get_style(&id, **cost, Color::Red, self.settings.heat_effects && is_today);
            
            Bar::default()
                .value((**cost * 100.0) as u64)
                .label(Line::from(day.split('-').last().unwrap_or("").to_string()))
                .text_value(format_currency(**cost))
                .style(style)
        }).collect();

        let barchart = BarChart::default()
            .block(Block::default().title(" Daily Burn (USD) ").borders(Borders::ALL))
            .data(BarGroup::default().bars(&daily_bars))
            .bar_width(8)
            .bar_gap(1);
        f.render_widget(barchart, area);
    }

    fn draw_daily_tokens(&self, f: &mut Frame, area: Rect) {
        let mut sorted_days: Vec<_> = self.stats.daily_tokens.iter().collect();
        sorted_days.sort_by(|a, b| a.0.cmp(b.0));

        // How many days fit? One line per day + block padding
        let num_days = (area.height as usize - 4).max(1);
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
            
            let id = format!("daily_total_tokens_{}", day);
            let is_today = **day == today;
            let total_style = tracker.get_style(&id, total as f64, Color::White, self.settings.heat_effects && is_today);
            
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
                Span::styled(format_tokens(total), total_style.add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled(format!("({})", format_int_with_commas(total)), Style::default().dim()),
            ];
            
            lines.push(Line::from(row));
        }

        let paragraph = Paragraph::new(lines)
            .block(Block::default().title(" Daily Token Breakdown (Stacked) ").borders(Borders::ALL));
        
        f.render_widget(paragraph, area);
    }

    fn draw_settings(&self, f: &mut Frame, area: Rect) {
        let check = if self.settings.heat_effects { "[X]" } else { "[ ]" };
        let text = vec![
            Line::from(""),
            Line::from(vec![
                Span::raw(format!("  {} ", check)),
                Span::styled("Enable Heat Decay Effects", Style::default().fg(Color::Yellow)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Description: ", Style::default().dim()),
                Span::raw("When values change, they flash 'white hot' and cool down to their base color."),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Controls: ", Style::default().dim()),
                Span::raw("Press [Space] to toggle."),
            ]),
        ];

        let p = Paragraph::new(text)
            .block(Block::default().title(" TUI Settings ").borders(Borders::ALL));
        f.render_widget(p, area);
    }
}
