use crate::club::staff::perception::CoachProfile;
use crate::club::{ClubPhilosophy, Staff};
use crate::{MatchTacticType, Player};

use super::{SelectionCompetition, SelectionContext};

/// Football-shape view of the upcoming fixture, built once per selection
/// pass and read by the slot / XI / bench scorers. Keeps the per-player
/// scoring code free of fixture-context plumbing — each component reads a
/// single bounded number off the model rather than re-deriving opponent
/// threat or competition rules at every call site.
#[derive(Debug, Clone)]
pub struct MatchSelectionGameModel {
    pub match_type: MatchTypeSignal,
    pub tactical_objective: TacticalObjective,
    pub opponent_profile: OpponentSelectionProfile,
    pub environmental_profile: EnvironmentSelectionProfile,
    pub competition_rules: CompetitionSelectionRules,
    pub squad_state: SquadStateProfile,
    pub coach_policy: CoachSelectionPolicy,
}

/// Coarse fixture classification — the manager's mental category for what
/// kind of match this actually is. Drives importance interpretation in
/// places where a 0..1 scalar is too thin (final vs derby vs cup final
/// against a minnow). Built from `SelectionContext` + competition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchTypeSignal {
    LeagueRoutine,
    TitleRace,
    RelegationSixPointer,
    Derby,
    CupEarlyRound,
    CupKnockout,
    CupFinal,
    ContinentalGroup,
    ContinentalKnockout,
    Friendly,
    PostInternationalBreak,
}

/// What the coach is actually trying to do this match. The XI-balance
/// scorer reads this directly — security-heavy bands for ProtectLead,
/// creation-heavy for ChaseGame, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalObjective {
    WinNowBalanced,
    ProtectLead,
    UnderdogAway,
    ChaseGame,
    FavoriteHome,
    DevelopmentFixture,
}

/// Per-opponent threat snapshot. Every field clamped to a documented band
/// before the model lands here — readers can take values at face value.
#[derive(Debug, Clone)]
pub struct OpponentSelectionProfile {
    pub expected_tactic: Option<MatchTacticType>,
    /// Own / opponent strength, clamped 0.25..4.0. 1.0 = even.
    pub strength_ratio: f32,
    /// 0..1 threat axes. Larger = bigger concern for the selecting side.
    pub pace_threat: f32,
    pub aerial_threat: f32,
    pub pressing_intensity: f32,
    pub low_block_likelihood: f32,
    pub set_piece_threat: f32,
    pub wide_threat_left: f32,
    pub wide_threat_right: f32,
    pub central_overload: f32,
}

impl OpponentSelectionProfile {
    pub fn neutral() -> Self {
        OpponentSelectionProfile {
            expected_tactic: None,
            strength_ratio: 1.0,
            pace_threat: 0.4,
            aerial_threat: 0.4,
            pressing_intensity: 0.4,
            low_block_likelihood: 0.3,
            set_piece_threat: 0.4,
            wide_threat_left: 0.4,
            wide_threat_right: 0.4,
            central_overload: 0.3,
        }
    }
}

/// Pitch / weather / travel context. Conservative defaults until the
/// fixture pipeline carries the real values.
#[derive(Debug, Clone, Copy)]
pub struct EnvironmentSelectionProfile {
    pub is_home: bool,
    pub artificial_surface: bool,
    /// 0..1 wet/heavy pitch signal.
    pub adverse_weather: f32,
    /// 0..1 travel-fatigue signal. 0 = home, 1 = long-haul continental.
    pub travel_fatigue: f32,
}

impl EnvironmentSelectionProfile {
    pub fn neutral_home() -> Self {
        EnvironmentSelectionProfile {
            is_home: true,
            artificial_surface: false,
            adverse_weather: 0.0,
            travel_fatigue: 0.0,
        }
    }
}

