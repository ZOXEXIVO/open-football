//! Free types and constants used by every other submodule of
//! `country::national`. Kept together so adding a new call-up reason or
//! tweaking a window only touches one file.

use crate::{Player, PlayerFieldPositionGroup, PlayerPositionType, TeamType};
use chrono::NaiveDate;

/// Which national-team level a squad / competition / call-up belongs to.
/// Senior is the established full international side; Under21 is the
/// parallel youth side selected from a separate (younger) candidate pool
/// with its own caps, schedule, and match-day statuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NationalTeamLevel {
    #[default]
    Senior,
    Under21,
}

impl NationalTeamLevel {
    /// i18n key for the level label shown in squad / competition UI.
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            NationalTeamLevel::Senior => "senior",
            NationalTeamLevel::Under21 => "u21",
        }
    }

    pub fn is_under21(&self) -> bool {
        matches!(self, NationalTeamLevel::Under21)
    }
}

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
    /// Already in the prior squad or has enough caps to keep the spot.
    Incumbent,
    /// Selected (or kept) because a specific tactical role was uncovered.
    RoleCoverage,
    /// Friendly window: experimental call-up for an uncapped young player.
    FriendlyExperiment,
    /// Tournament: brought along primarily for big-stage experience.
    TournamentExperience,
    /// U21 squad: standard developmental pick — a young player given
    /// minutes to grow into the senior setup.
    U21DevelopmentPick,
    /// U21 squad: stand-out prospect with an elite potential ceiling.
    U21EliteProspect,
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
            CallUpReason::Incumbent => "callup_reason_incumbent",
            CallUpReason::RoleCoverage => "callup_reason_role_coverage",
            CallUpReason::FriendlyExperiment => "callup_reason_friendly_experiment",
            CallUpReason::TournamentExperience => "callup_reason_tournament_experience",
            CallUpReason::U21DevelopmentPick => "callup_reason_u21_development_pick",
            CallUpReason::U21EliteProspect => "callup_reason_u21_elite_prospect",
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

/// Default squad call-up size for competitive/friendly windows
pub(super) const SQUAD_SIZE: usize = 23;

/// Tournament finals squad size (FIFA/UEFA expanded list)
pub(super) const TOURNAMENT_SQUAD_SIZE: usize = 26;

/// The kind of international call-up cycle that's about to be selected
/// for. Drives age curves, continuity weight, squad size, and
/// experimentation tolerance — managers don't pick the same 23 for a
/// World Cup as they do for a March friendly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallUpWindowType {
    /// June/July major-tournament window (World Cup / Euros / Copa).
    TournamentFinals,
    /// Regular international break with competitive matches scheduled.
    CompetitiveWindow,
    /// Pure friendly window — room to experiment with youth and depth.
    FriendlyWindow,
}

/// Inputs gathered once per call-up cycle. Replaces the old
/// `is_tournament: bool` flag with enough structure to differentiate
/// "March friendly" from "qualifying double-header" from "World Cup".
pub struct CallUpContext {
    pub date: NaiveDate,
    pub country_id: u32,
    pub window_type: CallUpWindowType,
    pub target_squad_size: usize,
    /// Which national-team level this selection cycle is for. Drives the
    /// scoring blend (senior axes vs. the U21 youth/potential blend) and
    /// the reason-derivation path.
    pub level: NationalTeamLevel,
}

impl CallUpContext {
    pub(crate) fn new(date: NaiveDate, country_id: u32, window_type: CallUpWindowType) -> Self {
        let target_squad_size = match window_type {
            CallUpWindowType::TournamentFinals => TOURNAMENT_SQUAD_SIZE,
            CallUpWindowType::CompetitiveWindow => SQUAD_SIZE,
            CallUpWindowType::FriendlyWindow => SQUAD_SIZE,
        };
        Self {
            date,
            country_id,
            window_type,
            target_squad_size,
            level: NationalTeamLevel::Senior,
        }
    }

    /// Build a context for a specific level with an explicit squad size.
    /// Senior callers should prefer [`CallUpContext::new`] so the
    /// window-derived size (23 / 26) is preserved unchanged.
    pub(crate) fn new_with_level(
        date: NaiveDate,
        country_id: u32,
        window_type: CallUpWindowType,
        level: NationalTeamLevel,
        target_squad_size: usize,
    ) -> Self {
        Self {
            date,
            country_id,
            window_type,
            target_squad_size,
            level,
        }
    }
}

