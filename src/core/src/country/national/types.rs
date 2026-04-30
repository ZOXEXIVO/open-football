//! Free types and constants used by every other submodule of
//! `country::national`. Kept together so adding a new call-up reason or
//! tweaking a window only touches one file.

use crate::{Player, PlayerFieldPositionGroup, PlayerPositionType};
use chrono::NaiveDate;

#[derive(Clone)]
pub struct NationalTeamStaffMember {
    pub first_name: String,
    pub last_name: String,
    pub role: NationalTeamStaffRole,
    pub country_id: u32,
    pub birth_year: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NationalTeamStaffRole {
    Manager,
    AssistantManager,
    Coach,
    GoalkeeperCoach,
    FitnessCoach,
}

impl NationalTeamStaffRole {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            NationalTeamStaffRole::Manager => "staff_manager",
            NationalTeamStaffRole::AssistantManager => "staff_assistant_manager",
            NationalTeamStaffRole::Coach => "staff_coach",
            NationalTeamStaffRole::GoalkeeperCoach => "staff_goalkeeper_coach",
            NationalTeamStaffRole::FitnessCoach => "staff_fitness_coach",
        }
    }
}

#[derive(Clone)]
pub struct NationalSquadPlayer {
    pub player_id: u32,
    pub club_id: u32,
    pub team_id: u32,
    pub primary_reason: CallUpReason,
    pub secondary_reasons: Vec<CallUpReason>,
}

/// Unified view over a national-team squad pick — covers both real
/// players (looked up from a club roster) and synthetic players
/// (generated to fill a thin pool, owned by `generated_squad`). UI
/// code should iterate `NationalTeam::squad_picks()` rather than
/// reaching into `squad` and `generated_squad` separately, so synthetic
/// depth players are visible everywhere a real player would be.
pub enum SquadPick<'a> {
    Real(&'a NationalSquadPlayer),
    Synthetic(&'a Player),
}

/// Why a player was selected for the national squad.
/// Surfaces in the squad UI and in debug logs so call-ups are auditable
/// instead of looking arbitrary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallUpReason {
    /// High ability and world reputation — the manager always picks them.
    KeyPlayer,
    /// Strong recent match ratings — riding a hot form streak.
    CurrentForm,
    /// Plays week-in week-out for a real club; reliable minutes.
    RegularStarter,
    /// Competes in a top-tier league — playing level signal.
    StrongLeague,
    /// Best tactical fit for a position the manager's tactic demands.
    TacticalFit,
    /// Selected primarily to fill a positional shortage.
    PositionNeed,
    /// Veteran with many caps — proven on the international stage.
    InternationalExperience,
    /// Captain material — leadership/composure carry weight in this squad.
    Leadership,
    /// Young player with high potential, blooded for the future.
    YouthProspect,
    /// Synthetic player generated to fill a thin pool (weak nation).
    SyntheticDepth,
}

impl CallUpReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CallUpReason::KeyPlayer => "callup_reason_key_player",
            CallUpReason::CurrentForm => "callup_reason_current_form",
            CallUpReason::RegularStarter => "callup_reason_regular_starter",
            CallUpReason::StrongLeague => "callup_reason_strong_league",
            CallUpReason::TacticalFit => "callup_reason_tactical_fit",
            CallUpReason::PositionNeed => "callup_reason_position_need",
            CallUpReason::InternationalExperience => "callup_reason_international_experience",
            CallUpReason::Leadership => "callup_reason_leadership",
            CallUpReason::YouthProspect => "callup_reason_youth_prospect",
            CallUpReason::SyntheticDepth => "callup_reason_synthetic_depth",
        }
    }
}

#[derive(Clone)]
pub struct NationalTeamFixture {
    pub date: NaiveDate,
    pub opponent_country_id: u32,
    pub opponent_country_name: String,
    pub is_home: bool,
    pub competition_name: String,
    pub match_id: String,
    pub result: Option<NationalTeamMatchResult>,
}

