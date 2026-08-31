//! ISO 8601 timestamp -> Unix seconds, without a chrono dependency.
//! Covers the formats delivered by GitHub/GitLab:
//! `2024-01-15T10:30:00Z`, `…T10:30:00.123Z`, `…T10:30:00+02:00`.

/// Parses an ISO 8601 timestamp (normalized to UTC). `None` on garbage.
pub(crate) fn parse_iso8601_utc(s: &str) -> Option<i64> {
    let s = s.trim();
    // get(..) instead of split_at_checked: the latter is only stable from Rust 1.80 (MSRV 1.77.2).
    let (date, rest) = (s.get(..10)?, s.get(10..)?);
    let mut d = date.split('-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let rest = rest.strip_prefix('T').or_else(|| rest.strip_prefix(' '))?;
    let (time, offset_secs) = split_offset(rest)?;
    let mut t = time.split(':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    // Seconds may carry a fraction ("00.123") — cut it off.
    let second: i64 = t.next()?.split('.').next()?.parse().ok()?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..61).contains(&second) {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second - offset_secs)
}

/// Splits the time part from the zone suffix; returns the offset in seconds.
fn split_offset(rest: &str) -> Option<(&str, i64)> {
    if let Some(t) = rest.strip_suffix('Z') {
        return Some((t, 0));
    }
    // +hh:mm / -hh:mm (search for the position; '-' never occurs in fractions either)
    if let Some(pos) = rest.rfind(['+', '-']) {
        let (time, zone) = rest.split_at(pos);
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let z = &zone[1..];
        let (h, m) = z.split_once(':').unwrap_or((z, "0"));
        let h: i64 = h.parse().ok()?;
        let m: i64 = m.parse().ok()?;
        return Some((time, sign * (h * 3_600 + m * 60)));
    }
    // No suffix: interpret as UTC.
    Some((rest, 0))
}

/// Days since 1970-01-01 (Howard Hinnant's "days from civil" algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u64; // [0, 399]
    let mp = u64::from((m + 9) % 12); // [0, 11], March = 0
    let doy = (153 * mp + 2) / 5 + u64::from(d) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe as i64 - 719_468
}

#[cfg(test)]
mod tests {
    use super::parse_iso8601_utc;

    #[test]
    fn known_points_in_time() {
        assert_eq!(parse_iso8601_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso8601_utc("2009-02-13T23:31:30Z"),
            Some(1_234_567_890)
        );
        // Leap-year edge case
        assert_eq!(
            parse_iso8601_utc("2024-02-29T00:00:00Z"),
            Some(1_709_164_800)
        );
    }

    #[test]
    fn fractions_and_offsets() {
        assert_eq!(
            parse_iso8601_utc("2009-02-13T23:31:30.123Z"),
            Some(1_234_567_890)
        );
        // +02:00 is AHEAD of UTC → subtract 2 h
        assert_eq!(
            parse_iso8601_utc("2009-02-14T01:31:30+02:00"),
            Some(1_234_567_890)
        );
        assert_eq!(
            parse_iso8601_utc("2009-02-13T23:31:30+00:00"),
            Some(1_234_567_890)
        );
    }

    #[test]
    fn garbage_returns_none() {
        assert_eq!(parse_iso8601_utc(""), None);
        assert_eq!(parse_iso8601_utc("yesterday"), None);
        assert_eq!(parse_iso8601_utc("2024-13-01T00:00:00Z"), None);
        assert_eq!(parse_iso8601_utc("2024-01-01"), None);
    }
}
