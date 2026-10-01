//! Minimal civil-date helpers (UTC) so no date/time crate is needed.

use std::time::{SystemTime, UNIX_EPOCH};

/// (year, month, day) for a count of days since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn year_now() -> i64 {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    civil_from_days((secs / 86_400) as i64).0
}

/// `YYYY-MM` for a millisecond Unix timestamp.
pub fn year_month(ms: u64) -> String {
    let (y, m, _) = civil_from_days((ms / 86_400_000) as i64);
    format!("{y}-{m:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(year_month(1_700_000_000_000), "2023-11");
    }
}
