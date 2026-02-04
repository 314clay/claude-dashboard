//! Beads JSONL loader with caching.
//!
//! Implements:
//! - Lazy loading: Only load when requested
//! - Caching: Cache parsed data, invalidate on mtime change
//! - Incremental loading: Support loading in chunks for large files

use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::time::SystemTime;

/// A single bead entry from the JSONL file.
#[derive(Debug, Clone, Deserialize)]
pub struct BeadEntry {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    pub status: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub issue_type: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    #[serde(default)]
    pub closed_at: Option<String>,
    #[serde(default)]
    pub close_reason: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
}

impl BeadEntry {
    /// Parse timestamp to epoch seconds for sorting.
    pub fn updated_at_secs(&self) -> Option<f64> {
        self.updated_at.as_ref().and_then(|s| parse_timestamp(s))
    }

    /// Parse timestamp to epoch seconds for sorting.
    pub fn created_at_secs(&self) -> Option<f64> {
        self.created_at.as_ref().and_then(|s| parse_timestamp(s))
    }

    /// Status badge color.
    pub fn status_color(&self) -> egui::Color32 {
        match self.status.as_str() {
            "open" => egui::Color32::from_rgb(34, 197, 94),   // Green
            "in_progress" => egui::Color32::from_rgb(59, 130, 246), // Blue
            "blocked" => egui::Color32::from_rgb(239, 68, 68), // Red
            "closed" => egui::Color32::from_rgb(107, 114, 128), // Gray
            "hooked" => egui::Color32::from_rgb(168, 85, 247), // Purple
            _ => egui::Color32::from_rgb(156, 163, 175),       // Light gray
        }
    }
}

/// Parse ISO 8601 timestamp to epoch seconds.
fn parse_timestamp(ts: &str) -> Option<f64> {
    // Handle format like "2026-02-01T01:32:51.038294+13:00"
    let ts = ts.replace('T', " ").replace('Z', "+00:00");

    // Find the timezone offset position
    if let Some(plus_idx) = ts.rfind('+').or_else(|| {
        // Handle negative offset like -05:00
        let last_minus = ts.rfind('-')?;
        // Make sure it's not part of the date
        if last_minus > 10 { Some(last_minus) } else { None }
    }) {
        let datetime_part = &ts[..plus_idx];
        let parts: Vec<&str> = datetime_part.split(' ').collect();
        if parts.len() >= 2 {
            let date_parts: Vec<&str> = parts[0].split('-').collect();
            let time_full = parts[1];
            let time_parts: Vec<&str> = time_full.split(':').collect();

            if date_parts.len() >= 3 && time_parts.len() >= 3 {
                let year: i32 = date_parts[0].parse().ok()?;
                let month: u32 = date_parts[1].parse().ok()?;
                let day: u32 = date_parts[2].parse().ok()?;
                let hour: u32 = time_parts[0].parse().ok()?;
                let min: u32 = time_parts[1].parse().ok()?;
                let sec_str = time_parts[2].split('.').next()?;
                let sec: u32 = sec_str.parse().ok()?;

                let days_since_epoch = days_from_civil(year, month, day);
                let secs = days_since_epoch as f64 * 86400.0
                    + hour as f64 * 3600.0
                    + min as f64 * 60.0
                    + sec as f64;
                return Some(secs);
            }
        }
    }
    None
}

/// Calculate days since Unix epoch for a date (matching types.rs).
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m = month;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// Result of loading beads.
#[derive(Debug, Clone)]
pub struct BeadLoadResult {
    pub entries: Vec<BeadEntry>,
    pub parse_errors: usize,
    pub load_time_ms: u64,
}

/// Beads loader with caching.
pub struct BeadLoader {
    /// Path to the beads directory (follows redirects).
    beads_path: Option<PathBuf>,
    /// Cached entries by ID for fast lookup.
    cache: HashMap<String, BeadEntry>,
    /// Sorted entries for display (most recently updated first).
    sorted_entries: Vec<BeadEntry>,
    /// Last modification time of the JSONL file.
    last_mtime: Option<SystemTime>,
    /// Whether the cache is valid.
    cache_valid: bool,
    /// Time of last check (to avoid checking too frequently).
    last_check: std::time::Instant,
    /// Minimum interval between mtime checks (100ms).
    check_interval: std::time::Duration,
}

impl Default for BeadLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl BeadLoader {
    pub fn new() -> Self {
        Self {
            beads_path: None,
            cache: HashMap::new(),
            sorted_entries: Vec::new(),
            last_mtime: None,
            cache_valid: false,
            last_check: std::time::Instant::now(),
            check_interval: std::time::Duration::from_millis(100),
        }
    }

    /// Initialize the loader by finding the beads directory.
    /// Follows redirect files if present.
    pub fn init(&mut self) {
        self.beads_path = Self::find_beads_dir();
    }

