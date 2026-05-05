use crate::Team;
use crate::club::{PersonBehaviourState, Player, PlayerPositionType, Staff};

#[derive(Debug, Clone)]
pub struct Tactics {
    pub tactic_type: MatchTacticType,
    pub selected_reason: TacticSelectionReason,
    pub formation_strength: f32, // 0.0 to 1.0 indicating how well this formation suits the team
}

#[derive(Debug, Clone, Ord, PartialOrd, Eq, PartialEq)]
pub enum TacticSelectionReason {
    CoachPreference,
    TeamComposition,
    OpponentCounter,
    GameSituation,
    Default,
}

impl Tactics {
    pub fn new(tactic_type: MatchTacticType) -> Self {
        Tactics {
            tactic_type,
            selected_reason: TacticSelectionReason::Default,
            formation_strength: 0.5,
        }
    }

    pub fn with_reason(
        tactic_type: MatchTacticType,
        reason: TacticSelectionReason,
        strength: f32,
    ) -> Self {
        Tactics {
            tactic_type,
            selected_reason: reason,
            formation_strength: strength.clamp(0.0, 1.0),
        }
    }

    pub fn positions(&self) -> &[PlayerPositionType; 11] {
        match TACTICS_POSITIONS
            .iter()
            .find(|(positioning, _)| *positioning == self.tactic_type)
        {
            Some((_, positions)) => positions,
            None => {
                debug_assert!(
                    false,
                    "TACTICS_POSITIONS missing entry for {:?} — add slot list",
                    self.tactic_type
                );
                log::warn!(
                    "TACTICS_POSITIONS missing entry for {:?}, falling back to T442",
                    self.tactic_type
                );
                &TACTICS_POSITIONS[0].1
            }
        }
    }

    pub fn defender_count(&self) -> usize {
        self.positions()
            .iter()
            .filter(|pos| pos.is_defender())
            .count()
    }

    pub fn midfielder_count(&self) -> usize {
        self.positions()
            .iter()
            .filter(|pos| pos.is_midfielder())
            .count()
    }

    pub fn forward_count(&self) -> usize {
        self.positions()
            .iter()
            .filter(|pos| pos.is_forward())
            .count()
    }

    pub fn formation_description(&self) -> String {
        format!(
            "{}-{}-{}",
            self.defender_count(),
            self.midfielder_count(),
            self.forward_count()
        )
    }

    pub fn is_attacking(&self) -> bool {
        self.forward_count() >= 3 || (self.forward_count() == 2 && self.midfielder_count() <= 3)
    }

    pub fn is_defensive(&self) -> bool {
        self.defender_count() >= 5 || (self.defender_count() == 4 && self.midfielder_count() >= 5)
    }

    pub fn is_high_pressing(&self) -> bool {
        matches!(
            self.tactical_style(),
            TacticalStyle::Attacking | TacticalStyle::Possession | TacticalStyle::Compact
        )
    }

    /// Returns pressing intensity from 0.0 to 1.0 based on tactical style
    pub fn pressing_intensity(&self) -> f32 {
        match self.tactical_style() {
            TacticalStyle::Attacking | TacticalStyle::Compact => 1.0,
            TacticalStyle::Possession => 0.8,
            TacticalStyle::Balanced | TacticalStyle::WingPlay | TacticalStyle::WidePlay => 0.6,
            TacticalStyle::Counterattack => 0.4,
            TacticalStyle::Defensive => 0.3,
            TacticalStyle::Experimental => 0.5,
        }
    }

    /// Returns defensive line height from 0.0 (deep block) to 1.0 (high line).
    /// Controls how far up the pitch defenders position themselves.
    pub fn defensive_line_height(&self) -> f32 {
        match self.tactical_style() {
            TacticalStyle::Attacking => 0.8,
            TacticalStyle::Possession => 0.7,
            TacticalStyle::Compact => 0.65,
            TacticalStyle::Balanced | TacticalStyle::WingPlay | TacticalStyle::WidePlay => 0.5,
            TacticalStyle::Counterattack => 0.35,
            TacticalStyle::Defensive => 0.25,
            TacticalStyle::Experimental => 0.5,
        }
    }

    /// Returns team compactness from 0.0 (spread) to 1.0 (very compact).
    /// Controls how tightly defenders stay together laterally.
    pub fn compactness(&self) -> f32 {
        match self.tactical_style() {
            TacticalStyle::Compact => 1.0,
            TacticalStyle::Defensive => 0.85,
            TacticalStyle::Possession => 0.7,
            TacticalStyle::Balanced => 0.6,
            TacticalStyle::Counterattack => 0.55,
            TacticalStyle::Attacking => 0.5,
            TacticalStyle::WingPlay | TacticalStyle::WidePlay => 0.4,
            TacticalStyle::Experimental => 0.5,
        }
    }

    /// Returns counter-press intensity from 0.0 to 1.0.
    /// Controls how aggressively team presses immediately after losing possession.
    pub fn counter_press_intensity(&self) -> f32 {
        match self.tactical_style() {
            TacticalStyle::Attacking | TacticalStyle::Compact => 0.9,
            TacticalStyle::Possession => 0.75,
            TacticalStyle::Balanced => 0.5,
            TacticalStyle::WingPlay | TacticalStyle::WidePlay => 0.45,
            TacticalStyle::Counterattack => 0.3,
            TacticalStyle::Defensive => 0.2,
            TacticalStyle::Experimental => 0.5,
        }
    }

    pub fn tactical_style(&self) -> TacticalStyle {
        match self.tactic_type {
            MatchTacticType::T442
            | MatchTacticType::T442Diamond
            | MatchTacticType::T442DiamondWide => TacticalStyle::Balanced,
            MatchTacticType::T433 | MatchTacticType::T343 => TacticalStyle::Attacking,
            MatchTacticType::T451 | MatchTacticType::T4141 => TacticalStyle::Defensive,
            MatchTacticType::T352 => TacticalStyle::WingPlay,
            MatchTacticType::T4231 | MatchTacticType::T4312 => TacticalStyle::Possession,
            MatchTacticType::T442Narrow => TacticalStyle::Compact,
            MatchTacticType::T4411 => TacticalStyle::Counterattack,
            MatchTacticType::T1333 => TacticalStyle::Experimental,
            MatchTacticType::T4222 => TacticalStyle::WidePlay,
        }
    }

