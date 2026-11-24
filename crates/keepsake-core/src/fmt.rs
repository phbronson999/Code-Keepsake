//! Small display helpers shared by the CLI and the TUI. std-only.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch, right now.
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `9.1 GB`, `48 KB`, `512 B`. Binary units, labelled the way Explorer does.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Compact age: `now`, `5m`, `3h`, `4d`, `3w`, `14mo`, `3y`.
pub fn age_short(secs: i64) -> String {
    let secs = secs.max(0);
    let days = secs / 86_400;
    match secs {
        s if s < 60 => "now".into(),
        s if s < 3_600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3_600),
        _ if days < 14 => format!("{days}d"),
        _ if days < 60 => format!("{}w", days / 7),
        _ if days < 730 => format!("{}mo", days / 30),
        _ => format!("{}y", days / 365),
    }
}

/// Sentence form: `just now`, `3 days ago`, `14 months ago`.
pub fn age_long(secs: i64) -> String {
    let secs = secs.max(0);
    let days = secs / 86_400;
    let (n, unit) = match secs {
        s if s < 60 => return "just now".into(),
        s if s < 3_600 => (s / 60, "minute"),
        s if s < 86_400 => (s / 3_600, "hour"),
        _ if days < 14 => (days, "day"),
        _ if days < 60 => (days / 7, "week"),
        _ if days < 730 => (days / 30, "month"),
        _ => (days / 365, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

/// Shortens text to `max` characters by cutting out the middle: `C:\src\…\old-tool`.
pub fn middle_truncate(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max || max < 5 {
        return text.to_string();
    }
    let keep = max - 1;
    let head = keep / 3;
    let tail = keep - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}…{end}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(9_771_000_000), "9.1 GB");
        assert_eq!(bytes(150 * 1024 * 1024), "150 MB");
    }

    #[test]
    fn formats_ages() {
        assert_eq!(age_short(30), "now");
        assert_eq!(age_short(5 * 86_400), "5d");
        assert_eq!(age_short(21 * 86_400), "3w");
        assert_eq!(age_short(425 * 86_400), "14mo");
        assert_eq!(age_short(1100 * 86_400), "3y");
        assert_eq!(age_long(86_400), "1 day ago");
        assert_eq!(age_long(425 * 86_400), "14 months ago");
    }

    #[test]
    fn truncates_middle() {
        assert_eq!(middle_truncate("short", 10), "short");
        let t = middle_truncate("C:/Users/bronson/src/very/deep/old-tool", 20);
        assert_eq!(t.chars().count(), 20);
        assert!(t.ends_with("old-tool"));
        assert!(t.contains('…'));
    }
}