/// Declarative description of who is eligible for a national squad and
/// how big it should be. Replaces the hard-coded senior assumptions in
/// the candidate-collection / call-up pipeline so the same code paths
/// serve both Senior and U21 selection by swapping the policy.
#[derive(Debug, Clone)]
pub struct NationalSelectionPolicy {
    pub level: NationalTeamLevel,
    /// Inclusive maximum age (`date.year() - birth_year`). `None` = no cap.
    pub max_age: Option<i32>,
    /// Club team types whose players are scouted for this level. Senior
    /// scouts the main team only; U21 reaches into reserve / youth setups.
    pub include_team_types: Vec<TeamType>,
    /// Target call-up size for non-tournament windows.
    pub target_squad_size: usize,
    /// Minimum real club players before synthetic depth is generated.
    pub min_real_players: usize,
}

impl NationalSelectionPolicy {
    /// Senior policy — preserves the historical behaviour exactly:
    /// main team only, no age cap, window-derived squad size, the
    /// existing 16-player synthetic floor.
    pub fn senior() -> Self {
        NationalSelectionPolicy {
            level: NationalTeamLevel::Senior,
            max_age: None,
            include_team_types: vec![TeamType::Main],
            target_squad_size: SQUAD_SIZE,
            min_real_players: MIN_REAL_PLAYERS,
        }
    }

    /// U21 policy — younger candidate pool drawn from the whole club
    /// pyramid, a 21-and-under cap, a 23-man target, and a lower
    /// synthetic floor (youth pools are thinner than senior ones).
    pub fn under21() -> Self {
        NationalSelectionPolicy {
            level: NationalTeamLevel::Under21,
            max_age: Some(21),
            include_team_types: vec![
                TeamType::Main,
                TeamType::Reserve,
                TeamType::B,
                TeamType::Second,
                TeamType::U23,
                TeamType::U21,
                TeamType::U20,
                TeamType::U19,
                TeamType::U18,
            ],
            target_squad_size: 23,
            min_real_players: 14,
        }
    }

    /// Minimum match-fitness condition for eligibility. U21 prospects are
    /// allowed in slightly less match-sharp (4500 vs the senior 5000)
    /// because youth players naturally accumulate fewer minutes.
    pub fn min_condition(&self) -> i16 {
        match self.level {
            NationalTeamLevel::Senior => 5000,
            NationalTeamLevel::Under21 => 4500,
        }
    }

    /// Synthetic-player age range `(min, max_exclusive)` for filling a
    /// thin pool. `IntegerUtils::random` is exclusive of `max`, so senior
    /// yields ages 22-33 (unchanged) and U21 yields 17-21.
    pub fn synthetic_age_range(&self) -> (i32, i32) {
        match self.level {
            NationalTeamLevel::Senior => (22, 34),
            NationalTeamLevel::Under21 => (17, 22),
        }
    }
}

/// Stylistic bias attached deterministically to each country. Folds the
/// old anonymous `country_id % 4` switch into a small enum so that the
/// scoring code is readable and individual coach traits are testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NationalCoachProfile {
    /// Trusts experienced internationals; cautious with uncapped picks.
    Conservative,
    /// Pushes high-potential youngsters into the squad early.
    YouthDeveloper,
    /// Prizes world reputation — picks famous names first.
    StarDriven,
    /// Rewards in-season form heavily.
    FormDriven,
    /// Picks for tactical positional fit above raw reputation.
    TacticalSpecialist,
}

impl NationalCoachProfile {
    /// Deterministic mapping from country to coach archetype.
    pub fn for_country(country_id: u32) -> Self {
        match country_id % 5 {
            0 => NationalCoachProfile::Conservative,
            1 => NationalCoachProfile::YouthDeveloper,
            2 => NationalCoachProfile::StarDriven,
            3 => NationalCoachProfile::FormDriven,
            _ => NationalCoachProfile::TacticalSpecialist,
        }
    }
}

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
#[derive(Clone)]
pub(crate) struct CallUpCandidate {
    pub(crate) player_id: u32,
    pub(super) club_id: u32,
    pub(super) team_id: u32,
    pub(super) current_ability: u8,
    /// Selectors' believed ceiling — an observable estimate populated
    /// at the single boundary in `callup.rs` from
    /// `PotentialEstimator::observable_ceiling`. Never the hidden
    /// biological `player_attributes.potential_ability`.
    pub(super) assessed_potential: u8,
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
