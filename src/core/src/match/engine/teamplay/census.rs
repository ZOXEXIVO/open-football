//! **The team census** (`match-logs`): what each side did with the ball and
//! without it, split by the state of the score, and how its possessions
//! began and ended. Fed once a tick from the match loop and folded into the
//! result's tally at full time; it reads the match and never changes it.

use crate::r#match::engine::ball::ball::OffsideLine;
use crate::r#match::engine::result::team_census::{
    LedgerState, PossessionEnd, PossessionTally, StateLedger, TeamCensusTally,
};
use crate::r#match::engine::result::{MatchTally, PlayingTime};
use crate::r#match::{MatchField, MatchState, PlayerSide};

/// One tick of the match as the census needs it.
pub struct TeamCensusInputs<'a> {
    pub field: &'a MatchField,
    pub holder: Option<u32>,
    pub home_team_id: u32,
    pub away_team_id: u32,
    pub home_goals: u8,
    pub away_goals: u8,
    pub tick: u64,
    pub period: MatchState,
    pub playing_time: PlayingTime,
}

/// A side's running totals, as last read.
#[derive(Debug, Clone, Copy, Default)]
struct SideCounters {
    shots: u32,
    xg: f32,
    passes: u32,
    defensive_actions: u32,
}

#[derive(Debug, Clone, Copy)]
struct OpenPossession {
    side: usize,
    start_tick: u64,
    start_progress: f32,
    passes_at_start: u32,
    shots_at_start: u32,
    goals_at_start: u8,
    won_in_play: bool,
    countered: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TeamCensus {
    tally: TeamCensusTally,
    counters: [SideCounters; 2],
    /// The tick last read, the ball's progress from each side's goal, and
    /// the pitch's length in game units.
    last_tick: u64,
    last_progress: [f32; 2],
    length: f32,
    open: Option<OpenPossession>,
    period: Option<MatchState>,
}

impl TeamCensus {
    /// Spatial samples are taken every quarter of a second.
    const SAMPLE_TICKS: u64 = 25;
    /// A regain that reaches a shot inside 15 s is a counter-attack.
    const COUNTER_TICKS: u64 = 1_500;
    /// Ball progress past which the opponent is in its own 60%.
    const PRESSING_ZONE: f32 = 0.4;
    /// One game unit is 125 mm.
    const MM_PER_UNIT: f32 = 125.0;

    pub fn sample(&mut self, inputs: &TeamCensusInputs<'_>) {
        if self.period != Some(inputs.period) {
            self.close(PossessionEnd::PeriodEnd);
            self.period = Some(inputs.period);
        }

        let field = inputs.field;
        let length = field.size.width as f32;
        let home_side = match field.left_side_players.as_ref() {
            Some(left) if left.team_id != inputs.home_team_id => PlayerSide::Right,
            _ => PlayerSide::Left,
        };
        let sides = [home_side, home_side.opposite()];
        let team_ids = [inputs.home_team_id, inputs.away_team_id];
        let goals = [inputs.home_goals, inputs.away_goals];

        let now = [
            Self::counters(field, team_ids[0]),
            Self::counters(field, team_ids[1]),
        ];
        let delta = |side: usize| SideCounters {
            shots: now[side].shots.saturating_sub(self.counters[side].shots),
            xg: (now[side].xg - self.counters[side].xg).max(0.0),
            passes: now[side].passes.saturating_sub(self.counters[side].passes),
            defensive_actions: now[side]
                .defensive_actions
                .saturating_sub(self.counters[side].defensive_actions),
        };
        let deltas = [delta(0), delta(1)];
        let ball_x = field.ball.position.x;
        let progress = [
            Self::progress(ball_x, sides[0], length),
            Self::progress(ball_x, sides[1], length),
        ];

        let spatial = inputs.tick.is_multiple_of(Self::SAMPLE_TICKS);
        for side in 0..2 {
            let other = 1 - side;
            let state = LedgerState::from_goal_difference(goals[side] as i32 - goals[other] as i32);
            let has_ball = inputs.holder == Some(team_ids[side]);
            let ledger = &mut self.tally.ledger[side][state.index()];
            ledger.ticks += 1;
            if inputs.holder.is_some() {
                ledger.held_ticks += 1;
            }
            if has_ball {
                ledger.possession_ticks += 1;
            }
            if progress[side] > 0.5 {
                ledger.territory_ticks += 1;
            }
            ledger.shots += deltas[side].shots;
            ledger.xg_milli += (deltas[side].xg * 1000.0).round() as u64;
            if progress[side] >= Self::PRESSING_ZONE {
                ledger.opposition_passes += deltas[other].passes;
                ledger.defensive_actions += deltas[side].defensive_actions;
            }
            if spatial && inputs.playing_time == PlayingTime::Live {
                if has_ball {
                    if let Some((_, width)) = Self::block(field, team_ids[side]) {
                        let style = &mut self.tally.style[side];
                        style.width_samples += 1;
                        style.width_in_possession_mm += Self::mm(width);
                    }
                } else {
                    Self::sample_block(ledger, field, team_ids[side], sides[side], length);
                }
            }
        }

        self.counters = now;
        self.last_tick = inputs.tick;
        self.last_progress = progress;
        self.length = length;
        self.track_possession(inputs, team_ids, goals, length);
    }

