use crate::club::player::load::{
    FATIGUE_LOAD_DANGER, FATIGUE_LOAD_THRESHOLD, PHYSICAL_LOAD_DANGER, PHYSICAL_LOAD_THRESHOLD,
    RECOVERY_DEBT_HEAVY,
};
use crate::club::staff::CoachPlayerBond;
use crate::club::staff::perception::{AbilityEstimator, CoachProfile};
use crate::club::{
    PlayerFieldPositionGroup, PlayerPositionType, PositionCoverage, Staff,
};
use crate::{Player, PlayerStatusType, SelectionScoreFactor, Tactics};
use chrono::NaiveDate;

use super::helpers::SlotFit;
use std::cmp::Ordering;

/// Per-component breakdown of a slot score. Mirrors what
/// `score_player_for_slot` adds together — each field is the same
/// signed contribution the total carries, so summing them reproduces
/// the total exactly. Used by the squad selector to explain
/// omissions: the dominant factors (largest absolute contributions
/// where the selected player edged ahead of the omitted one) become
/// the comparison line in the player-events render.
#[derive(Debug, Clone, Copy, Default)]
pub struct SlotScoreBreakdown {
    /// Football the manager's plan owes him, weighed by the room the
    /// fixture leaves. Everything else here is his fit for the shirt.
    pub football_owed: f32,
    pub position_fit: f32,
    pub perceived_quality: f32,
    pub match_readiness: f32,
    pub condition_floor: f32,
    pub tactical_style: f32,
    pub side_foot: f32,
    pub reputation: f32,
    pub coach_relationship: f32,
    pub newcomer: f32,
    pub training_impression: f32,
    pub cohesion: f32,
    pub force_selection: f32,
    pub group_mismatch: f32,
    /// The physical risk of starting him, as a non-positive penalty. Left
    /// at zero by `score_player_for_slot_with_breakdown` and filled in by
    /// the competitive selector.
    pub injury_risk: f32,
    /// Bounded opponent-matchup adjustment (pace, aerial, press, low block,
    /// wide). Populated by the omissions builder when the selection layer
    /// carries a richer game model. The pure scoring engine leaves it at
    /// zero so existing callers behave identically.
    pub opponent_matchup: f32,
    /// Role / duty fit on top of the raw position-level score — rewards
    /// a player whose attribute profile matches the slot's role recipe.
    pub role_duty_fit: f32,
    /// Whole-XI balance bonus (defensive security, ball progression, …).
    /// Aggregated per-player from the post-DP balance pass.
    pub lineup_balance: f32,
    /// Bench scenario coverage — only populated for bench candidates.
    pub bench_scenario: f32,
    /// Extra medical-caution premium on top of `injury_risk` — non-zero
    /// when the coach policy is medical-cautious and the player is
    /// fragile in this fixture.
    pub medical_risk: f32,
    /// Eligibility-rule penalty (registration, cup-tie, loan clause).
    /// Hard blocks register as a large negative; soft limits a small one.
    pub eligibility_rule: f32,
}

impl SlotScoreBreakdown {
    pub fn total(&self) -> f32 {
        self.football_owed
            + self.position_fit
            + self.perceived_quality
            + self.match_readiness
            + self.condition_floor
            + self.tactical_style
            + self.side_foot
            + self.reputation
            + self.coach_relationship
            + self.newcomer
            + self.training_impression
            + self.cohesion
            + self.force_selection
            + self.group_mismatch
            + self.injury_risk
            + self.opponent_matchup
            + self.role_duty_fit
            + self.lineup_balance
            + self.bench_scenario
            + self.medical_risk
            + self.eligibility_rule
    }

