//! UTC dates without a date library.

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// `YYYY-MM-DD` for a Unix time.
pub fn date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// `YYYY-MM-DD HH:MM UTC` for a Unix time.
pub fn date_time(secs: u64) -> String {
    let minutes = secs % 86_400 / 60;
    format!("{} {:02}:{:02} UTC", date(secs), minutes / 60, minutes % 60)
}

/// Local `(YYYY-MM-DD, HH:MM)` for a Unix time, for showing to the user.
pub fn local(secs: u64) -> (String, String) {
    let time = secs as libc::time_t;
    // SAFETY: an all-zero `tm` is valid, and `localtime_r` only writes into it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        let minutes = secs % 86_400 / 60;
        return (
            date(secs),
            format!("{:02}:{:02}", minutes / 60, minutes % 60),
        );
    }
    (
        format!(
            "{:04}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday
        ),
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date_time(951_782_400 + 3_660), "2000-02-29 01:01 UTC");
        assert_eq!(date(1_794_268_800), "2026-11-10");
    }
}