#[derive(Clone)]
pub struct NationalTeamMatchResult {
    pub home_score: u8,
    pub away_score: u8,
    pub date: NaiveDate,
    pub opponent_country_id: u32,
}

/// Break windows matching League::is_international_break:
/// Sep 4-12, Oct 9-17, Nov 13-21, Mar 20-28
pub(super) const BREAK_WINDOWS: [(u32, u32, u32); 4] =
    [(9, 4, 12), (10, 9, 17), (11, 13, 21), (3, 20, 28)];

/// Tournament window: June-July for World Cup / Euro finals
pub(super) const TOURNAMENT_WINDOW: (u32, u32, u32, u32) = (6, 10, 7, 15);

pub(super) const DEFAULT_STAFF_ROLES: [NationalTeamStaffRole; 5] = [
    NationalTeamStaffRole::Manager,
    NationalTeamStaffRole::AssistantManager,
    NationalTeamStaffRole::Coach,
    NationalTeamStaffRole::GoalkeeperCoach,
    NationalTeamStaffRole::FitnessCoach,
];

/// Minimum number of real club players before generating synthetic ones
pub(super) const MIN_REAL_PLAYERS: usize = 16;

/// Default squad call-up size
pub(super) const SQUAD_SIZE: usize = 23;

/// Positions template for generating a balanced synthetic squad
pub(super) const SYNTHETIC_POSITIONS: [PlayerPositionType; 23] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::DefenderRight,
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::MidfielderLeft,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::MidfielderCenterRight,
    PlayerPositionType::MidfielderRight,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::AttackingMidfielderCenter,
    PlayerPositionType::ForwardLeft,
    PlayerPositionType::ForwardCenter,
    PlayerPositionType::ForwardRight,
    PlayerPositionType::Striker,
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::ForwardCenter,
    PlayerPositionType::Striker,
];

/// Data collected from a candidate player for call-up scoring.
/// Captures enough information that the selection result is explainable
/// — every field here can be cited as a reason ("regular starter",
/// "strong league", "veteran caps", …) without needing to re-read the
/// underlying Player struct.
pub(crate) struct CallUpCandidate {
    pub(super) player_id: u32,
    pub(super) club_id: u32,
    pub(super) team_id: u32,
    pub(super) current_ability: u8,
    pub(super) potential_ability: u8,
    pub(super) age: i32,
    pub(super) condition_pct: f32,
    pub(super) match_readiness: f32,
    pub(super) average_rating: f32,
    pub(super) played: u16,
    pub(super) international_apps: u16,
    pub(super) international_goals: u16,
    pub(super) leadership: f32,
    pub(super) composure: f32,
    pub(super) teamwork: f32,
    pub(super) determination: f32,
    pub(super) pressure_handling: f32,
    pub(super) world_reputation: i16,
    /// Club reputation where the player plays — was previously misnamed
    /// "league_reputation" while actually holding team.reputation.world.
    pub(super) club_reputation: u16,
    /// True league reputation (0-1000) looked up via team.league_id —
    /// represents the strength of the division, not the individual club.
    pub(super) league_reputation: u16,
    pub(super) position_levels: Vec<(PlayerPositionType, u8)>,
    pub(super) position_group: PlayerFieldPositionGroup,
    /// Current-season stats
    pub(super) goals: u16,
    pub(super) assists: u16,
    pub(super) player_of_the_match: u8,
    pub(super) clean_sheets: u16,
    pub(super) yellow_cards: u8,
    pub(super) red_cards: u8,
    /// Total apps (league + cup) in the most recent prior season —
    /// keeps early-season call-ups grounded in last year's body of work,
    /// not a 0–4 game sample from the new season.
    pub(super) last_season_apps: u16,
    /// Weighted average rating across all entries from the most recent
    /// prior season. 0.0 means no prior history.
    pub(super) last_season_rating: f32,
    /// Goals scored in the most recent prior season.
    pub(super) last_season_goals: u16,
}
