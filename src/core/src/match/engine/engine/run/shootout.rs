//! **The penalty shootout** — a discrete resolver, not a tick loop. It
//! runs after `play_inner` has exhausted every playing state and the tie
//! still needs breaking, which is why it sits with the lifecycle rather
//! than with the tick.

use crate::r#match::engine::engine::*;
use std::cmp::Ordering;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    // ───────────────────────────────────────────────────────────────────────
    // Penalty shootout — discrete resolver, not tick-based
    // ───────────────────────────────────────────────────────────────────────

    pub(in crate::r#match::engine::engine) fn run_penalty_shootout(
        field: &mut MatchField,
        context: &mut MatchContext,
    ) {
        let mut shootout = Shootout::new(
            field,
            context.field_home_team_id,
            context.field_away_team_id,
        );
        shootout.best_of_five(context);
        shootout.sudden_death(context);
        context.score.home_shootout = shootout.home_score;
        context.score.away_shootout = shootout.away_score;
    }
}

/// One shootout in progress: the takers, the keepers, the rotation and the
/// running score. The phases below are the two laws of the format — five
/// pairs, then sudden death — and nothing else.
struct Shootout<'a> {
    field: &'a MatchField,
    home_id: u32,
    away_id: u32,
    home_takers: Vec<u32>,
    away_takers: Vec<u32>,
    home_keeper: Option<u32>,
    away_keeper: Option<u32>,
    home_idx: usize,
    away_idx: usize,
    home_score: u8,
    away_score: u8,
}

impl<'a> Shootout<'a> {
    fn new(field: &'a MatchField, home_id: u32, away_id: u32) -> Self {
        let home_takers = Self::takers_for(field, home_id);
        let away_takers = Self::takers_for(field, away_id);
        let home_keeper = Self::keeper_for(field, home_id);
        let away_keeper = Self::keeper_for(field, away_id);
        Shootout {
            field,
            home_id,
            away_id,
            home_takers,
            away_takers,
            home_keeper,
            away_keeper,
            home_idx: 0,
            away_idx: 0,
            home_score: 0,
            away_score: 0,
        }
    }

    /// Sort available outfield takers by penalty skill + composure.
    /// Sent-off players (and the keeper) can't take kicks. Reuses the
    /// shared `score_penalty_taker` helper so taker selection here
    /// and in-play penalty taker selection use the same ranking
    /// formula (penalty_taking, finishing, composure, pressure,
    /// technique, confidence).
    fn takers_for(field: &MatchField, team_id: u32) -> Vec<u32> {
        let mut candidates: Vec<(u32, f32)> = field
            .players
            .iter()
            .filter(|p| p.team_id == team_id && !p.off_pitch)
            .filter(|p| {
                p.tactical_position.current_position.position_group()
                    != PlayerFieldPositionGroup::Goalkeeper
            })
            .map(|p| {
                let t = &p.skills.technical;
                let m = &p.skills.mental;
                let pressure = p.attributes.pressure;
                let score = score_penalty_taker(
                    t.penalty_taking,
                    t.finishing,
                    m.composure,
                    pressure,
                    t.technique,
                    0.0,
                );
                (p.id, score)
            })
            .collect();
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        candidates.into_iter().take(11).map(|(id, _)| id).collect()
    }

    /// Active keeper per team — prefer the nominated GK. If sent off
    /// without a replacement (used all subs), pick the outfielder with
    /// the best innate goalkeeping ability. Real football: an outfield
    /// player has to go in goal — their save probability is poor but
    /// non-zero.
    fn keeper_for(field: &MatchField, team_id: u32) -> Option<u32> {
        // First: an actual goalkeeper still on the field.
        let gk = field.players.iter().find(|p| {
            p.team_id == team_id
                && !p.off_pitch
                && p.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
        });
        if let Some(p) = gk {
            return Some(p.id);
        }
        // Fallback: outfielder with the best goalkeeping composite.
        // Most outfielders have near-zero reflexes/handling so this
        // typically yields a 5-15% save rate, not the ~0% that a
        // missing GK would imply.
        field
            .players
            .iter()
            .filter(|p| p.team_id == team_id && !p.off_pitch)
            .max_by(|a, b| {
                let sa = a.skills.goalkeeping.reflexes * 0.4
                    + a.skills.goalkeeping.handling * 0.3
                    + a.skills.goalkeeping.one_on_ones * 0.3;
                let sb = b.skills.goalkeeping.reflexes * 0.4
                    + b.skills.goalkeeping.handling * 0.3
                    + b.skills.goalkeeping.one_on_ones * 0.3;
                sa.partial_cmp(&sb).unwrap_or(Ordering::Equal)
            })
            .map(|p| p.id)
    }

    /// Per-kick taker score (0..1). Routes through the shared set-piece
    /// helper so the shootout uses the same skill weighting as in-play
    /// penalties.
    fn taker_score_of(field: &MatchField, id: u32) -> f32 {
        if let Some(p) = field.players.iter().find(|p| p.id == id) {
            let t = &p.skills.technical;
            let m = &p.skills.mental;
            let pressure = p.attributes.pressure;
            score_penalty_taker(
                t.penalty_taking,
                t.finishing,
                m.composure,
                pressure,
                t.technique,
                0.0,
            )
            .clamp(0.05, 1.0)
        } else {
            0.5
        }
    }

