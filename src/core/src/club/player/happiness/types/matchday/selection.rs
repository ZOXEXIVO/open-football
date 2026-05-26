/// Why a player ended up off the team-sheet for a match. Passed in by
/// the squad selector at emit time so the player-events renderer can
/// describe the decision in football-realistic terms ("rested after
/// heavy minutes", "lost out to a fitter teammate", "no natural role
/// in the current shape") instead of a generic drop line.
///
/// Closed enum, mirrored by the renderer's i18n token list. Adding a
/// new variant means adding a copy line in every locale and a renderer
/// branch — fail loud at compile time rather than show the raw key.
#[derive(Debug, Clone)]
pub struct MatchSelectionContext {
    /// Where in the matchday selection ladder the player ended up —
    /// dropped from XI to bench, left off the matchday squad entirely,
    /// or named to the bench but never came on.
    pub scope: SelectionDecisionScope,
    /// Football-realistic reason the manager picked the chosen player
    /// over this one.
    pub reason: SelectionOmissionReason,
    /// Concrete comparison to the player who took the slot. `None`
    /// when no direct counterpart exists (e.g. left out of squad with
    /// no positional rival).
    pub comparison: Option<SelectionComparison>,
    /// Player's expected role given his squad status / promises. Drives
    /// severity and copy variants — a `KeyPlayer` left out reads
    /// differently from a fringe `MainBackupPlayer`.
    pub role: SelectionRole,
    /// Match importance the selection was made under (0.0–1.0). Lets
    /// the renderer tag low-importance cup nights as "rotation" and
    /// soften the impact line.
    pub match_importance: f32,
    /// True when the omission has happened in consecutive matches.
    /// Drives the "if repeated" outlook and the severity bump.
    pub repeated: bool,
    /// True for friendlies / development matches. Renderer dampens the
    /// outlook ("a friendly snub is rarely held against the manager").
    pub is_friendly: bool,
}

/// Bucket the selection decision falls into. Distinct from
/// `SelectionOmissionReason` (the *why*) — this is the *what*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionDecisionScope {
    /// Named to the bench but never came on.
    UnusedSubstitute,
    /// Dropped from a starting role to the bench — did not start.
    DroppedToBench,
    /// Left out of the matchday squad entirely.
    LeftOutOfMatchdaySquad,
    /// Explicitly rested by the manager (load-management call).
    Rested,
    /// Available, but not picked for non-injury reasons (discipline,
    /// personal). Distinct from full unavailability (suspension /
    /// injury) which is filtered before selection.
    UnavailableButNotInjured,
    /// Cup / low-importance fixture rotation.
    Rotation,
}

impl SelectionDecisionScope {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SelectionDecisionScope::UnusedSubstitute => "selection_scope_unused_substitute",
            SelectionDecisionScope::DroppedToBench => "selection_scope_dropped_to_bench",
            SelectionDecisionScope::LeftOutOfMatchdaySquad => {
                "selection_scope_left_out_of_matchday_squad"
            }
            SelectionDecisionScope::Rested => "selection_scope_rested",
            SelectionDecisionScope::UnavailableButNotInjured => {
                "selection_scope_unavailable_not_injured"
            }
            SelectionDecisionScope::Rotation => "selection_scope_rotation",
        }
    }
}

