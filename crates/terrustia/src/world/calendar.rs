//! The real-world calendar, which the game reads off the wall clock rather than out of the world.
//!
//! Two of Terraria's seasons are not world state at all: `Main.halloween` and `Main.xMas` are
//! recomputed from `DateTime.Now` every dawn (`Main.cs:66375-66376`, inside the same block that
//! stops the moons and turns the blood moon off), and they gate a whole ambient roster - the Raven,
//! Hoppin' Jack, the costumed zombies and the Christmas ones. Nothing about them is saved, synced
//! or seeded: a client works them out for itself the moment it receives world data
//! (`MessageBuffer.cs:660-661`), so the server owes the wire nothing here. It only has to agree
//! about what day it is.
//!
//! **Disclosed narrowing: this reads UTC, and the game reads local time.** `DateTime.Now` is the
//! machine's local clock; `std::time::SystemTime` is the only clock in the standard library and it
//! is UTC, and neither `chrono` nor `time` is worth a dependency for one date comparison (the
//! workspace's dependency rule, `AGENTS.md` rule 3). The two disagree only within a few hours of a
//! season's boundary, and a dedicated server and its players are rarely in one timezone anyway.

/// `Main.isHalloweenDateNow` (`Main.cs:13311-13327`), transcribed with its own odd shape intact:
///
/// ```csharp
/// int day = now.Day;
/// int month = now.Month;
/// if (day < 10 || month != 10)
/// {
///     if (day <= 1) return month == 11;
///     return false;
/// }
/// return true;
/// ```
///
/// So: the tenth of October to the end of it, plus the first of November. The `day <= 1` reads as
/// `day == 1` for any real date, since a month has no day zero; it is left as the game writes it.
pub fn is_halloween(month: u32, day: u32) -> bool {
    if day < 10 || month != 10 {
        if day <= 1 {
            return month == 11;
        }
        return false;
    }
    true
}

/// `Main.checkXMas` (`Main.cs:13290-13301`): `day >= 15 && month == 12`, the fifteenth of December
/// to the end of the year.
///
/// Note what it is *not*: it does not run into January, and it is nothing to do with
/// `Progress::downed_halloween_king` or `downed_halloween_tree`, which are Pumpking and Everscream
/// boss-defeat flags for the Frost Moon.
pub fn is_xmas(month: u32, day: u32) -> bool {
    day >= 15 && month == 12
}

/// Today's `(month, day)`, UTC, or `(1, 1)` if the clock is somehow before the epoch.
pub fn today() -> (u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    month_day(secs as i64)
}

/// The civil month and day for a Unix timestamp, UTC.
///
/// Howard Hinnant's `civil_from_days`, the standard branch-free days-to-date conversion, with the
/// year dropped because nothing here wants it. Hand-rolled rather than pulled in: it is eight lines
/// of integer arithmetic with no edge cases past the ones the algorithm already handles, which is
/// exactly the shape `AGENTS.md` rule 3 says to write rather than depend on.
fn month_day(secs: i64) -> (u32, u32) {
    // Shift the epoch to 0000-03-01 so a leap day lands at the end of a four-century cycle.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // [0, 399]
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let march_month = (5 * day_of_year + 2) / 153; // [0, 11], March is 0
    let day = day_of_year - (153 * march_month + 2) / 5 + 1; // [1, 31]
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };
    (month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The date arithmetic against dates checked by hand, including both leap-year edges and the
    /// two boundaries the seasons below actually care about.
    #[test]
    fn the_epoch_and_the_awkward_dates_convert() {
        assert_eq!(month_day(0), (1, 1)); // 1970-01-01
        assert_eq!(month_day(86_399), (1, 1)); // one second before 1970-01-02
        assert_eq!(month_day(86_400), (1, 2));
        assert_eq!(month_day(68_255_999), (2, 29)); // 1972-02-29, a leap year
        assert_eq!(month_day(68_256_000), (3, 1));
        assert_eq!(month_day(68_342_400), (3, 2));
        assert_eq!(month_day(951_782_400), (2, 29)); // 2000-02-29, the century that is a leap year
        assert_eq!(month_day(4_107_542_400), (3, 1)); // 2100-03-01, the century that is not
        assert_eq!(month_day(1_760_054_400), (10, 10)); // 2025-10-10, Halloween's first day
        assert_eq!(month_day(1_762_041_600), (11, 2)); // 2025-11-02, its first day off
        assert_eq!(month_day(1_765_756_800), (12, 15)); // 2025-12-15, Christmas's first day
        assert_eq!(month_day(-86_400), (12, 31)); // 1969-12-31, before the epoch
    }

    /// The Halloween window is the tenth of October to the first of November inclusive, and nothing
    /// either side of it.
    #[test]
    fn halloween_runs_from_october_the_tenth_to_november_the_first() {
        assert!(!is_halloween(10, 9));
        assert!(is_halloween(10, 10));
        assert!(is_halloween(10, 31));
        assert!(is_halloween(11, 1));
        assert!(!is_halloween(11, 2));
        // The `day < 10` half of the first test is a *day* test, not a month one: an early day in
        // any other month must not slip through the `day <= 1` fallback either.
        assert!(!is_halloween(9, 1));
        assert!(!is_halloween(1, 1));
        assert!(!is_halloween(12, 25));
    }

    /// Christmas is the second half of December only, and does not run into the new year.
    #[test]
    fn christmas_is_the_back_half_of_december() {
        assert!(!is_xmas(12, 14));
        assert!(is_xmas(12, 15));
        assert!(is_xmas(12, 31));
        assert!(!is_xmas(1, 1));
        assert!(!is_xmas(11, 20));
    }
}
