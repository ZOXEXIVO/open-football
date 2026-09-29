use crate::club::player::position::PlayerPositionType;
mod fitting;
mod weights;
use crate::club::player::position_weights::{
    PositionWeights, SK_ACCELERATION, SK_AGGRESSION, SK_AGILITY, SK_ANTICIPATION, SK_BALANCE,
    SK_BRAVERY, SK_COMPOSURE, SK_CONCENTRATION, SK_CORNERS, SK_CROSSING, SK_DECISIONS,
    SK_DETERMINATION, SK_DRIBBLING, SK_FINISHING, SK_FIRST_TOUCH, SK_FLAIR, SK_FREE_KICKS,
    SK_HEADING, SK_JUMPING, SK_LEADERSHIP, SK_LONG_SHOTS, SK_LONG_THROWS, SK_MARKING,
    SK_NATURAL_FITNESS, SK_OFF_THE_BALL, SK_PACE, SK_PASSING, SK_PENALTY_TAKING, SK_POSITIONING,
    SK_STAMINA, SK_STRENGTH, SK_TACKLING, SK_TEAMWORK, SK_TECHNIQUE, SK_VISION, SK_WORK_RATE,
    SKILL_COUNT,
};
use weights::AbilityWeights;

#[derive(Debug, Copy, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PlayerSkills {
    pub technical: Technical,
    pub mental: Mental,
    pub physical: Physical,
    pub goalkeeping: Goalkeeping,
}

/// Goalkeeper activity intensity for speed calculation.
/// GKs have low pace (60% of max_speed formula) but need explosive short-distance speed
/// for diving, catching, and shot-stopping. Agility and acceleration matter more.
#[derive(Debug, Clone, Copy)]
pub enum GoalkeeperSpeedContext {
    /// Off his feet, travelling on the line he pushed off along.
    ///
    /// Separate from `Explosive` because a dive is not a sprint that
    /// happens to be sideways: it is one push, and the ground it covers is
    /// bounded by how far a man can throw his own body. Measured on a real
    /// recording, keepers on the `Explosive` band travelled a median
    /// **4.1 m while airborne in 0.39 s** — 10.6 m/s, roughly double a real
    /// full-length dive, and fast enough that walking to the ball was
    /// always the better option, so he never dived at all.
    Dive,
    /// Preparing for save, jumping — explosive reactions
    Explosive,
    /// Catching, coming out, under pressure — active pursuit
    Active,
    /// Attentive, standing, returning — positioning
    Positioning,
    /// Walking, holding, distributing — minimal
    Casual,
}

impl PlayerSkills {
    /// Derive current_ability (1-200) from the average of all skills (1-20 each).
    /// Technical (14) + Mental (14) + Physical (8) averaged, then mapped to 1-200.
    pub fn calculate_ability(&self) -> u8 {
        let tech_avg = self.technical.average();
        let mental_avg = self.mental.average();
        let physical_avg = self.physical.average();
        let overall = (tech_avg + mental_avg + physical_avg) / 3.0;
        Self::skill_to_ability(overall)
    }

    /// Position-weighted ability calibrated against recorded CA/attribute
    /// pairs. Scoring weights are separate from the role-shaping weights:
    /// a generation preference is not an attribute's measured CA cost.
    pub fn calculate_ability_for_position(&self, position: PlayerPositionType) -> u8 {
        self.ability_score_for_position(position)
            .round()
            .clamp(1.0, 200.0) as u8
    }

    /// Continuous score for fitting. Keeping the unclamped value prevents
    /// endpoint plateaus from flattening a CA-200 player to all twenties.
    fn ability_score_for_position(&self, position: PlayerPositionType) -> f32 {
        let weights = AbilityWeights::for_position(position);
        let total: f32 = weights.iter().sum();
        let average: f32 = self
            .iter_all()
            .zip(weights)
            .map(|((_, value), weight)| value * weight)
            .sum::<f32>()
            / total;
        average * 19.0 - 110.0
    }