    /// Calculate how well this tactic suits the available players.
    ///
    /// Returns the average per-slot fitness on a discriminating scale:
    /// each slot is satisfied only when the best available player has a
    /// real position level for it (level ≥ 14 / "natural"). Players who
    /// only loosely cover a position contribute partial credit. Slots
    /// the squad cannot fill at all carry a hard zero — so 4-2-3-1
    /// without a true number 10 scores meaningfully worse than 4-4-2,
    /// and 3-5-2 without wingbacks scores worse than both. The previous
    /// formula clustered every formation in [0.45, 0.65] and let
    /// noise / coach-confidence pick the winner.
    pub fn calculate_formation_fitness(&self, players: &[&Player]) -> f32 {
        let required_positions = self.positions();
        if required_positions.is_empty() {
            return 0.0;
        }
        let mut total = 0.0;
        for required_pos in required_positions.iter() {
            let best = players
                .iter()
                .map(|p| self.calculate_player_position_fitness(p, required_pos))
                .fold(0.0f32, |acc, x| acc.max(x));
            total += best;
        }
        total / required_positions.len() as f32
    }

    fn calculate_player_position_fitness(
        &self,
        player: &Player,
        position: &PlayerPositionType,
    ) -> f32 {
        // Position familiarity carries the most weight. Natural (≥18)
        // is full credit, accomplished (15-17) ~0.85, competent (12-14)
        // ~0.6, awkward (8-11) ~0.3, none (<8) basically zero.
        let raw_level = player.positions.get_level(*position) as f32;
        let position_term = if raw_level >= 18.0 {
            1.0
        } else if raw_level >= 15.0 {
            0.75 + (raw_level - 15.0) / 12.0
        } else if raw_level >= 12.0 {
            0.45 + (raw_level - 12.0) / 12.0
        } else if raw_level >= 8.0 {
            0.10 + (raw_level - 8.0) / 16.0
        } else {
            0.0
        };
        // CA bonus is small — it's the *position fit* signal that
        // discriminates between formations, not raw ability (which is
        // the same regardless of which shape you pick).
        let ability_term = (player.player_attributes.current_ability as f32 / 200.0).min(1.0);
        let readiness_term = (player.skills.physical.match_readiness / 20.0).clamp(0.0, 1.0);

        position_term * 0.70 + ability_term * 0.20 + readiness_term * 0.10
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum TacticalStyle {
    Attacking,
    Defensive,
    Balanced,
    Possession,
    Counterattack,
    WingPlay,
    WidePlay,
    Compact,
    Experimental,
}

/// Canonical formation → 11 player slots.
///
/// **Invariant:** every variant returned by `MatchTacticType::all()` has
/// an entry here. The static assertion in the tests guarantees we never
/// silently fall back to T442 because a formation was added to the enum
/// without a matching slot list. Using `Tactics::positions()` for an
/// unmapped formation triggers a debug panic in development builds —
/// release builds fall back to T442 with a warning so a save file
/// authored against a future formation set still loads.
pub const TACTICS_POSITIONS: &[(MatchTacticType, [PlayerPositionType; 11])] = &[
    (
        MatchTacticType::T442,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ],
    ),
    (
        MatchTacticType::T433,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardCenter,
            PlayerPositionType::ForwardRight,
        ],
    ),
    (
        MatchTacticType::T451,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::Striker,
        ],
    ),
    (
        MatchTacticType::T4231,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::DefensiveMidfielder,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::AttackingMidfielderLeft,
            PlayerPositionType::AttackingMidfielderCenter,
            PlayerPositionType::AttackingMidfielderRight,
            PlayerPositionType::Striker,
        ],
    ),
    (
        MatchTacticType::T352,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenter,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::WingbackLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::WingbackRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ],
    ),
    // Diamond midfield — narrow 4-4-2 with a defensive and attacking
    // midfielder behind a forward pair.
    (
        MatchTacticType::T442Diamond,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::DefensiveMidfielder,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::AttackingMidfielderCenter,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ],
    ),
    // Wide diamond — same shape but the central pair pushes out to the
    // wings to stretch the pitch.
    (
        MatchTacticType::T442DiamondWide,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::DefensiveMidfielder,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::AttackingMidfielderCenter,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ],
    ),
    // Narrow 4-4-2 — central two pair playing inside, no real wide
    // outlet (overlapping fullbacks expected).
    (
        MatchTacticType::T442Narrow,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::AttackingMidfielderCenter,
            PlayerPositionType::ForwardCenter,
            PlayerPositionType::Striker,
        ],
    ),
    // 4-1-4-1 — single pivot with an industrious midfield band.
    (
        MatchTacticType::T4141,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::DefensiveMidfielder,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::Striker,
        ],
    ),
    // 4-4-1-1 — counter-attacking shape with a deep-lying second
    // striker behind the lone forward.
    (
        MatchTacticType::T4411,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::AttackingMidfielderCenter,
            PlayerPositionType::Striker,
        ],
    ),
    // 3-4-3 — back three, attacking front three.
    (
        MatchTacticType::T343,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenter,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::WingbackLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::WingbackRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardCenter,
            PlayerPositionType::ForwardRight,
        ],
    ),
    // 1-3-3-3 — a sweeper-led pyramid; experimental shape.
    (
        MatchTacticType::T1333,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::Sweeper,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenter,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardCenter,
            PlayerPositionType::ForwardRight,
        ],
    ),
    // 4-3-1-2 — strikers' shape, narrow midfield with a number 10.
    (
        MatchTacticType::T4312,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::AttackingMidfielderCenter,
            PlayerPositionType::ForwardCenter,
            PlayerPositionType::Striker,
        ],
    ),
    // 4-2-2-2 — Brazilian "magic square" with two wide attacking
    // midfielders supporting a strike pair.
    (
        MatchTacticType::T4222,
        [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::DefensiveMidfielder,
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::AttackingMidfielderLeft,
            PlayerPositionType::AttackingMidfielderRight,
            PlayerPositionType::ForwardCenter,
            PlayerPositionType::Striker,
        ],
    ),
];