    pub fn finish(&mut self, tally: &mut MatchTally) {
        self.close(PossessionEnd::PeriodEnd);
        tally.team_census = std::mem::take(&mut self.tally);
    }

    fn track_possession(
        &mut self,
        inputs: &TeamCensusInputs<'_>,
        team_ids: [u32; 2],
        goals: [u8; 2],
        length: f32,
    ) {
        if let Some(open) = self.open.as_mut()
            && open.won_in_play
            && !open.countered
            && self.counters[open.side].shots > open.shots_at_start
            && inputs.tick.saturating_sub(open.start_tick) <= Self::COUNTER_TICKS
        {
            open.countered = true;
            self.tally.style[open.side].counter_attacks += 1;
        }

        let Some(holder) = inputs.holder else {
            return;
        };
        let side = usize::from(holder != team_ids[0]);
        if self.open.is_some_and(|open| open.side == side) {
            return;
        }
        if let Some(open) = self.open {
            let held = open.side;
            let reason = if goals[held] > open.goals_at_start {
                PossessionEnd::Goal
            } else if self.counters[held].shots > open.shots_at_start {
                PossessionEnd::Shot
            } else if inputs.playing_time == PlayingTime::Live {
                PossessionEnd::LostInPlay
            } else {
                PossessionEnd::DeadBall
            };
            self.close(reason);
        }

        let won_in_play = inputs.playing_time == PlayingTime::Live;
        if won_in_play {
            let style = &mut self.tally.style[side];
            style.regains += 1;
            style.regain_height_mm += Self::mm(self.last_progress[side] * length);
        }
        self.open = Some(OpenPossession {
            side,
            start_tick: inputs.tick,
            start_progress: self.last_progress[side],
            passes_at_start: self.counters[side].passes,
            shots_at_start: self.counters[side].shots,
            goals_at_start: goals[side],
            won_in_play,
            countered: false,
        });
    }

    /// Close the open possession as of the last tick read.
    fn close(&mut self, reason: PossessionEnd) {
        let Some(open) = self.open.take() else {
            return;
        };
        let passes = self.counters[open.side]
            .passes
            .saturating_sub(open.passes_at_start);
        let tally: &mut PossessionTally = &mut self.tally.possessions[open.side];
        tally.possessions += 1;
        tally.passes += passes;
        tally.ticks += self.last_tick.saturating_sub(open.start_tick);
        tally.progress_mm += ((self.last_progress[open.side] - open.start_progress)
            * self.length
            * Self::MM_PER_UNIT)
            .round() as i64;
        tally.ended[reason.index()] += 1;
        tally.by_passes[PossessionTally::pass_bucket(passes)] += 1;
    }

    fn sample_block(
        ledger: &mut StateLedger,
        field: &MatchField,
        team_id: u32,
        side: PlayerSide,
        length: f32,
    ) {
        let Some((block_length, block_width)) = Self::block(field, team_id) else {
            return;
        };
        let ours = field
            .players
            .iter()
            .filter(|p| p.team_id == team_id && !p.off_pitch)
            .map(|p| p.position.x);
        let Some(line) = OffsideLine::second_last(ours, side.opposite()) else {
            return;
        };
        ledger.block_samples += 1;
        ledger.line_height_mm += Self::mm(Self::progress(line, side, length) * length);
        ledger.block_length_mm += Self::mm(block_length);
        ledger.block_width_mm += Self::mm(block_width);
    }

    /// The length and width the outfielders cover, in game units.
    fn block(field: &MatchField, team_id: u32) -> Option<(f32, f32)> {
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for p in field.players.iter().filter(|p| {
            p.team_id == team_id
                && !p.off_pitch
                && !p.tactical_position.current_position.is_goalkeeper()
        }) {
            let (x, y) = (p.position.x, p.position.y);
            bounds = Some(match bounds {
                None => (x, x, y, y),
                Some((x0, x1, y0, y1)) => (x0.min(x), x1.max(x), y0.min(y), y1.max(y)),
            });
        }
        bounds.map(|(x0, x1, y0, y1)| (x1 - x0, y1 - y0))
    }

    fn counters(field: &MatchField, team_id: u32) -> SideCounters {
        let mut c = SideCounters::default();
        for p in field.players.iter().filter(|p| p.team_id == team_id) {
            c.shots += p.memory.shots_taken;
            c.xg += p.memory.xg_total;
            c.passes += p.statistics.passes_completed as u32;
            c.defensive_actions += p.statistics.tackles as u32
                + p.statistics.interceptions as u32
                + p.fouls_committed as u32;
        }
        c
    }

    /// How far up the pitch `x` is from `side`'s own goal, 0..1.
    fn progress(x: f32, side: PlayerSide, length: f32) -> f32 {
        let from_left = (x / length).clamp(0.0, 1.0);
        match side {
            PlayerSide::Left => from_left,
            PlayerSide::Right => 1.0 - from_left,
        }
    }

    fn mm(units: f32) -> u64 {
        (units.max(0.0) * Self::MM_PER_UNIT).round() as u64
    }
}