    /// Compute Σ(skill_i · w_i) / Σ(w_i) using the position-weight table.
    /// match_readiness is excluded because its slot weight is always 0.
    pub fn weighted_skill_average(&self, weights: &[f32; SKILL_COUNT]) -> f32 {
        let total = PositionWeights::total(weights);
        if total <= 0.0 {
            return 1.0;
        }
        let t = &self.technical;
        let m = &self.mental;
        let p = &self.physical;
        let mut acc = 0.0;
        acc += t.corners * weights[SK_CORNERS];
        acc += t.crossing * weights[SK_CROSSING];
        acc += t.dribbling * weights[SK_DRIBBLING];
        acc += t.finishing * weights[SK_FINISHING];
        acc += t.first_touch * weights[SK_FIRST_TOUCH];
        acc += t.free_kicks * weights[SK_FREE_KICKS];
        acc += t.heading * weights[SK_HEADING];
        acc += t.long_shots * weights[SK_LONG_SHOTS];
        acc += t.long_throws * weights[SK_LONG_THROWS];
        acc += t.marking * weights[SK_MARKING];
        acc += t.passing * weights[SK_PASSING];
        acc += t.penalty_taking * weights[SK_PENALTY_TAKING];
        acc += t.tackling * weights[SK_TACKLING];
        acc += t.technique * weights[SK_TECHNIQUE];
        acc += m.aggression * weights[SK_AGGRESSION];
        acc += m.anticipation * weights[SK_ANTICIPATION];
        acc += m.bravery * weights[SK_BRAVERY];
        acc += m.composure * weights[SK_COMPOSURE];
        acc += m.concentration * weights[SK_CONCENTRATION];
        acc += m.decisions * weights[SK_DECISIONS];
        acc += m.determination * weights[SK_DETERMINATION];
        acc += m.flair * weights[SK_FLAIR];
        acc += m.leadership * weights[SK_LEADERSHIP];
        acc += m.off_the_ball * weights[SK_OFF_THE_BALL];
        acc += m.positioning * weights[SK_POSITIONING];
        acc += m.teamwork * weights[SK_TEAMWORK];
        acc += m.vision * weights[SK_VISION];
        acc += m.work_rate * weights[SK_WORK_RATE];
        acc += p.acceleration * weights[SK_ACCELERATION];
        acc += p.agility * weights[SK_AGILITY];
        acc += p.balance * weights[SK_BALANCE];
        acc += p.jumping * weights[SK_JUMPING];
        acc += p.natural_fitness * weights[SK_NATURAL_FITNESS];
        acc += p.pace * weights[SK_PACE];
        acc += p.stamina * weights[SK_STAMINA];
        acc += p.strength * weights[SK_STRENGTH];
        acc / total
    }

    /// Empirical CA scale. Complete source profiles fit a substantially
    /// steeper slope than mapping all-1 to CA 1 and all-20 to CA 200.
    /// Clamping keeps weak/extreme profiles inside the game's 1..200 scale.
    fn skill_to_ability(avg: f32) -> u8 {
        (avg * 19.0 - 110.0).round().clamp(1.0, 200.0) as u8
    }

    /// Skill baseline corresponding to an ability budget. Shared by
    /// generation and development so they use the same scale as CA scoring.
    pub fn ability_skill_level(ability: u8) -> f32 {
        (ability.clamp(1, 200) as f32 + 110.0) / 19.0
    }

    /// Level `L` at which the profile `L × shape` (registry order) scores
    /// `ability` for this position. Development shapes skills by role, CA
    /// prices them by measured cost; sized by the flat level alone, a role
    /// whose shape under-weights its costly attributes never reached its PA.
    pub fn shaped_skill_level(position: PlayerPositionType, ability: u8, shape: &[f32; 50]) -> f32 {
        let weights = AbilityWeights::for_position(position);
        let share = weights.iter().zip(shape).map(|(w, s)| w * s).sum::<f32>()
            / weights.iter().sum::<f32>();
        Self::ability_skill_level(ability) / share
    }

    /// Build a flat skill set (every attribute equal) whose visible ability —
    /// [`Self::calculate_ability_for_position`] for *any* position, and
    /// [`Self::calculate_ability`] — evaluates to `target` on the 1..200
    /// scale. The inverse of [`Self::skill_to_ability`]; the canonical way to
    /// construct a synthetic player of a known, *visible* level without
    /// touching the hidden `current_ability` digit.
    pub fn flat_for_ability(target: u8) -> PlayerSkills {
        let v = Self::ability_skill_level(target);
        let mut skills = PlayerSkills::default();
        skills.technical.raise_floor(v);
        skills.mental.raise_floor(v);
        skills.physical.raise_floor(v);
        skills.goalkeeping.raise_floor(v);
        skills
    }

    /// The ends of the `pace` band, in units per physics tick.
    pub const MIN_MAX_SPEED: f32 = 0.36;
    pub const MAX_MAX_SPEED: f32 = 0.63;

