//! Today's date for a release heading. `std` has neither a calendar nor time zones.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Today as `YYYY-MM-DD` in the committer's time zone, which is the date the release commit
/// is going to carry. git reports the zone (`git var GIT_COMMITTER_IDENT` ends in
/// `<unix time> <+hhmm>`, honouring `TZ` as `git commit` does). Without git it is UTC.
pub fn today() -> String {
    let (seconds, offset) = git_now().unwrap_or_else(|| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after 1970");
        (now.as_secs() as i64, 0)
    });
    format_date((seconds + offset).div_euclid(86_400))
}

fn git_now() -> Option<(i64, i64)> {
    let output = Command::new("git")
        .args(["var", "GIT_COMMITTER_IDENT"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_ident(&String::from_utf8_lossy(&output.stdout))
}

/// `Name <email> 1760000000 +0900` → `(1760000000, 32400)`.
fn parse_ident(ident: &str) -> Option<(i64, i64)> {
    let mut fields = ident.split_whitespace().rev();
    let zone = fields.next()?;
    let seconds = fields.next()?.parse().ok()?;
    let sign = match zone.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = zone.get(1..).filter(|digits| digits.len() == 4)?;
    let hhmm: i64 = digits.parse().ok()?;
    Some((seconds, sign * (hhmm / 100 * 3600 + hhmm % 100 * 60)))
}

/// Days since 1970-01-01 as `YYYY-MM-DD` (proleptic Gregorian). This is Howard Hinnant's
/// `civil_from_days`: shift the epoch to 0000-03-01 so the leap day ends the year, then
/// count 400-year eras, years in the era, and days in the year.
fn format_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_days_since_the_epoch() {
        assert_eq!(format_date(0), "1970-01-01");
        assert_eq!(format_date(-1), "1969-12-31");
        assert_eq!(format_date(11_016), "2000-02-29");
        assert_eq!(format_date(19_782), "2024-02-29");
        assert_eq!(format_date(19_783), "2024-03-01");
        assert_eq!(format_date(20_739), "2026-10-13");
        assert_eq!(format_date(20_818), "2026-12-31");
    }

    #[test]
    fn reads_the_time_and_zone_from_a_git_ident() {
        assert_eq!(
            parse_ident("yo_waka <y@example.com> 1760000000 +0900\n"),
            Some((1_760_000_000, 9 * 3600))
        );
        assert_eq!(
            parse_ident("A B <a@b> 1760000000 -0330"),
            Some((1_760_000_000, -(3 * 3600 + 30 * 60)))
        );
        assert_eq!(parse_ident("A <a@b> 1760000000 0900"), None);
        assert_eq!(parse_ident("A <a@b> soon +0900"), None);
        assert_eq!(parse_ident(""), None);
    }

    #[test]
    fn the_zone_moves_the_date() {
        // 2026-10-12 23:30 UTC is already the 13th in Japan.
        let (seconds, offset) = parse_ident("A <a@b> 1791847800 +0900").unwrap();
        assert_eq!(format_date(seconds.div_euclid(86_400)), "2026-10-12");
        assert_eq!(
            format_date((seconds + offset).div_euclid(86_400)),
            "2026-10-13"
        );
    }
}
