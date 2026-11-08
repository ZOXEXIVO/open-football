//! A stubbed man's stat line, drawn in real-football units.
//!
//! The per-90 means are the references `rating/volume.rs` converts the
//! engine's counters to, so the rating model reads these lines as it reads
//! a normalised engine line. Where `volume.rs` states no reference (blocks,
//! clearances, pressures, passing volume) the means follow the season
//! fixtures in `rating/season_tests.rs`, which the rating bands are pinned on.

use super::performance::StubPerformance;
use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::result::PlayerMatchEndStats;
use crate::r#match::{MatchPlayer, MatchRng};

/// Counts drawn from a rate.
pub(super) struct StubDraw<'a> {
    pub rng: &'a MatchRng,
}

impl StubDraw<'_> {
    /// Past this mean a count is drawn from its normal approximation: pass
    /// volumes would otherwise cost one uniform draw per pass.
    const KNUTH_LIMIT: f32 = 10.0;

    /// Poisson: Knuth's draw, exact and cheap at football's event rates.
    pub(super) fn count(&self, mean: f32) -> u16 {
        if mean >= Self::KNUTH_LIMIT {
            return (mean + mean.sqrt() * self.bell()).round().max(0.0) as u16;
        }
        let floor = (-mean).exp();
        let mut product = self.rng.unit_f32();
        let mut count = 0;
        while product > floor {
            product *= self.rng.unit_f32();
            count += 1;
        }
        count
    }

    /// How many of `tries` come off at `chance` each.
    pub(super) fn successes(&self, tries: u16, chance: f32) -> u16 {
        (0..tries).filter(|_| self.rng.unit_f32() < chance).count() as u16
    }

    /// Bell-shaped, zero mean, unit variance, bounded at ±3.
    fn bell(&self) -> f32 {
        (self.rng.unit_f32() + self.rng.unit_f32() + self.rng.unit_f32() - 1.5) / 0.5
    }
}

/// Per-90 volumes for one position group, at level with the opposition.
struct GroupVolumes {
    shots_saved: f32,
    shots_wide: f32,
    xg_per_shot: f32,
    passes: f32,
    pass_completion: f32,
    key_passes: f32,
    passes_into_box: f32,
    crosses: f32,
    cross_completion: f32,
    progressive_passes: f32,
    progressive_carries: f32,
    dribbles: f32,
    dribble_success: f32,
    tackles: f32,
    interceptions: f32,
    blocks: f32,
    clearances: f32,
    own_box_clearances: f32,
    pressures: f32,
    pressure_success: f32,
    high_pressure_share: f32,
    offsides: f32,
    miscontrols: f32,
    heavy_touches: f32,
}

impl GroupVolumes {
    const DEFENDER: GroupVolumes = GroupVolumes {
        shots_saved: 0.09,
        shots_wide: 0.32,
        xg_per_shot: 0.07,
        passes: 38.0,
        pass_completion: 0.82,
        key_passes: 0.40,
        passes_into_box: 0.40,
        crosses: 0.80,
        cross_completion: 0.25,
        progressive_passes: 3.0,
        progressive_carries: 1.2,
        dribbles: 0.40,
        dribble_success: 0.50,
        tackles: 1.6,
        interceptions: 1.3,
        blocks: 0.50,
        clearances: 2.2,
        own_box_clearances: 0.20,
        pressures: 4.0,
        pressure_success: 0.30,
        high_pressure_share: 0.0,
        offsides: 0.05,
        miscontrols: 0.25,
        heavy_touches: 0.25,
    };

    const MIDFIELDER: GroupVolumes = GroupVolumes {
        shots_saved: 0.20,
        shots_wide: 0.77,
        xg_per_shot: 0.10,
        passes: 42.0,
        pass_completion: 0.80,
        key_passes: 1.0,
        passes_into_box: 0.70,
        crosses: 0.70,
        cross_completion: 0.25,
        progressive_passes: 5.0,
        progressive_carries: 2.0,
        dribbles: 1.0,
        dribble_success: 0.45,
        tackles: 1.8,
        interceptions: 1.0,
        blocks: 0.25,
        clearances: 0.40,
        own_box_clearances: 0.0,
        pressures: 6.0,
        pressure_success: 0.30,
        high_pressure_share: 0.20,
        offsides: 0.10,
        miscontrols: 0.70,
        heavy_touches: 0.70,
    };