    /// Fresh top speed in units/tick — 1u = 0.125 m on a 100 Hz tick, so
    /// `pace` = 1 is 0.36 u/tick (4.5 m/s) and `pace` = 20 is 0.63 (7.9 m/s).
    ///
    /// The band is ~0.8× real top-end speed (Mbappé ~10.5 m/s) and stays
    /// that way deliberately: with the engine's ball velocities the wider
    /// 0.48–0.84 band made outfield play frantic — defenders closing an
    /// attacker down in half a second, waypoint cycles flickering.
    ///
    /// **`pace` alone.** `acceleration` and `agility` used to take 0.2 and
    /// 0.1 of the blend, which counted both of them twice: since the
    /// velocity ramp landed, `acceleration` owns the burst budget
    /// (`MovementEffort::accel_budget`) and `agility` the braking /
    /// change-of-direction multiplier on it. Blending them in here also
    /// diluted the one attribute whose name means top speed — a squad's
    /// realised spread is ~10% narrower under the blend than under `pace`,
    /// because generated physicals correlate and the blend averages them
    /// back toward the squad mean.
    pub fn max_speed(&self) -> f32 {
        let pace01 = ((self.physical.pace - 1.0) / 19.0).clamp(0.0, 1.0);
        Self::MIN_MAX_SPEED + pace01 * (Self::MAX_MAX_SPEED - Self::MIN_MAX_SPEED)
    }

    /// Top speed as the legs are RIGHT NOW — [`Self::max_speed`] shaded by
    /// how drained the player is, with `stamina` deciding how much of the
    /// drain he actually pays for.
    ///
    /// The contract, at the ~65% condition outfielders finish a match on
    /// (measured, 60 harness fixtures at level 14): **~3.5% off top speed
    /// for `stamina` 20, ~7% for the population mean, ~12.5% for `stamina`
    /// 1** — against real full-time peak-sprint losses of a few percent
    /// for a conditioned player and about double that for an unconditioned
    /// one. Fresh legs pay nothing, exactly.
    ///
    /// Two things the previous linear form got wrong:
    ///
    /// * it spread its whole response over 100%→0% condition, but a match
    ///   only ever travels 100%→~65%, so barely a third of the curve was
    ///   ever reached and full time cost 6.7% regardless of who was
    ///   running. The `sqrt` puts the response in the band that is
    ///   actually visited, which is also how legs really go — the first
    ///   quarter of the tank costs more than the last;
    /// * `stamina` moved that figure by 2.8 points across its whole 1..20
    ///   range, i.e. the attribute did not reach movement at all. It now
    ///   owns most of the band, so a low-stamina defender is genuinely
    ///   walked away from late on while an elite-stamina one finishes near
    ///   his own top speed.
    ///
    /// The second fatigue channel is [`MovementEffort::self_pacing`],
    /// which shortens the efforts he is willing to make rather than the
    /// speed he is able to reach.
    ///
    /// Bounded by construction in `1 - max_reduction ..= 1`, so nothing
    /// here needs a clamp.
    ///
    /// [`MovementEffort::self_pacing`]: crate::r#match::MovementEffort
    pub fn max_speed_with_condition(&self, condition: i16) -> f32 {
        let condition_pct = (condition as f32 / 10000.0).clamp(0.0, 1.0);
        let stamina01 = (self.physical.stamina / 20.0).clamp(0.0, 1.0);

        let max_reduction = 0.22 - stamina01 * 0.16;
        let condition_factor = 1.0 - max_reduction * (1.0 - condition_pct).sqrt();

        self.max_speed() * condition_factor
    }

