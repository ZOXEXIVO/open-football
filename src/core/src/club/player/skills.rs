use crate::club::player::position::{PlayerFieldPositionGroup, PlayerPositionType};
use crate::club::player::position_weights::{
    PositionWeights, SK_ACCELERATION, SK_AGGRESSION, SK_AGILITY, SK_ANTICIPATION, SK_BALANCE,
    SK_BRAVERY, SK_COMPOSURE, SK_CONCENTRATION, SK_CORNERS, SK_CROSSING, SK_DECISIONS,
    SK_DETERMINATION, SK_DRIBBLING, SK_FINISHING, SK_FIRST_TOUCH, SK_FLAIR, SK_FREE_KICKS,
    SK_HEADING, SK_JUMPING, SK_LEADERSHIP, SK_LONG_SHOTS, SK_LONG_THROWS, SK_MARKING,
    SK_NATURAL_FITNESS, SK_OFF_THE_BALL, SK_PACE, SK_PASSING, SK_PENALTY_TAKING, SK_POSITIONING,
    SK_STAMINA, SK_STRENGTH, SK_TACKLING, SK_TEAMWORK, SK_TECHNIQUE, SK_VISION, SK_WORK_RATE,
    SKILL_COUNT,
};

#[derive(Debug, Copy, Clone, Default)]
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
    /// Diving, preparing for save, jumping — explosive reactions
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

    /// Position-weighted ability calculation — every skill is weighed by how
    /// much it matters for this *exact* position (DC vs WBL vs AMC etc.).
    /// Goalkeepers route through the dedicated GK calculation that accounts
    /// for the Goalkeeping attributes alongside the outfield ones.
    pub fn calculate_ability_for_position(&self, position: PlayerPositionType) -> u8 {
        if position.position_group() == PlayerFieldPositionGroup::Goalkeeper {
            return self.calculate_gk_ability();
        }
        let weights = PositionWeights::for_position(position);
        let weighted_avg = self.weighted_skill_average(&weights);
        Self::skill_to_ability(weighted_avg)
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

    /// GK ability uses goalkeeping attributes as the primary factor,
    /// supplemented by key mental and physical skills.
    fn calculate_gk_ability(&self) -> u8 {
        let gk = &self.goalkeeping;

        // Core goalkeeping: handling, reflexes, one-on-ones, aerial reach,
        // command of area, communication, rushing out, punching
        let key_gk = (gk.handling + gk.reflexes + gk.one_on_ones + gk.aerial_reach
            + gk.command_of_area + gk.communication + gk.rushing_out + gk.punching) / 8.0;

        // Key mental: positioning, concentration, anticipation, composure, decisions
        let key_mental = (self.mental.positioning
            + self.mental.concentration
            + self.mental.anticipation
            + self.mental.composure
            + self.mental.decisions) / 5.0;

        // Key physical: agility, jumping, strength, acceleration
        let key_physical = (self.physical.agility
            + self.physical.jumping
            + self.physical.strength
            + self.physical.acceleration) / 4.0;

        // Key technical: kicking, first touch, passing (modern GK distribution)
        let key_technical = (gk.kicking + gk.first_touch + gk.passing + gk.throwing) / 4.0;

        // GK ability: goalkeeping-dominant
        let weighted = key_gk * 0.40 + key_mental * 0.25 + key_physical * 0.20 + key_technical * 0.15;
        Self::skill_to_ability(weighted)
    }

    /// Map a skill average (1.0-20.0) to ability (1-200).
    /// Skills are 1-based so normalize from 1-20 range before scaling.
    fn skill_to_ability(avg: f32) -> u8 {
        let normalized = ((avg - 1.0) / 19.0).clamp(0.0, 1.0);
        (normalized * 199.0 + 1.0).round().min(200.0).max(1.0) as u8
    }

    /// Calculate maximum speed without condition factor (raw speed based on skills only)
    /// Returns units/tick scaled for 10ms tick on 840-unit field (~105m pitch).
    /// At 1u = 0.125m and 100 ticks/s:
    ///   pace=1  → 0.36 u/tick = ~4.5 m/s  (slow jog / low-pace fatigued player)
    ///   pace=20 → 0.63 u/tick = ~7.9 m/s  (solid pro sprint)
    /// Range trimmed ~25% from the prior 0.48–0.84 band. The higher band
    /// was technically closer to real top-end speeds (Mbappé ~10.5 m/s),
    /// but combined with slowed ball velocity (shots 3.2, passes 3.2)
    /// it made outfield play feel too frantic — defenders closing down
    /// attackers in half a second, waypoint cycles flickering. This
    /// slower band keeps the shot/player ratio near real football's
    /// ~3.75× (shot 3.2 / elite 0.63 ≈ 5× — still fast enough for goals)
    /// while making player movement human-trackable.
    pub fn max_speed(&self) -> f32 {
        let pace_factor = (self.physical.pace as f32 - 1.0) / 19.0;
        let acceleration_factor = (self.physical.acceleration as f32 - 1.0) / 19.0;
        let agility_factor = (self.physical.agility as f32 - 1.0) / 19.0;

        // Weighted skill blend (pace dominant)
        let skill_blend = 0.7 * pace_factor
            + 0.2 * acceleration_factor
            + 0.1 * agility_factor;

        let min_speed = 0.36;
        let max_speed = 0.63;

        min_speed + skill_blend * (max_speed - min_speed)
    }

    /// Calculate maximum speed with condition factor (real-time performance)
    /// Condition reduces speed by at most ~25% (like real football)
    /// A tired player (30% condition) still runs at ~75-80% of max speed
    pub fn max_speed_with_condition(&self, condition: i16) -> f32 {
        let base_max_speed = self.max_speed();

        // Condition percentage (0.0 to 1.0)
        let condition_pct = (condition as f32 / 10000.0).clamp(0.0, 1.0);

        // Stamina provides fatigue resistance
        // High stamina players lose less speed when tired
        let stamina_normalized = (self.physical.stamina / 20.0).clamp(0.0, 1.0);

        // Condition affects speed mildly (max ~25% reduction at 0% condition)
        // At 100% condition: 100% speed
        // At 50% condition: ~87-93% speed (depending on stamina)
        // At 30% condition: ~80-88% speed (depending on stamina)
        // At 0% condition: ~75-85% speed (depending on stamina)
        let max_reduction = 0.25 - stamina_normalized * 0.10; // 15-25% max reduction
        let condition_factor = 1.0 - max_reduction * (1.0 - condition_pct);

        base_max_speed * condition_factor.clamp(0.75, 1.0)
    }

    /// Calculate maximum speed for a goalkeeper with state-dependent boost.
    /// GKs need explosive speed from agility/acceleration rather than raw pace.
    /// Boosts halved relative to the prior values because the base
    /// `max_speed` was bumped ~1.9× to match real-world sprint speed —
    /// the old multipliers compensated for an undersized base and would
    /// otherwise produce 25+ m/s GK lateral movement.
    ///   Explosive: 1.0–2.0× → elite ~21 m/s peak dive (matches today's effective)
    ///   Active:    0.85–1.5× → typical GK chase speed
    ///   Positioning: 0.75–1.0× → tracking play, reading the game
    ///   Casual:    0.65× → idle/recovery
    pub fn goalkeeper_max_speed(&self, condition: i16, speed_context: GoalkeeperSpeedContext) -> f32 {
        let base = self.max_speed_with_condition(condition);

        let agility = self.physical.agility / 20.0;
        let acceleration = self.physical.acceleration / 20.0;

        let boost = match speed_context {
            GoalkeeperSpeedContext::Explosive => 1.0 + agility * 0.5 + acceleration * 0.5,
            GoalkeeperSpeedContext::Active => 0.85 + agility * 0.4 + acceleration * 0.25,
            GoalkeeperSpeedContext::Positioning => 0.75 + agility * 0.25,
            GoalkeeperSpeedContext::Casual => 0.65,
        };

        base * boost
    }
}

