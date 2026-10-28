use chrono::{DateTime, Local};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crate::colors::*;
use crate::format::{format_float_with_commas, format_int_with_commas};
use crate::languages::LanguageAnalyzer;
use crate::viz::{TokenStats, print_cost_bar, print_token_bar};

const MSG_TAIL_WINDOW: i64 = 64;
const PART_TAIL_WINDOW: i64 = 128;

#[derive(Default, Clone)]
pub struct OpencodeStats {
    pub daily_costs: BTreeMap<String, f64>,
    pub monthly_costs: BTreeMap<String, f64>,
    pub total_cost: f64,
    pub daily_stats: BTreeMap<String, TokenStats>,
    pub monthly_stats: BTreeMap<String, TokenStats>,
    pub model_stats: HashMap<String, TokenStats>,
    pub sessions_found: usize,
    pub languages: LanguageAnalyzer,
}

#[derive(Default, Clone)]
pub struct OpencodeCacheState {
    pub checkpoint_msg_rowid: i64,
    pub checkpoint_part_rowid: i64,
    pub checkpoint_sessions: HashSet<String>,
    pub checkpoint_stats: OpencodeStats,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct OpencodeModelJson {
    pub id: String,
    #[serde(rename = "providerID")]
    pub provider_id: Option<String>,
}

#[derive(Deserialize, Default)]
struct OpencodeCacheTokens {
    #[serde(default)]
    read: Option<i64>,
    #[serde(default)]
    write: Option<i64>,
}

#[derive(Deserialize, Default)]
struct OpencodeTokens {
    #[serde(default)]
    input: Option<i64>,
    #[serde(default)]
    output: Option<i64>,
    #[serde(default)]
    reasoning: Option<i64>,
    #[serde(default)]
    cache: Option<OpencodeCacheTokens>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OpencodeModelField {
    Str(String),
    Obj {
        #[serde(default)]
        id: Option<String>,
        #[serde(rename = "modelID", default)]
        model_id: Option<String>,
    },
    Other(serde_json::Value),
}

impl OpencodeModelField {
    fn into_model_name(self) -> String {
        match self {
            OpencodeModelField::Str(s) => parse_model_id(&s),
            OpencodeModelField::Obj { id, model_id } => {
                id.or(model_id).unwrap_or_else(|| "unknown".to_string())
            }
            OpencodeModelField::Other(val) => {
                if let Some(s) = val.as_str() {
                    parse_model_id(s)
                } else if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
                    id.to_string()
                } else if let Some(id) = val.get("modelID").and_then(|v| v.as_str()) {
                    id.to_string()
                } else {
                    "unknown".to_string()
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct OpencodeMessageData {
    #[serde(default)]
    role: Option<String>,
    #[serde(rename = "modelID", default)]
    model_id: Option<OpencodeModelField>,
    #[serde(default)]
    cost: Option<f64>,
    #[serde(default)]
    tokens: Option<OpencodeTokens>,
}

pub fn get_opencode_db_path() -> Option<PathBuf> {
    let base = dirs::home_dir()?;
    let path = base.join(".local/share/opencode/opencode.db");
    if path.exists() { Some(path) } else { None }
}

pub fn open_readonly_connection(db_path: &Path) -> Option<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(db_path, flags)
        .or_else(|_| Connection::open(db_path))
        .ok()?;
    let _ = conn.busy_timeout(Duration::from_millis(250));
    let _ = conn.execute_batch("PRAGMA temp_store = MEMORY; PRAGMA mmap_size = 2147483648;");
    Some(conn)
}

pub fn parse_model_id(raw: &str) -> String {
    if !raw.starts_with('{') {
        return raw.to_string();
    }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
        if let Some(id) = val.get("id").and_then(|v| v.as_str()) {
            return id.to_string();
        }
        if let Some(id) = val.get("modelID").and_then(|v| v.as_str()) {
            return id.to_string();
        }
    }
    "unknown".to_string()
}

pub fn parse_opencode_db() -> Option<OpencodeStats> {
    let db_path = get_opencode_db_path()?;
    parse_opencode_db_at(db_path)
}

pub fn parse_opencode_db_at(db_path: PathBuf) -> Option<OpencodeStats> {
    let conn = open_readonly_connection(&db_path)?;
    parse_opencode_connection(&conn)
}

#[allow(dead_code)]
pub fn parse_opencode_db_incremental(
    db_path: &Path,
    prev: Option<&OpencodeCacheState>,
) -> Option<(OpencodeStats, OpencodeCacheState)> {
    let conn = open_readonly_connection(db_path)?;
    parse_opencode_connection_incremental(&conn, prev)
}

pub fn parse_opencode_connection(conn: &Connection) -> Option<OpencodeStats> {
    let (stats, _) = parse_opencode_connection_incremental(conn, None)?;
    Some(stats)
}

pub fn parse_opencode_connection_incremental(
    conn: &Connection,
    prev: Option<&OpencodeCacheState>,
) -> Option<(OpencodeStats, OpencodeCacheState)> {
    parse_opencode_connection_incremental_with_progress(conn, prev, None)
}

pub fn count_opencode_sessions(db_path: &Path) -> usize {
    let Some(conn) = open_readonly_connection(db_path) else {
        return 1;
    };
    count_opencode_sessions_conn(&conn)
}

pub fn count_opencode_sessions_conn(conn: &Connection) -> usize {
    if let Ok(count) = conn.query_row("SELECT COUNT(*) FROM session", [], |r| r.get::<_, i64>(0)) {
        if count > 0 {
            return count as usize;
        }
    }
    if let Ok(count) = conn.query_row("SELECT COUNT(DISTINCT session_id) FROM message", [], |r| {
        r.get::<_, i64>(0)
    }) {
        if count > 0 {
            return count as usize;
        }
    }
    1
}

pub fn parse_opencode_db_incremental_with_progress(
    db_path: &Path,
    prev: Option<&OpencodeCacheState>,
    progress: Option<(&AtomicU32, u32)>,
) -> Option<(OpencodeStats, OpencodeCacheState)> {
    let conn = open_readonly_connection(db_path)?;
    parse_opencode_connection_incremental_with_progress(&conn, prev, progress)
}

pub fn parse_opencode_connection_incremental_with_progress(
    conn: &Connection,
    prev: Option<&OpencodeCacheState>,
    progress: Option<(&AtomicU32, u32)>,
) -> Option<(OpencodeStats, OpencodeCacheState)> {
    parse_opencode_connection_with_path(conn, prev, progress)
}

struct OpencodeProgressTracker<'a> {
    counter: Option<&'a AtomicU32>,
    total_units: u32,
    emitted_units: u32,
    msg_start: i64,
    msg_span: i64,
    part_start: i64,
    part_span: i64,
}

impl<'a> OpencodeProgressTracker<'a> {
    fn new(
        progress: Option<(&'a AtomicU32, u32)>,
        msg_start: i64,
        msg_end: i64,
        part_start: i64,
        part_end: i64,
    ) -> Self {
        let (counter, total_units) = match progress {
            Some((c, t)) => (Some(c), t),
            None => (None, 0),
        };
        Self {
            counter,
            total_units,
            emitted_units: 0,
            msg_start,
            msg_span: (msg_end - msg_start).max(0),
            part_start,
            part_span: (part_end - part_start).max(0),
        }
    }

    fn update_fraction(&mut self, frac: f64) {
        let Some(counter) = self.counter else {
            return;
        };
        if self.total_units == 0 {
            return;
        }
        let clamped = frac.clamp(0.0, 1.0);
        let max_in_flight = self.total_units.saturating_sub(1);
        let target = ((clamped * self.total_units as f64).floor() as u32).min(max_in_flight);
        if target > self.emitted_units {
            let delta = target - self.emitted_units;
            counter.fetch_add(delta, Ordering::Relaxed);
            self.emitted_units = target;
        }
    }

    fn on_message_rowid(&mut self, rowid: i64) {
        if self.msg_span <= 0 {
            return;
        }
        let msg_frac = ((rowid - self.msg_start) as f64 / self.msg_span as f64).clamp(0.0, 1.0);
        let overall = if self.part_span > 0 {
            msg_frac * 0.65
        } else {
            msg_frac
        };
        self.update_fraction(overall);
    }

    fn on_part_rowid(&mut self, rowid: i64) {
        if self.part_span <= 0 {
            return;
        }
        let part_frac = ((rowid - self.part_start) as f64 / self.part_span as f64).clamp(0.0, 1.0);
        let overall = if self.msg_span > 0 {
            0.65 + part_frac * 0.35
        } else {
            part_frac
        };
        self.update_fraction(overall);
    }

    fn on_fallback_session(&mut self) {
        let Some(counter) = self.counter else {
            return;
        };
        let max_in_flight = self.total_units.saturating_sub(1);
        if self.emitted_units < max_in_flight {
            counter.fetch_add(1, Ordering::Relaxed);
            self.emitted_units += 1;
        }
    }

    fn finish(&mut self) {
        if let Some(counter) = self.counter {
            if self.emitted_units < self.total_units {
                counter.fetch_add(self.total_units - self.emitted_units, Ordering::Relaxed);
                self.emitted_units = self.total_units;
            }
        }
    }
}

fn scan_message_range(
    conn: &Connection,
    start_rowid: i64,
    end_rowid: i64,
    stats: &mut OpencodeStats,
    unique_sessions: &mut HashSet<String>,
    tracker: &mut OpencodeProgressTracker<'_>,
) -> bool {
    if end_rowid <= start_rowid {
        return true;
    }

    let Ok(mut stmt) = conn.prepare(
        "SELECT rowid, session_id, data, time_created \
         FROM message \
         WHERE rowid > ?1 AND rowid <= ?2 \
           AND LENGTH(data) < 65536 \
           AND (data GLOB '{\"role\":\"assistant\"*' OR json_extract(data, '$.role') = 'assistant')",
    ) else {
        return false;
    };

    let Ok(rows) = stmt.query_map([start_rowid, end_rowid], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    }) else {
        return false;
    };