    /// Calculate maximum speed for a goalkeeper with state-dependent boost.
    /// GKs need explosive speed from agility/acceleration rather than raw pace.
    /// Boosts halved relative to the prior values because the base
    /// `max_speed` was bumped ~1.9× to match real-world sprint speed —
    /// the old multipliers compensated for an undersized base and would
    /// otherwise produce 25+ m/s GK lateral movement.
    ///   Dive:      0.85–1.20× → one push, 4.5–6.5 m/s under the body
    ///   Explosive: 0.95–1.25× → the set, the punch, the standing leap
    ///   Active:    0.85–1.5× → typical GK chase speed
    ///   Positioning: 0.75–1.0× → tracking play, reading the game
    ///   Casual:    0.65× → idle/recovery
    pub fn goalkeeper_max_speed(
        &self,
        condition: i16,
        speed_context: GoalkeeperSpeedContext,
    ) -> f32 {
        let base = self.max_speed_with_condition(condition);

        let agility = self.physical.agility / 20.0;
        let acceleration = self.physical.acceleration / 20.0;

        let boost = match speed_context {
            // 1.00-1.45 × base → 5.3 m/s for a heavy-footed keeper and 8.1
            // for an athletic one. Over the ~0.45 s a dive hangs that is
            // 2.4-3.6 m of ground under the body: a full-length dive, and
            // still well under the **4.1 m in 0.39 s** measured off a
            // recording before this band existed, which is not a dive, it is
            // a man being fired out of something.
            //
            // ⚠ Not lower. 0.85-1.20 was tried and cost ~4 points of
            // saves/on-target on its own, because `SaveModel`'s reach was
            // calibrated against a keeper who covered that 4.1 m — the dive
            // travel and the reach are one budget split across two models,
            // and cutting one without the other just shrinks the goal he
            // defends.
            GoalkeeperSpeedContext::Dive => 1.0 + agility * 0.25 + acceleration * 0.20,
            // ⚠ **1.0-2.0 is 4.5-15.7 m/s and that is NOT a goalkeeper.**
            // Measured on a recording, his p99 ground speed was 11 m/s —
            // faster than any outfielder in the game, sideways — and
            // `PreparingForSave` sits in this band, so that is what "setting
            // himself" looked like.
            //
            // Cutting it to 0.95-1.25 was tried and REVERTED (2026-08-17):
            // saves/on-target 56% → 50% and +0.4 goals a match, because this
            // band is not doing the job its name says. It is not the save —
            // `KeeperShotReaction` caps him to a set keeper's shuffle for the
            // whole of any shot in flight — it is how he REPOSITIONS during
            // the build-up, on a branch whose shot supply is still twice
            // real. Fixing it properly means making `KeeperRestPosition` and
            // `KeeperSetPosition` good enough that he does not need to
            // sprint, which is a positioning problem, not a speed one.
            GoalkeeperSpeedContext::Explosive => 1.0 + agility * 0.5 + acceleration * 0.5,
            GoalkeeperSpeedContext::Active => 0.85 + agility * 0.4 + acceleration * 0.25,
            GoalkeeperSpeedContext::Positioning => 0.75 + agility * 0.25,
            GoalkeeperSpeedContext::Casual => 0.65,
        };

        base * boost
    }
}

#[derive(Debug, Copy, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Technical {
    pub corners: f32,
    pub crossing: f32,
    pub dribbling: f32,
    pub finishing: f32,
    pub first_touch: f32,
    pub free_kicks: f32,
    pub heading: f32,
    pub long_shots: f32,
    pub long_throws: f32,
    pub marking: f32,
    pub passing: f32,
    pub penalty_taking: f32,
    pub tackling: f32,
    pub technique: f32,
}

impl Technical {
    pub fn average(&self) -> f32 {
        (self.corners
            + self.crossing
            + self.dribbling
            + self.finishing
            + self.first_touch
            + self.free_kicks
            + self.heading
            + self.long_shots
            + self.long_throws
            + self.marking
            + self.passing
            + self.penalty_taking
            + self.tackling
            + self.technique)
            / 14.0
    }

    pub fn raise_floor(&mut self, min: f32) {
        self.corners = self.corners.max(min);
        self.crossing = self.crossing.max(min);
        self.dribbling = self.dribbling.max(min);
        self.finishing = self.finishing.max(min);
        self.first_touch = self.first_touch.max(min);
        self.free_kicks = self.free_kicks.max(min);
        self.heading = self.heading.max(min);
        self.long_shots = self.long_shots.max(min);
        self.long_throws = self.long_throws.max(min);
        self.marking = self.marking.max(min);
        self.passing = self.passing.max(min);
        self.penalty_taking = self.penalty_taking.max(min);
        self.tackling = self.tackling.max(min);
        self.technique = self.technique.max(min);
    }
}

#[derive(Debug, Copy, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Mental {
    pub aggression: f32,
    pub anticipation: f32,
    pub bravery: f32,
    pub composure: f32,
    pub concentration: f32,
    pub decisions: f32,
    pub determination: f32,
    pub flair: f32,
    pub leadership: f32,
    pub off_the_ball: f32,
    pub positioning: f32,
    pub teamwork: f32,
    pub vision: f32,
    pub work_rate: f32,
}

impl Mental {
    pub fn average(&self) -> f32 {
        (self.aggression
            + self.anticipation
            + self.bravery
            + self.composure
            + self.concentration
            + self.decisions
            + self.determination
            + self.flair
            + self.leadership
            + self.off_the_ball
            + self.positioning
            + self.teamwork
            + self.vision
            + self.work_rate)
            / 14.0
    }

    pub fn raise_floor(&mut self, min: f32) {
        self.aggression = self.aggression.max(min);
        self.anticipation = self.anticipation.max(min);
        self.bravery = self.bravery.max(min);
        self.composure = self.composure.max(min);
        self.concentration = self.concentration.max(min);
        self.decisions = self.decisions.max(min);
        self.determination = self.determination.max(min);
        self.flair = self.flair.max(min);
        self.leadership = self.leadership.max(min);
        self.off_the_ball = self.off_the_ball.max(min);
        self.positioning = self.positioning.max(min);
        self.teamwork = self.teamwork.max(min);
        self.vision = self.vision.max(min);
        self.work_rate = self.work_rate.max(min);
    }
}

