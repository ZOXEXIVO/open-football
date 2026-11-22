use crate::Country;

impl Country {
    /// Off-season training camps, one day for every club player.
    pub(crate) fn on_preseason_activities(&mut self) {
        for club in &mut self.clubs {
            let training_quality = club.facilities.training.multiplier();
            for team in &mut club.teams.teams {
                for player in &mut team.players.players {
                    player.on_offseason_camp_day(training_quality);
                }
            }
        }
    }
}