    for row in rows.flatten() {
        let (rowid, session_id, data_str, time_created) = row;
        tracker.on_message_rowid(rowid);
        let Ok(msg) = serde_json::from_str::<OpencodeMessageData>(&data_str) else {
            continue;
        };
        if msg.role.as_deref() != Some("assistant") {
            continue;
        }

        if let Some(sid) = session_id {
            unique_sessions.insert(sid);
        }

        let cost = msg.cost.unwrap_or(0.0);
        let tokens = msg.tokens.unwrap_or_default();
        let tokens_in = tokens.input.unwrap_or(0);
        let tokens_out = tokens.output.unwrap_or(0);
        let tokens_reasoning = tokens.reasoning.unwrap_or(0);
        let cache = tokens.cache.unwrap_or_default();
        let cache_read = cache.read.unwrap_or(0);
        let cache_write = cache.write.unwrap_or(0);

        let model_name = msg
            .model_id
            .map(OpencodeModelField::into_model_name)
            .unwrap_or_else(|| "unknown".to_string());

        let dt = DateTime::from_timestamp(
            time_created / 1000,
            ((time_created % 1000) * 1_000_000) as u32,
        )
        .map(|dt| dt.with_timezone(&Local))
        .unwrap_or_else(Local::now);

        let date_key = dt.format("%Y-%m-%d").to_string();
        let month_key = dt.format("%Y-%m").to_string();

        *stats.daily_costs.entry(date_key.clone()).or_insert(0.0) += cost;
        *stats.monthly_costs.entry(month_key.clone()).or_insert(0.0) += cost;
        stats.total_cost += cost;

        let entry = TokenStats {
            in_tokens: tokens_in,
            out_tokens: tokens_out + tokens_reasoning,
            cache_read_tokens: cache_read,
            cache_create_tokens: cache_write,
        };

        stats.daily_stats.entry(date_key).or_default().add(&entry);
        stats
            .monthly_stats
            .entry(month_key)
            .or_default()
            .add(&entry);
        stats.model_stats.entry(model_name).or_default().add(&entry);
    }

