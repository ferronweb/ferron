//! Utility functions shared across Ferron modules.

use std::time::Duration;

/// Parse a duration string (e.g., "12h", "30m", "90s", "1d") into a `Duration`.
///
/// Supported suffixes (case-insensitive):
/// - `h` or `H`: hours
/// - `m` or `M`: minutes
/// - `s` or `S`: seconds
/// - `d` or `D`: days
///
/// Plain numbers (without suffix) are treated as hours for backward compatibility.
///
/// # Examples
///
/// ```
/// use ferron_core::util::parse_duration;
/// use std::time::Duration;
///
/// assert_eq!(parse_duration("12h").unwrap(), Duration::from_secs(12 * 3600));
/// assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
/// assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
/// assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
/// assert_eq!(parse_duration("12").unwrap(), Duration::from_secs(12 * 3600));
/// ```
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();

    if let Some(num_str) = s.strip_suffix(['h', 'H']) {
        let hours: u64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid hours '{}': {}", s, e))?;
        Ok(Duration::from_secs(hours * 3600))
    } else if let Some(num_str) = s.strip_suffix(['m', 'M']) {
        let minutes: u64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid minutes '{}': {}", s, e))?;
        Ok(Duration::from_secs(minutes * 60))
    } else if let Some(num_str) = s.strip_suffix(['s', 'S']) {
        let seconds: u64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid seconds '{}': {}", s, e))?;
        Ok(Duration::from_secs(seconds))
    } else if let Some(num_str) = s.strip_suffix(['d', 'D']) {
        let days: u64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid days '{}': {}", s, e))?;
        Ok(Duration::from_secs(days * 86400))
    } else {
        // Try plain number (assume hours)
        let hours: u64 = s
            .parse()
            .map_err(|e| format!("Invalid duration '{}': {}", s, e))?;
        Ok(Duration::from_secs(hours * 3600))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration_hours() {
        assert_eq!(
            parse_duration("12h").unwrap(),
            Duration::from_secs(12 * 3600)
        );
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(
            parse_duration("24H").unwrap(),
            Duration::from_secs(24 * 3600)
        );
    }

    #[test]
    fn test_parse_duration_minutes() {
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("60M").unwrap(), Duration::from_secs(3600));
    }

    #[test]
    fn test_parse_duration_seconds() {
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
    }

    #[test]
    fn test_parse_duration_days() {
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
        assert_eq!(
            parse_duration("2D").unwrap(),
            Duration::from_secs(2 * 86400)
        );
    }

    #[test]
    fn test_parse_duration_plain_number() {
        // Plain numbers are treated as hours
        assert_eq!(
            parse_duration("12").unwrap(),
            Duration::from_secs(12 * 3600)
        );
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn test_parse_duration_whitespace() {
        assert_eq!(parse_duration(" 12h ").unwrap(), Duration::from_secs(43200));
        assert_eq!(parse_duration(" 30m ").unwrap(), Duration::from_secs(1800));
    }
}