/// Competition-imposed roster rules. Populated by the caller from the
/// real registration tables when available; defaults keep every player
/// eligible so legacy / test callers behave as before.
#[derive(Debug, Clone)]
pub struct CompetitionSelectionRules {
    /// When `Some`, only listed player ids are registered for the comp.
    pub registered_player_ids: Option<Vec<u32>>,
    /// Player ids cup-tied to a previous club this season.
    pub cup_tied_player_ids: Vec<u32>,
    /// Player ids the loan contract forbids fielding (parent club, etc).
    pub clause_blocked_player_ids: Vec<u32>,
    /// Player ids suspended *for this competition* (different bans
    /// per competition — a league suspension does not bar cup minutes).
    pub competition_suspended_player_ids: Vec<u32>,
    /// Player ids exempt from the registered list cap (U21, homegrown).
    pub registration_exempt_player_ids: Vec<u32>,
}

impl CompetitionSelectionRules {
    pub fn open() -> Self {
        CompetitionSelectionRules {
            registered_player_ids: None,
            cup_tied_player_ids: Vec::new(),
            clause_blocked_player_ids: Vec::new(),
            competition_suspended_player_ids: Vec::new(),
            registration_exempt_player_ids: Vec::new(),
        }
    }
}

/// Snapshot of the selecting side's own state — depth, congestion, and
/// the medical infrastructure that conditions fitness-call boldness.
#[derive(Debug, Clone, Copy)]
pub struct SquadStateProfile {
    /// 0..1 depth: 0 = paper-thin, 1 = full senior squad available.
    pub depth: f32,
    /// 0..1 congestion: 0 = nothing pending, 1 = three games in a week.
    pub fixture_congestion: f32,
    /// 0..1 medical staff quality (drives recurrence-risk tolerance).
    pub medical_quality: f32,
}

impl SquadStateProfile {
    pub fn from_signals(available_len: usize, staff: &Staff) -> Self {
        let depth = ((available_len as f32 - 18.0) / 12.0).clamp(0.0, 1.0);
        let med = &staff.staff_attributes.medical;
        let medical_quality = ((med.physiotherapy as f32 + med.sports_science as f32) / 40.0)
            .clamp(0.0, 1.0);
        SquadStateProfile {
            depth,
            fixture_congestion: 0.0,
            medical_quality,
        }
    }
}

/// Coach selection personality, derived from staff attributes + profile.
/// Used by the per-component scalers (rotation discipline, big-match
/// conservatism, …) so a methodical manager and a swashbuckler don't
/// pick the same XI off the same scoring base.
#[derive(Debug, Clone, Copy)]
pub struct CoachSelectionPolicy {
    pub rotation_discipline: f32,
    pub star_favoritism: f32,
    pub academy_trust: f32,
    pub tactical_flexibility: f32,
    pub big_match_conservatism: f32,
    pub medical_caution: f32,
    pub form_reactivity: f32,
    pub relationship_bias: f32,
}

impl CoachSelectionPolicy {
    pub fn from_profile(profile: &CoachProfile, philosophy: Option<&ClubPhilosophy>) -> Self {
        let academy_trust = (profile.youth_preference
            * 0.6
            + profile.potential_accuracy * 0.4
            + match philosophy {
                Some(ClubPhilosophy::DevelopAndSell) => 0.3,
                Some(ClubPhilosophy::SignToCompete) => -0.2,
                _ => 0.0,
            })
            .clamp(0.0, 1.0);

        CoachSelectionPolicy {
            rotation_discipline: (profile.conservatism * 0.5 + profile.judging_accuracy * 0.3
                + 0.2)
                .clamp(0.0, 1.0),
            star_favoritism: (1.0 - profile.judging_accuracy * 0.6).clamp(0.0, 1.0),
            academy_trust,
            tactical_flexibility: (1.0 - profile.tactical_blindness * 0.7).clamp(0.0, 1.0),
            big_match_conservatism: profile.conservatism.clamp(0.0, 1.0),
            medical_caution: (profile.conservatism * 0.5 + (1.0 - profile.risk_tolerance) * 0.5)
                .clamp(0.0, 1.0),
            form_reactivity: profile.recency_bias.clamp(0.0, 1.0),
            relationship_bias: (profile.attitude_weight * 0.4 + profile.man_management * 0.6)
                .clamp(0.0, 1.0),
        }
    }

