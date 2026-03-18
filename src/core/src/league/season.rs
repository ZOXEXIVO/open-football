use chrono::{Datelike, NaiveDate};

#[derive(Debug, Clone)]
pub struct Season {
    pub display: String,
    pub start_year: u16,
}

impl Season {
    pub fn new(start_year: u16) -> Self {
        let end_year = start_year + 1;
        Season {
            display: format!("{}/{}", start_year, end_year % 100),
            start_year,
        }
    }

    /// Determine which season a date falls in.
    /// Seasons run Aug–Jul: Aug 2033 → season 2033/34, Jun 2033 → season 2032/33.
    pub fn from_date(date: NaiveDate) -> Self {
        let start_year = if date.month() >= 8 {
            date.year() as u16
        } else {
            (date.year() - 1) as u16
        };
        Self::new(start_year)
    }

    /// Approximate start date of this season (Aug 1).
    pub fn start_date(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.start_year as i32, 8, 1).unwrap()
    }

    /// Approximate end date of this season (May 31 of next year).
    pub fn end_date(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.start_year as i32 + 1, 5, 31).unwrap()
    }
}