    const FORWARD: GroupVolumes = GroupVolumes {
        shots_saved: 0.85,
        shots_wide: 2.40,
        xg_per_shot: 0.12,
        passes: 22.0,
        pass_completion: 0.74,
        key_passes: 1.0,
        passes_into_box: 1.0,
        crosses: 0.60,
        cross_completion: 0.25,
        progressive_passes: 2.5,
        progressive_carries: 2.5,
        dribbles: 1.2,
        dribble_success: 0.50,
        tackles: 0.8,
        interceptions: 0.6,
        blocks: 0.10,
        clearances: 0.30,
        own_box_clearances: 0.0,
        pressures: 6.0,
        pressure_success: 0.30,
        high_pressure_share: 0.70,
        offsides: 0.70,
        miscontrols: 0.80,
        heavy_touches: 0.80,
    };

    fn of(group: PlayerFieldPositionGroup) -> &'static GroupVolumes {
        match group {
            PlayerFieldPositionGroup::Defender => &Self::DEFENDER,
            PlayerFieldPositionGroup::Midfielder => &Self::MIDFIELDER,
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Goalkeeper => {
                &Self::FORWARD
            }
        }
    }
}

/// One man's line for one stint.
pub(super) struct StubLine<'a> {
    pub draw: StubDraw<'a>,
    pub player: &'a MatchPlayer,
    pub performance: f32,
    /// Time on the pitch, in shares of ninety minutes.
    pub share: f32,
}