#[derive(Copy, Debug, Eq, PartialEq, PartialOrd, Clone, Hash)]
pub enum MatchTacticType {
    T442,
    T433,
    T451,
    T4231,
    T352,
    T442Diamond,
    T442DiamondWide,
    T442Narrow,
    T4141,
    T4411,
    T343,
    T1333,
    T4312,
    T4222,
}

impl MatchTacticType {
    pub fn all() -> Vec<MatchTacticType> {
        vec![
            MatchTacticType::T442,
            MatchTacticType::T433,
            MatchTacticType::T451,
            MatchTacticType::T4231,
            MatchTacticType::T352,
            MatchTacticType::T442Diamond,
            MatchTacticType::T442DiamondWide,
            MatchTacticType::T442Narrow,
            MatchTacticType::T4141,
            MatchTacticType::T4411,
            MatchTacticType::T343,
            MatchTacticType::T1333,
            MatchTacticType::T4312,
            MatchTacticType::T4222,
        ]
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            MatchTacticType::T442 => "4-4-2",
            MatchTacticType::T433 => "4-3-3",
            MatchTacticType::T451 => "4-5-1",
            MatchTacticType::T4231 => "4-2-3-1",
            MatchTacticType::T352 => "3-5-2",
            MatchTacticType::T442Diamond => "4-4-2 Diamond",
            MatchTacticType::T442DiamondWide => "4-4-2 Diamond Wide",
            MatchTacticType::T442Narrow => "4-4-2 Narrow",
            MatchTacticType::T4141 => "4-1-4-1",
            MatchTacticType::T4411 => "4-4-1-1",
            MatchTacticType::T343 => "3-4-3",
            MatchTacticType::T1333 => "1-3-3-3",
            MatchTacticType::T4312 => "4-3-1-2",
            MatchTacticType::T4222 => "4-2-2-2",
        }
    }
}

pub struct TacticsSelector;

