use crate::Country;
use crate::league::LeagueResult;

impl Country {
    pub(crate) fn on_media_coverage(&mut self, league_results: &[LeagueResult]) {
        self.media_coverage.update_from_results(league_results);
        self.media_coverage.generate_weekly_stories(&self.clubs);
    }
}
