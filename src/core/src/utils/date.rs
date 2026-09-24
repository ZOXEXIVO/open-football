use chrono::NaiveDate;
use chrono::prelude::*;

pub struct DateUtils;

impl DateUtils {
    #[inline]
    pub fn is_birthday(birth_date: NaiveDate, current_date: NaiveDate) -> bool {
        birth_date.month() == current_date.month() && birth_date.day() == current_date.day()
    }

    #[inline]
    pub fn age(birthdate: NaiveDate, now: NaiveDate) -> u8 {
        let age_duration = now.signed_duration_since(birthdate);
        (age_duration.num_days() / 365) as u8
    }

    #[inline]
    pub fn is_quarter_start(date: NaiveDate) -> bool {
        date.day() == 1 && date.month().is_multiple_of(3)
    }

    #[inline]
    pub fn is_year_start(date: NaiveDate) -> bool {
        date.month() == 1 && date.day() == 1
    }

    #[inline]
    pub fn is_year_end(date: NaiveDate) -> bool {
        date.month() == 12 && date.day() == 31
    }

    #[inline]
    pub fn is_month_beginning(date: NaiveDate) -> bool {
        date.day() == 1
    }

    /// `date` itself when it already falls on `weekday`.
    pub fn next_weekday(date: NaiveDate, weekday: Weekday) -> NaiveDate {
        let mut current_date = date;

        while current_date.weekday() != weekday {
            current_date = current_date.succ_opt().unwrap();
        }

        current_date
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_birthday() {
        let birth_date = NaiveDate::from_ymd_opt(1990, 3, 16).unwrap();
        let current_date = NaiveDate::from_ymd_opt(2024, 3, 16).unwrap();
        assert!(DateUtils::is_birthday(birth_date, current_date));

        let birth_date = NaiveDate::from_ymd_opt(1990, 3, 16).unwrap();
        let current_date = NaiveDate::from_ymd_opt(2024, 3, 17).unwrap();
        assert!(!DateUtils::is_birthday(birth_date, current_date));
    }

    #[test]
    fn test_age() {
        let birth_date = NaiveDate::from_ymd_opt(1990, 3, 16).unwrap();
        let current_date = NaiveDate::from_ymd_opt(2024, 3, 16).unwrap();

        assert_eq!(DateUtils::age(birth_date, current_date), 34);

        let birth_date = NaiveDate::from_ymd_opt(1990, 3, 16).unwrap();
        let current_date = NaiveDate::from_ymd_opt(2024, 3, 15).unwrap();

        assert_eq!(DateUtils::age(birth_date, current_date), 34);
    }

    #[test]
    fn test_next_weekday() {
        let tuesday = NaiveDate::from_ymd_opt(2024, 3, 12).unwrap();
        assert_eq!(
            DateUtils::next_weekday(tuesday, Weekday::Sat),
            NaiveDate::from_ymd_opt(2024, 3, 16).unwrap()
        );
        assert_eq!(
            DateUtils::next_weekday(tuesday, Weekday::Fri),
            NaiveDate::from_ymd_opt(2024, 3, 15).unwrap()
        );

        let saturday = NaiveDate::from_ymd_opt(2024, 3, 16).unwrap();
        assert_eq!(DateUtils::next_weekday(saturday, Weekday::Sat), saturday);
        assert_eq!(
            DateUtils::next_weekday(saturday, Weekday::Fri),
            NaiveDate::from_ymd_opt(2024, 3, 22).unwrap()
        );

        let sunday = NaiveDate::from_ymd_opt(2024, 3, 17).unwrap();
        assert_eq!(
            DateUtils::next_weekday(sunday, Weekday::Sat),
            NaiveDate::from_ymd_opt(2024, 3, 23).unwrap()
        );
    }
}