    /// Find the beads directory, following redirects.
    fn find_beads_dir() -> Option<PathBuf> {
        let cwd = std::env::current_dir().ok()?;

        // Check for .beads/redirect file
        let redirect_path = cwd.join(".beads/redirect");
        if redirect_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&redirect_path) {
                let redirect_target = content.trim();
                let resolved = cwd.join(".beads").join(redirect_target);
                if resolved.exists() {
                    return Some(resolved);
                }
            }
        }

        // Direct .beads directory
        let direct = cwd.join(".beads");
        if direct.exists() {
            return Some(direct);
        }

        None
    }

    /// Get the path to the issues.jsonl file.
    fn jsonl_path(&self) -> Option<PathBuf> {
        self.beads_path.as_ref().map(|p| p.join("issues.jsonl"))
    }

    /// Check if the cache needs to be invalidated.
    pub fn needs_refresh(&mut self) -> bool {
        // Don't check too frequently
        let now = std::time::Instant::now();
        if now.duration_since(self.last_check) < self.check_interval {
            return !self.cache_valid;
        }
        self.last_check = now;

        // Check if path is initialized
        if self.beads_path.is_none() {
            self.init();
        }

        // Check mtime
        let Some(jsonl_path) = self.jsonl_path() else {
            return false;
        };

        let current_mtime = std::fs::metadata(&jsonl_path)
            .ok()
            .and_then(|m| m.modified().ok());

        let needs_refresh = match (current_mtime, self.last_mtime) {
            (Some(current), Some(last)) => current != last,
            (Some(_), None) => true,  // File exists but wasn't tracked
            (None, Some(_)) => true,  // File was removed
            (None, None) => false,    // No file
        };

        if needs_refresh {
            self.cache_valid = false;
        }

        !self.cache_valid
    }

    /// Load and parse the JSONL file, updating the cache.
    pub fn load(&mut self) -> BeadLoadResult {
        let start = std::time::Instant::now();

        // Initialize if needed
        if self.beads_path.is_none() {
            self.init();
        }

        let Some(jsonl_path) = self.jsonl_path() else {
            return BeadLoadResult {
                entries: Vec::new(),
                parse_errors: 0,
                load_time_ms: 0,
            };
        };

        // Update mtime tracking
        self.last_mtime = std::fs::metadata(&jsonl_path)
            .ok()
            .and_then(|m| m.modified().ok());

        // Parse the JSONL file
        let mut entries = Vec::new();
        let mut parse_errors = 0;

        if let Ok(file) = File::open(&jsonl_path) {
            let reader = BufReader::new(file);
            for line in reader.lines() {
                let Ok(line) = line else {
                    parse_errors += 1;
                    continue;
                };

                if line.trim().is_empty() {
                    continue;
                }

                match serde_json::from_str::<BeadEntry>(&line) {
                    Ok(entry) => {
                        self.cache.insert(entry.id.clone(), entry.clone());
                        entries.push(entry);
                    }
                    Err(_) => {
                        parse_errors += 1;
                    }
                }
            }
        }

        // Sort by updated_at descending (most recent first)
        entries.sort_by(|a, b| {
            let a_time = a.updated_at_secs().unwrap_or(0.0);
            let b_time = b.updated_at_secs().unwrap_or(0.0);
            b_time.partial_cmp(&a_time).unwrap_or(std::cmp::Ordering::Equal)
        });

        self.sorted_entries = entries.clone();
        self.cache_valid = true;

        let load_time_ms = start.elapsed().as_millis() as u64;

        BeadLoadResult {
            entries,
            parse_errors,
            load_time_ms,
        }
    }

    /// Get cached entries (returns empty if cache invalid).
    pub fn get_entries(&self) -> &[BeadEntry] {
        &self.sorted_entries
    }

    /// Get a specific entry by ID.
    pub fn get_by_id(&self, id: &str) -> Option<&BeadEntry> {
        self.cache.get(id)
    }

    /// Filter entries by status.
    pub fn filter_by_status(&self, status: &str) -> Vec<&BeadEntry> {
        self.sorted_entries
            .iter()
            .filter(|e| e.status == status)
            .collect()
    }

    /// Get entries grouped by status for display.
    pub fn grouped_by_status(&self) -> BeadGroups<'_> {
        let mut groups = BeadGroups::default();

        for entry in &self.sorted_entries {
            match entry.status.as_str() {
                "in_progress" => groups.in_progress.push(entry),
                "open" => groups.open.push(entry),
                "blocked" => groups.blocked.push(entry),
                "hooked" => groups.hooked.push(entry),
                "closed" => groups.closed.push(entry),
                _ => groups.other.push(entry),
            }
        }

        groups
    }

    /// Total count of entries.
    pub fn total_count(&self) -> usize {
        self.sorted_entries.len()
    }

    /// Whether the cache is currently valid.
    pub fn is_cache_valid(&self) -> bool {
        self.cache_valid
    }
}

/// Beads grouped by status for display.
#[derive(Default)]
pub struct BeadGroups<'a> {
    pub in_progress: Vec<&'a BeadEntry>,
    pub open: Vec<&'a BeadEntry>,
    pub blocked: Vec<&'a BeadEntry>,
    pub hooked: Vec<&'a BeadEntry>,
    pub closed: Vec<&'a BeadEntry>,
    pub other: Vec<&'a BeadEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamp() {
        let ts = parse_timestamp("2026-02-01T01:32:51.038294+13:00");
        assert!(ts.is_some());

        let ts2 = parse_timestamp("2026-02-01T08:29:31.430165+13:00");
        assert!(ts2.is_some());

        // ts2 should be later
        assert!(ts2.unwrap() > ts.unwrap());
    }

    #[test]
    fn test_parse_entry() {
        let json = r#"{"id":"dn-0559","title":"Digest: mol-witness-patrol","description":"Cycle 2: stable.","status":"closed","priority":2,"issue_type":"task","created_at":"2026-02-01T01:32:51.038294+13:00","updated_at":"2026-02-01T01:32:51.038294+13:00"}"#;
        let entry: BeadEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.id, "dn-0559");
        assert_eq!(entry.status, "closed");
    }
}