    true
}

fn scan_part_range(
    conn: &Connection,
    start_rowid: i64,
    end_rowid: i64,
    languages: &mut LanguageAnalyzer,
    tracker: &mut OpencodeProgressTracker<'_>,
) {
    if end_rowid <= start_rowid {
        return;
    }

    let Ok(mut part_stmt) = conn.prepare(
        "SELECT \
         rowid, \
         COALESCE( \
           json_extract(data, '$.path'), \
           json_extract(data, '$.file_path'), \
           json_extract(data, '$.filePath'), \
           json_extract(data, '$.state.input.filePath'), \
           json_extract(data, '$.state.input.file_path'), \
           json_extract(data, '$.state.input.path') \
         ), \
         COALESCE( \
           json_extract(data, '$.bytes'), \
           json_extract(data, '$.size'), \
           LENGTH(json_extract(data, '$.content')), \
           LENGTH(json_extract(data, '$.patch')), \
           LENGTH(json_extract(data, '$.state.input.content')), \
           LENGTH(json_extract(data, '$.state.input.newString')), \
           LENGTH(json_extract(data, '$.state.input.new_string')), \
           0 \
         ) \
         FROM part \
         WHERE rowid > ?1 AND rowid <= ?2 \
           AND ( \
             data GLOB '{\"type\":\"patch\"*' \
             OR data GLOB '{\"type\":\"file\"*' \
             OR data GLOB '{\"type\":\"tool\",\"tool\":\"edit\"*' \
             OR data GLOB '{\"type\":\"tool\",\"tool\":\"write\"*' \
             OR (LENGTH(data) < 4096 AND json_extract(data, '$.type') IN ('patch', 'file', 'tool')) \
           )",
    ) else {
        return;
    };

    if let Ok(part_rows) = part_stmt.query_map([start_rowid, end_rowid], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    }) {
        for row in part_rows.flatten() {
            let (rowid, path_opt, bytes) = row;
            tracker.on_part_rowid(rowid);
            if let Some(p) = path_opt {
                languages.record_file_edit(&p, bytes.max(0) as usize);
            }
        }
    }
}

fn parse_opencode_connection_with_path(
    conn: &Connection,
    prev: Option<&OpencodeCacheState>,
    progress: Option<(&AtomicU32, u32)>,
) -> Option<(OpencodeStats, OpencodeCacheState)> {
    let max_msg_rowid_res: Result<i64, _> =
        conn.query_row("SELECT COALESCE(MAX(rowid), 0) FROM message", [], |r| {
            r.get(0)
        });

    let max_part_rowid: i64 = conn
        .query_row("SELECT COALESCE(MAX(rowid), 0) FROM part", [], |r| r.get(0))
        .unwrap_or(0);

    if let Ok(max_msg_rowid) = max_msg_rowid_res {
        let mut cache_state = match prev {
            Some(p)
                if max_msg_rowid >= p.checkpoint_msg_rowid
                    && max_part_rowid >= p.checkpoint_part_rowid =>
            {
                p.clone()
            }
            _ => OpencodeCacheState::default(),
        };

        let mut tracker = OpencodeProgressTracker::new(
            progress,
            cache_state.checkpoint_msg_rowid,
            max_msg_rowid,
            cache_state.checkpoint_part_rowid,
            max_part_rowid,
        );

        let target_msg_cp = max_msg_rowid.saturating_sub(MSG_TAIL_WINDOW);
        if target_msg_cp > cache_state.checkpoint_msg_rowid {
            let prev_cp = cache_state.checkpoint_msg_rowid;
            scan_message_range(
                conn,
                prev_cp,
                target_msg_cp,
                &mut cache_state.checkpoint_stats,
                &mut cache_state.checkpoint_sessions,
                &mut tracker,
            );
            cache_state.checkpoint_msg_rowid = target_msg_cp;
        }

        let target_part_cp = max_part_rowid.saturating_sub(PART_TAIL_WINDOW);
        if target_part_cp > cache_state.checkpoint_part_rowid {
            let prev_cp = cache_state.checkpoint_part_rowid;
            scan_part_range(
                conn,
                prev_cp,
                target_part_cp,
                &mut cache_state.checkpoint_stats.languages,
                &mut tracker,
            );
            cache_state.checkpoint_part_rowid = target_part_cp;
        }

        let mut live_stats = cache_state.checkpoint_stats.clone();
        let mut live_sessions = cache_state.checkpoint_sessions.clone();

        if max_msg_rowid > cache_state.checkpoint_msg_rowid {
            scan_message_range(
                conn,
                cache_state.checkpoint_msg_rowid,
                max_msg_rowid,
                &mut live_stats,
                &mut live_sessions,
                &mut tracker,
            );
        }
        live_stats.sessions_found = live_sessions.len();

        if max_part_rowid > cache_state.checkpoint_part_rowid {
            scan_part_range(
                conn,
                cache_state.checkpoint_part_rowid,
                max_part_rowid,
                &mut live_stats.languages,
                &mut tracker,
            );
        }

        tracker.finish();
        return Some((live_stats, cache_state));
    }

    // Fallback to `session` table if `message` table is not present
    let mut stats = OpencodeStats::default();
    let mut tracker = OpencodeProgressTracker::new(progress, 0, 0, 0, max_part_rowid);
    if let Ok(mut stmt) = conn.prepare(
        "SELECT model, cost, tokens_input, tokens_output, tokens_reasoning, \
         tokens_cache_read, tokens_cache_write, time_created FROM session",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        }) {
            for row in rows.flatten() {
                tracker.on_fallback_session();
                let (
                    model_json,
                    cost,
                    tokens_in,
                    tokens_out,
                    tokens_reasoning,
                    cache_read,
                    cache_write,
                    time_created,
                ) = row;

                stats.sessions_found += 1;

                let model_name = parse_model_id(&model_json);

                let dt = DateTime::from_timestamp(
                    time_created / 1000,
                    ((time_created % 1000) * 1_000_000) as u32,
                )
                .map(|dt| dt.with_timezone(&Local))
                .unwrap_or_else(Local::now);

                let date_key = dt.format("%Y-%m-%d").to_string();
                let month_key = dt.format("%Y-%m").to_string();

                *stats.daily_costs.entry(date_key.clone()).or_insert(0.0) += cost;
                *stats.monthly_costs.entry(month_key.clone()).or_insert(0.0) += cost;
                stats.total_cost += cost;

                let entry = TokenStats {
                    in_tokens: tokens_in,
                    out_tokens: tokens_out + tokens_reasoning,
                    cache_read_tokens: cache_read,
                    cache_create_tokens: cache_write,
                };

                stats.daily_stats.entry(date_key).or_default().add(&entry);
                stats
                    .monthly_stats
                    .entry(month_key)
                    .or_default()
                    .add(&entry);
                stats.model_stats.entry(model_name).or_default().add(&entry);
            }
        }
    }

