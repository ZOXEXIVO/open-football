use crate::continent::continent::Continent;
use chrono::NaiveDate;

impl Continent {
    /// Skip international friendly match simulation — these are not worth the CPU cost.
    /// Fixtures remain in the schedule but are never played.
    #[allow(dead_code)]
    pub(crate) fn simulate_international_friendlies(&mut self, _date: NaiveDate) {}
}
