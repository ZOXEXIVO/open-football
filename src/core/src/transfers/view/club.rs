//! What the market can read off a club without deciding anything.

use crate::transfers::scouting::config::ScoutingConfig;
use crate::{Club, Country, ReputationLevel, StaffPosition, TeamType};

/// Read-only questions the market asks about a club.
///
/// The market layer reads the world constantly — squad size, capacity, who
/// is registered where — and every one of those reads used to live wherever
/// the first caller happened to need it. `can_accept_player` in particular
/// sat in `country::result::transfers::types`, which made `transfers` import
/// `country::result` and closed a module cycle: the country pass calls the
/// pipeline, and the pipeline reached back into the country pass for one
/// predicate.
///
/// Nothing here decides anything. A view answers a question about the world
/// as it stands; the thresholds and the gates live in
/// [`crate::transfers::gate`].
pub struct ClubView;

impl ClubView {
    /// Squad cap for a club whose board has set no season targets.
    const DEFAULT_MAX_SQUAD: usize = 50;

    /// Whether the club has room on its main-team roster for one more
    /// player.
    ///
    /// The cap is the board's own `max_squad_size` season target, defaulting
    /// to [`Self::DEFAULT_MAX_SQUAD`]. Only the MAIN team counts — youth and
    /// reserve rosters are separate registrations — and the main team is
    /// resolved by type rather than by `teams[0]`, because the collection
    /// does not guarantee the main side is first and counting a B squad
    /// against the first-team cap gates the wrong roster.
    ///
    /// Read by every door into a squad: the free-agent matcher, the
    /// emergency fill, the pre-contract stage, the negotiation opener and
    /// the transfer executor, so a club that is full stops fielding
    /// approaches rather than discovering the problem at execution.
    pub fn can_accept_player(club: &Club) -> bool {
        let max_squad = club
            .board
            .season_targets
            .as_ref()
            .map(|t| t.max_squad_size as usize)
            .unwrap_or(Self::DEFAULT_MAX_SQUAD);
        let main_squad = club.teams.main().map(|t| t.players.len()).unwrap_or(0);
        main_squad < max_squad
    }

    pub(in crate::transfers) fn club_world_reputation(club: &Club) -> i16 {
        club.teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main))
            .map(|t| t.reputation.world as i16)
            .unwrap_or(0)
    }

    /// Best `judging_player_data` across the club's scouting staff.
    /// Drives how aggressively the data department narrows the scout pool.
    /// Defaults from `ScoutingConfig::data_prefilter::default_data_skill`
    /// when the club has no scouts at all.
    pub(in crate::transfers) fn club_data_analysis_skill(club: &Club) -> u8 {
        let default_skill = ScoutingConfig::default().data_prefilter.default_data_skill;
        club.teams
            .iter()
            .flat_map(|t| t.staffs.iter())
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(
                        |c| matches!(c.position, StaffPosition::Scout | StaffPosition::ChiefScout,),
                    )
                    .unwrap_or(false)
            })
            .map(|s| s.staff_attributes.data_analysis.judging_player_data)
            .max()
            .unwrap_or(default_skill)
    }

    pub(in crate::transfers) fn get_scout_skills(club: &Club, scout_id: u32) -> (u8, u8) {
        for team in &club.teams.teams {
            if let Some(staff) = team.staffs.find(scout_id) {
                return (
                    staff.staff_attributes.knowledge.judging_player_ability,
                    staff.staff_attributes.knowledge.judging_player_potential,
                );
            }
        }
        // Pointer is stale (staff was removed mid-tick or assignment was
        // never tied to a real scout). Use the configured "missing staff"
        // defaults rather than panic — quality silently downgrades.
        let cfg = ScoutingConfig::default();
        (
            cfg.observation.default_judging_when_staff_missing,
            cfg.observation.default_judging_when_staff_missing,
        )
    }

    pub(in crate::transfers) fn get_club_reputation(country: &Country, club_id: u32) -> f32 {
        country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.reputation.attractiveness_factor())
            .unwrap_or(0.3)
    }

    pub(in crate::transfers) fn get_club_reputation_level(
        country: &Country,
        club_id: u32,
    ) -> ReputationLevel {
        country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur)
    }
}