#[derive(Debug, Copy, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Physical {
    pub acceleration: f32,
    pub agility: f32,
    pub balance: f32,
    pub jumping: f32,
    pub natural_fitness: f32,
    pub pace: f32,
    pub stamina: f32,
    pub strength: f32,

    pub match_readiness: f32,
}

impl Physical {
    pub fn average(&self) -> f32 {
        (self.acceleration
            + self.agility
            + self.balance
            + self.jumping
            + self.natural_fitness
            + self.pace
            + self.stamina
            + self.strength)
            / 8.0
    }

    pub fn raise_floor(&mut self, min: f32) {
        self.acceleration = self.acceleration.max(min);
        self.agility = self.agility.max(min);
        self.balance = self.balance.max(min);
        self.jumping = self.jumping.max(min);
        self.natural_fitness = self.natural_fitness.max(min);
        self.pace = self.pace.max(min);
        self.stamina = self.stamina.max(min);
        self.strength = self.strength.max(min);
    }
}

#[derive(Debug, Copy, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Goalkeeping {
    pub aerial_reach: f32,
    pub command_of_area: f32,
    pub communication: f32,
    pub eccentricity: f32,
    pub first_touch: f32,
    pub handling: f32,
    pub kicking: f32,
    pub one_on_ones: f32,
    pub passing: f32,
    pub punching: f32,
    pub reflexes: f32,
    pub rushing_out: f32,
    pub throwing: f32,
}

impl Goalkeeping {
    pub fn average(&self) -> f32 {
        (self.aerial_reach
            + self.command_of_area
            + self.communication
            + self.eccentricity
            + self.first_touch
            + self.handling
            + self.kicking
            + self.one_on_ones
            + self.passing
            + self.punching
            + self.reflexes
            + self.rushing_out
            + self.throwing)
            / 13.0
    }

    pub fn raise_floor(&mut self, min: f32) {
        self.aerial_reach = self.aerial_reach.max(min);
        self.command_of_area = self.command_of_area.max(min);
        self.communication = self.communication.max(min);
        self.eccentricity = self.eccentricity.max(min);
        self.first_touch = self.first_touch.max(min);
        self.handling = self.handling.max(min);
        self.kicking = self.kicking.max(min);
        self.one_on_ones = self.one_on_ones.max(min);
        self.passing = self.passing.max(min);
        self.punching = self.punching.max(min);
        self.reflexes = self.reflexes.max(min);
        self.rushing_out = self.rushing_out.max(min);
        self.throwing = self.throwing.max(min);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_technical_average() {
        let technical = Technical {
            corners: 10.0,
            crossing: 20.0,
            dribbling: 30.0,
            finishing: 40.0,
            first_touch: 50.0,
            free_kicks: 60.0,
            heading: 70.0,
            long_shots: 80.0,
            long_throws: 90.0,
            marking: 100.0,
            passing: 110.0,
            penalty_taking: 120.0,
            tackling: 130.0,
            technique: 140.0,
        };
        assert_eq!(technical.average(), 75.0); // (10 + 20 + 30 + 40 + 50 + 60 + 70 + 80 + 90 + 100 + 110 + 120 + 130 + 140) / 14
    }

    #[test]
    fn test_mental_average() {
        let mental = Mental {
            aggression: 10.0,
            anticipation: 20.0,
            bravery: 30.0,
            composure: 40.0,
            concentration: 50.0,
            decisions: 60.0,
            determination: 70.0,
            flair: 80.0,
            leadership: 90.0,
            off_the_ball: 100.0,
            positioning: 110.0,
            teamwork: 120.0,
            vision: 130.0,
            work_rate: 140.0,
        };

        assert_eq!(mental.average(), 75.0); // (10 + 20 + 30 + 40 + 50 + 60 + 70 + 80 + 90 + 100 + 110 + 120 + 130 + 140) / 14
    }

    #[test]
    fn test_physical_average() {
        let physical = Physical {
            acceleration: 10.0,
            agility: 20.0,
            balance: 30.0,
            jumping: 40.0,
            natural_fitness: 50.0,
            pace: 60.0,
            stamina: 70.0,
            strength: 80.0,
            match_readiness: 90.0,
        };
        assert_eq!(physical.average(), 45.0); // (10 + 20 + 30 + 40 + 50 + 60 + 70 + 80) / 8
    }
}