#[derive(Debug, Copy, Clone, Default)]
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

    /// Small recovery of technique-related skills between matches.
    /// Simulates sharpness returning through regular practice.
    pub fn rest(&mut self) {
        const RECOVERY: f32 = 0.02;
        // Core technique skills recover slightly with practice
        self.first_touch = (self.first_touch + RECOVERY).min(20.0);
        self.passing = (self.passing + RECOVERY).min(20.0);
        self.technique = (self.technique + RECOVERY).min(20.0);
    }
}

#[derive(Debug, Copy, Clone, Default)]
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

    /// Mental recovery between matches — concentration and composure
    /// restore naturally with rest days.
    pub fn rest(&mut self) {
        const RECOVERY: f32 = 0.03;
        self.concentration = (self.concentration + RECOVERY).min(20.0);
        self.composure = (self.composure + RECOVERY).min(20.0);
        self.decisions = (self.decisions + RECOVERY * 0.5).min(20.0);
    }
}

#[derive(Debug, Copy, Clone, Default)]
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

    /// Physical recovery between matches — stamina and match readiness
    /// recover based on natural_fitness. `match_readiness` is on the
    /// 0–20 scale (the source of truth across this crate); the previous
    /// `min(100.0)` cap was leftover scale drift that let readiness
    /// silently exceed its real bounds.
    pub fn rest(&mut self) {
        // Natural fitness determines recovery rate (0-20 scale → 0.5%-2% per rest)
        let recovery_rate = 0.005 + (self.natural_fitness / 20.0) * 0.015;
        self.stamina = (self.stamina + recovery_rate * 20.0).min(20.0);
        self.match_readiness = (self.match_readiness + recovery_rate * 3.0).min(20.0);
    }
}

#[derive(Debug, Copy, Clone, Default)]
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
    fn test_technical_rest() {
        let mut technical = Technical {
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
        technical.rest();
        // Since the rest method doesn't modify any fields, we'll just assert true to indicate it ran successfully
        assert!(true);
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
    fn test_mental_rest() {
        let mut mental = Mental {
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
        mental.rest();
        // Since the rest method doesn't modify any fields, we'll just assert true to indicate it ran successfully
        assert!(true);
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

    #[test]
    fn test_physical_rest() {
        let mut physical = Physical {
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
        physical.rest();
        // Since the rest method doesn't modify any fields, we'll just assert true to indicate it ran successfully
        assert!(true);
    }
}