/// Football-realistic reason the manager picked someone else. Closed
/// enum, every variant maps to a localised sentence the renderer turns
/// into the "why" line. Multiple reasons can apply at once — the
/// selector picks the dominant one (highest weight in the scoring
/// breakdown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOmissionReason {
    /// Chosen player was sharper / fresher.
    LowerMatchReadiness,
    /// Manager protected a fragile player (returning, accumulating risk).
    FitnessProtection,
    /// Recent workload drove rotation.
    FatigueManagement,
    /// Bad recent ratings cost the player his place.
    PoorRecentForm,
    /// Tactical shape demands a different profile.
    TacticalMismatch,
    /// Player's positions don't fit any open slot well.
    PositionFitIssue,
    /// Direct rival was preferred on perceived ability.
    TeammatePreferredOnAbility,
    /// Rival was preferred because his form was stronger.
    TeammatePreferredOnForm,
    /// Rival was preferred on physical readiness.
    TeammatePreferredOnFitness,
    /// Coach trusts the rival more (relationship / professional respect).
    TeammatePreferredOnTrust,
    /// Manager preferred the rival to balance the shape (eg. defensive
    /// reliability against a tough opponent).
    TeammatePreferredForTacticalBalance,
    /// Manager promoted a youth player as part of development plan.
    YouthDevelopmentRotation,
    /// Cup / League Cup rotation call.
    CupRotation,
    /// Low-importance match — manager rotated for managed minutes.
    LowMatchImportanceRotation,
    /// Player's squad status doesn't match the moment (e.g. fringe
    /// player overlooked when the manager could afford his best XI).
    SquadStatusMismatch,
    /// Coach has limited trust in the player despite squad-status label.
    ManagerDoesNotTrustPlayer,
    /// New signing still inside the integration window.
    NewcomerStillIntegrating,
    /// Returning from injury — protected start.
    ReturningFromInjury,
    /// Disciplinary call (training-ground row, public apology pending).
    DisciplinarySelection,
    /// Bench-balance call: the manager wanted a different option for
    /// in-match flexibility.
    BenchBalance,
    /// Formation has no slot anywhere near the player's preferred role.
    NoNaturalRoleInFormation,
}

impl SelectionOmissionReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SelectionOmissionReason::LowerMatchReadiness => {
                "selection_reason_lower_match_readiness"
            }
            SelectionOmissionReason::FitnessProtection => "selection_reason_fitness_protection",
            SelectionOmissionReason::FatigueManagement => "selection_reason_fatigue_management",
            SelectionOmissionReason::PoorRecentForm => "selection_reason_poor_recent_form",
            SelectionOmissionReason::TacticalMismatch => "selection_reason_tactical_mismatch",
            SelectionOmissionReason::PositionFitIssue => "selection_reason_position_fit_issue",
            SelectionOmissionReason::TeammatePreferredOnAbility => {
                "selection_reason_teammate_preferred_on_ability"
            }
            SelectionOmissionReason::TeammatePreferredOnForm => {
                "selection_reason_teammate_preferred_on_form"
            }
            SelectionOmissionReason::TeammatePreferredOnFitness => {
                "selection_reason_teammate_preferred_on_fitness"
            }
            SelectionOmissionReason::TeammatePreferredOnTrust => {
                "selection_reason_teammate_preferred_on_trust"
            }
            SelectionOmissionReason::TeammatePreferredForTacticalBalance => {
                "selection_reason_teammate_preferred_for_tactical_balance"
            }
            SelectionOmissionReason::YouthDevelopmentRotation => {
                "selection_reason_youth_development_rotation"
            }
            SelectionOmissionReason::CupRotation => "selection_reason_cup_rotation",
            SelectionOmissionReason::LowMatchImportanceRotation => {
                "selection_reason_low_match_importance_rotation"
            }
            SelectionOmissionReason::SquadStatusMismatch => {
                "selection_reason_squad_status_mismatch"
            }
            SelectionOmissionReason::ManagerDoesNotTrustPlayer => {
                "selection_reason_manager_does_not_trust"
            }
            SelectionOmissionReason::NewcomerStillIntegrating => {
                "selection_reason_newcomer_still_integrating"
            }
            SelectionOmissionReason::ReturningFromInjury => {
                "selection_reason_returning_from_injury"
            }
            SelectionOmissionReason::DisciplinarySelection => "selection_reason_disciplinary",
            SelectionOmissionReason::BenchBalance => "selection_reason_bench_balance",
            SelectionOmissionReason::NoNaturalRoleInFormation => "selection_reason_no_natural_role",
        }
    }
}