    /// Pairwise comparison: rank scoring factors where `selected`
    /// beat `omitted`, sorted by gap descending. Up to `limit` atoms
    /// returned. Used by the squad selector to populate
    /// `SelectionComparison::top_factors` with the dominant reasons
    /// the rival was chosen.
    pub fn top_factors_against(
        &self,
        omitted: &SlotScoreBreakdown,
        limit: usize,
    ) -> Vec<SelectionScoreFactor> {
        let factors: [(SelectionScoreFactor, f32); 21] = [
            (
                SelectionScoreFactor::ManagerPlan,
                self.football_owed - omitted.football_owed,
            ),
            (
                SelectionScoreFactor::PositionFit,
                self.position_fit - omitted.position_fit,
            ),
            (
                SelectionScoreFactor::PerceivedQuality,
                self.perceived_quality - omitted.perceived_quality,
            ),
            (
                SelectionScoreFactor::MatchReadiness,
                self.match_readiness - omitted.match_readiness,
            ),
            // condition_floor is a penalty (subtracted) — a smaller
            // (less negative) value for the selected player means the
            // selected player is fitter than the omitted one.
            (
                SelectionScoreFactor::Fatigue,
                self.condition_floor - omitted.condition_floor,
            ),
            (
                SelectionScoreFactor::TacticalFit,
                self.tactical_style - omitted.tactical_style,
            ),
            (
                SelectionScoreFactor::SideFootFit,
                self.side_foot - omitted.side_foot,
            ),
            (
                SelectionScoreFactor::Reputation,
                self.reputation - omitted.reputation,
            ),
            (
                SelectionScoreFactor::CoachRelationship,
                self.coach_relationship - omitted.coach_relationship,
            ),
            // Newcomer contributes negatively; smaller penalty = better.
            (
                SelectionScoreFactor::Newcomer,
                self.newcomer - omitted.newcomer,
            ),
            (
                SelectionScoreFactor::TrainingImpression,
                self.training_impression - omitted.training_impression,
            ),
            (
                SelectionScoreFactor::Cohesion,
                self.cohesion - omitted.cohesion,
            ),
            (
                SelectionScoreFactor::ForceSelection,
                self.force_selection - omitted.force_selection,
            ),
            (
                SelectionScoreFactor::PositionFit,
                self.group_mismatch - omitted.group_mismatch,
            ),
            // injury_risk is stored as a non-positive penalty — a smaller
            // (less negative) value for the selected player means he was
            // the safer pick.
            (
                SelectionScoreFactor::InjuryRisk,
                self.injury_risk - omitted.injury_risk,
            ),
            (
                SelectionScoreFactor::OpponentMatchup,
                self.opponent_matchup - omitted.opponent_matchup,
            ),
            (
                SelectionScoreFactor::RoleDutyFit,
                self.role_duty_fit - omitted.role_duty_fit,
            ),
            (
                SelectionScoreFactor::LineupBalance,
                self.lineup_balance - omitted.lineup_balance,
            ),
            (
                SelectionScoreFactor::BenchScenario,
                self.bench_scenario - omitted.bench_scenario,
            ),
            (
                SelectionScoreFactor::MedicalRisk,
                self.medical_risk - omitted.medical_risk,
            ),
            (
                SelectionScoreFactor::EligibilityRule,
                self.eligibility_rule - omitted.eligibility_rule,
            ),
        ];

        let mut wins: Vec<(SelectionScoreFactor, f32)> = factors
            .into_iter()
            .filter(|(_, delta)| *delta > 0.05)
            .collect();
        wins.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        wins.truncate(limit);
        wins.into_iter().map(|(f, _)| f).collect()
    }
}

pub(crate) struct ScoringEngine {
    /// Squad roles belong to the roster which assigned them. A borrowed
    /// player brings only the club-wide verdicts
    /// ([`PlayerSquadStatus::carries_across_squads`]) into this team.
    pub(crate) owning_roster: Option<rustc_hash::FxHashSet<u32>>,
    pub(crate) profile: CoachProfile,
    /// `is_force_match_selection` is a Main-team pin — the manager wants
    /// the player in the senior matchday XI. B / Reserve / U-team squad
    /// selection ignores it, so a U18 starlet flagged for the first team
    /// doesn't also override scoring on his youth team's match day.
    pub(crate) honor_force_selection: bool,
}

impl ScoringEngine {
    pub fn from_staff(staff: &Staff) -> Self {
        ScoringEngine {
            owning_roster: None,
            profile: CoachProfile::from_staff(staff),
            honor_force_selection: true,
        }
    }

    pub fn from_staff_for_team(staff: &Staff, is_main_team: bool) -> Self {
        ScoringEngine {
            owning_roster: None,
            profile: CoachProfile::from_staff(staff),
            honor_force_selection: is_main_team,
        }
    }

    pub(crate) fn owns_player(&self, player: &Player) -> bool {
        self.owning_roster
            .as_ref()
            .is_none_or(|ids| ids.contains(&player.id))
    }

    /// Lens-weighted skill composite using the coach's perception lens
    pub fn perceived_quality(&self, player: &Player) -> f32 {
        let lens = &self.profile.perception_lens;
        let t = &player.skills.technical;
        let m = &player.skills.mental;
        let p = &player.skills.physical;

        // Technical composite
        let atk_tech =
            (t.finishing + t.dribbling + t.crossing + t.first_touch + t.technique + t.long_shots)
                / 6.0;
        let def_tech = (t.tackling + t.marking + t.heading + t.passing) / 4.0;
        let tech_score = atk_tech * lens.attacking_focus + def_tech * (1.0 - lens.attacking_focus);

        // Mental composite
        let creative_mental =
            (m.flair + m.vision + m.composure + m.decisions + m.anticipation) / 5.0;
        let discipline_mental =
            (m.work_rate + m.determination + m.positioning + m.teamwork + m.concentration) / 5.0;
        let mental_score = creative_mental * lens.creativity_focus
            + discipline_mental * (1.0 - lens.creativity_focus);

        // Physical composite
        let explosive = (p.pace + p.acceleration + p.strength + p.jumping) / 4.0;
        let endurance = (p.stamina + p.natural_fitness + p.agility + p.balance) / 4.0;
        let physical_score =
            explosive * lens.physicality_focus + endurance * (1.0 - lens.physicality_focus);

        let skill_composite = tech_score * lens.technical_weight
            + mental_score * lens.mental_weight
            + physical_score * lens.physical_weight;

        // Position mastery dampened by tactical blindness
        let position_level = player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0) as f32;
        let position_contribution = position_level * (1.0 - self.profile.tactical_blindness * 0.5);

        let base = skill_composite * 0.75 + position_contribution * 0.25;

        // Form bonus amplified by recency_bias. Prefer the fast-moving EMA
        // (`load.form_rating`) when the player has accumulated form data;
        // fall back to the *regressed* season-average only for players
        // without a recent match rating (e.g. just arrived from another
        // club). Regression keeps a 9-app 8.2 prospect from being
        // selected ahead of a 30-app 7.4 starter.
        let raw_form_bonus = if player.load.form_rating > 0.0 {
            (player.load.form_rating - 6.5).clamp(-1.5, 1.5)
        } else if player.statistics.played + player.statistics.played_subs > 3 {
            let pos = player.position().position_group();
            (player.statistics.average_rating_realistic(pos) - 6.5).clamp(-1.5, 1.5)
        } else {
            0.0
        };
        let form_bonus = raw_form_bonus * (1.0 + self.profile.recency_bias * 0.8);