    if max_part_rowid > 0 {
        scan_part_range(conn, 0, max_part_rowid, &mut stats.languages, &mut tracker);
    }

    tracker.finish();
    Some((stats, OpencodeCacheState::default()))
}

pub fn run_opencode_report() -> Option<(OpencodeStats, f64)> {
    let start = Instant::now();
    let stats = parse_opencode_db()?;
    let elapsed = start.elapsed().as_secs_f64();
    Some((stats, elapsed))
}

pub fn print_opencode_report(stats: &OpencodeStats, elapsed: f64, daily_days: usize) {
    println!("\n{}", "=".repeat(95));
    println!(
        "{}📊 OPENCODE USAGE & COST REPORT{}",
        TERM_HEADER, TERM_RESET
    );
    println!("{}", "=".repeat(95));

    println!(
        "\n{}=== TOKEN USAGE (STACKED) ==={}",
        TERM_HEADER, TERM_RESET
    );
    println!(
        "Legend: {}█ Input{} | {}█ Output{} | {}▒ Cache Read{} | {}░ Cache Create{}",
        TERM_BLUE,
        TERM_RESET,
        TERM_GREEN,
        TERM_RESET,
        TERM_YELLOW,
        TERM_RESET,
        TERM_ORANGE,
        TERM_RESET
    );

    println!("\n{}--- Monthly Token Usage ---{}", TERM_BOLD, TERM_RESET);
    let max_monthly = stats
        .monthly_stats
        .values()
        .map(|s| s.total())
        .max()
        .unwrap_or(0);
    for (month, s) in &stats.monthly_stats {
        print_token_bar(&format!("{:^10}", month), s, max_monthly, 35, true);
    }

    println!("\n{}--- Usage by Model ---{}", TERM_BOLD, TERM_RESET);
    let max_model = stats
        .model_stats
        .values()
        .map(|s| s.total())
        .max()
        .unwrap_or(0);
    let mut sorted_models: Vec<_> = stats.model_stats.iter().collect();
    sorted_models.sort_by_key(|b| std::cmp::Reverse(b.1.total()));
    for (model, s) in sorted_models {
        print_token_bar(
            &format!("{:<30}", model.get(..30).unwrap_or(model)),
            s,
            max_model,
            35,
            true,
        );
    }

    println!("\n{}=== FINANCIAL COSTS ==={}", TERM_HEADER, TERM_RESET);

    println!(
        "\n{}--- Daily Costs (Last {} Days) ---{}",
        TERM_BOLD, daily_days, TERM_RESET
    );
    let max_daily_cost = stats.daily_costs.values().copied().fold(0.0, f64::max);
    for (day, cost) in stats.daily_costs.iter().rev().take(daily_days) {
        print_cost_bar(&format!("{:<12}", day), *cost, max_daily_cost, 35);
    }

    println!("\n{}", "=".repeat(50));
    println!("{}GRAND TOTALS (OPENCODE){}", TERM_HEADER, TERM_RESET);
    println!("{}", "-".repeat(50));
    println!("{}Tokens:{}", TERM_BOLD, TERM_RESET);
    let in_sum: i64 = stats.monthly_stats.values().map(|s| s.in_tokens).sum();
    let out_sum: i64 = stats.monthly_stats.values().map(|s| s.out_tokens).sum();
    let cache_sum: i64 = stats
        .monthly_stats
        .values()
        .map(|s| s.cache_read_tokens)
        .sum();
    let cache_create_sum: i64 = stats
        .monthly_stats
        .values()
        .map(|s| s.cache_create_tokens)
        .sum();
    println!("  Input:        {:>12}", format_int_with_commas(in_sum));
    println!("  Output:       {:>12}", format_int_with_commas(out_sum));
    println!("  Cache Read:   {:>12}", format_int_with_commas(cache_sum));
    if cache_create_sum > 0 {
        println!(
            "  Cache Create: {:>12}",
            format_int_with_commas(cache_create_sum)
        );
    }
    println!("{}Cost:{}", TERM_BOLD, TERM_RESET);
    println!(
        "  {} ${}{}",
        TERM_GREEN,
        format_float_with_commas(stats.total_cost),
        TERM_RESET
    );
    println!("{}", "-".repeat(50));
    println!("{}Performance:{}", TERM_BOLD, TERM_RESET);
    println!("  Sessions Parsed: {}", stats.sessions_found);
    println!("  Parse Time:      {:.4} seconds", elapsed);
    println!("{}", "=".repeat(50));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opencode_model_json_parsing() {
        // Test that our Model deserialization handles default formats correctly
        let raw_json = r#"{"id":"gemini-3.5-flash","providerID":"google","variant":"default"}"#;
        let parsed: Result<OpencodeModelJson, _> = serde_json::from_str(raw_json);

        assert!(parsed.is_ok());
        let model = parsed.unwrap();
        assert_eq!(model.id, "gemini-3.5-flash");
    }

    #[test]
    fn test_opencode_timestamp_arithmetic() {
        use chrono::{DateTime, Utc};
        // Verify epoch millisecond to UTC standard conversion logic
        let time_created: i64 = 1781514498227; // June 15, 2026

        let dt = DateTime::from_timestamp(
            time_created / 1000,
            ((time_created % 1000) * 1_000_000) as u32,
        )
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap();

        let date_key = dt.format("%Y-%m-%d").to_string();
        let month_key = dt.format("%Y-%m").to_string();

        assert_eq!(date_key, "2026-06-15");
        assert_eq!(month_key, "2026-06");
    }

    #[test]
    fn test_model_id_parsing() {
        // - Standard JSON: {"id": "gemini-3.5-flash", "providerID": "google"}
        assert_eq!(
            parse_model_id(r#"{"id": "gemini-3.5-flash", "providerID": "google"}"#),
            "gemini-3.5-flash"
        );

        // - Alternate JSON: {"modelID": "claude-3-5-sonnet", "provider": "anthropic"}
        assert_eq!(
            parse_model_id(r#"{"modelID": "claude-3-5-sonnet", "provider": "anthropic"}"#),
            "claude-3-5-sonnet"
        );

        // - Plain string: "gpt-4o"
        assert_eq!(parse_model_id("gpt-4o"), "gpt-4o");

        // - Fallback / Malformed: "{invalid_json" -> "unknown"
        assert_eq!(parse_model_id("{invalid_json"), "unknown");
    }

    #[test]
    fn test_timezone_consistency() {
        let time_created: i64 = 1781514498227; // June 15, 2026 epoch ms

        let dt = DateTime::from_timestamp(
            time_created / 1000,
            ((time_created % 1000) * 1_000_000) as u32,
        )
        .map(|dt| dt.with_timezone(&Local))
        .unwrap();

        let date_key = dt.format("%Y-%m-%d").to_string();
        let month_key = dt.format("%Y-%m").to_string();

        let expected_dt = DateTime::from_timestamp(
            time_created / 1000,
            ((time_created % 1000) * 1_000_000) as u32,
        )
        .map(|dt| dt.with_timezone(&Local))
        .unwrap();

        assert_eq!(date_key, expected_dt.format("%Y-%m-%d").to_string());
        assert_eq!(month_key, expected_dt.format("%Y-%m").to_string());
    }

    #[test]
    fn test_in_memory_opencode_db_parsing() {
        let conn = Connection::open_in_memory().unwrap();

        // 1. Create schema
        conn.execute(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                model TEXT,
                cost REAL,
                tokens_input INTEGER,
                tokens_output INTEGER,
                tokens_reasoning INTEGER,
                tokens_cache_read INTEGER,
                tokens_cache_write INTEGER,
                time_created INTEGER
            )",
            [],
        )
        .unwrap();

        conn.execute(
            "CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                data TEXT,
                time_created INTEGER,
                FOREIGN KEY(session_id) REFERENCES session(id)
            )",
            [],
        )
        .unwrap();

        conn.execute(
            "CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT,
                data TEXT,
                time_created INTEGER,
                FOREIGN KEY(message_id) REFERENCES message(id)
            )",
            [],
        )
        .unwrap();

        // 2. Insert test data
        let day1_ms: i64 = 1781514498227; // June 15, 2026
        let day2_ms: i64 = 1781600898227; // June 16, 2026

        // Insert session
        conn.execute(
            "INSERT INTO session (id, model, cost, tokens_input, tokens_output, tokens_reasoning, tokens_cache_read, tokens_cache_write, time_created) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            (
                "session-1",
                "gemini-3.5-flash",
                0.0,
                0,
                0,
                0,
                0,
                0,
                day1_ms,
            ),
        )
        .unwrap();

        // Message 1 (Day 1) with reasoning tokens: input=100, output=100, reasoning=250, cost=0.001
        let message_data1 = serde_json::json!({
            "role": "assistant",
            "modelID": {
                "id": "gemini-3.5-flash",
                "providerID": "google"
            },
            "cost": 0.001,
            "tokens": {
                "input": 100,
                "output": 100,
                "reasoning": 250,
                "cache": {
                    "read": 10,
                    "write": 20
                }
            }
        })
        .to_string();

        conn.execute(
            "INSERT INTO message (id, session_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
            ("msg-1", "session-1", message_data1, day1_ms),
        )
        .unwrap();

        // Message 2 (Day 2) with reasoning tokens: input=200, output=150, reasoning=100, cost=0.002
        let message_data2 = serde_json::json!({
            "role": "assistant",
            "modelID": "gemini-3.5-flash",
            "cost": 0.002,
            "tokens": {
                "input": 200,
                "output": 150,
                "reasoning": 100,
                "cache": {
                    "read": 30,
                    "write": 40
                }
            }
        })
        .to_string();

        conn.execute(
            "INSERT INTO message (id, session_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
            ("msg-2", "session-1", message_data2, day2_ms),
        )
        .unwrap();

        // Part 1 (Day 1): type='patch', path='src/main.rs', bytes=150
        let part_data1 = serde_json::json!({
            "type": "patch",
            "path": "src/main.rs",
            "bytes": 150
        })
        .to_string();

        // Part 2 (Day 2): type='file', file_path='app.py', size=300
        let part_data2 = serde_json::json!({
            "type": "file",
            "file_path": "app.py",
            "size": 300
        })
        .to_string();

        conn.execute(
            "INSERT INTO part (id, message_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
            ("part-1", "msg-1", part_data1, day1_ms),
        )
        .unwrap();

        conn.execute(
            "INSERT INTO part (id, message_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
            ("part-2", "msg-2", part_data2, day2_ms),
        )
        .unwrap();

        // Parse and verify
        let stats =
            parse_opencode_connection(&conn).expect("Should successfully parse in-memory db");

        // Verify sessions found (we inserted messages for 1 unique session)
        assert_eq!(stats.sessions_found, 1);

        // Verify grand totals
        assert_eq!(stats.total_cost, 0.003);

        // Verify model stats: out_tokens should sum output + reasoning for both messages
        // Msg 1: 100 output + 250 reasoning = 350 out_tokens
        // Msg 2: 150 output + 100 reasoning = 250 out_tokens
        // Total: 600 out_tokens, 300 in_tokens
        let model_stat = stats
            .model_stats
            .get("gemini-3.5-flash")
            .expect("Model should be found");
        assert_eq!(model_stat.in_tokens, 300);
        assert_eq!(model_stat.out_tokens, 600); // 100 + 250 + 150 + 100

        // Verify daily breakdown (checking keys exist, accounting for local timezone mapping)
        let dt1 = DateTime::from_timestamp(day1_ms / 1000, 0)
            .unwrap()
            .with_timezone(&Local);
        let dt2 = DateTime::from_timestamp(day2_ms / 1000, 0)
            .unwrap()
            .with_timezone(&Local);
        let date_key1 = dt1.format("%Y-%m-%d").to_string();
        let date_key2 = dt2.format("%Y-%m-%d").to_string();

        assert!(stats.daily_costs.contains_key(&date_key1));
        assert!(stats.daily_costs.contains_key(&date_key2));

        // Verify language stats: Rust should be 150 bytes, Python should be 300 bytes
        assert_eq!(
            stats.languages.stats.get("Rust").map(|s| s.bytes),
            Some(150)
        );
        assert_eq!(
            stats.languages.stats.get("Python").map(|s| s.bytes),
            Some(300)
        );

        // 3. Test incremental caching with an additional message and part
        let (stats_init, cache_state) = parse_opencode_connection_incremental(&conn, None).unwrap();
        assert_eq!(stats_init.sessions_found, 1);

        let day3_ms: i64 = 1781700898227; // June 17, 2026
        let message_data3 = serde_json::json!({
            "role": "assistant",
            "modelID": "gemini-3.5-flash",
            "cost": 0.005,
            "tokens": {
                "input": 500,
                "output": 200,
                "reasoning": 50,
                "cache": {
                    "read": 100,
                    "write": 50
                }
            }
        })
        .to_string();

        conn.execute(
            "INSERT INTO message (id, session_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
            ("msg-3", "session-1", message_data3, day3_ms),
        )
        .unwrap();

        let part_data3 = serde_json::json!({
            "type": "tool",
            "tool": "edit",
            "state": {
                "status": "completed",
                "input": {
                    "filePath": "src/lib.rs",
                    "newString": "pub fn hello() {}"
                }
            }
        })
        .to_string();

        conn.execute(
            "INSERT INTO part (id, message_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
            ("part-3", "msg-3", part_data3, day3_ms),
        )
        .unwrap();

        let (stats_inc, _) =
            parse_opencode_connection_incremental(&conn, Some(&cache_state)).unwrap();

        // Must accurately reflect the newly added message and part
        assert_eq!(stats_inc.total_cost, 0.008);
        let model_stat_inc = stats_inc.model_stats.get("gemini-3.5-flash").unwrap();
        assert_eq!(model_stat_inc.in_tokens, 800); // 300 + 500
        assert_eq!(model_stat_inc.out_tokens, 850); // 600 + 250
        assert_eq!(
            stats_inc.languages.stats.get("Rust").map(|s| s.occurrences),
            Some(2)
        );

        // Incremental stats must match a fresh cold parse exactly
        let stats_cold = parse_opencode_connection(&conn).unwrap();
        assert_eq!(stats_inc.total_cost, stats_cold.total_cost);
        assert_eq!(
            stats_inc
                .model_stats
                .get("gemini-3.5-flash")
                .unwrap()
                .total(),
            stats_cold
                .model_stats
                .get("gemini-3.5-flash")
                .unwrap()
                .total()
        );
    }

    #[test]
    fn test_opencode_session_count_and_progress_tracking() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE session (id TEXT PRIMARY KEY, time_created INTEGER)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, data TEXT, time_created INTEGER)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, data TEXT, time_created INTEGER)",
            [],
        )
        .unwrap();

        for i in 1..=5 {
            conn.execute(
                "INSERT INTO session (id, time_created) VALUES (?1, ?2)",
                (format!("s-{}", i), 1781514498227i64),
            )
            .unwrap();
            let msg = serde_json::json!({
                "role": "assistant",
                "modelID": "gemini-3.5-flash",
                "cost": 0.01,
                "tokens": { "input": 10, "output": 5 }
            })
            .to_string();
            conn.execute(
                "INSERT INTO message (id, session_id, data, time_created) VALUES (?1, ?2, ?3, ?4)",
                (
                    format!("m-{}", i),
                    format!("s-{}", i),
                    msg,
                    1781514498227i64,
                ),
            )
            .unwrap();
        }

        let session_count = count_opencode_sessions_conn(&conn);
        assert_eq!(session_count, 5);

        let progress = AtomicU32::new(0);
        let (stats, _) = parse_opencode_connection_incremental_with_progress(
            &conn,
            None,
            Some((&progress, session_count as u32)),
        )
        .unwrap();

        assert_eq!(stats.sessions_found, 5);
        assert_eq!(progress.load(Ordering::Relaxed), 5);
    }
}
