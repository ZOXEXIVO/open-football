use crate::transfers::ScoutingRegion;
use crate::world::SimulatorData;
use crate::{Player, PlayerCollection};
use chrono::{Datelike, NaiveDate};
use rayon::prelude::*;
use std::collections::HashMap;

/// Where the world's players are FROM, denormalised onto the players
/// themselves.
///
/// The region needs the country CODE, which is exactly what a player's
/// own nationality cannot supply from inside another country's borrow —
/// so it is stamped here alongside the continent and read everywhere
/// through [`Player::home_region`].
pub struct PassportOffice {
    /// `country_id -> (continent_id, region)`, built once per pass.
    stamps: HashMap<u32, (u32, ScoutingRegion)>,
}

impl PassportOffice {
    fn open(data: &SimulatorData) -> Option<Self> {
        let stamps: HashMap<u32, (u32, ScoutingRegion)> = data
            .country_info
            .iter()
            .map(|(id, info)| {
                (
                    *id,
                    (
                        info.continent_id,
                        ScoutingRegion::from_country(info.continent_id, &info.code),
                    ),
                )
            })
            .collect();
        (!stamps.is_empty()).then_some(PassportOffice { stamps })
    }

    /// Fill in whichever half of the passport is still blank. A player
    /// already carrying both is left alone, which is what makes the
    /// re-run cheap.
    fn stamp(&self, player: &mut Player) {
        if player.nationality_continent_id.is_some() && player.nationality_region.is_some() {
            return;
        }
        let Some((continent_id, region)) = self.stamps.get(&player.country_id) else {
            return;
        };
        if player.nationality_continent_id.is_none() {
            player.nationality_continent_id = Some(*continent_id);
        }
        if player.nationality_region.is_none() {
            player.nationality_region = Some(*region);
        }
    }

    fn stamp_squad(&self, players: &mut PlayerCollection) {
        for player in &mut players.players {
            self.stamp(player);
        }
    }
}

impl SimulatorData {
    /// Days on which the world re-stamps passports.
    ///
    /// Everything created after load — an academy intake, a regen, a
    /// synthetic international — starts with `nationality_continent_id`
    /// `None` and `nationality_region` `None`, and an unstamped passport
    /// reads as "no home" everywhere the loan-home pathway looks. The two
    /// transfer-window opens are when that matters, and the pass skips
    /// anyone already stamped, so it costs a walk and nothing else.
    pub fn is_nationality_reseed_day(date: NaiveDate) -> bool {
        date.day() == 1 && matches!(date.month(), 1 | 7)
    }

    /// Populate `Player.nationality_continent_id` and
    /// `Player.nationality_region` from `country_info` for every player on
    /// every roster + retired + national-team + free-agent pool. Called at
    /// construction and again on each reseed day. Cheap parallel pass.
    pub fn seed_player_nationality_continents(&mut self) {
        let Some(office) = PassportOffice::open(self) else {
            return;
        };
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                for club in &mut country.clubs {
                    for team in club.teams.iter_mut() {
                        office.stamp_squad(&mut team.players);
                    }
                }
                for player in country
                    .retired_players
                    .iter_mut()
                    .chain(country.national_team.generated_squad.iter_mut())
                    .chain(country.u21_national_team.generated_squad.iter_mut())
                {
                    office.stamp(player);
                }
            });
        for player in &mut self.free_agents {
            office.stamp(player);
        }
    }
}