        // Attitude bleed
        let attitude_bleed = {
            let visible_effort =
                (player.skills.mental.work_rate + player.skills.mental.determination) / 2.0;
            (visible_effort - 10.0) * self.profile.attitude_weight * 0.15
        };

        // Condition factor
        let condition =
            (player.player_attributes.condition_percentage() as f32 / 100.0).clamp(0.5, 1.0);

        (base + form_bonus + attitude_bleed) * condition
    }

    /// Match readiness: condition + fitness + sharpness + physical_readiness
    pub fn match_readiness(&self, player: &Player) -> f32 {
        let condition = player.player_attributes.condition_percentage() as f32 / 100.0;
        let fitness = player.player_attributes.fitness as f32 / 10000.0;

        let days_since = player.player_attributes.days_since_last_match as f32;
        let sharpness = if days_since <= 3.0 {
            1.0
        } else if days_since <= 7.0 {
            0.95
        } else if days_since <= 14.0 {
            0.85
        } else if days_since <= 28.0 {
            0.70
        } else {
            0.55
        };

        let physical_readiness = player.skills.physical.match_readiness / 20.0;

        let raw_readiness = (condition * 0.35
            + fitness.clamp(0.0, 1.0) * 0.25
            + sharpness * 0.25
            + physical_readiness * 0.15)
            .clamp(0.0, 1.0);

        // Noise scaled by readiness_intuition
        let noise_scale = (1.0 - self.profile.readiness_intuition) * 0.25;
        let noise = self.profile.perception_noise(player.id, 0xFE57) * noise_scale;

        (raw_readiness + noise).clamp(0.0, 1.0) * 20.0
    }

    /// Training impression: blends visible effort with actual training performance.
    pub fn training_impression(&self, player: &Player) -> f32 {
        let visible_effort = (player.skills.mental.work_rate
            + player.skills.mental.determination
            + player.skills.mental.teamwork)
            / 3.0;

        let actual_performance = player.training.training_performance;

        let actual_weight = 0.30 + self.profile.judging_accuracy * 0.40;
        let blended = actual_performance * actual_weight + visible_effort * (1.0 - actual_weight);

        blended * (0.5 + self.profile.attitude_weight * 0.5)
    }

    /// Recent-workload penalty for squad rotation. Returns a non-positive
    /// bonus: zero for fresh players, down to roughly −6 for players on
    /// the edge of overload. Combined with selection scoring so managers
    /// naturally rotate through weeks of congested fixtures instead of
    /// flogging the same XI into the ground.
    ///
    /// The signal is the *worse* of:
    ///   * weekly minutes vs. FATIGUE_LOAD_THRESHOLD (legacy line)
    ///   * weekly physical load vs. PHYSICAL_LOAD_THRESHOLD (position-
    ///     weighted: 90 wingback minutes register heavier than 90 GK
    ///     minutes)
    ///   * recovery debt vs. RECOVERY_DEBT_HEAVY (deep tiredness even
    ///     when weekly minutes are low — e.g., a player who came off a
    ///     punishing midweek cup tie)
    ///
    /// Goalkeepers are protected from outfield-style rotation: a #1
    /// keeper plays every week in real football, so the load-based
    /// penalty is heavily damped for them.
    ///
    /// Friendlies don't rotate — preseason / testimonial XIs already
    /// feature a different player pool — so this returns 0 there.
    pub fn fatigue_penalty(&self, player: &Player, is_friendly: bool) -> f32 {
        if is_friendly {
            return 0.0;
        }
        let minutes_load = player.load.minutes_last_7;
        let physical_load = player.load.physical_load_7;
        let debt = player.load.recovery_debt;

        let minutes_t = ScoringCurves::ramp(minutes_load, FATIGUE_LOAD_THRESHOLD, FATIGUE_LOAD_DANGER);
        let physical_t = ScoringCurves::ramp(physical_load, PHYSICAL_LOAD_THRESHOLD, PHYSICAL_LOAD_DANGER);
        // Debt ramp: 0 at no debt, full penalty at 2× HEAVY threshold.
        let debt_t = ScoringCurves::ramp(debt, RECOVERY_DEBT_HEAVY, RECOVERY_DEBT_HEAVY * 2.0);

        let t = minutes_t.max(physical_t).max(debt_t);
        if t <= 0.0 {
            return 0.0;
        }

        let mut scale = 1.0 - self.profile.risk_tolerance * 0.4;
        // Goalkeepers don't rotate the way outfielders do.
        if player.position().position_group() == PlayerFieldPositionGroup::Goalkeeper {
            scale *= 0.4;
        }
        -(t * 3.0) * scale
    }

    /// Unified condition floor penalty
    pub fn condition_floor_penalty(&self, player: &Player, is_friendly: bool) -> f32 {
        let p = &self.profile;
        let condition_pct = player.player_attributes.condition_percentage() as f32;
        let condition_threshold = if is_friendly {
            25.0
        } else {
            40.0 - p.risk_tolerance * 8.0
        };
        if condition_pct < condition_threshold {
            let deficit = (condition_threshold - condition_pct) / condition_threshold;
            deficit * 40.0 * (1.0 - p.risk_tolerance * 0.3)
        } else {
            0.0
        }
    }

    /// Player reputation score (0..~2.5)
    pub fn reputation_score(&self, player: &Player) -> f32 {
        let p = &self.profile;

        let current_rep =
            (player.player_attributes.current_reputation as f32 / 3000.0).clamp(0.0, 1.0);
        let world_rep = (player.player_attributes.world_reputation as f32 / 1200.0).clamp(0.0, 1.0);
        let intl_factor =
            (player.player_attributes.international_apps as f32 / 50.0).clamp(0.0, 1.0);

        let raw_rep = current_rep * 0.4 + world_rep * 0.4 + intl_factor * 0.2;
        let rep_susceptibility = 1.0 - p.judging_accuracy * 0.5;

        raw_rep * rep_susceptibility * 2.5
    }

    /// Social preference only: rapport, personal relationship and conversations.
    /// Sporting memory is scored by the coach engine. Role dissatisfaction
    /// does not make an already omitted player less likely to get a chance.
    /// Bounded to -0.8..+0.6 so relationships shade close sporting decisions.
    pub fn relationship_score(&self, player: &Player, staff: &Staff, date: NaiveDate) -> f32 {
        CoachPlayerBond::social_selection_adjustment(player, staff, date)
    }

    /// Newcomer integration penalty
    pub fn newcomer_penalty(player: &Player, date: NaiveDate, profile: &CoachProfile) -> f32 {
        let transfer_date = match player.last_transfer_date {
            Some(d) => d,
            None => return 0.0,
        };

        let days_at_club = (date - transfer_date).num_days().max(0) as f32;
        let appearances = (player.statistics.played + player.statistics.played_subs) as f32;

        let rep_factor =
            (player.player_attributes.world_reputation as f32 / 1200.0).clamp(0.0, 1.0);
        let max_penalty = 3.5 * (1.0 - rep_factor * 0.77);
        let apps_to_integrate = 3.0 + (1.0 - rep_factor) * 5.0;

        let coach_speed = 1.0 + profile.risk_tolerance * 0.3 - profile.conservatism * 0.3
            + profile.judging_accuracy * 0.2;

        let app_factor = (appearances * coach_speed / apps_to_integrate).clamp(0.0, 1.0);

        let time_to_integrate = 30.0 + (1.0 - rep_factor) * 30.0;
        let time_factor = (days_at_club / time_to_integrate).clamp(0.0, 1.0);

        let integration = (app_factor * 0.7 + time_factor * 0.3).clamp(0.0, 1.0);

        max_penalty * (1.0 - integration)
    }

    /// Pairwise chemistry bonus (-1.2..+1.0). Sharper position-proximity
    /// weights so a CB-CB pair (1.0) clearly outweighs a striker-fullback
    /// pair (0.15) — defensive units feel rapport more than far-flung ones.
    /// Deep-disliked teammates in the same unit floor the score so the
    /// manager sees the friction.
    pub fn cohesion_bonus(
        &self,
        player: &Player,
        selected_players: &[&Player],
        slot_position: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
    ) -> f32 {
        if selected_players.is_empty() {
            return 0.0;
        }

        let p = &self.profile;
        let mut total = 0.0f32;
        let mut weight_sum = 0.0f32;
        let mut worst_same_unit_rel: Option<f32> = None;

        let player_pos = player.position();

        for teammate in selected_players {
            let teammate_pos = teammate.position();
            let teammate_group = teammate.position().position_group();

            // Sharper proximity weighting reflecting football positional units.
            let proximity_weight = ScoringCurves::position_proximity_weight(
                player_pos,
                slot_position,
                teammate_pos,
                slot_group,
                teammate_group,
            );

            let rel_quality = match player.relations.get_player(teammate.id) {
                Some(rel) => {
                    let level_norm = rel.level / 100.0;
                    let trust_norm = (rel.trust - 50.0) / 100.0;
                    let prof_norm = (rel.professional_respect - 50.0) / 100.0;
                    level_norm * 0.4 + trust_norm * 0.3 + prof_norm * 0.3
                }
                None => 0.0,
            };

            // Track worst same-unit relation — a deep dislike between two
            // CBs is worth more than the average pulls.
            if teammate_group == slot_group {
                worst_same_unit_rel = Some(match worst_same_unit_rel {
                    Some(prev) if prev <= rel_quality => prev,
                    _ => rel_quality,
                });
            }

            total += rel_quality * proximity_weight;
            weight_sum += proximity_weight;
        }

        if weight_sum == 0.0 {
            return 0.0;
        }

        let avg = total / weight_sum;
        let scale = 1.0 + p.conservatism * 0.3;
        let mut score = (avg * scale * 2.0).clamp(-1.2, 1.0);

        // Floor for severe same-unit dislike — even if every other pair is
        // cordial, two CBs at -50 should pull at least -0.4.
        if let Some(worst) = worst_same_unit_rel
            && worst <= -0.5
        {
            score = score.min(-0.4);
        }

        score.clamp(-1.2, 1.2)
    }

    /// Score for a specific tactical slot (starting XI selection)
    pub fn score_player_for_slot(
        &self,
        player: &Player,
        slot_position: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
        staff: &Staff,
        tactics: &Tactics,
        date: NaiveDate,
        is_friendly: bool,
        selected_players: &[&Player],
    ) -> f32 {
        self.score_player_for_slot_with_breakdown(
            player,
            slot_position,
            slot_group,
            staff,
            tactics,
            date,
            is_friendly,
            selected_players,
        )
        .0
    }

    /// Score with per-component breakdown. The total is identical to
    /// what `score_player_for_slot` returns — every existing caller
    /// keeps the same number. The breakdown is consumed by the squad
    /// selector to explain omissions in the player-events feed.
    pub fn score_player_for_slot_with_breakdown(
        &self,
        player: &Player,
        slot_position: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
        staff: &Staff,
        tactics: &Tactics,
        date: NaiveDate,
        is_friendly: bool,
        selected_players: &[&Player],
    ) -> (f32, SlotScoreBreakdown) {
        let p = &self.profile;
        let mut b = SlotScoreBreakdown {
            position_fit: SlotFit::position_fit_score(player, slot_position)
                * (0.20 * (1.0 - p.tactical_blindness * 0.3)),
            perceived_quality: self.perceived_quality(player) * (0.40 + p.judging_accuracy * 0.05),
            match_readiness: self.match_readiness(player) * (0.15 + p.conservatism * 0.05),
            condition_floor: -self.condition_floor_penalty(player, is_friendly),
            tactical_style: SlotFit::tactical_style_bonus(player, slot_position, tactics)
                * (0.05 * (1.0 - p.tactical_blindness * 0.5)),
            side_foot: SlotFit::side_foot_bonus(player, slot_position)
                * (0.6 * (1.0 - p.tactical_blindness * 0.3)),
            ..Default::default()
        };

        let rep = self.reputation_score(player);
        let rel = self.relationship_score(player, staff, date);
        b.reputation = rep;
        let rel_dampening = if rel < 0.0 {
            1.0
        } else {
            (1.0 - rep * 0.15).max(0.3)
        };
        b.coach_relationship = rel * rel_dampening;

        b.newcomer = -Self::newcomer_penalty(player, date, p);

        b.training_impression = (self.training_impression(player) - 10.0) * p.attitude_weight * 0.3;

        b.cohesion = self.cohesion_bonus(player, selected_players, slot_position, slot_group);

        b.force_selection = self.force_selection_bonus(player);

        // Out of his line — charged on what the player can actually play,
        // not on the single label his record happens to list first. A man
        // down for the slot's own shirt is not playing out of position,
        // however deep the arbitrary "primary" position ahead of it sits.
        if !PositionCoverage::of(&player.positions).covers_group(slot_group) {
            b.group_mismatch = -1.5;
        }

        let total = b.total();
        (total, b)
    }

    /// Manager override pinning a player into the starting XI. Returns a
    /// constant large enough to dominate every other signal (quality,
    /// fatigue, relationship, philosophy, …) so the DP slot assigner picks
    /// flagged players before anyone else when available. Pre-selection
    /// availability filters (injury / banned / suspended) are applied
    /// upstream in `available`, so this only fires for players who *can*
    /// play but might otherwise lose the contest on merit.
    pub fn force_selection_bonus(&self, player: &Player) -> f32 {
        if self.honor_force_selection && player.is_force_match_selection {
            1000.0
        } else {
            0.0
        }
    }

    /// What a want-away player's visible disaffection costs him. A man who
    /// has asked to go or is unhappy, whose morale and whose relationship
    /// with the manager are both poor, is marked down; a professional who
    /// wants out and still trains well is not. Nothing for a settled
    /// player, and nothing in a friendly.
    ///
    /// A player whose move is agreed is not scored here at all: he is
    /// withheld from the squad (see `EligibilityEvaluator`).
    pub fn want_away_adjustment(
        &self,
        player: &Player,
        staff: &Staff,
        date: NaiveDate,
        is_friendly: bool,
    ) -> f32 {
        let requested = player.statuses.has(PlayerStatusType::Req);
        let unhappy = player.statuses.has(PlayerStatusType::Unh);
        if is_friendly || !(requested || unhappy) {
            return 0.0;
        }
        let morale = player.happiness.morale;
        let relationship = self.relationship_score(player, staff, date);
        if morale >= 40.0 || relationship >= 0.0 {
            return 0.0;
        }
        let morale_deficit = ((40.0 - morale) / 40.0).clamp(0.0, 1.0);
        -(morale_deficit * 0.8 + (-relationship) * 0.6).min(2.0)
    }

    /// Risk of asking a player to start while physically fragile. This is
    /// separate from the hard availability gate: managers will sometimes
    /// risk a tired star in a final, but usually protect them in normal games.
    ///
    /// Now reads the richer load model:
    ///   * physical_load_7 (position-weighted) instead of raw minutes
    ///   * recovery_debt (deep tiredness flag)
    ///   * acute:chronic workload spike (sports-science danger zone)
    ///   * is_in_recovery() — Lmp players carry a big risk premium
    pub fn injury_risk_penalty(
        &self,
        player: &Player,
        match_importance: f32,
        is_friendly: bool,
    ) -> f32 {
        if is_friendly {
            return 0.0;
        }

        let condition = player.player_attributes.condition_percentage() as f32;
        let fitness = (player.player_attributes.fitness as f32 / 10000.0).clamp(0.0, 1.0);
        let natural_fitness = (player.skills.physical.natural_fitness / 20.0).clamp(0.0, 1.0);
        let physical_load_norm =
            (player.load.physical_load_7 / PHYSICAL_LOAD_DANGER).clamp(0.0, 1.8);
        let debt_norm = (player.load.recovery_debt / (RECOVERY_DEBT_HEAVY * 2.0)).clamp(0.0, 1.5);
        let matches_14 = player.load.matches_last_14() as f32;
        let spike = if player.load.is_workload_spike() {
            (player.load.workload_spike_ratio() - 1.0).clamp(0.0, 1.5)
        } else {
            0.0
        };

        let condition_risk = ((65.0 - condition) / 65.0).clamp(0.0, 1.0);
        let fitness_risk = 1.0 - fitness;
        let durability_risk = 1.0 - natural_fitness;
        let match_density_risk = ((matches_14 - 3.0) / 3.0).clamp(0.0, 1.0);

        let mut raw = condition_risk * 2.4
            + fitness_risk * 1.4
            + durability_risk * 0.8
            + physical_load_norm * 1.6
            + debt_norm * 1.4
            + match_density_risk * 1.2
            + spike * 1.8;

        // Recovery phase: starting a Lmp player is a coaching choice with
        // a real recurrence risk. Heavy premium so managers don't rush
        // returns unless match_importance forces their hand.
        if player.player_attributes.is_in_recovery() {
            raw += 4.5;
        }

        raw * Self::caution(match_importance) * (1.0 - self.profile.risk_tolerance * 0.35)
    }

    /// How much weight a coach gives tiredness and physical risk in a
    /// fixture: all of it when little is at stake, a quarter in a must-win.
    pub fn caution(match_importance: f32) -> f32 {
        (1.15 - match_importance).clamp(0.25, 1.10)
    }

    /// Overall quality score (bench selection)
    pub fn overall_quality(
        &self,
        player: &Player,
        staff: &Staff,
        tactics: &Tactics,
        date: NaiveDate,
        is_friendly: bool,
    ) -> f32 {
        let p = &self.profile;

        let quality = self.perceived_quality(player);
        let readiness = self.match_readiness(player);
        let primary_level = player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0) as f32;

        let mut score = quality * (0.40 + p.judging_accuracy * 0.05)
            + readiness * (0.15 + p.conservatism * 0.05)
            + primary_level * (0.20 * (1.0 - p.tactical_blindness * 0.3));

        score -= self.condition_floor_penalty(player, is_friendly);

        let rep = self.reputation_score(player);
        let rel = self.relationship_score(player, staff, date);
        score += rep;
        let rel_dampening = if rel < 0.0 {
            1.0
        } else {
            (1.0 - rep * 0.15).max(0.3)
        };
        score += rel * rel_dampening;

        let best_pos = SlotFit::best_tactical_position(player, tactics);
        if player.positions.get_level(best_pos) > 0 {
            score += 0.5;
        }

        score -= Self::newcomer_penalty(player, date, p);

        score += (self.training_impression(player) - 10.0) * p.attitude_weight * 0.3;

        score
    }

    /// Goalkeeper score — the level the staff see first, keeper-specific
    /// skills second, everything else a tiebreaker. `perceived_quality()`
    /// composes from outfield skills (finishing, dribbling, tackling,
    /// passing…) and never reads the goalkeeping block, so for a
    /// keeper-vs-keeper comparison it reflects the wrong attributes. This
    /// anchors on the observable level and adds a GK-specific composite
    /// that reads handling, reflexes, aerial ability and distribution.
    pub fn goalkeeper_score(
        &self,
        player: &Player,
        staff: &Staff,
        is_friendly: bool,
        date: NaiveDate,
    ) -> f32 {
        let ca = f32::from(AbilityEstimator::observable_level(player)) / 10.0;
        let gk_q = self.gk_perceived_quality(player);
        let gk_level = player.positions.get_level(PlayerPositionType::Goalkeeper) as f32;
        let readiness = self.match_readiness(player);

        let mut score = ca * 2.0 + gk_q * 1.0 + gk_level * 0.10 + readiness * 0.20;

        score -= self.condition_floor_penalty(player, is_friendly);

        score += self.reputation_score(player) * 0.30;
        score += self.relationship_score(player, staff, date) * 0.30;
        score += self.force_selection_bonus(player);

        score
    }

    /// Keeper-specific skill composite. Mirrors `perceived_quality` but
    /// built from the goalkeeping skill block plus the mental/physical
    /// attributes that actually matter for shot-stopping, crosses, and
    /// distribution. All inputs are on the 1-20 scale, so the result is
    /// 1-20 too — directly comparable to readiness and gk_level terms.
    fn gk_perceived_quality(&self, player: &Player) -> f32 {
        let gk = &player.skills.goalkeeping;
        let m = &player.skills.mental;
        let ph = &player.skills.physical;

        let shot_stopping = (gk.handling + gk.reflexes + gk.one_on_ones) / 3.0;
        let aerial = (gk.aerial_reach + gk.command_of_area + ph.jumping) / 3.0;
        let distribution = (gk.kicking + gk.throwing + gk.passing) / 3.0;
        let sweeper = (gk.rushing_out + gk.communication + m.decisions + m.anticipation) / 4.0;
        let keeper_mind = (m.concentration + m.positioning + m.composure + m.bravery) / 4.0;

        shot_stopping * 0.40
            + aerial * 0.20
            + keeper_mind * 0.20
            + sweeper * 0.10
            + distribution * 0.10
    }
}