    pub fn neutral() -> Self {
        CoachSelectionPolicy {
            rotation_discipline: 0.5,
            star_favoritism: 0.5,
            academy_trust: 0.5,
            tactical_flexibility: 0.5,
            big_match_conservatism: 0.5,
            medical_caution: 0.5,
            form_reactivity: 0.5,
            relationship_bias: 0.5,
        }
    }
}

impl MatchSelectionGameModel {
    /// Build the game model from a selection context and the resolved squad.
    /// Falls back to neutral / open / home defaults wherever the context
    /// doesn't carry the richer signal yet — callers that want richer per-
    /// fixture data fill in the relevant block on the resulting model after
    /// construction.
    pub fn build(
        ctx: &SelectionContext,
        staff: &Staff,
        available_len: usize,
    ) -> Self {
        let profile = CoachProfile::from_staff(staff);
        MatchSelectionGameModel {
            match_type: MatchTypeClassifier::classify(ctx),
            tactical_objective: TacticalObjectiveResolver::resolve(ctx),
            opponent_profile: OpponentSelectionProfile::neutral(),
            environmental_profile: EnvironmentSelectionProfile::neutral_home(),
            competition_rules: CompetitionSelectionRules::open(),
            squad_state: SquadStateProfile::from_signals(available_len, staff),
            coach_policy: CoachSelectionPolicy::from_profile(&profile, ctx.philosophy.as_ref()),
        }
    }
}

impl Default for MatchSelectionGameModel {
    fn default() -> Self {
        MatchSelectionGameModel {
            match_type: MatchTypeSignal::LeagueRoutine,
            tactical_objective: TacticalObjective::WinNowBalanced,
            opponent_profile: OpponentSelectionProfile::neutral(),
            environmental_profile: EnvironmentSelectionProfile::neutral_home(),
            competition_rules: CompetitionSelectionRules::open(),
            squad_state: SquadStateProfile {
                depth: 0.5,
                fixture_congestion: 0.0,
                medical_quality: 0.5,
            },
            coach_policy: CoachSelectionPolicy::neutral(),
        }
    }
}

/// Stateless namespace deriving the [`MatchTypeSignal`] from a
/// [`SelectionContext`]. Pure mapping, kept on its own type so tests can
/// drive the classification without building a full game model.
pub struct MatchTypeClassifier;

impl MatchTypeClassifier {
    pub fn classify(ctx: &SelectionContext) -> MatchTypeSignal {
        if ctx.is_friendly {
            return MatchTypeSignal::Friendly;
        }
        match ctx.competition {
            SelectionCompetition::ContinentalCup => {
                if ctx.match_importance >= 0.82 {
                    MatchTypeSignal::ContinentalKnockout
                } else {
                    MatchTypeSignal::ContinentalGroup
                }
            }
            SelectionCompetition::DomesticCup {
                round,
                total_rounds,
                ..
            } => {
                if total_rounds <= 1 || round >= total_rounds {
                    MatchTypeSignal::CupFinal
                } else if round + 1 == total_rounds || round + 2 == total_rounds {
                    MatchTypeSignal::CupKnockout
                } else {
                    MatchTypeSignal::CupEarlyRound
                }
            }
            SelectionCompetition::Friendly => MatchTypeSignal::Friendly,
            SelectionCompetition::League => {
                if ctx.match_importance >= 0.9 {
                    MatchTypeSignal::TitleRace
                } else {
                    MatchTypeSignal::LeagueRoutine
                }
            }
        }
    }
}