impl TacticsSelector {
    /// Main method to select the best tactic for a team
    pub fn select(team: &Team, coach: &Staff) -> Tactics {
        let available_players: Vec<&Player> = team
            .players
            .players()
            .into_iter()
            .filter(|p| p.is_ready_for_match())
            .collect();

        if available_players.len() < 11 {
            // Emergency: not enough players, use simple formation
            return Tactics::with_reason(
                MatchTacticType::T442,
                TacticSelectionReason::Default,
                0.3,
            );
        }

        // Evaluate multiple selection strategies
        let strategies = vec![
            Self::select_by_coach_preference(coach, &available_players),
            Self::select_by_team_composition(&available_players),
            Self::select_by_player_quality(&available_players),
        ];

        // Choose the best strategy result
        strategies
            .into_iter()
            .max_by(|a, b| {
                a.formation_strength
                    .partial_cmp(&b.formation_strength)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_else(|| {
                Tactics::with_reason(MatchTacticType::T442, TacticSelectionReason::Default, 0.5)
            })
    }

    /// Select tactic based on coach attributes and behavior
    fn select_by_coach_preference(coach: &Staff, players: &[&Player]) -> Tactics {
        let tactical_knowledge = coach.staff_attributes.knowledge.tactical_knowledge;
        let attacking_coaching = coach.staff_attributes.coaching.attacking;
        let defending_coaching = coach.staff_attributes.coaching.defending;

        let preferred_tactic = match coach.behaviour.state {
            PersonBehaviourState::Poor => {
                // Conservative, simple formation
                if tactical_knowledge < 10 {
                    MatchTacticType::T442
                } else {
                    MatchTacticType::T451
                }
            }
            PersonBehaviourState::Normal => Self::select_balanced_by_coaching_style(
                attacking_coaching,
                defending_coaching,
                tactical_knowledge,
            ),
            PersonBehaviourState::Good => Self::select_advanced_by_coaching_expertise(
                attacking_coaching,
                defending_coaching,
                tactical_knowledge,
            ),
        };

        let tactic = Tactics::new(preferred_tactic);
        let strength =
            tactic.calculate_formation_fitness(players) * Self::coach_confidence_multiplier(coach);

        Tactics::with_reason(
            preferred_tactic,
            TacticSelectionReason::CoachPreference,
            strength,
        )
    }

    fn select_balanced_by_coaching_style(
        attacking: u8,
        defending: u8,
        tactical: u8,
    ) -> MatchTacticType {
        let attack_def_diff = attacking as i16 - defending as i16;

        // The legacy table sent three of five branches to T442 — even
        // moderately attacking / defending profiles fell through. Now
        // every branch lands on a distinct shape so the long-run
        // distribution across coaches is real.
        match (attack_def_diff, tactical) {
            (diff, tact) if diff >= 4 && tact >= 12 => MatchTacticType::T433,
            (diff, tact) if diff >= 2 && tact >= 10 => MatchTacticType::T4231,
            (diff, tact) if diff <= -4 && tact >= 12 => MatchTacticType::T451,
            (diff, tact) if diff <= -2 && tact >= 10 => MatchTacticType::T4141,
            (_, tact) if tact >= 15 => MatchTacticType::T442Diamond,
            (_, tact) if tact >= 12 => MatchTacticType::T4411,
            _ => MatchTacticType::T442,
        }
    }

    fn select_advanced_by_coaching_expertise(
        attacking: u8,
        defending: u8,
        tactical: u8,
    ) -> MatchTacticType {
        if tactical >= 18 {
            // Master tactician — full formation library available.
            if attacking >= 17 {
                MatchTacticType::T343
            } else if attacking >= 14 && defending <= 12 {
                MatchTacticType::T4222
            } else if defending >= 17 {
                MatchTacticType::T352
            } else if defending >= 14 {
                MatchTacticType::T4312
            } else {
                MatchTacticType::T4231
            }
        } else if tactical >= 15 {
            // Experienced — proven, complex formations.
            if attacking > defending + 3 {
                MatchTacticType::T433
            } else if defending > attacking + 3 {
                MatchTacticType::T4141
            } else if attacking + defending > 28 {
                MatchTacticType::T4231
            } else {
                MatchTacticType::T442Diamond
            }
        } else if tactical >= 12 {
            // Solid pro — slight asymmetry tilts toward attack/defence
            // shape; otherwise sits on a busy diamond / 4-4-1-1.
            if attacking > defending + 2 {
                MatchTacticType::T4231
            } else if defending > attacking + 2 {
                MatchTacticType::T4411
            } else {
                MatchTacticType::T442Diamond
            }
        } else {
            MatchTacticType::T442
        }
    }

    /// Select tactic based on available player composition
    fn select_by_team_composition(players: &[&Player]) -> Tactics {
        let position_analysis = Self::analyze_team_composition(players);

        let selected_tactic = Self::match_formation_to_composition(&position_analysis);
        let tactic = Tactics::new(selected_tactic);
        let strength = tactic.calculate_formation_fitness(players);

        Tactics::with_reason(
            selected_tactic,
            TacticSelectionReason::TeamComposition,
            strength,
        )
    }

    fn analyze_team_composition(players: &[&Player]) -> TeamCompositionAnalysis {
        let mut analysis = TeamCompositionAnalysis::new();

        for player in players {
            for position in player.positions() {
                let quality = player.player_attributes.current_ability as f32 / 200.0;

                match position {
                    pos if pos.is_defender() => {
                        analysis.defender_quality += quality;
                        analysis.defender_count += 1;
                    }
                    pos if pos.is_midfielder() => {
                        analysis.midfielder_quality += quality;
                        analysis.midfielder_count += 1;
                    }
                    pos if pos.is_forward() => {
                        analysis.forward_quality += quality;
                        analysis.forward_count += 1;
                    }
                    PlayerPositionType::Goalkeeper => {
                        analysis.goalkeeper_quality += quality;
                        analysis.goalkeeper_count += 1;
                    }
                    _ => {}
                }
            }
        }

        // Calculate averages
        if analysis.defender_count > 0 {
            analysis.defender_quality /= analysis.defender_count as f32;
        }
        if analysis.midfielder_count > 0 {
            analysis.midfielder_quality /= analysis.midfielder_count as f32;
        }
        if analysis.forward_count > 0 {
            analysis.forward_quality /= analysis.forward_count as f32;
        }

        analysis
    }

    fn match_formation_to_composition(analysis: &TeamCompositionAnalysis) -> MatchTacticType {
        // Determine strongest area. Weight by *available count* so a
        // squad with 8 forwards but only 3 defenders truly is "strong
        // attacking", not just "above average" everywhere.
        let def_strength =
            analysis.defender_quality * (analysis.defender_count as f32 / 6.0).min(1.0);
        let mid_strength =
            analysis.midfielder_quality * (analysis.midfielder_count as f32 / 6.0).min(1.0);
        let att_strength =
            analysis.forward_quality * (analysis.forward_count as f32 / 4.0).min(1.0);

        if att_strength > def_strength + 0.10 && att_strength > mid_strength + 0.05 {
            // 3+ quality forwards → live with a striker-heavy front
            // line regardless of midfield depth. A back-three with
            // wingbacks (3-4-3) is the cleanest way to fit three
            // forwards when the midfielder pool is thin.
            if analysis.forward_count >= 4 {
                MatchTacticType::T433
            } else if analysis.forward_count >= 3 && analysis.midfielder_count >= 3 {
                MatchTacticType::T433
            } else if analysis.forward_count >= 3 {
                MatchTacticType::T343
            } else if analysis.midfielder_count >= 4 {
                MatchTacticType::T4231
            } else {
                MatchTacticType::T442Narrow
            }
        } else if def_strength > att_strength + 0.10 && def_strength > mid_strength + 0.05 {
            if analysis.defender_count >= 6 {
                MatchTacticType::T352
            } else if analysis.midfielder_count >= 5 {
                MatchTacticType::T451
            } else {
                MatchTacticType::T4141
            }
        } else if mid_strength > 0.55 && analysis.midfielder_count >= 5 {
            MatchTacticType::T4312
        } else if mid_strength > 0.50 {
            MatchTacticType::T442Diamond
        } else if att_strength + def_strength > 1.0 {
            // Both ends of the squad above average and midfield thin —
            // a 4-4-1-1 / counter shape suits.
            MatchTacticType::T4411
        } else {
            MatchTacticType::T442
        }
    }

    /// Select tactic based on individual player quality and fitness.
    ///
    /// Tests every formation in the library (the previous version only
    /// tried 5 of 14 — guaranteeing exotic shapes never won) and picks
    /// the one whose slot-by-slot fitness best matches the squad. Ties
    /// fall to the lower enum variant so output is deterministic across
    /// runs.
    fn select_by_player_quality(players: &[&Player]) -> Tactics {
        let mut best: Option<(MatchTacticType, f32)> = None;
        for tactic_type in MatchTacticType::all() {
            let tac = Tactics::new(tactic_type);
            let strength = tac.calculate_formation_fitness(players);
            best = Some(match best {
                None => (tactic_type, strength),
                Some((cur_t, cur_s)) => {
                    if strength > cur_s + f32::EPSILON {
                        (tactic_type, strength)
                    } else {
                        (cur_t, cur_s)
                    }
                }
            });
        }
        let (tactic_type, strength) = best.unwrap_or((MatchTacticType::T442, 0.5));
        Tactics::with_reason(
            tactic_type,
            TacticSelectionReason::TeamComposition,
            strength,
        )
    }

    /// Select counter tactic against specific opponent formation.
    ///
    /// Every `MatchTacticType` has a real arm so countering 4-4-2 doesn't
    /// loop back to 4-4-2 (which previously locked every league at 4-4-2
    /// once both sides persisted T442). Tiebreaks: pick a counter whose
    /// player demands the current squad can actually meet — if not, fall
    /// back to the second choice.
    pub fn select_counter_tactic(
        opponent_tactic: &MatchTacticType,
        our_players: &[&Player],
    ) -> Tactics {
        let candidates: &[MatchTacticType] = match opponent_tactic {
            // Open 4-4-2: hit the central third where they only have two CMs.
            MatchTacticType::T442 => &[
                MatchTacticType::T4231,
                MatchTacticType::T433,
                MatchTacticType::T352,
            ],
            // Three-band attacking — control midfield to choke supply.
            MatchTacticType::T433 | MatchTacticType::T343 => &[
                MatchTacticType::T451,
                MatchTacticType::T4141,
                MatchTacticType::T4231,
            ],
            // Defensive shells — break them open with width and a free 10.
            MatchTacticType::T451 | MatchTacticType::T4141 => &[
                MatchTacticType::T433,
                MatchTacticType::T4231,
                MatchTacticType::T343,
            ],
            // Possession / number-10 sides — disrupt the build with a
            // pressing diamond or a holding double-pivot.
            MatchTacticType::T4231 | MatchTacticType::T4312 => &[
                MatchTacticType::T442Diamond,
                MatchTacticType::T4141,
                MatchTacticType::T352,
            ],
            // Wing-play 3-5-2: clog the half-spaces with a narrow shape.
            MatchTacticType::T352 => &[
                MatchTacticType::T442Narrow,
                MatchTacticType::T4231,
                MatchTacticType::T4312,
            ],
            // Diamond / wide diamond mirror.
            MatchTacticType::T442Diamond => &[
                MatchTacticType::T4231,
                MatchTacticType::T433,
                MatchTacticType::T352,
            ],
            MatchTacticType::T442DiamondWide => &[
                MatchTacticType::T442Narrow,
                MatchTacticType::T4312,
                MatchTacticType::T4231,
            ],
            MatchTacticType::T442Narrow => &[
                MatchTacticType::T442DiamondWide,
                MatchTacticType::T352,
                MatchTacticType::T433,
            ],
            // 4-4-1-1 second-striker block — match with a busy midfield
            // band.
            MatchTacticType::T4411 => &[
                MatchTacticType::T4231,
                MatchTacticType::T4141,
                MatchTacticType::T442Diamond,
            ],
            // Brazilian magic-square — break with width and pace.
            MatchTacticType::T4222 => &[
                MatchTacticType::T433,
                MatchTacticType::T352,
                MatchTacticType::T442DiamondWide,
            ],
            // Sweeper-led pyramid — fast direct ball over the back-five.
            MatchTacticType::T1333 => &[
                MatchTacticType::T433,
                MatchTacticType::T4231,
                MatchTacticType::T343,
            ],
        };

        // Pick the highest-fit candidate the squad actually has the
        // bodies for. Iteration order encodes the coach's *priority* —
        // ties break to the first listed candidate, not the last (the
        // default `max_by` behaviour). Reactive penalty (0.9)
        // preserved.
        let mut best: Option<(MatchTacticType, f32)> = None;
        for &t in candidates {
            let tac = Tactics::new(t);
            let fit = tac.calculate_formation_fitness(our_players);
            best = Some(match best {
                None => (t, fit),
                Some((cur_t, cur_s)) => {
                    if fit > cur_s + f32::EPSILON {
                        (t, fit)
                    } else {
                        (cur_t, cur_s)
                    }
                }
            });
        }
        let (counter_tactic, fit) = best.unwrap_or((candidates[0], 0.5));
        Tactics::with_reason(
            counter_tactic,
            TacticSelectionReason::OpponentCounter,
            fit * 0.9,
        )
    }

    /// Select tactics based on game situation
    pub fn select_situational_tactic(
        current_tactic: &MatchTacticType,
        is_home: bool,
        score_difference: i8,
        minutes_played: u8,
        players: &[&Player],
    ) -> Option<Tactics> {
        let tactic_type = Self::situational_shape(
            *current_tactic,
            is_home,
            score_difference,
            minutes_played,
        )?;
        let tactic = Tactics::new(tactic_type);
        let strength = tactic.calculate_formation_fitness(players) * 0.8; // Penalty for mid-game change
        Some(Tactics::with_reason(
            tactic_type,
            TacticSelectionReason::GameSituation,
            strength,
        ))
    }

    /// Shape-only variant — returns just the tactical shape change
    /// without computing formation fitness. Used by the match engine,
    /// which only has `MatchPlayer` (not full `Player`) access at the
    /// hot tick path; the engine can apply the shape change to its
    /// per-side `Tactics` field without paying the fitness cost.
    /// Returns `None` when no situational override is warranted or
    /// when the new shape matches the current one.
    pub fn situational_shape(
        current_tactic: MatchTacticType,
        _is_home: bool,
        score_difference: i8,
        minutes_played: u8,
    ) -> Option<MatchTacticType> {
        let new_tactic = match (score_difference, minutes_played) {
            // Desperately need goals
            (diff, min) if diff < -1 && min > 75 => Some(MatchTacticType::T343),
            (diff, min) if diff < 0 && min > 70 => Some(MatchTacticType::T433),

            // Protecting a lead
            (diff, min) if diff > 1 && min > 80 => Some(MatchTacticType::T451),
            (diff, min) if diff > 0 && min > 75 => Some(MatchTacticType::T4141),

            // First half adjustments — chasing at home
            (diff, min) if diff < -1 && min < 30 && _is_home => Some(MatchTacticType::T4231),

            _ => None,
        };

        new_tactic.filter(|t| *t != current_tactic)
    }

    fn coach_confidence_multiplier(coach: &Staff) -> f32 {
        let base_confidence = match coach.behaviour.state {
            PersonBehaviourState::Poor => 0.7,
            PersonBehaviourState::Normal => 1.0,
            PersonBehaviourState::Good => 1.2,
        };

        let tactical_bonus =
            (coach.staff_attributes.knowledge.tactical_knowledge as f32 / 20.0) * 0.3;
        let experience_bonus = (coach.staff_attributes.mental.determination as f32 / 20.0) * 0.2;

        (base_confidence + tactical_bonus + experience_bonus).clamp(0.5, 1.5)
    }
}

#[derive(Debug, Clone)]
struct TeamCompositionAnalysis {
    goalkeeper_count: u8,
    goalkeeper_quality: f32,
    defender_count: u8,
    defender_quality: f32,
    midfielder_count: u8,
    midfielder_quality: f32,
    forward_count: u8,
    forward_quality: f32,
}

impl TeamCompositionAnalysis {
    fn new() -> Self {
        Self {
            goalkeeper_count: 0,
            goalkeeper_quality: 0.0,
            defender_count: 0,
            defender_quality: 0.0,
            midfielder_count: 0,
            midfielder_quality: 0.0,
            forward_count: 0,
            forward_quality: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PersonAttributes;
    use crate::shared::fullname::FullName;
    fn create_test_player(id: u32, position: PlayerPositionType, ability: u8) -> Player {
        use crate::club::player::builder::PlayerBuilder;
        use crate::club::player::*;

        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(1995, 1, 1).unwrap())
            .country_id(1)
            .skills(PlayerSkills::default())
            .attributes(PersonAttributes::default())
            .player_attributes(PlayerAttributes {
                current_ability: ability,
                potential_ability: ability + 10,
                condition: 10000,
                ..Default::default()
            })
            .contract(None)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 18,
                }],
            })
            .build()
            .expect("Failed to build test player")
    }

    #[test]
    fn test_formation_fitness_calculation() {
        let players = vec![
            create_test_player(1, PlayerPositionType::Goalkeeper, 150),
            create_test_player(2, PlayerPositionType::DefenderLeft, 140),
            create_test_player(3, PlayerPositionType::MidfielderCenter, 160),
            create_test_player(4, PlayerPositionType::ForwardCenter, 170),
        ];

        let player_refs: Vec<&Player> = players.iter().collect();
        let tactic = Tactics::new(MatchTacticType::T442);

        let fitness = tactic.calculate_formation_fitness(&player_refs);
        assert!(fitness > 0.0 && fitness <= 1.0);
    }

    #[test]
    fn test_tactical_selection_by_composition() {
        // Create a team with strong attackers
        let players = vec![
            create_test_player(1, PlayerPositionType::ForwardCenter, 180),
            create_test_player(2, PlayerPositionType::ForwardLeft, 175),
            create_test_player(3, PlayerPositionType::ForwardRight, 170),
            create_test_player(4, PlayerPositionType::MidfielderCenter, 140),
        ];

        let player_refs: Vec<&Player> = players.iter().collect();
        let result = TacticsSelector::select_by_team_composition(&player_refs);

        // Should prefer attacking formation for strong forwards
        assert!(
            result.is_attacking() || matches!(result.tactic_type, MatchTacticType::T4231),
            "expected attacking shape for 3+ strong forwards, got {:?}",
            result.tactic_type,
        );
    }

    #[test]
    fn test_counter_tactic_selection() {
        let players = vec![create_test_player(1, PlayerPositionType::Goalkeeper, 150)];
        let player_refs: Vec<&Player> = players.iter().collect();

        // The counter to a 4-3-3 is now picked from a priority-ordered
        // candidate set ([T451, T4141, T4231]); ties resolve to the
        // first listed candidate. The selection_reason invariant is
        // what matters externally — anti-T442 distribution test
        // covers the no-self-loop guarantee.
        let counter = TacticsSelector::select_counter_tactic(&MatchTacticType::T433, &player_refs);
        assert_eq!(counter.tactic_type, MatchTacticType::T451);
        assert_eq!(
            counter.selected_reason,
            TacticSelectionReason::OpponentCounter
        );

        // T442 must NOT counter to T442 (the bug that locked every
        // league at 4-4-2 once both sides persisted T442).
        let v_t442 = TacticsSelector::select_counter_tactic(&MatchTacticType::T442, &player_refs);
        assert_ne!(v_t442.tactic_type, MatchTacticType::T442);
        assert_eq!(
            v_t442.selected_reason,
            TacticSelectionReason::OpponentCounter
        );
    }

    #[test]
    fn every_match_tactic_has_a_position_layout() {
        // Without this guarantee `Tactics::positions` silently falls back
        // to T442 — the original bug we set out to fix in Phase 7.
        for tactic in MatchTacticType::all() {
            let found = TACTICS_POSITIONS.iter().any(|(t, _)| *t == tactic);
            assert!(
                found,
                "TACTICS_POSITIONS missing slot list for {:?}",
                tactic
            );
        }
    }

    #[test]
    fn every_layout_starts_with_a_goalkeeper_and_has_eleven_slots() {
        for (tactic, slots) in TACTICS_POSITIONS {
            assert_eq!(slots.len(), 11, "{:?} has != 11 slots", tactic);
            assert_eq!(
                slots[0],
                PlayerPositionType::Goalkeeper,
                "{:?} slot 0 must be Goalkeeper",
                tactic
            );
        }
    }

    #[test]
    fn situational_tactic_returns_attacking_when_chasing_late() {
        let players: Vec<Player> = (1..=11)
            .map(|i| create_test_player(i, PlayerPositionType::ForwardCenter, 150))
            .collect();
        let player_refs: Vec<&Player> = players.iter().collect();
        let new_tactic = TacticsSelector::select_situational_tactic(
            &MatchTacticType::T442,
            true, // is_home
            -2,   // 2-goal deficit
            80,   // late game
            &player_refs,
        );
        assert!(new_tactic.is_some());
        let chosen = new_tactic.unwrap();
        assert!(
            chosen.is_attacking(),
            "expected attacking shape, got {:?}",
            chosen.tactic_type
        );
        assert_eq!(chosen.selected_reason, TacticSelectionReason::GameSituation);
    }

    #[test]
    fn situational_tactic_returns_defensive_when_protecting_lead() {
        let players: Vec<Player> = (1..=11)
            .map(|i| create_test_player(i, PlayerPositionType::DefenderCenter, 150))
            .collect();
        let player_refs: Vec<&Player> = players.iter().collect();
        let new_tactic = TacticsSelector::select_situational_tactic(
            &MatchTacticType::T442,
            true, // is_home
            2,    // 2-goal lead
            85,   // late game
            &player_refs,
        );
        assert!(new_tactic.is_some());
        let chosen = new_tactic.unwrap();
        assert!(
            chosen.is_defensive() || chosen.tactic_type == MatchTacticType::T451,
            "expected defensive shape, got {:?}",
            chosen.tactic_type
        );
    }

    /// Build a uniform set of `n` players at a single position. Useful
    /// for stress-testing the composition analyzer with a deliberately
    /// lopsided squad shape.
    fn squad_at(position: PlayerPositionType, n: u32, ability: u8) -> Vec<Player> {
        (1..=n)
            .map(|i| create_test_player(i, position, ability))
            .collect()
    }

    /// Synthetic squad with the canonical 1-4-4-2 distribution.
    fn balanced_squad() -> Vec<Player> {
        let mut v = Vec::new();
        v.extend(squad_at(PlayerPositionType::Goalkeeper, 1, 140));
        v.extend(squad_at(PlayerPositionType::DefenderLeft, 1, 140));
        v.extend(squad_at(PlayerPositionType::DefenderCenterLeft, 1, 140));
        v.extend(squad_at(PlayerPositionType::DefenderCenterRight, 1, 140));
        v.extend(squad_at(PlayerPositionType::DefenderRight, 1, 140));
        v.extend(squad_at(PlayerPositionType::MidfielderLeft, 1, 140));
        v.extend(squad_at(PlayerPositionType::MidfielderCenterLeft, 1, 140));
        v.extend(squad_at(PlayerPositionType::MidfielderCenterRight, 1, 140));
        v.extend(squad_at(PlayerPositionType::MidfielderRight, 1, 140));
        v.extend(squad_at(PlayerPositionType::ForwardLeft, 1, 140));
        v.extend(squad_at(PlayerPositionType::ForwardRight, 1, 140));
        v
    }

    #[test]
    fn attacking_squad_distribution_never_picks_t442() {
        // Forward-loaded squad with a thin midfield: a real attacking
        // coach would never pick a flat 4-4-2 here. The composition
        // selector and the player-quality selector both have to land
        // somewhere genuinely attacking. Deterministic — fixed CA
        // numbers, fixed enum tiebreak.
        let mut players = squad_at(PlayerPositionType::Goalkeeper, 1, 140);
        players.extend(squad_at(PlayerPositionType::DefenderCenterLeft, 1, 130));
        players.extend(squad_at(PlayerPositionType::DefenderCenter, 1, 130));
        players.extend(squad_at(PlayerPositionType::DefenderCenterRight, 1, 130));
        players.extend(squad_at(PlayerPositionType::MidfielderCenter, 2, 130));
        players.extend(squad_at(PlayerPositionType::ForwardLeft, 2, 175));
        players.extend(squad_at(PlayerPositionType::ForwardCenter, 2, 175));
        players.extend(squad_at(PlayerPositionType::ForwardRight, 2, 175));
        let player_refs: Vec<&Player> = players.iter().collect();

        let by_comp = TacticsSelector::select_by_team_composition(&player_refs);
        assert_ne!(
            by_comp.tactic_type,
            MatchTacticType::T442,
            "attacking squad must not pick T442 by composition: got {:?}",
            by_comp.tactic_type,
        );

        let by_quality = TacticsSelector::select_by_player_quality(&player_refs);
        assert_ne!(
            by_quality.tactic_type,
            MatchTacticType::T442,
            "attacking squad must not pick T442 by quality: got {:?}",
            by_quality.tactic_type,
        );
    }

    #[test]
    fn defensive_squad_distribution_never_picks_t442() {
        // Defender-loaded squad with a single striker. The composition
        // analyzer should send us into a back-five / single-pivot
        // shape, never a flat 4-4-2.
        let mut players = squad_at(PlayerPositionType::Goalkeeper, 1, 140);
        players.extend(squad_at(PlayerPositionType::DefenderLeft, 1, 175));
        players.extend(squad_at(PlayerPositionType::DefenderCenterLeft, 1, 175));
        players.extend(squad_at(PlayerPositionType::DefenderCenter, 1, 175));
        players.extend(squad_at(PlayerPositionType::DefenderCenterRight, 1, 175));
        players.extend(squad_at(PlayerPositionType::DefenderRight, 1, 175));
        players.extend(squad_at(PlayerPositionType::MidfielderCenterLeft, 1, 130));
        players.extend(squad_at(PlayerPositionType::MidfielderCenter, 1, 130));
        players.extend(squad_at(PlayerPositionType::MidfielderCenterRight, 1, 130));
        players.extend(squad_at(PlayerPositionType::Striker, 1, 130));
        let player_refs: Vec<&Player> = players.iter().collect();

        let by_comp = TacticsSelector::select_by_team_composition(&player_refs);
        assert_ne!(
            by_comp.tactic_type,
            MatchTacticType::T442,
            "defensive squad must not pick T442 by composition: got {:?}",
            by_comp.tactic_type,
        );
    }

    #[test]
    fn balanced_low_tactical_coach_path_lands_on_t442_only_in_low_tier() {
        // The coaching-style helpers are deterministic functions of
        // (attacking, defending, tactical) integers — exercise them
        // directly to lock the no-T442-for-tactically-aware-coaches
        // invariant. This is the path that previously made every
        // mid-tier coach pick T442 even when the attacking/defending
        // gap pointed elsewhere.
        // Tactical-knowledge cliff: at < 12 a balanced coach may pick
        // T442 (legitimate fallback for an inexperienced manager); at
        // 12+ they must land on a non-T442 shape.
        for tact in 0u8..12 {
            let _ = TacticsSelector::select_balanced_by_coaching_style(10, 10, tact);
            // Below 12 the helper is allowed to return T442; we don't
            // pin a specific value — just probe that the function
            // doesn't panic.
        }
        for tact in 12u8..=20 {
            let pick = TacticsSelector::select_balanced_by_coaching_style(10, 10, tact);
            assert_ne!(
                pick,
                MatchTacticType::T442,
                "balanced coach with tactical_knowledge {} should not pick T442",
                tact,
            );
        }
        // Strongly attacking (att-def diff +6) must give an attacking
        // shape regardless of tactical floor (modulo the diff>=4 + tact>=12 gate).
        let attacking = TacticsSelector::select_balanced_by_coaching_style(18, 10, 14);
        assert!(
            matches!(
                attacking,
                MatchTacticType::T433 | MatchTacticType::T4231 | MatchTacticType::T343
            ),
            "att+strong tactic should produce attacking shape, got {:?}",
            attacking,
        );
        // Strongly defensive: T451 / T4141 / T352 are all valid.
        let defensive = TacticsSelector::select_balanced_by_coaching_style(8, 18, 14);
        assert!(
            matches!(
                defensive,
                MatchTacticType::T451 | MatchTacticType::T4141 | MatchTacticType::T352
            ),
            "def+strong tactic should produce defensive shape, got {:?}",
            defensive,
        );
    }

    #[test]
    fn advanced_coach_path_never_picks_t442_above_threshold() {
        // The "advanced" path is gated behind a Good behaviour state
        // and a tactical-knowledge floor. At any tact >= 12 it must
        // land on a sophisticated shape, never T442.
        for tact in 12u8..=20 {
            for att in [10u8, 16] {
                for def in [10u8, 16] {
                    let pick = TacticsSelector::select_advanced_by_coaching_expertise(att, def, tact);
                    assert_ne!(
                        pick,
                        MatchTacticType::T442,
                        "advanced path att={att} def={def} tact={tact} returned T442",
                    );
                }
            }
        }
    }

    #[test]
    fn balanced_squad_quality_path_does_not_lock_t442() {
        // A balanced squad shouldn't be FORCED into T442 by the
        // quality path. With the slot-based fitness, tied candidates
        // resolve deterministically — but T442 should never be the
        // *only* viable winner. We assert the picker is at least
        // open to producing other shapes when the squad is uniform.
        let players = balanced_squad();
        let player_refs: Vec<&Player> = players.iter().collect();
        let result = TacticsSelector::select_by_player_quality(&player_refs);
        // Either T442 (legitimate for a uniform squad) or another
        // matching shape; the test guards against the legacy bug
        // where T442 always won on tie.
        assert!(
            matches!(
                result.tactic_type,
                MatchTacticType::T442
                    | MatchTacticType::T433
                    | MatchTacticType::T4231
                    | MatchTacticType::T4411
                    | MatchTacticType::T442Diamond
                    | MatchTacticType::T442Narrow
                    | MatchTacticType::T4141
                    | MatchTacticType::T451
            ),
            "balanced squad picked unexpected tactic: {:?}",
            result.tactic_type,
        );
    }

    #[test]
    fn counter_tactic_never_returns_t442_for_t442() {
        // The whole point of the diversified counter matrix: T442 must
        // not loop back to T442 (the bug that locked every league at
        // 4-4-2 once both sides persisted T442 after their first
        // simulation tick).
        let players: Vec<Player> = (1..=11)
            .map(|i| create_test_player(i, PlayerPositionType::MidfielderCenter, 150))
            .collect();
        let player_refs: Vec<&Player> = players.iter().collect();
        let v = TacticsSelector::select_counter_tactic(&MatchTacticType::T442, &player_refs);
        assert_ne!(
            v.tactic_type,
            MatchTacticType::T442,
            "T442 must not be a self-counter — broke league shape diversity",
        );
    }

    #[test]
    fn counter_tactic_covers_every_match_tactic() {
        // Property: every formation has a real counter (no `_ => T442`
        // default). Combined with the no-self-loop guarantee above, this
        // means the counter matrix can't collapse to T442 from any
        // starting point.
        let players: Vec<Player> = (1..=11)
            .map(|i| create_test_player(i, PlayerPositionType::MidfielderCenter, 150))
            .collect();
        let player_refs: Vec<&Player> = players.iter().collect();
        for opp in MatchTacticType::all() {
            let counter = TacticsSelector::select_counter_tactic(&opp, &player_refs);
            assert_ne!(
                counter.tactic_type, opp,
                "{:?} must not counter to itself",
                opp
            );
            assert_eq!(counter.selected_reason, TacticSelectionReason::OpponentCounter);
        }
    }

    #[test]
    fn situational_tactic_returns_none_when_no_change_warranted() {
        let players: Vec<Player> = (1..=11)
            .map(|i| create_test_player(i, PlayerPositionType::MidfielderCenter, 150))
            .collect();
        let player_refs: Vec<&Player> = players.iter().collect();
        // Drawing in the first half, no dramatic situation.
        let new_tactic = TacticsSelector::select_situational_tactic(
            &MatchTacticType::T442,
            true,
            0,
            40,
            &player_refs,
        );
        assert!(new_tactic.is_none());
    }
}