impl StubLine<'_> {
    /// A man's output is at most double or half his level, however far he
    /// is above the opposition: the volumes of a position have a ceiling
    /// the goal rate does not.
    const OUTPUT_BOUND: f32 = 0.7;
    /// Completion per point of log output, per skill point over his level.
    const COMPLETION_PER_OUTPUT: f32 = 0.10;
    const COMPLETION_PER_SKILL: f32 = 0.012;
    /// Pitch units per progressive carry: about twenty metres.
    const UNITS_PER_CARRY: u32 = 160;
    /// A chance made is worth about a tenth of a goal.
    const BUILDUP_PER_KEY_PASS: f32 = 0.10;

    /// Shots a keeper saves per goal his opponents are expected to score.
    const SAVES_PER_EXPECTED_GOAL: f32 = 2.6;
    /// Chance value of a shot on target. The keeper model is anchored on
    /// `SaveModel`'s 0.562 and 1.15 goals conceded a match; sides here score
    /// 1.35, so a stubbed strike is harder by about that much.
    const STRIKE_VALUE: f32 = 0.70;
    /// The chance value that is a goal an ordinary keeper concedes.
    const GOALS_PER_STRIKE_VALUE: f32 = 0.52;
    const KEEPER_PASSES: f32 = 25.0;
    const KEEPER_PASS_COMPLETION: f32 = 0.74;
    const KEEPER_COMMAND_ACTIONS: f32 = 2.5;

    pub(super) fn outfield(&self, s: &mut PlayerMatchEndStats) {
        let v = GroupVolumes::of(s.position_group);
        let t = &self.player.skills.technical;
        let m = &self.player.skills.mental;
        let creative = (t.passing + m.vision) * 0.5;

        s.shots_on_target = s.goals + self.volume(v.shots_saved, t.finishing);
        s.shots_total = s.shots_on_target + self.volume(v.shots_wide, m.off_the_ball);
        s.xg = s.shots_total as f32 * v.xg_per_shot * self.draw.rng.jitter(1.0, 0.25);

        s.key_passes = self.volume(v.key_passes, creative).max(s.assists);
        s.passes_into_box = self.volume(v.passes_into_box, creative);
        s.xg_buildup = s.key_passes as f32 * Self::BUILDUP_PER_KEY_PASS;
        s.xg_chain = s.xg + s.xg_buildup;

        s.progressive_passes = self.volume(v.progressive_passes, t.passing);
        s.progressive_carries = self.volume(v.progressive_carries, t.dribbling);
        s.carry_distance = s.progressive_carries as u32 * Self::UNITS_PER_CARRY;

        let attempted = self.volume(v.passes, t.passing);
        let completion = self.chance(v.pass_completion, creative);
        let chance_passes = s.key_passes + s.passes_into_box;
        s.passes_completed = self
            .draw
            .successes(attempted, completion)
            .max(chance_passes);
        s.passes_attempted = attempted.max(s.passes_completed);

        s.attempted_dribbles = self.volume(v.dribbles, t.dribbling);
        s.successful_dribbles = self.draw.successes(
            s.attempted_dribbles,
            self.chance(v.dribble_success, t.dribbling),
        );
        s.crosses_attempted = self.volume(v.crosses, t.crossing);
        s.crosses_completed = self.draw.successes(
            s.crosses_attempted,
            self.chance(v.cross_completion, t.crossing),
        );

        s.tackles = self.volume(v.tackles, t.tackling);
        s.interceptions = self.volume(v.interceptions, m.positioning);
        s.blocks = self.volume(v.blocks, m.positioning);
        s.clearances = self.volume(v.clearances, t.heading);
        s.zone_stats.clearances_own_box = self.volume(v.own_box_clearances, m.positioning);

        s.pressures = self.volume(v.pressures, m.work_rate);
        s.successful_pressures = self
            .draw
            .successes(s.pressures, self.chance(v.pressure_success, m.work_rate));
        s.zone_stats.pressures_won_final_third = self
            .draw
            .successes(s.successful_pressures, v.high_pressure_share);

        s.offsides = self.volume(v.offsides, m.off_the_ball);
        s.miscontrols = self.lapses(v.miscontrols, t.first_touch);
        s.heavy_touches = self.lapses(v.heavy_touches, t.first_touch);
    }

    /// Goals he let in are the match's. What he did about the shots around
    /// them is his: a better keeper saves more of what the same side put on
    /// target, so his line separates him from the defence in front of him.
    pub(super) fn keeper(
        &self,
        s: &mut PlayerMatchEndStats,
        conceded: u16,
        opponent_goal_rate: f32,
    ) {
        let g = &self.player.skills.goalkeeping;
        let stopping = (g.reflexes + g.handling + g.one_on_ones) / 3.0;

        s.saves = self.volume(Self::SAVES_PER_EXPECTED_GOAL * opponent_goal_rate, stopping);
        s.shots_faced = s.saves + conceded;
        s.xg_faced = s.shots_faced as f32 * Self::STRIKE_VALUE * self.draw.rng.jitter(1.0, 0.15);
        s.xg_prevented = s.xg_faced * Self::GOALS_PER_STRIKE_VALUE - conceded as f32;

        let attempted = self.volume(Self::KEEPER_PASSES, g.passing);
        let completion = self.chance(Self::KEEPER_PASS_COMPLETION, g.passing);
        s.passes_completed = self.draw.successes(attempted, completion);
        s.passes_attempted = attempted;
        s.zone_stats.gk_command_actions =
            self.volume(Self::KEEPER_COMMAND_ACTIONS, g.command_of_area);
    }

    fn output(&self) -> f32 {
        self.performance
            .clamp(-Self::OUTPUT_BOUND, Self::OUTPUT_BOUND)
    }

    /// Drawn for his minutes, moved by the skill behind it and his day.
    fn volume(&self, per_90: f32, skill: f32) -> u16 {
        let shift = StubPerformance::skill_shift(self.player, skill);
        self.draw
            .count(per_90 * self.share * (self.output() + shift).exp())
    }

    /// The other way round: a better man on a better day has fewer of them.
    fn lapses(&self, per_90: f32, skill: f32) -> u16 {
        let shift = StubPerformance::skill_shift(self.player, skill);
        self.draw
            .count(per_90 * self.share * (-self.output() - shift).exp())
    }

    fn chance(&self, base: f32, skill: f32) -> f32 {
        (base
            + Self::COMPLETION_PER_SKILL * StubPerformance::skill_gap(self.player, skill)
            + Self::COMPLETION_PER_OUTPUT * self.output())
        .clamp(0.05, 0.97)
    }
}