/// Stateless namespace deriving a default [`TacticalObjective`] from the
/// fixture context. The selection caller can override this on the
/// resulting `MatchSelectionGameModel` when richer signals (current
/// score in a two-legged tie, league standing, …) are available.
pub struct TacticalObjectiveResolver;

impl TacticalObjectiveResolver {
    pub fn resolve(ctx: &SelectionContext) -> TacticalObjective {
        if ctx.is_friendly {
            return TacticalObjective::DevelopmentFixture;
        }
        if let SelectionCompetition::DomesticCup {
            own_reputation,
            opponent_reputation,
            ..
        } = ctx.competition
        {
            let ratio = own_reputation.max(1) as f32 / opponent_reputation.max(1) as f32;
            return if ratio >= 1.4 {
                TacticalObjective::FavoriteHome
            } else if ratio <= 0.7 {
                TacticalObjective::UnderdogAway
            } else {
                TacticalObjective::WinNowBalanced
            };
        }
        if ctx.match_importance >= 0.75 {
            TacticalObjective::WinNowBalanced
        } else if ctx.match_importance <= 0.35 {
            TacticalObjective::DevelopmentFixture
        } else {
            TacticalObjective::WinNowBalanced
        }
    }
}

/// Per-player eligibility decision relative to the competition rules.
/// The selector reads this before scoring — hard blocks drop the player
/// from the available pool entirely; soft limits become a penalty term
/// in the slot score so a returning player can still be chosen in an
/// emergency.
#[derive(Debug, Clone, Copy)]
pub enum EligibilityDecision {
    Eligible,
    SoftLimited {
        reason: EligibilityReason,
        penalty: f32,
    },
    HardBlocked {
        reason: EligibilityReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EligibilityReason {
    Injured,
    SuspendedInCompetition,
    NotRegistered,
    CupTied,
    LoanClause,
    InternationalDuty,
    ReturningFromInjury,
    BelowPreferredCondition,
    ParentClubDiscouragement,
    YouthMinutesCap,
    DisciplinaryWarning,
}

/// Stateless evaluator producing an [`EligibilityDecision`] per player
/// from the competition rules and the player's current state. Kept as
/// a struct namespace so tests can drive it without instantiating the
/// full selection pipeline.
pub struct EligibilityEvaluator;

impl EligibilityEvaluator {
    pub fn evaluate(player: &Player, rules: &CompetitionSelectionRules) -> EligibilityDecision {
        if player.player_attributes.is_injured {
            return EligibilityDecision::HardBlocked {
                reason: EligibilityReason::Injured,
            };
        }
        if rules
            .competition_suspended_player_ids
            .iter()
            .any(|id| *id == player.id)
        {
            return EligibilityDecision::HardBlocked {
                reason: EligibilityReason::SuspendedInCompetition,
            };
        }
        if rules.cup_tied_player_ids.iter().any(|id| *id == player.id) {
            return EligibilityDecision::HardBlocked {
                reason: EligibilityReason::CupTied,
            };
        }
        if rules
            .clause_blocked_player_ids
            .iter()
            .any(|id| *id == player.id)
        {
            return EligibilityDecision::HardBlocked {
                reason: EligibilityReason::LoanClause,
            };
        }
        if player.statuses.is_on_international_duty() {
            return EligibilityDecision::HardBlocked {
                reason: EligibilityReason::InternationalDuty,
            };
        }
        if let Some(registered) = rules.registered_player_ids.as_ref() {
            let id = player.id;
            let is_registered = registered.iter().any(|rid| *rid == id);
            let is_exempt = rules
                .registration_exempt_player_ids
                .iter()
                .any(|rid| *rid == id);
            if !is_registered && !is_exempt {
                return EligibilityDecision::HardBlocked {
                    reason: EligibilityReason::NotRegistered,
                };
            }
        }
        if player.player_attributes.is_in_recovery() {
            return EligibilityDecision::SoftLimited {
                reason: EligibilityReason::ReturningFromInjury,
                penalty: 2.0,
            };
        }
        EligibilityDecision::Eligible
    }
}