/// How near two shirts stand, and how far a load is into its danger band.
struct ScoringCurves;

impl ScoringCurves {
    /// Sharper position-proximity weight for cohesion calculations.
    ///
    /// Football positional units rather than abstract groups:
    ///   * GK ↔ CB: 1.0 (set-piece communication, last-line trust)
    ///   * CB ↔ CB: 1.0 (back-line partnership)
    ///   * Fullback ↔ Winger same side: 0.9 (overlapping runs)
    ///   * CM/DM/AM cluster: 0.8 (midfield triangulation)
    ///   * Striker ↔ AM/winger: 0.7 (final-third combination)
    ///   * Adjacent groups: 0.4 fallback (better than the legacy 0.5
    ///     because it forces the calculation to lean on the unit pairs above)
    ///   * Distant unrelated roles: 0.15
    fn position_proximity_weight(
        player_pos: PlayerPositionType,
        slot_pos: PlayerPositionType,
        teammate_pos: PlayerPositionType,
        slot_group: PlayerFieldPositionGroup,
        teammate_group: PlayerFieldPositionGroup,
    ) -> f32 {
        use PlayerPositionType::*;

        // GK ↔ CB
        let gk_cb = |a: PlayerPositionType, b: PlayerPositionType| -> bool {
            matches!(a, Goalkeeper)
                && matches!(b, DefenderCenter | DefenderCenterLeft | DefenderCenterRight)
        };
        if gk_cb(slot_pos, teammate_pos) || gk_cb(teammate_pos, slot_pos) {
            return 1.0;
        }

        // CB ↔ CB
        let is_cb = |p: PlayerPositionType| -> bool {
            matches!(p, DefenderCenter | DefenderCenterLeft | DefenderCenterRight)
        };
        if is_cb(slot_pos) && is_cb(teammate_pos) {
            return 1.0;
        }

        // Fullback ↔ Winger same side
        let left_pair = |a: PlayerPositionType, b: PlayerPositionType| -> bool {
            matches!(a, DefenderLeft | WingbackLeft)
                && matches!(b, MidfielderLeft | AttackingMidfielderLeft | ForwardLeft)
        };
        let right_pair = |a: PlayerPositionType, b: PlayerPositionType| -> bool {
            matches!(a, DefenderRight | WingbackRight)
                && matches!(b, MidfielderRight | AttackingMidfielderRight | ForwardRight)
        };
        if left_pair(slot_pos, teammate_pos)
            || left_pair(teammate_pos, slot_pos)
            || right_pair(slot_pos, teammate_pos)
            || right_pair(teammate_pos, slot_pos)
        {
            return 0.9;
        }

        // Midfield cluster (CM / DM / AM, any flank)
        let is_mid_cluster = |p: PlayerPositionType| -> bool {
            matches!(
                p,
                MidfielderCenter
                    | MidfielderCenterLeft
                    | MidfielderCenterRight
                    | DefensiveMidfielder
                    | AttackingMidfielderCenter
                    | AttackingMidfielderLeft
                    | AttackingMidfielderRight
                    | MidfielderLeft
                    | MidfielderRight
            )
        };
        if is_mid_cluster(slot_pos) && is_mid_cluster(teammate_pos) {
            return 0.8;
        }

        // Striker ↔ AM / winger
        let is_striker = |p: PlayerPositionType| -> bool {
            matches!(p, Striker | ForwardCenter | ForwardLeft | ForwardRight)
        };
        let is_am_winger = |p: PlayerPositionType| -> bool {
            matches!(
                p,
                AttackingMidfielderLeft
                    | AttackingMidfielderRight
                    | AttackingMidfielderCenter
                    | MidfielderLeft
                    | MidfielderRight
            )
        };
        if (is_striker(slot_pos) && is_am_winger(teammate_pos))
            || (is_striker(teammate_pos) && is_am_winger(slot_pos))
        {
            return 0.7;
        }

        // Same group fallback (post-specific-pair).
        if slot_group == teammate_group {
            return 0.6;
        }

        // Adjacent groups — defenders↔midfielders or midfielders↔forwards.
        if SlotFit::is_adjacent_group(slot_group, teammate_group) {
            return 0.4;
        }

        // Distant pairs (e.g. striker↔fullback or GK↔striker).
        let _ = player_pos; // reserved for future per-player fine-tuning
        0.15
    }