    /// Per-kick keeper score (0..1). None means no keeper → score
    /// collapses to a small baseline so an outfielder in goal still
    /// has a non-zero save floor (set_pieces clamp 0.58–0.90 won't
    /// drop conversion below 0.58 either way).
    fn keeper_score_of(field: &MatchField, id: Option<u32>) -> f32 {
        match id {
            Some(gk_id) => {
                if let Some(p) = field.players.iter().find(|p| p.id == gk_id) {
                    let g = &p.skills.goalkeeping;
                    let m = &p.skills.mental;
                    let pressure = p.attributes.pressure;
                    score_keeper_save(
                        g.reflexes,
                        p.skills.physical.agility,
                        g.handling,
                        m.anticipation,
                        pressure,
                        m.concentration,
                    )
                    .clamp(0.05, 1.0)
                } else {
                    0.5
                }
            }
            None => 0.05,
        }
    }

    /// Round-pressure model: best-of-5 ramps 0.04 per round (round 1
    /// = 0.04, round 5 = 0.20); sudden death sits at 0.65. Combined
    /// with the 0.35 base this stays inside the [0,1] band that
    /// `penalty_conversion_prob` expects.
    fn round_pressure(round_idx: u8, sudden_death: bool) -> f32 {
        if sudden_death {
            0.65
        } else {
            (round_idx as f32) * 0.04
        }
    }

    /// Per-kick scoring probability. The roll itself happens at the call
    /// site via `context.rng` so this can borrow `field` immutably without
    /// also needing `&mut context`.
    fn kick_prob(
        &self,
        taker_id: u32,
        gk_id: Option<u32>,
        round_idx: u8,
        sudden_death: bool,
    ) -> f32 {
        let taker = Self::taker_score_of(self.field, taker_id);
        let keeper = Self::keeper_score_of(self.field, gk_id);
        let pressure = (0.35 + Self::round_pressure(round_idx, sudden_death)).clamp(0.0, 1.0);
        penalty_conversion_prob(taker, keeper, pressure, true)
    }

    fn next_home_taker(&mut self) -> Option<u32> {
        if self.home_takers.is_empty() {
            return None;
        }
        let id = self.home_takers[self.home_idx % self.home_takers.len()];
        self.home_idx += 1;
        Some(id)
    }

    fn next_away_taker(&mut self) -> Option<u32> {
        if self.away_takers.is_empty() {
            return None;
        }
        let id = self.away_takers[self.away_idx % self.away_takers.len()];
        self.away_idx += 1;
        Some(id)
    }

    fn record_kick(
        &self,
        context: &mut MatchContext,
        team_id: u32,
        taker_id: u32,
        goalkeeper_id: Option<u32>,
        round: u8,
        scored: bool,
        sudden_death: bool,
    ) {
        context.penalty_shootout_kicks.push(PenaltyShootoutKick {
            team_id,
            taker_id,
            goalkeeper_id,
            round,
            scored,
            sudden_death,
        });
    }

    /// Best-of-five phase, with the early termination the laws allow.
    fn best_of_five(&mut self, context: &mut MatchContext) {
        for round in 0..5u8 {
            let home_remaining_kicks = 5 - round;
            let away_remaining_kicks = 5 - round;

            // Home kick.
            if let Some(id) = self.next_home_taker() {
                let p = self.kick_prob(id, self.away_keeper, round + 1, false);
                let scored = context.rng.bernoulli(p);
                self.record_kick(
                    context,
                    self.home_id,
                    id,
                    self.away_keeper,
                    round + 1,
                    scored,
                    false,
                );
                if scored {
                    self.home_score += 1;
                }
            }
            // Early termination — if one side can no longer catch up, stop.
            if (self.home_score as i32 - self.away_score as i32).abs()
                > (home_remaining_kicks as i32 - 1).max(0) + away_remaining_kicks as i32
            {
                break;
            }

            // Away kick.
            if let Some(id) = self.next_away_taker() {
                let p = self.kick_prob(id, self.home_keeper, round + 1, false);
                let scored = context.rng.bernoulli(p);
                self.record_kick(
                    context,
                    self.away_id,
                    id,
                    self.home_keeper,
                    round + 1,
                    scored,
                    false,
                );
                if scored {
                    self.away_score += 1;
                }
            }
            if (self.home_score as i32 - self.away_score as i32).abs()
                > (home_remaining_kicks as i32 - 1).max(0)
                    + (away_remaining_kicks as i32 - 1).max(0)
            {
                break;
            }
        }
    }

    /// Sudden death: one pair at a time until a decisive difference.
    /// Hard cap at 30 rounds so we never loop indefinitely on bad data.
    fn sudden_death(&mut self, context: &mut MatchContext) {
        let mut sudden_rounds = 0u8;
        while self.home_score == self.away_score && sudden_rounds < 30 {
            sudden_rounds += 1;
            let h = self.next_home_taker();
            let a = self.next_away_taker();
            if h.is_none() || a.is_none() {
                break; // Shouldn't happen — takers wrap — but guard anyway.
            }
            let home_taker = h.unwrap();
            let away_taker = a.unwrap();
            let round = 5 + sudden_rounds;
            let p_home = self.kick_prob(home_taker, self.away_keeper, round, true);
            let home_scored = context.rng.bernoulli(p_home);
            self.record_kick(
                context,
                self.home_id,
                home_taker,
                self.away_keeper,
                round,
                home_scored,
                true,
            );
            if home_scored {
                self.home_score += 1;
            }
            let p_away = self.kick_prob(away_taker, self.home_keeper, round, true);
            let away_scored = context.rng.bernoulli(p_away);
            self.record_kick(
                context,
                self.away_id,
                away_taker,
                self.home_keeper,
                round,
                away_scored,
                true,
            );
            if away_scored {
                self.away_score += 1;
            }
        }
    }
}
