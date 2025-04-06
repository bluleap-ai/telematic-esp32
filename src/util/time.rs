/// Days per month for non-leap and leap years
const DAYS_IN_MONTH: [[u16; 12]; 2] = [
    [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31], // Normal year
    [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31], // Leap year
];

/// Check if a year is a leap year
const fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Convert UTC and date to UNIX timestamp
pub fn utc_date_to_unix_timestamp(utc: &[u8], date: &[u8]) -> u64 {
    let day = (date[0] - b'0') as u16 * 10 + (date[1] - b'0') as u16;
    let month = (date[2] - b'0') as u16 * 10 + (date[3] - b'0') as u16;
    let year = (date[4] - b'0') as u16 * 10 + (date[5] - b'0') as u16 + 2000; // Convert to full year

    let hour = (utc[0] - b'0') as u16 * 10 + (utc[1] - b'0') as u16;
    let minute = (utc[2] - b'0') as u16 * 10 + (utc[3] - b'0') as u16;
    let second = (utc[4] - b'0') as u16 * 10 + (utc[5] - b'0') as u16;

    let millis =
        (utc[7] - b'0') as u16 * 100 + (utc[8] - b'0') as u16 * 10 + (utc[9] - b'0') as u16;

    // Compute days since Unix epoch (1970-01-01)
    let mut days = 0;
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }

    // Add days for past months in the current year
    let leap = is_leap_year(year) as usize;
    for m in 0..(month - 1) as usize {
        days += DAYS_IN_MONTH[leap][m] as u64;
    }

    // Add days in the current month
    days += day as u64 - 1;

    // Convert to seconds
    let mut timestamp = days * 86400 + (hour as u64) * 3600 + (minute as u64) * 60 + second as u64;

    // Convert milliseconds to UNIX timestamp with ms precision
    timestamp = timestamp * 1000 + millis as u64;

    timestamp
}