/// Concrete comparison to the player who took the omitted player's
/// slot. Stores ids and scores for tests / debugging plus the
/// dominant scoring components so the renderer can produce a
/// "stronger condition / sharper form" sentence rather than guessing.
#[derive(Debug, Clone)]
pub struct SelectionComparison {
    /// Player id that was selected for the slot the omitted player
    /// would naturally have filled.
    pub selected_player_id: u32,
    /// Whether the selected player was a starter or substitute.
    pub selected_was_starter: bool,
    /// Position / slot the selected player took. `None` when the
    /// player's preferred role isn't in the formation at all.
    pub slot: Option<SelectionRole>,
    /// Selected player's total score for that slot.
    pub selected_score: f32,
    /// Omitted player's total score for the same slot.
    pub omitted_score: f32,
    /// Top scoring factors where the selected player edged ahead. Up
    /// to four factors, stored in dominance order so the renderer can
    /// pick the first one or two for the comparison sentence.
    pub top_factors: Vec<SelectionScoreFactor>,
}

/// Coarse positional bucket used in the comparison line. Mirrors the
/// engine's positional groupings — keeping it as a render-safe enum
/// avoids dragging the full `PlayerPositionType` into the events
/// module's i18n surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionRole {
    Goalkeeper,
    CentreBack,
    Fullback,
    DefensiveMidfielder,
    CentralMidfielder,
    AttackingMidfielder,
    Winger,
    Striker,
    /// Free-floating / unclassified — fallback for unusual slots.
    Other,
}

impl SelectionRole {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SelectionRole::Goalkeeper => "selection_role_goalkeeper",
            SelectionRole::CentreBack => "selection_role_centre_back",
            SelectionRole::Fullback => "selection_role_fullback",
            SelectionRole::DefensiveMidfielder => "selection_role_defensive_midfielder",
            SelectionRole::CentralMidfielder => "selection_role_central_midfielder",
            SelectionRole::AttackingMidfielder => "selection_role_attacking_midfielder",
            SelectionRole::Winger => "selection_role_winger",
            SelectionRole::Striker => "selection_role_striker",
            SelectionRole::Other => "selection_role_other",
        }
    }
}

/// Single-component breakdown atom from the scoring engine. The
/// selector picks the top few factors where the selected player beat
/// the omitted player and packs them into `SelectionComparison` so
/// the renderer doesn't have to expose raw f32 scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionScoreFactor {
    PositionFit,
    PerceivedQuality,
    MatchReadiness,
    Fatigue,
    TacticalFit,
    SideFootFit,
    Reputation,
    CoachRelationship,
    Newcomer,
    YouthPreference,
    TrainingImpression,
    Cohesion,
    SquadStatus,
    ForceSelection,
    ClubPhilosophy,
    InjuryRisk,
    DevelopmentMinutes,
}

impl SelectionScoreFactor {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SelectionScoreFactor::PositionFit => "selection_factor_position_fit",
            SelectionScoreFactor::PerceivedQuality => "selection_factor_perceived_quality",
            SelectionScoreFactor::MatchReadiness => "selection_factor_match_readiness",
            SelectionScoreFactor::Fatigue => "selection_factor_fatigue",
            SelectionScoreFactor::TacticalFit => "selection_factor_tactical_fit",
            SelectionScoreFactor::SideFootFit => "selection_factor_side_foot_fit",
            SelectionScoreFactor::Reputation => "selection_factor_reputation",
            SelectionScoreFactor::CoachRelationship => "selection_factor_coach_relationship",
            SelectionScoreFactor::Newcomer => "selection_factor_newcomer",
            SelectionScoreFactor::YouthPreference => "selection_factor_youth_preference",
            SelectionScoreFactor::TrainingImpression => "selection_factor_training_impression",
            SelectionScoreFactor::Cohesion => "selection_factor_cohesion",
            SelectionScoreFactor::SquadStatus => "selection_factor_squad_status",
            SelectionScoreFactor::ForceSelection => "selection_factor_force_selection",
            SelectionScoreFactor::ClubPhilosophy => "selection_factor_club_philosophy",
            SelectionScoreFactor::InjuryRisk => "selection_factor_injury_risk",
            SelectionScoreFactor::DevelopmentMinutes => "selection_factor_development_minutes",
        }
    }
}