    /// Linear ramp: 0.0 below `lo`, 1.0 at `hi`, allowed to overshoot up to
    /// 2.0 for "deep into the danger zone" signals.
    fn ramp(value: f32, lo: f32, hi: f32) -> f32 {
        if value <= lo {
            return 0.0;
        }
        let span = (hi - lo).max(1.0);
        ((value - lo) / span).min(2.0)
    }
}

#[cfg(test)]
mod relationship_score_tests {
    //! End-to-end test of the relation-store-mismatch fix.
    //!
    //! The legacy `relationship_score` read `staff.relations.get_player`
    //! while every relationship-update path wrote to
    //! `player.relations.update_staff_relationship` — the read and
    //! write stores were different objects, so the selection layer was
    //! effectively blind to every manager talk. These tests assert the
    //! read now sees the writes, and that the sign of the talk outcome
    //! propagates into the selection slot score.
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::staff::StaffStub;
    use crate::shared::fullname::FullName;
    use crate::{
        ChangeType, PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositions,
        PlayerSkills, RelationshipChange, Staff,
    };
    use chrono::NaiveDate;

    struct Fixture;

    impl Fixture {
        fn d() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        }

        fn build_player() -> Player {
            PlayerBuilder::new()
                .id(101)
                .full_name(FullName::new("Rel".into(), "Test".into()))
                .birth_date(NaiveDate::from_ymd_opt(1998, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap()
        }

        fn build_staff(id: u32) -> Staff {
            let mut s = StaffStub::build();
            s.id = id;
            s
        }
    }

    #[test]
    fn baseline_no_relation_reads_zero() {
        let player = Fixture::build_player();
        let staff = Fixture::build_staff(7);
        let engine = ScoringEngine::from_staff(&staff);
        assert_eq!(
            engine.relationship_score(&player, &staff, Fixture::d()),
            0.0,
            "no relation → neutral 0.0 score"
        );
    }

    #[test]
    fn successful_manager_talk_lifts_future_selection_score() {
        // The fix: writes to `player.relations.update_staff_relationship`
        // are now visible to `scoring.relationship_score(player, staff)`.
        // A successful manager talk should produce a strictly positive
        // selection adjustment.
        let mut player = Fixture::build_player();
        let staff = Fixture::build_staff(7);
        let engine = ScoringEngine::from_staff(&staff);

        let baseline = engine.relationship_score(&player, &staff, Fixture::d());
        let positive = RelationshipChange::positive(ChangeType::CoachingSuccess, 1.0);
        player
            .relations
            .update_staff_relationship(staff.id, positive, Fixture::d());

        let after = engine.relationship_score(&player, &staff, Fixture::d());
        assert!(
            after > baseline,
            "successful talk must raise selection score (baseline {} → after {})",
            baseline,
            after
        );
    }

    #[test]
    fn failed_manager_talk_drops_future_selection_score() {
        let mut player = Fixture::build_player();
        let staff = Fixture::build_staff(7);
        let engine = ScoringEngine::from_staff(&staff);

        let baseline = engine.relationship_score(&player, &staff, Fixture::d());
        // TacticalDisagreement is the canonical "failed talk" change
        // used by manager_credibility.rs (line 119).
        let negative = RelationshipChange::negative(ChangeType::TacticalDisagreement, 1.0);
        player
            .relations
            .update_staff_relationship(staff.id, negative, Fixture::d());

        let after = engine.relationship_score(&player, &staff, Fixture::d());
        assert!(
            after < baseline,
            "failed talk must drop selection score (baseline {} → after {})",
            baseline,
            after
        );
    }

    #[test]
    fn relationship_score_stays_inside_design_band() {
        // Even a swarm of positive updates can't push the slot bonus
        // beyond +0.6 — the relation can nudge close calls, never
        // override quality.
        let mut player = Fixture::build_player();
        let staff = Fixture::build_staff(7);
        let engine = ScoringEngine::from_staff(&staff);
        for _ in 0..20 {
            let positive = RelationshipChange::positive(ChangeType::CoachingSuccess, 2.0);
            player
                .relations
                .update_staff_relationship(staff.id, positive, Fixture::d());
        }
        let score = engine.relationship_score(&player, &staff, Fixture::d());
        assert!(
            (-0.8..=0.6).contains(&score),
            "score {} must stay inside design band -0.8..+0.6",
            score
        );
        assert!(
            score > 0.0,
            "stack of positive updates should still tilt positive"
        );
    }

    #[test]
    fn negative_relationship_stings_more_than_equivalent_positive() {
        // Spec rule: negative selection_trust deltas must hit harder
        // than the same-magnitude positive delta. The asymmetry lives
        // in `CoachPlayerBond::selection_adjustment` — positive ×0.85,
        // negative ×1.20, so a delta of ±0.2 produces magnitudes of
        // 0.20 * 0.85 vs 0.20 * 1.20 = 0.17 vs 0.24.
        //
        // We test the property directly on the bond rather than via the
        // staff-relation update path, because different `ChangeType`s
        // produce different magnitudes inside `StaffRelation::apply_change`
        // — using the relation pipeline would conflate two asymmetries.
        let mut high_trust_bond = CoachPlayerBond::default();
        let mut low_trust_bond = CoachPlayerBond::default();
        high_trust_bond.selection_trust = 0.7;
        low_trust_bond.selection_trust = 0.3;

        let positive_adj = high_trust_bond.selection_adjustment(1.4);
        let negative_adj = low_trust_bond.selection_adjustment(1.4);
        assert!(
            negative_adj < 0.0 && positive_adj > 0.0,
            "sanity: pos={} neg={}",
            positive_adj,
            negative_adj
        );
        assert!(
            negative_adj.abs() > positive_adj.abs() * 1.30,
            "negative delta must hit ≥1.30× harder than positive (pos_mag={:.4} neg_mag={:.4})",
            positive_adj.abs(),
            negative_adj.abs()
        );
    }

    #[test]
    fn selection_adjustment_neutral_bond_reads_zero() {
        // A neutral bond (selection_trust = 0.5) produces no signed
        // adjustment regardless of scale.
        let bond = CoachPlayerBond::default();
        assert_eq!(bond.selection_adjustment(1.4), 0.0);
        assert_eq!(bond.selection_adjustment(10.0), 0.0);
    }
}
