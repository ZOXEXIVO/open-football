use super::*;
use std::cmp::Ordering;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    // ───────────────────────────────────────────────────────────────────────
    // Penalty shootout — discrete resolver, not tick-based
    // ───────────────────────────────────────────────────────────────────────

    pub(super) fn run_penalty_shootout(field: &mut MatchField, context: &mut MatchContext) {
        let home_id = context.field_home_team_id;
        let away_id = context.field_away_team_id;

        // Sort available outfield takers by penalty skill + composure.
        // Sent-off players (and the keeper) can't take kicks. Reuses the
        // shared `score_penalty_taker` helper so taker selection here
        // and in-play penalty taker selection use the same ranking
        // formula (penalty_taking, finishing, composure, pressure,
        // technique, confidence).
        let takers_for = |team_id: u32| -> Vec<u32> {
            let mut candidates: Vec<(u32, f32)> = field
                .players
                .iter()
                .filter(|p| p.team_id == team_id && !p.is_sent_off)
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
        };

        // Active keeper per team — prefer the nominated GK. If sent off
        // without a replacement (used all subs), pick the outfielder with
        // the best innate goalkeeping ability. Real football: an outfield
        // player has to go in goal — their save probability is poor but
        // non-zero.
        let keeper_for = |team_id: u32| -> Option<u32> {
            // First: an actual goalkeeper still on the field.
            let gk = field.players.iter().find(|p| {
                p.team_id == team_id
                    && !p.is_sent_off
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
                .filter(|p| p.team_id == team_id && !p.is_sent_off)
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
        };

        let home_takers = takers_for(home_id);
        let away_takers = takers_for(away_id);
        let home_keeper = keeper_for(home_id);
        let away_keeper = keeper_for(away_id);

        // Per-kick taker score (0..1). Routes through the shared set-piece
        // helper so the shootout uses the same skill weighting as in-play
        // penalties.
        let taker_score_of = |fld: &MatchField, id: u32| -> f32 {
            if let Some(p) = fld.players.iter().find(|p| p.id == id) {
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
        };
        // Per-kick keeper score (0..1). None means no keeper → score
        // collapses to a small baseline so an outfielder in goal still
        // has a non-zero save floor (set_pieces clamp 0.58–0.90 won't
        // drop conversion below 0.58 either way).
        let keeper_score_of = |fld: &MatchField, id: Option<u32>| -> f32 {
            match id {
                Some(gk_id) => {
                    if let Some(p) = fld.players.iter().find(|p| p.id == gk_id) {
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
        };

        // Round-pressure model: best-of-5 ramps 0.04 per round (round 1
        // = 0.04, round 5 = 0.20); sudden death sits at 0.65. Combined
        // with the 0.35 base this stays inside the [0,1] band that
        // `penalty_conversion_prob` expects.
        let round_pressure = |round_idx: u8, sudden_death: bool| -> f32 {
            if sudden_death {
                0.65
            } else {
                (round_idx as f32) * 0.04
            }
        };

        // Per-kick scoring probability. The roll itself happens at the
        // call site via `context.rng` so the closure can borrow `field`
        // immutably without also needing `&mut context`.
        let kick_prob =
            |taker_id: u32, gk_id: Option<u32>, round_idx: u8, sudden_death: bool| -> f32 {
                let taker = taker_score_of(field, taker_id);
                let keeper = keeper_score_of(field, gk_id);
                let pressure = (0.35 + round_pressure(round_idx, sudden_death)).clamp(0.0, 1.0);
                penalty_conversion_prob(taker, keeper, pressure, true)
            };

        // Takers in rotation; sudden-death wraps the order.
        let mut home_idx: usize = 0;
        let mut away_idx: usize = 0;
        let next_home_taker = |idx: &mut usize| -> Option<u32> {
            if home_takers.is_empty() {
                return None;
            }
            let id = home_takers[*idx % home_takers.len()];
            *idx += 1;
            Some(id)
        };
        let next_away_taker = |idx: &mut usize| -> Option<u32> {
            if away_takers.is_empty() {
                return None;
            }
            let id = away_takers[*idx % away_takers.len()];
            *idx += 1;
            Some(id)
        };

        let mut home_score: u8 = 0;
        let mut away_score: u8 = 0;

        // Best-of-5 phase.
        for round in 0..5u8 {
            let home_remaining_kicks = 5 - round;
            let away_remaining_kicks = 5 - round;

            // Home kick.
            if let Some(id) = next_home_taker(&mut home_idx) {
                let p = kick_prob(id, away_keeper, round + 1, false);
                let scored = context.rng.bernoulli(p);
                context.penalty_shootout_kicks.push(PenaltyShootoutKick {
                    team_id: home_id,
                    taker_id: id,
                    goalkeeper_id: away_keeper,
                    round: round + 1,
                    scored,
                    sudden_death: false,
                });
                if scored {
                    home_score += 1;
                }
            }
            // Early termination — if one side can no longer catch up, stop.
            if (home_score as i32 - away_score as i32).abs()
                > (home_remaining_kicks as i32 - 1).max(0) + away_remaining_kicks as i32
            {
                break;
            }

            // Away kick.
            if let Some(id) = next_away_taker(&mut away_idx) {
                let p = kick_prob(id, home_keeper, round + 1, false);
                let scored = context.rng.bernoulli(p);
                context.penalty_shootout_kicks.push(PenaltyShootoutKick {
                    team_id: away_id,
                    taker_id: id,
                    goalkeeper_id: home_keeper,
                    round: round + 1,
                    scored,
                    sudden_death: false,
                });
                if scored {
                    away_score += 1;
                }
            }
            if (home_score as i32 - away_score as i32).abs()
                > (home_remaining_kicks as i32 - 1).max(0)
                    + (away_remaining_kicks as i32 - 1).max(0)
            {
                break;
            }
        }

        // Sudden death: one pair at a time until a decisive difference.
        // Hard cap at 30 rounds so we never loop indefinitely on bad data.
        let mut sudden_rounds = 0u8;
        while home_score == away_score && sudden_rounds < 30 {
            sudden_rounds += 1;
            let h = next_home_taker(&mut home_idx);
            let a = next_away_taker(&mut away_idx);
            if h.is_none() || a.is_none() {
                break; // Shouldn't happen — takers wrap — but guard anyway.
            }
            let home_taker = h.unwrap();
            let away_taker = a.unwrap();
            let round = 5 + sudden_rounds;
            let p_home = kick_prob(home_taker, away_keeper, round, true);
            let home_scored = context.rng.bernoulli(p_home);
            context.penalty_shootout_kicks.push(PenaltyShootoutKick {
                team_id: home_id,
                taker_id: home_taker,
                goalkeeper_id: away_keeper,
                round,
                scored: home_scored,
                sudden_death: true,
            });
            if home_scored {
                home_score += 1;
            }
            let p_away = kick_prob(away_taker, home_keeper, round, true);
            let away_scored = context.rng.bernoulli(p_away);
            context.penalty_shootout_kicks.push(PenaltyShootoutKick {
                team_id: away_id,
                taker_id: away_taker,
                goalkeeper_id: home_keeper,
                round,
                scored: away_scored,
                sudden_death: true,
            });
            if away_scored {
                away_score += 1;
            }
        }

        context.score.home_shootout = home_score;
        context.score.away_shootout = away_score;
    }
}
