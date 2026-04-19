use crate::continent::continent::Continent;
use crate::r#match::MatchSquad;
use crate::Club;
use crate::NationalCompetitionFixture;
use crate::NationalTeam;
use chrono::NaiveDate;

impl Continent {
    /// Build squads for all today's competition matches
    pub(crate) fn prepare_match_squads(
        &mut self,
        fixtures: &[NationalCompetitionFixture],
        date: NaiveDate,
    ) -> Vec<(usize, MatchSquad, MatchSquad)> {
        fixtures
            .iter()
            .enumerate()
            .filter_map(|(idx, fixture)| {
                let home = self.build_country_match_squad(fixture.home_country_id, date)?;
                let away = self.build_country_match_squad(fixture.away_country_id, date)?;
                Some((idx, home, away))
            })
            .collect()
    }

    /// Build a MatchSquad for a country, ensuring national team has called up players
    pub(crate) fn build_country_match_squad(&mut self, country_id: u32, date: NaiveDate) -> Option<MatchSquad> {
        let country_ids: Vec<(u32, String)> = self.countries.iter().map(|c| (c.id, c.name.clone())).collect();

        let country_idx = self.countries.iter().position(|c| c.id == country_id)?;

        // Ensure the national team has a squad called up
        if self.countries[country_idx].national_team.squad.is_empty()
            && self.countries[country_idx].national_team.generated_squad.is_empty()
        {
            // Collect candidates from ALL clubs across ALL countries
            let mut all_candidates = NationalTeam::collect_all_candidates_by_country(&self.countries, date);
            let candidates = all_candidates.remove(&country_id).unwrap_or_default();

            let country = &mut self.countries[country_idx];
            country.national_team.country_name = country.name.clone();
            country.national_team.reputation = country.reputation;

            let cid = country.id;
            country.national_team.call_up_squad(&mut country.clubs, candidates, date, cid, &country_ids);
        }

        // Collect clubs from ALL countries — squad members may play abroad
        let all_clubs: Vec<&Club> = self.countries.iter()
            .flat_map(|c| &c.clubs)
            .collect();
        let country = &self.countries[country_idx];
        let squad = country.national_team.build_match_squad_from_refs(&all_clubs);
        Some(squad)
    }
}
