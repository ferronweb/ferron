//! Utility functions shared across Ferron modules.

use std::time::Duration;

#[inline]
fn checked_secs(value: f64, multiplier: f64, label: &str) -> Result<Duration, String> {
    if !value.is_finite() {
        return Err(format!("Invalid {label}: value is not finite"));
    }
    if value < 0.0 {
        return Err(format!("Invalid {label}: value is negative"));
    }
    // Reject values that would overflow u64 seconds when scaled. The
    // maximum representable u64 seconds is approximately 1.8446744e19.
    // Use a conservative bound to stay safely within `Duration` limits.
    const MAX_SECS: f64 = (u64::MAX as f64) / 2.0;
    let total = value * multiplier;
    if total > MAX_SECS {
        return Err(format!("Invalid {label}: value is too large"));
    }
    Ok(Duration::from_secs_f64(total))
}

/// Parse a duration string (e.g., "12h", "30m", "90s", "1d") into a `Duration`.
///
/// Supported suffixes (case-insensitive):
/// - `h` or `H`: hours
/// - `m` or `M`: minutes
/// - `s` or `S`: seconds
/// - `d` or `D`: days
///
/// Plain numbers (without suffix) are treated as seconds.
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
/// assert_eq!(parse_duration("12").unwrap(), Duration::from_secs(12));
/// ```
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();

    if let Some(num_str) = s.strip_suffix(['h', 'H']) {
        let hours: f64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid hours '{}': {}", s, e))?;
        checked_secs(hours, 3600.0, &format!("hours '{s}'"))
    } else if let Some(num_str) = s.strip_suffix(['m', 'M']) {
        let minutes: f64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid minutes '{}': {}", s, e))?;
        checked_secs(minutes, 60.0, &format!("minutes '{s}'"))
    } else if let Some(num_str) = s.strip_suffix(['s', 'S']) {
        let seconds: f64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid seconds '{}': {}", s, e))?;
        checked_secs(seconds, 1.0, &format!("seconds '{s}'"))
    } else if let Some(num_str) = s.strip_suffix(['d', 'D']) {
        let days: f64 = num_str
            .trim()
            .parse()
            .map_err(|e| format!("Invalid days '{}': {}", s, e))?;
        checked_secs(days, 86400.0, &format!("days '{s}'"))
    } else {
        // Try plain number (assume seconds)
        let seconds: f64 = s
            .parse()
            .map_err(|e| format!("Invalid duration '{}': {}", s, e))?;
        checked_secs(seconds, 1.0, &format!("duration '{s}'"))
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
        // Plain numbers are treated as seconds
        assert_eq!(parse_duration("12").unwrap(), Duration::from_secs(12));
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

    #[test]
    fn test_parse_duration_rejects_negative() {
        assert!(parse_duration("-1h").is_err());
        assert!(parse_duration("-30m").is_err());
        assert!(parse_duration("-10").is_err());
    }

    #[test]
    fn test_parse_duration_rejects_overflow() {
        // A value that would overflow u64 seconds when scaled
        assert!(parse_duration("999999999999999999h").is_err());
        assert!(parse_duration("999999999999999d").is_err());
        assert!(parse_duration("99999999999999999999").is_err());
    }

    #[test]
    fn test_parse_duration_rejects_nan_and_infinity() {
        assert!(parse_duration("NaNh").is_err());
        assert!(parse_duration("infinitys").is_err());
        assert!(parse_duration("-infinitym").is_err());
    }
}
