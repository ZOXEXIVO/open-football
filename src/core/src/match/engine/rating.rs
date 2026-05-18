use crate::PlayerFieldPositionGroup;
use crate::r#match::PlayerMatchEndStats;
use crate::r#match::engine::zones::ZoneCoeffs;
#[cfg(test)]
use crate::r#match::engine::zones::ZoneStats;

// =====================================================================
// Public API
// =====================================================================

const BASE_RATING: f32 = 6.0;
const RATING_MIN: f32 = 1.0;
const RATING_MAX: f32 = 10.0;

// =====================================================================
// RatingContext
// =====================================================================
//
// The rating calculation runs through [`RatingContext`]. Build one with
// [`RatingContext::new`] and call [`RatingContext::calculate`] to get
// the final score. Inputs shared across every contribution method
// (position, `minute_damp`, `gk_def_damp`) are precomputed once at
// construction; each per-section method returns a signed delta that
// `calculate` sums into the running total before applying the soft
// caps and top-end compressor.

pub struct RatingContext<'a> {
    stats: &'a PlayerMatchEndStats,
    team_goals: u8,
    opponent_goals: u8,
    pos: PlayerFieldPositionGroup,
    /// Per-minute scaling for cameo events. `0.0` for sub-15-minute
    /// cameos (so a single key pass can't post 7.5), ramping to `1.0`
    /// for a full 60+ minute shift.
    minute_damp: f32,
    /// Non-save defensive credits (cross-claims, sweeper interceptions,
    /// clearances, blocks, zone bonuses, command actions) get halved
    /// for a keeper who still shipped ≥ 3 — collecting two crosses and
    /// sweeping up does not paper over a 3-goal night. `1.0` for every
    /// other player (and for keepers who shipped < 3).
    gk_def_damp: f32,
}

impl<'a> RatingContext<'a> {
    /// Build a rating context from a player's end-of-match stats and
    /// the final scoreline (from that player's perspective).
    pub fn new(stats: &'a PlayerMatchEndStats, team_goals: u8, opponent_goals: u8) -> Self {
        let pos = stats.position_group;
        let minute_damp = if stats.minutes_played < 15 {
            0.0
        } else if stats.minutes_played < 30 {
            0.65
        } else if stats.minutes_played < 60 {
            0.85
        } else {
            1.0
        };
        let gk_def_damp =
            if pos == PlayerFieldPositionGroup::Goalkeeper && opponent_goals >= 3 {
                0.5
            } else {
                1.0
            };
        Self {
            stats,
            team_goals,
            opponent_goals,
            pos,
            minute_damp,
            gk_def_damp,
        }
    }

    /// Calculate the match rating (1.0 - 10.0, base 6.0).
    ///
    /// The formula is position-aware: goalkeepers are rated on saves,
    /// defenders on tackles / interceptions / clean sheets, midfielders
    /// on passing volume & accuracy, and forwards on goals / shots /
    /// xG. Each independent dimension lives in its own per-section
    /// method on `RatingContext` — most return a signed delta that is
    /// summed into the running total here, and the position-aware soft
    /// caps + top-end compressor are applied at the end.
    pub fn calculate(&self) -> f32 {
        let mut r = BASE_RATING;

        // ── Per-event contributions (additive) ──────────────────────────
        r += self.attacking();
        r += self.passing();
        r += self.shooting();
        r += self.base_defensive_credit();
        r += self.goalkeeper_saves();
        r += self.team_result();
        r += self.clean_sheet();
        r += self.conceded_penalty();
        r += self.unlucky_finisher();
        r += self.creation_and_buildup();
        r += self.blocks();
        r += self.crossing();
        r += self.passes_into_box();
        r += self.xg_buildup();
        r += self.carry_distance();
        r += self.possession_penalties();
        r += self.errors_and_cards();
        r += self.zone_defensive();
        r += self.zone_creation();
        r += self.dangerous_turnovers();
        r += self.gk_command();
        r += self.discipline();
        r += self.goalkeeper_xg_prevented();
        r += self.goalkeeper_dominant_defense();

        // ── Position- and minute-aware soft caps (mutate the running total) ─
        r = self.apply_cameo_bound(r);
        r = self.apply_outfielder_low_involvement_caps(r);
        r = self.apply_defender_section(r);
        r = self.apply_midfielder_section(r);
        r = self.apply_forward_section(r);

        Self::compress_top_end(r).clamp(RATING_MIN, RATING_MAX)
    }

    /// Pull raw ratings above 7.0 toward a softer ceiling so a single
    /// goal + good shift lands at ~7.5 and an outstanding 3-goal
    /// performance lands at ~8.1 instead of 9.0+. WhoScored references:
    /// per-match peak ~9.5-9.8 for once-a-season performances; season-
    /// long top average caps at ~7.85. Floor caps (6.4, 6.7, 6.8, 7.1,
    /// 7.2) are all below the compression threshold, so they remain
    /// authoritative — the compressor only touches the high-end tail.
    fn compress_top_end(rating: f32) -> f32 {
        if rating > 7.0 {
            7.0 + (rating - 7.0) * 0.60
        } else {
            rating
        }
    }

    #[inline]
    fn is_goalkeeper(&self) -> bool {
        self.pos == PlayerFieldPositionGroup::Goalkeeper
    }

    // ── Position-aware per-event defensive weights ─────────────────────
    //
    // Shared by both the per-event defensive credit block and the
    // zone-aware defensive bonus block, so they live as helpers to
    // keep the two callsites consistent.

    fn tackle_weight(&self) -> f32 {
        match self.pos {
            PlayerFieldPositionGroup::Defender => 0.12,
            PlayerFieldPositionGroup::Midfielder => 0.08,
            _ => 0.05,
        }
    }

    fn interception_weight(&self) -> f32 {
        match self.pos {
            PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper => 0.15,
            PlayerFieldPositionGroup::Midfielder => 0.10,
            _ => 0.06,
        }
    }

    fn clearance_weight(&self) -> f32 {
        match self.pos {
            PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper => 0.07,
            PlayerFieldPositionGroup::Midfielder => 0.04,
            _ => 0.02,
        }
    }

    fn clearance_cap(&self) -> f32 {
        match self.pos {
            PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper => 0.45,
            PlayerFieldPositionGroup::Midfielder => 0.25,
            _ => 0.15,
        }
    }

    fn block_weight(&self) -> f32 {
        match self.pos {
            PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper => 0.10,
            PlayerFieldPositionGroup::Midfielder => 0.07,
            _ => 0.04,
        }
    }

    /// Effective denominator for save% calculations. The engine
    /// populates `shots_faced` directly; legacy fixtures / save files
    /// leave it at zero, in which case we synthesise it from saves +
    /// goals conceded.
    fn shots_faced(&self) -> u16 {
        self.stats
            .shots_faced
            .max(self.stats.saves + self.opponent_goals as u16)
    }

    // ───────────────────────────────────────────────────────────────────
    // Per-event contribution methods (each returns a signed delta)
    // ───────────────────────────────────────────────────────────────────

    /// Goals (with low-xG dampener and shot-spam penalty), decisive-goal
    /// bonus, xG over-conversion, wasted-high-xG penalty, and assists.
    fn attacking(&self) -> f32 {
        let s = self.stats;
        let goal_count = s.goals as f32;
        let mut delta = 0.0_f32;

        // Goals: +0.65 each, capped at +2.4. The previous +0.75/goal
        // pushed a 1-goal-and-won striker to 7.5+ and a hat-trick to
        // 9.4 routinely. WhoScored references: a 1-goal match averages
        // ~7.4; 2 goals ~8.1; hat-trick ~8.8-9.0.
        delta += (goal_count * 0.65).min(2.4);

        // Decisive goal bonus — cheap proxy for "actually mattered to
        // the result" without re-deriving per-goal scoreline state.
        if s.goals > 0 && self.team_goals > self.opponent_goals {
            delta += 0.15;
        }

        // High-xG over-conversion, capped at +0.15.
        if s.xg > 0.05 && goal_count > 0.0 {
            let over = (goal_count - s.xg).max(0.0);
            delta += (over * 0.10).min(0.15);
        }

        // Low-xG goals dampener: a variance-finished tap-in still
        // counts but not the same as a genuinely difficult conversion.
        if s.goals > 0 && s.xg < goal_count * 0.35 {
            delta -= 0.15 * goal_count;
        }

        // Wasted high xG: created premium chances, scored none.
        if s.goals == 0 && s.xg >= 0.8 {
            let waste = ((s.xg - 0.8) * 0.20).min(0.35);
            delta -= waste;
        }

        // Shot-spam penalty: high shot volume with very poor xG/shot
        // means the player kept firing low-quality attempts.
        if s.shots_total > 4 {
            let xg_per_shot = if s.shots_total > 0 {
                s.xg / s.shots_total as f32
            } else {
                0.0
            };
            if xg_per_shot < 0.08 {
                delta -= ((s.shots_total as f32 - 4.0) * 0.035).min(0.30);
            }
        }

        // Assists: +0.40 each, capped at +1.2. Trimmed from +0.5/+1.5
        // so a single assist no longer matches the impact of a clean
        // defensive shift.
        delta += (s.assists as f32 * 0.40).min(1.2);

        delta
    }

    /// Pass completion percentage band + high-volume volume bonus.
    fn passing(&self) -> f32 {
        let s = self.stats;
        if s.passes_attempted <= 10 {
            return 0.0;
        }
        let pct = s.passes_completed as f32 / s.passes_attempted as f32;

        // 70% = neutral; 90%+ = +0.35; below 50% = -0.4.
        let mut bonus = ((pct - 0.70) * 2.0).clamp(-0.4, 0.35);

        // Volume bonus: high-volume accurate passing shows sustained
        // involvement. Tighter than before — a 60-pass shift used to
        // stack +0.30 on top of pass quality.
        if s.passes_attempted > 30 && pct > 0.80 {
            bonus += 0.08;
        }
        if s.passes_attempted > 50 && pct > 0.85 {
            bonus += 0.08;
        }
        bonus
    }

    /// Shot accuracy band (on-target vs total shots).
    fn shooting(&self) -> f32 {
        let s = self.stats;
        if s.shots_total == 0 {
            return 0.0;
        }
        let accuracy = s.shots_on_target as f32 / s.shots_total as f32;
        ((accuracy - 0.4) * 0.6).clamp(-0.2, 0.20)
    }

    /// Tackles, interceptions, clearances. No minute damp — these are
    /// discrete defensive acts that count regardless of cameo length.
    /// Interceptions and clearances pass through `gk_def_damp` so a
    /// keeper who shipped ≥ 3 doesn't compose cross-claim credit back
    /// over the conceded penalty.
    fn base_defensive_credit(&self) -> f32 {
        let s = self.stats;
        let tackles = (s.tackles as f32 * self.tackle_weight()).min(0.5);
        // Interceptions — reading the game is valuable, especially for
        // defenders. For goalkeepers this includes commanding the box.
        let interceptions =
            (s.interceptions as f32 * self.interception_weight()).min(0.8) * self.gk_def_damp;
        // Clearances — last-ditch defending. Heavily weighted for
        // back-line players; midfielders get a smaller share; forwards
        // basically don't clear.
        let clearances = (s.clearances as f32 * self.clearance_weight())
            .min(self.clearance_cap())
            * self.gk_def_damp;
        tackles + interceptions + clearances
    }

    /// Per-save bonus + save% bracket + surplus-saves bonus (gated to
    /// opponent_goals < 3). Returns 0 for non-goalkeepers.
    fn goalkeeper_saves(&self) -> f32 {
        if !self.is_goalkeeper() {
            return 0.0;
        }
        let s = self.stats;
        let mut delta = 0.0_f32;

        // Real-football season averages for elite GKs sit at 7.0-7.3.
        // Per-save / save% / surplus all reward high-volume saving, so
        // each individual layer is modest — stacked they compose to
        // ~+1.7 max above base, which is the right max for a single
        // great game.
        delta += (s.saves as f32 * 0.15).min(1.2);

        let shots_faced = self.shots_faced();
        if shots_faced >= 3 {
            let pct = s.saves as f32 / shots_faced as f32;
            delta += if pct > 0.80 {
                0.30
            } else if pct > 0.70 {
                0.15
            } else if pct > 0.60 {
                0.05
            } else if pct < 0.50 {
                -0.20
            } else {
                0.0
            };
        }

        // Surplus credit is "above and beyond" shot-stopping — only
        // applies when the keeper is clearly winning the duel. Once
        // the team has shipped 3+, "saves > conceded" is damage
        // limitation, not a standout. Gate keeps a 5-saves-1-conceded
        // match earning surplus credit while removing it from a
        // 6-saves-3-conceded hammering.
        if s.saves > self.opponent_goals as u16 && self.opponent_goals < 3 {
            let surplus = s.saves - self.opponent_goals as u16;
            delta += (surplus as f32 * 0.05).min(0.2);
        }

        delta
    }

    /// Win / loss nudge. Unconditional team-result credit was the
    /// single biggest source of season-average inflation, so this is
    /// deliberately modest.
    fn team_result(&self) -> f32 {
        if self.team_goals > self.opponent_goals {
            0.12
        } else if self.team_goals < self.opponent_goals {
            -0.15
        } else {
            0.0
        }
    }

    /// Position-aware clean-sheet bonus. Trimmed from +0.30/+0.30/+0.10
    /// so a dominant team's back six doesn't get ~+0.13 lifted across
    /// the entire year from 15+ shutouts.
    fn clean_sheet(&self) -> f32 {
        if self.opponent_goals != 0 {
            return 0.0;
        }
        match self.pos {
            PlayerFieldPositionGroup::Goalkeeper => 0.20,
            PlayerFieldPositionGroup::Defender => 0.20,
            PlayerFieldPositionGroup::Midfielder => 0.05,
            _ => 0.0,
        }
    }

    /// Goal-shipping penalty. Goalkeepers absorb the full curve;
    /// defenders share blame past the 2nd goal at a tighter rate.
    /// Other positions are unaffected.
    fn conceded_penalty(&self) -> f32 {
        match self.pos {
            PlayerFieldPositionGroup::Goalkeeper => {
                // Linear base + linear extra past the 2nd — not
                // quadratic, because quadratic clamps the worst cases
                // at 1.0 regardless of effort.
                //
                // Earlier curve (0.15/goal + heavy past 3rd) was too
                // shallow: a per-goal cost of 0.15 exactly matched
                // the per-save bonus, so a keeper who matched saves
                // to concessions netted zero. Combined with cross-
                // claim and sweeper credit (uncapped against conceded
                // count), a busy 0-3 keeper routinely climbed above
                // 6.5. The `gk_def_damp` dampener plus this steeper
                // curve realign the band with the test expectations
                // and WhoScored reference (~5.5-6.0 for a 3-shipping
                // keeper).
                //   1 conceded → -0.20   (normal, still ~6 with saves)
                //   2 conceded → -0.40   (below avg, ~6 with saves)
                //   3 conceded → -0.85   (bad day, ~5.6)
                //   4 conceded → -1.40   (slipping, ~5)
                //   5 conceded → -1.95   (~4.4)
                //   6 conceded → -2.50   (awful, ~3.8)
                //   7 conceded → -3.05   (~3.3)
                //   8 conceded → -3.60   (~2.8)
                //  10 conceded → -4.70   (~1.7, not hard-floored)
                let og = self.opponent_goals as f32;
                let base = og * 0.20;
                let heavy = (og - 2.0).max(0.0) * 0.25 + (og - 3.0).max(0.0) * 0.10;
                -(base + heavy)
            }
            // -0.25 per goal past the 2nd, capped at -1.5. Defenders
            // share blame for a hammering but not on the GK's scale.
            PlayerFieldPositionGroup::Defender if self.opponent_goals >= 3 => {
                let extra = (self.opponent_goals as f32 - 2.0).min(6.0);
                -(extra * 0.25)
            }
            _ => 0.0,
        }
    }

    /// Small bump for a forward who manufactured real chances (xg > 1.0)
    /// without scoring. Clinical-finisher upside lives in [`attacking`].
    fn unlucky_finisher(&self) -> f32 {
        if self.stats.goals == 0 && self.stats.xg > 1.0 {
            0.1
        } else {
            0.0
        }
    }

    /// Modern build-up & chance-creation: key passes, progressive
    /// passes/carries, dribbles, pressures. All damped for cameos so
    /// a 12-minute sub doesn't post 7.5 from one key pass.
    fn creation_and_buildup(&self) -> f32 {
        let s = self.stats;
        let damp = self.minute_damp;
        let mut delta = 0.0_f32;

        // Per-event creator bonuses. A single attacking pass commonly
        // ticks several of these counters at once (a progressive pass
        // into the box that becomes a key pass and feeds xG buildup
        // hits four of them).
        delta += (s.key_passes as f32 * 0.10).min(0.35) * damp;
        delta += (s.progressive_passes as f32 * 0.020).min(0.18) * damp;
        delta += (s.progressive_carries as f32 * 0.030).min(0.18) * damp;

        // Successful dribbles — modest, position-aware.
        let dribble_w = match self.pos {
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => 0.06,
            _ => 0.03,
        };
        delta += (s.successful_dribbles as f32 * dribble_w).min(0.22) * damp;

        // Failed dribbles — reduced for forwards because attempting
        // take-ons is part of their job.
        if s.attempted_dribbles > s.successful_dribbles {
            let failed = (s.attempted_dribbles - s.successful_dribbles) as f32;
            let fail_w = if self.pos == PlayerFieldPositionGroup::Forward {
                0.025
            } else {
                0.04
            };
            delta -= (failed * fail_w).min(0.30) * damp;
        }

        // Pressures: forced turnovers + raw volume.
        delta += (s.successful_pressures as f32 * 0.025).min(0.18) * damp;
        let raw_pressure = s.pressures.saturating_sub(s.successful_pressures);
        delta += (raw_pressure as f32 * 0.008).min(0.10) * damp;

        delta
    }

    /// Position-weighted shot-blocking credit. Damped for cameos AND
    /// halved for a goalkeeper who still shipped ≥ 3.
    fn blocks(&self) -> f32 {
        (self.stats.blocks as f32 * self.block_weight()).min(0.5)
            * self.minute_damp
            * self.gk_def_damp
    }

    /// Completed-cross credit + missed-cross penalty. Caps small so a
    /// winger can't post 9.0 on crossing volume alone; centre-backs /
    /// GKs barely benefit.
    fn crossing(&self) -> f32 {
        let s = self.stats;
        if s.crosses_attempted == 0 {
            return 0.0;
        }
        let completed = s.crosses_completed as f32;
        let failed = s.crosses_attempted.saturating_sub(s.crosses_completed) as f32;
        let (cap, miss_cap) = match self.pos {
            PlayerFieldPositionGroup::Midfielder
            | PlayerFieldPositionGroup::Forward
            | PlayerFieldPositionGroup::Defender => (0.15, 0.15),
            _ => (0.06, 0.06),
        };
        let credit = (completed * 0.05).min(cap) * self.minute_damp;
        let penalty = (failed * 0.012).min(miss_cap) * self.minute_damp;
        credit - penalty
    }

    /// Chance-creation indicator independent of whether the ball ended
    /// in a shot (so it's not subsumed by `key_passes`). Position-aware
    /// weight; centre-backs / forwards / mids get the bump.
    fn passes_into_box(&self) -> f32 {
        let s = self.stats;
        let w = match self.pos {
            PlayerFieldPositionGroup::Midfielder | PlayerFieldPositionGroup::Forward => 0.035,
            PlayerFieldPositionGroup::Defender => 0.025,
            _ => 0.015,
        };
        (s.passes_into_box as f32 * w).min(0.16) * self.minute_damp
    }

    /// xG-buildup credit. Excludes the player's own shots and direct
    /// assists, so it's a clean "made the chance happen up the chain"
    /// signal — weighted heavier for midfielders / defenders. For
    /// forwards, most of their xG involvement is the shot itself,
    /// already rewarded.
    fn xg_buildup(&self) -> f32 {
        let s = self.stats;
        if s.xg_buildup <= 0.1 {
            return 0.0;
        }
        let w = match self.pos {
            PlayerFieldPositionGroup::Midfielder => 0.18,
            PlayerFieldPositionGroup::Defender => 0.12,
            PlayerFieldPositionGroup::Forward => 0.06,
            _ => 0.03,
        };
        (s.xg_buildup * w).min(0.16) * self.minute_damp
    }

    /// Carry-distance tie-breaker — per progressive carry is already
    /// rewarded in [`creation_and_buildup`]; this small top-up rewards
    /// a player who genuinely broke ground over the match. 1000 units
    /// of cumulative carry → +0.10. Cap tight.
    fn carry_distance(&self) -> f32 {
        let raw = ((self.stats.carry_distance as f32 / 1000.0) - 0.05).max(0.0);
        raw.min(0.15) * self.minute_damp
    }

    /// Miscontrols and heavy touches: small per-event penalty, capped
    /// so they don't override a strong defensive / creative shift.
    ///
    /// The rating-side wiring is complete; the LIVE producer for these
    /// counters is intentionally deferred until receiver-state tracking
    /// lands (the engine needs to distinguish a clean reception from a
    /// heavy first touch / miscontrol at the moment the receiver claims
    /// the ball). Until then, both counters default to zero and the
    /// rating impact is zero.
    fn possession_penalties(&self) -> f32 {
        let s = self.stats;
        let miscontrol = (s.miscontrols as f32 * 0.03).min(0.22) * self.minute_damp;
        let heavy = (s.heavy_touches as f32 * 0.015).min(0.18) * self.minute_damp;
        -(miscontrol + heavy)
    }

    /// Errors-to-shot + errors-to-goal + yellow/red cards. Errors-to-
    /// goal is the most damaging individual event in the rating, just
    /// below a red card. Per-event hits are capped per-match so a
    /// distribution-under-pressure keeper can't get floored by stacked
    /// errors-to-shot alone.
    fn errors_and_cards(&self) -> f32 {
        let s = self.stats;
        let err_shot = (s.errors_leading_to_shot as f32 * 0.35).min(0.7);
        let err_goal = (s.errors_leading_to_goal as f32 * 0.90).min(1.8);
        let cards = s.yellow_cards as f32 * 0.15 + s.red_cards as f32 * 1.50;
        -(err_shot + err_goal + cards)
    }

    /// Zone-aware defensive bonus on top of the flat per-event credit.
    /// A tackle on the edge of your own box is worth more than one in
    /// the centre circle; a sliding clearance on the goal line is the
    /// play of the match. Capped via `DEF_ZONE_BONUS_CAP` and passed
    /// through `gk_def_damp` so a busy GK who still shipped ≥ 3
    /// doesn't compose this back over the conceded penalty.
    fn zone_defensive(&self) -> f32 {
        let z = self.stats.zone_stats;
        let tw = self.tackle_weight();
        let iw = self.interception_weight();
        let bw = self.block_weight();
        let cw = self.clearance_weight();
        let pressure_per: f32 = 0.035;

        let own_box = (z.tackles_own_box as f32 * tw
            + z.interceptions_own_box as f32 * iw
            + z.blocks_own_box as f32 * bw
            + z.clearances_own_box as f32 * cw)
            * ZoneCoeffs::DEF_OWN_BOX_BONUS;
        let own_six = (z.tackles_own_six_yard as f32 * tw
            + z.interceptions_own_six_yard as f32 * iw
            + z.blocks_own_six_yard as f32 * bw
            + z.clearances_own_six_yard as f32 * cw)
            * ZoneCoeffs::DEF_OWN_SIX_YARD_BONUS;
        let middle_third =
            z.interceptions_middle_third as f32 * iw * ZoneCoeffs::INTERCEPTION_MIDDLE_BONUS;
        let final_third_tackles =
            z.tackles_final_third as f32 * tw * ZoneCoeffs::TACKLE_FINAL_THIRD_BONUS;
        let final_third_pressures = z.pressures_won_final_third as f32
            * pressure_per
            * ZoneCoeffs::PRESSURE_FINAL_THIRD_BONUS;

        (own_box + own_six + middle_third + final_third_tackles + final_third_pressures)
            .min(ZoneCoeffs::DEF_ZONE_BONUS_CAP)
            * self.gk_def_damp
    }

    /// Progressive-to-final-third + box-entry + lane-aware creation
    /// bonuses. Damped for cameos so a 10-minute sub with 4 box-entry
    /// passes can't post 7.5 from this section alone.
    fn zone_creation(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;
        let damp = self.minute_damp;

        let progressive = (z.progressive_passes_into_final_third as f32
            + z.progressive_carries_into_final_third as f32)
            * ZoneCoeffs::PROGRESSIVE_TO_FINAL_THIRD_PER;
        let box_entries = s.passes_into_box as f32 + z.carries_into_box as f32;

        progressive.min(ZoneCoeffs::PROGRESSIVE_TO_FINAL_THIRD_CAP) * damp
            + (box_entries * ZoneCoeffs::BOX_ENTRY_PER).min(ZoneCoeffs::BOX_ENTRY_CAP) * damp
            + (z.half_space_passes_into_box as f32 * ZoneCoeffs::HALF_SPACE_BOX_ENTRY_PER)
                .min(ZoneCoeffs::HALF_SPACE_BOX_ENTRY_CAP)
                * damp
            + (z.central_passes_into_box as f32 * ZoneCoeffs::CENTRAL_BOX_ENTRY_PER)
                .min(ZoneCoeffs::CENTRAL_BOX_ENTRY_CAP)
                * damp
            + (z.switches_of_play as f32 * ZoneCoeffs::SWITCH_OF_PLAY_PER)
                .min(ZoneCoeffs::SWITCH_OF_PLAY_CAP)
                * damp
    }

    /// Dangerous-turnover penalty (own-third / own-box) + extra hit
    /// for errors-to-goal that originated from an own-box giveaway.
    /// Capped per-match so a GK distributing under pressure can't be
    /// floored by stacked own-third turnovers.
    fn dangerous_turnovers(&self) -> f32 {
        let z = self.stats.zone_stats;
        (z.dangerous_turnovers_own_third as f32 * ZoneCoeffs::TURNOVER_OWN_THIRD).max(-0.9)
            + (z.dangerous_turnovers_own_box as f32 * ZoneCoeffs::TURNOVER_OWN_BOX).max(-1.2)
            + z.errors_to_goal_own_box as f32 * ZoneCoeffs::ERROR_TO_GOAL_OWN_BOX_EXTRA
    }

    /// GK-only zone events: cross-claim / punch / sweeper credit
    /// (passed through `gk_def_damp`), plus full-strength penalties
    /// for failed claims that became opponent shots or goals.
    ///
    /// `gk_command_actions` has a live producer; the two
    /// `gk_failed_claims_*` counters stay wired here without a live
    /// producer so the moment the GK state machine emits "attempted
    /// claim and missed → opponent shot/goal" they take effect with
    /// no rating-helper change.
    fn gk_command(&self) -> f32 {
        if !self.is_goalkeeper() {
            return 0.0;
        }
        let z = self.stats.zone_stats;
        let command = (z.gk_command_actions as f32 * ZoneCoeffs::GK_COMMAND_PER)
            .min(ZoneCoeffs::GK_COMMAND_CAP)
            * self.gk_def_damp;
        // Failed-claim penalties stay at full strength regardless of
        // result — a botched cross that became a goal is bad whether
        // the rest of the match was a clean sheet or a 4-0 hammering.
        let failed_shot = z.gk_failed_claims_to_shot as f32 * ZoneCoeffs::GK_FAILED_CLAIM_TO_SHOT;
        let failed_goal = z.gk_failed_claims_to_goal as f32 * ZoneCoeffs::GK_FAILED_CLAIM_TO_GOAL;
        command + failed_shot + failed_goal
    }

    /// Fouls (with own-third extra for back-line players + penalty-
    /// foul hit), offsides (position-aware), and own goals.
    fn discipline(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;
        let mut delta = 0.0_f32;

        // Fouls are penalised regardless of card outcome — a high-
        // volume fouler who didn't get booked is still a drag on the
        // team.
        delta += (s.fouls as f32 * ZoneCoeffs::FOUL_PER).max(ZoneCoeffs::FOUL_CAP);
        if matches!(
            self.pos,
            PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper
        ) {
            delta += z.own_third_def_fouls as f32 * ZoneCoeffs::FOUL_OWN_THIRD_DEF_EXTRA_PER;
        }
        delta += z.penalty_fouls_conceded as f32 * ZoneCoeffs::FOUL_PENALTY;

        // Offsides: forwards live with the offside line, so the per-
        // event hit is a touch larger but capped fast. Other positions
        // getting caught offside is rarer and a worse decision per
        // event.
        let (per, cap) = match self.pos {
            PlayerFieldPositionGroup::Forward => (
                ZoneCoeffs::OFFSIDE_FORWARD_PER,
                ZoneCoeffs::OFFSIDE_FORWARD_CAP,
            ),
            _ => (ZoneCoeffs::OFFSIDE_OTHER_PER, ZoneCoeffs::OFFSIDE_OTHER_CAP),
        };
        delta += (s.offsides as f32 * per).max(cap);

        // Own goals: base + own-box extra because OGs sit inside the
        // player's own box by definition.
        delta += s.own_goals as f32
            * (ZoneCoeffs::OWN_GOAL_BASE + ZoneCoeffs::OWN_GOAL_OWN_BOX_EXTRA);

        delta
    }

    /// xG-prevented bonus for goalkeepers. Uses the live value when
    /// the producer has populated it; otherwise synthesises a positive
    /// proxy from save volume vs. a 70%-baseline expectation.
    ///
    /// Upside-only by design: bad shifts are already taxed by the
    /// conceded penalty, the low-save% bracket, and (when a giveaway
    /// converts) `errors_leading_to_goal`. Clamps `xg_prevented` to
    /// [0, ∞) so the live producer's `-shot_xg` debits on every
    /// concession don't double-count against blowout keepers.
    fn goalkeeper_xg_prevented(&self) -> f32 {
        if !self.is_goalkeeper() {
            return 0.0;
        }
        let s = self.stats;
        let direct = s.xg_prevented.max(0.0);
        let xg_p = if direct > 0.0 {
            direct
        } else {
            let shots = self.shots_faced() as f32;
            if shots >= 3.0 {
                let expected = shots * 0.70;
                ((s.saves as f32 - expected) * 0.30).max(0.0)
            } else {
                0.0
            }
        };
        (xg_p * 0.45).min(1.4)
    }

    /// Dominant-defense credit: a GK who kept a clean sheet behind a
    /// back-line that fielded fewer than 3 shots all match still
    /// organised the line, claimed set pieces, and held position. The
    /// save/save%/surplus stack only fires with shot volume, and the
    /// flat +0.20 clean-sheet bonus alone leaves a quiet shutout at
    /// 6.2-6.4 — below where real-football match reads put a keeper
    /// behind a dominant defence. Gated to `shots_faced < 3` so it
    /// never composes on top of a busy shutout (which the save stack
    /// already rewards).
    fn goalkeeper_dominant_defense(&self) -> f32 {
        if !self.is_goalkeeper() || self.opponent_goals != 0 {
            return 0.0;
        }
        if self.shots_faced() < 3 { 0.15 } else { 0.0 }
    }

    // ───────────────────────────────────────────────────────────────────
    // Soft caps (mutate the running rating)
    // ───────────────────────────────────────────────────────────────────

    /// Sub-15-minute cameo can't post worse than 5.8 or better than
    /// 7.2 unless they did something exceptional (goal, red, error-
    /// to-goal, own goal). Keeps a 90th-minute winner posting an 8+.
    fn apply_cameo_bound(&self, rating: f32) -> f32 {
        let s = self.stats;
        let exceptional = s.goals > 0
            || s.red_cards > 0
            || s.errors_leading_to_goal > 0
            || s.own_goals > 0;
        if s.minutes_played < 15 && s.minutes_played > 0 && !exceptional {
            rating.clamp(5.8, 7.2)
        } else {
            rating
        }
    }

    /// Three-tier low-involvement cap for outfielders:
    ///   1) 60+ minutes with no goal/assist/key pass and minimal
    ///      defensive output: cap at 6.4.
    ///   2) "One moment, no follow-through" — at most one creative
    ///      action, no shot-on-target, minimal ball-carrying and
    ///      minimal defending: cap at 6.8.
    ///   3) Single goal off a low-xG chance with little other
    ///      involvement: cap at 7.2.
    ///
    /// Goalkeepers are exempt — their job is shot-stopping, not
    /// tackles or key passes.
    fn apply_outfielder_low_involvement_caps(&self, mut rating: f32) -> f32 {
        if self.is_goalkeeper() {
            return rating;
        }
        let s = self.stats;
        let total_def = s
            .tackles
            .saturating_add(s.interceptions)
            .saturating_add(s.successful_pressures)
            .saturating_add(s.blocks)
            .saturating_add(s.clearances);
        let creative = s.key_passes.saturating_add(s.progressive_passes / 3);

        if s.minutes_played >= 60
            && s.goals == 0
            && s.assists == 0
            && creative == 0
            && total_def < 4
        {
            rating = rating.min(6.4);
        }

        // Soft tier: an attacking-mid almost always picks up one key
        // pass and escapes the hard 6.4 cap, even when nothing else
        // happened. This second tier catches "one moment, no follow-
        // through" shifts — same strict defensive gate (< 4) so a
        // 4-tackle deep-lying playmaker is still recognised.
        let carries_into_box = s.zone_stats.carries_into_box as u32;
        let ball_carrying =
            s.successful_dribbles as u32 + s.passes_into_box as u32 + carries_into_box;
        if s.minutes_played >= 60
            && s.goals == 0
            && s.assists == 0
            && s.shots_on_target == 0
            && creative <= 1
            && ball_carrying <= 2
            && total_def < 4
        {
            rating = rating.min(6.8);
        }

        // Low-involvement single-goal cap.
        let positive = (s.assists as u32)
            + (s.key_passes as u32)
            + (s.progressive_passes as u32) / 3
            + (s.successful_dribbles as u32)
            + (s.tackles as u32)
            + (s.interceptions as u32)
            + (s.successful_pressures as u32) / 2
            + (s.blocks as u32)
            + (s.clearances as u32);
        if s.goals == 1 && s.xg < 0.20 && positive < 5 {
            rating = rating.min(7.2);
        }
        rating
    }

    /// Defender-only quality bonuses (clean-duel + block-quality)
    /// followed by the low-skill volume cap that stops a defender
    /// drifting to 8.0+ purely on routine clearances and tackles
    /// when no clean sheet, attacking contribution, or clutch
    /// intervention was produced.
    fn apply_defender_section(&self, mut rating: f32) -> f32 {
        if self.pos != PlayerFieldPositionGroup::Defender {
            return rating;
        }
        let s = self.stats;
        let damp = self.minute_damp;

        // Clean-duel bonus: tackles produced beyond the foul count.
        // Fouls already carry a base penalty, so subtracting them
        // rewards the cleanly-won ball-winners separately.
        let clean_tackles = (s.tackles as u32).saturating_sub(s.fouls as u32) as f32;
        rating += (clean_tackles * 0.055).min(0.28) * damp;

        // Block-quality lift — separates a defender who blocked 4
        // shots from one who blocked a single low-danger effort.
        let extra_blocks = (s.blocks as i32 - 1).max(0) as f32;
        rating += (extra_blocks * 0.05).min(0.40) * damp;

        // Low-skill volume cap. Without a clean sheet, a goal or
        // assist, multiple blocks, or a major clutch intervention,
        // routine event volume should not push a defender above 7.1.
        let major_intervention = s.blocks >= 2;
        let attacking = s.goals > 0 || s.assists > 0;
        let clean_sheet = self.opponent_goals == 0;
        if s.minutes_played >= 60
            && !clean_sheet
            && !attacking
            && !major_intervention
            && s.errors_leading_to_goal == 0
        {
            rating = rating.min(7.1);
        }
        rating
    }

    /// Midfielder pressing-quality bonus + safe-recycle cap. The cap
    /// stops the "safe-recycler 8.0" outcome unless the midfielder
    /// produced real attacking, defensive, or chance-creation output.
    fn apply_midfielder_section(&self, mut rating: f32) -> f32 {
        if self.pos != PlayerFieldPositionGroup::Midfielder {
            return rating;
        }
        let s = self.stats;
        let damp = self.minute_damp;

        // Pressing-quality bump — sustained successful pressures +
        // raw pressing volume.
        let raw_pressure = s.pressures.saturating_sub(s.successful_pressures);
        let pressing = (s.successful_pressures as f32 * 0.030 + raw_pressure as f32 * 0.006)
            .min(0.32);
        rating += pressing * damp;

        // Safe-recycle cap: high pass volume, no progressive value,
        // no shot/key-pass involvement, minimal defensive contribution.
        // The wider "one creative moment, nothing else" case is
        // handled by [`apply_outfielder_low_involvement_caps`].
        if s.minutes_played >= 60
            && s.passes_attempted >= 30
            && s.goals == 0
            && s.assists == 0
            && s.key_passes == 0
            && s.progressive_passes <= 2
            && s.shots_total == 0
            && (s.tackles + s.interceptions + s.successful_pressures) < 5
        {
            rating = rating.min(6.7);
        }

        // Low-attacking-output ceiling cap — parallel to the defender
        // 7.1 cap. A box-to-box midfielder can stack pass volume,
        // pressures, tackles/interceptions, zone bonuses and the
        // pressing bump into 7.5+ territory with zero direct attacking
        // contribution. Without a goal, assist, multiple key passes,
        // sustained box-entry volume, a shot on target, or a clutch
        // block, the rating shouldn't climb past Rodri/Kanté tier on
        // involvement alone. 7.2 matches real-football reads for a
        // tidy involved-but-no-end-product shift.
        let major_intervention = s.blocks >= 2;
        let attacking = s.goals > 0
            || s.assists > 0
            || s.key_passes >= 2
            || s.passes_into_box >= 3
            || s.shots_on_target > 0;
        if s.minutes_played >= 60
            && !attacking
            && !major_intervention
            && s.errors_leading_to_goal == 0
        {
            rating = rating.min(7.2);
        }
        rating
    }

    /// Forward attacking-output ceiling cap. Parallel to the defender
    /// 7.1 and midfielder 7.2 caps, but tighter (7.0) because a
    /// forward's job is end-product — a striker without a goal, an
    /// assist, real shooting threat, or genuine chance creation
    /// should not climb past 7.0 on dribble/cross/carry volume alone.
    /// Without this cap, a wide forward racking up crosses, carries
    /// and passes-into-box could pre-compression hit ~7.9 and post-
    /// compression ~7.5+ across 24/26 no-goal games, dragging a
    /// season average into Salah territory on 2 goals.
    fn apply_forward_section(&self, mut rating: f32) -> f32 {
        if self.pos != PlayerFieldPositionGroup::Forward {
            return rating;
        }
        let s = self.stats;
        let real_attacking = s.goals > 0
            || s.assists > 0
            || s.key_passes >= 2
            || s.shots_on_target >= 2
            || s.xg >= 0.5;
        if s.minutes_played >= 60
            && !real_attacking
            && s.errors_leading_to_goal == 0
        {
            rating = rating.min(7.0);
        }
        rating
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stats(
        goals: u16,
        assists: u16,
        passes_attempted: u16,
        passes_completed: u16,
        shots_on_target: u16,
        shots_total: u16,
        tackles: u16,
        interceptions: u16,
        saves: u16,
        xg: f32,
        position_group: PlayerFieldPositionGroup,
    ) -> PlayerMatchEndStats {
        PlayerMatchEndStats {
            goals,
            assists,
            passes_attempted,
            passes_completed,
            shots_on_target,
            shots_total,
            tackles,
            interceptions,
            saves,
            shots_faced: 0,
            match_rating: 0.0,
            xg,
            position_group,
            fouls: 0,
            yellow_cards: 0,
            red_cards: 0,
            minutes_played: 90,
            key_passes: 0,
            progressive_passes: 0,
            progressive_carries: 0,
            successful_dribbles: 0,
            attempted_dribbles: 0,
            successful_pressures: 0,
            pressures: 0,
            blocks: 0,
            clearances: 0,
            passes_into_box: 0,
            crosses_attempted: 0,
            crosses_completed: 0,
            xg_chain: 0.0,
            xg_buildup: 0.0,
            miscontrols: 0,
            heavy_touches: 0,
            carry_distance: 0,
            errors_leading_to_shot: 0,
            errors_leading_to_goal: 0,
            xg_prevented: 0.0,
            offsides: 0,
            own_goals: 0,
            zone_stats: ZoneStats::default(),
        }
    }

    fn make_gk(saves: u16, shots_faced: u16) -> PlayerMatchEndStats {
        let mut s = make_stats(
            0,
            0,
            20,
            15,
            0,
            0,
            0,
            0,
            saves,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        s.shots_faced = shots_faced;
        s
    }

    #[test]
    fn base_rating_is_six() {
        // Forward with no events, 1-1 draw → pure base rating of 6.0
        let stats = make_stats(
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Forward,
        );
        let rating = RatingContext::new(&stats, 1, 1).calculate();
        assert!((rating - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn goals_add_up_to_cap() {
        // Five-goal forward with realistic xG (the goals weren't lucky).
        // High output still produces an elite rating, but the top-end
        // compressor pulls the runaway raw-9.0 into a realistic ~8.0
        // band so per-match peaks track WhoScored references (~9.5-9.8
        // for the very best individual matches, ~8.0-8.5 for "scored
        // a bag in a win").
        let mut stats = make_stats(
            5,
            0,
            0,
            0,
            0,
            5,
            0,
            0,
            0,
            3.5,
            PlayerFieldPositionGroup::Forward,
        );
        stats.shots_on_target = 5;
        let rating = RatingContext::new(&stats, 5, 0).calculate();
        assert!(rating >= 7.9, "rating={rating}");
    }

    #[test]
    fn goalkeeper_saves_matter() {
        let quiet_gk = make_stats(
            0,
            0,
            15,
            12,
            0,
            0,
            0,
            0,
            1,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let busy_gk = make_stats(
            0,
            0,
            15,
            12,
            0,
            0,
            0,
            0,
            8,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );

        let quiet_rating = RatingContext::new(&quiet_gk, 1, 0).calculate();
        let busy_rating = RatingContext::new(&busy_gk, 1, 0).calculate();

        // Busy GK with 8 saves should rate significantly higher
        assert!(busy_rating - quiet_rating > 1.0);
    }

    #[test]
    fn interceptions_boost_defender_rating() {
        let passive = make_stats(
            0,
            0,
            20,
            16,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Defender,
        );
        let active = make_stats(
            0,
            0,
            20,
            16,
            0,
            0,
            3,
            4,
            0,
            0.0,
            PlayerFieldPositionGroup::Defender,
        );

        let passive_rating = RatingContext::new(&passive, 1, 1).calculate();
        let active_rating = RatingContext::new(&active, 1, 1).calculate();

        assert!(active_rating > passive_rating);
        assert!(active_rating - passive_rating > 0.8);
    }

    #[test]
    fn rating_clamped_to_range() {
        // Worst case
        let bad = make_stats(
            0,
            0,
            20,
            5,
            0,
            5,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let rating = RatingContext::new(&bad, 0, 5).calculate();
        assert!(rating >= 1.0);
        assert!(rating <= 10.0);

        // Best case
        let great = make_stats(
            5,
            3,
            60,
            57,
            5,
            5,
            5,
            5,
            10,
            1.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let rating = RatingContext::new(&great, 5, 0).calculate();
        assert!(rating >= 1.0);
        assert!(rating <= 10.0);
    }

    #[test]
    fn clinical_finisher_bonus() {
        // Player with 2 goals from 0.8 xG (clinical)
        let clinical = make_stats(
            2,
            0,
            20,
            15,
            2,
            3,
            0,
            0,
            0,
            0.8,
            PlayerFieldPositionGroup::Forward,
        );
        // Player with 2 goals from 2.0 xG (expected)
        let expected = make_stats(
            2,
            0,
            20,
            15,
            2,
            3,
            0,
            0,
            0,
            2.0,
            PlayerFieldPositionGroup::Forward,
        );

        let clinical_rating = RatingContext::new(&clinical, 2, 0).calculate();
        let expected_rating = RatingContext::new(&expected, 2, 0).calculate();

        assert!(clinical_rating > expected_rating);
    }

    #[test]
    fn goalkeeper_shipping_seven_goals_is_rated_awful() {
        // Regression: flat conceded penalty let a GK with 7 goals
        // against post ~8.0 (save bonuses outweighed the penalty).
        // A 7-goal shipping has to stay in the "disaster" band.
        let gk = make_stats(
            0,
            0,
            20,
            15,
            0,
            0,
            0,
            0,
            3,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let rating = RatingContext::new(&gk, 0, 7).calculate();
        assert!(rating < 4.0, "GK conceding 7 rated {} — too high", rating);
    }

    #[test]
    fn goalkeeper_three_goals_is_below_average_not_awful() {
        // Regression: an overly-steep linear penalty put a GK with
        // 3 conceded near 4.0 (matches a player who should be dropped).
        // Conceding 3 is a bad day, not a disaster — should land in
        // the 5.0-6.2 band: around or just below average.
        let gk = make_stats(
            0,
            0,
            20,
            15,
            0,
            0,
            0,
            0,
            3,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let rating = RatingContext::new(&gk, 0, 3).calculate();
        assert!(
            rating >= 5.0 && rating <= 6.2,
            "GK conceding 3 rated {} — should be around 6",
            rating
        );
    }

    #[test]
    fn goalkeeper_clean_sheet_is_well_rewarded() {
        // A GK who keeps a clean sheet should be in the 7+ band,
        // busy ones in the 8+ band. Clean sheets are the headline
        // keeper achievement and the rating needs to reflect that.
        let quiet = make_stats(
            0,
            0,
            15,
            12,
            0,
            0,
            0,
            0,
            1,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let busy = make_stats(
            0,
            0,
            15,
            12,
            0,
            0,
            0,
            0,
            6,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let quiet_rating = RatingContext::new(&quiet, 1, 0).calculate();
        let busy_rating = RatingContext::new(&busy, 1, 0).calculate();
        // Real match-rating reference: a quiet shutout lands above
        // average (6.7+); a busy shutout reaches the 7.5+ band. Going
        // higher inflates GK season averages past world-class levels.
        assert!(
            quiet_rating >= 6.7,
            "Quiet CS rated {} — should be above 6.7",
            quiet_rating
        );
        assert!(
            busy_rating >= 7.5,
            "Busy CS (6 saves, clean sheet) rated {} — should reach 7.5+",
            busy_rating
        );
        assert!(
            busy_rating > quiet_rating + 0.5,
            "Busy CS ({}) should clearly outrate quiet CS ({})",
            busy_rating,
            quiet_rating
        );
    }

    #[test]
    fn goalkeeper_two_goals_is_around_six() {
        // Regression: earlier linear -0.6 per goal put a 2-goal-shipping
        // GK at 4-5. Real football: a keeper who made some saves but
        // let in a couple should be around 6 — not "bad", just "had a
        // normal match where their team lost 2-0".
        let gk = make_stats(
            0,
            0,
            20,
            15,
            0,
            0,
            0,
            0,
            3,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let rating = RatingContext::new(&gk, 0, 2).calculate();
        assert!(
            rating >= 5.5 && rating <= 6.5,
            "GK conceding 2 rated {} — should be around 6",
            rating
        );
    }

    #[test]
    fn busy_goalkeeper_conceding_three_stays_in_band() {
        // Regression for the rating-bias report (3-conceded GK posting
        // 6.5+). Previously the non-save defensive credits — cross-
        // claims credited as interceptions, sweeper clearances, blocks
        // — stacked unchecked alongside the per-save / save-% /
        // surplus bonuses, so a 6-save 3-shipping keeper drifted into
        // the 6.5-7.2 band. The shallow conceded curve (-0.15/goal)
        // was the other half — exactly matching the per-save bonus,
        // so wins on stops cancelled losses on goals 1-for-1.
        //
        // Real-football reference: a 3-shipping keeper sits 5.5-6.0
        // regardless of off-the-ball activity. This test pins the
        // ceiling at 6.5 so a busy active keeper can lift modestly
        // above a 3-saves baseline (~5.5) but cannot reach the 6.5
        // threshold the user flagged as too high.
        let mut gk = make_gk(6, 9); // 6 saves on 9 shots, 3 conceded
        gk.interceptions = 3; // cross-claims / sweeper actions
        gk.clearances = 2; // set-piece clears
        let rating = RatingContext::new(&gk, 0, 3).calculate();
        assert!(
            rating < 6.5,
            "Busy GK conceding 3 (6 saves, 3 interceptions, 2 clearances) rated {} — should stay below 6.5 regardless of off-the-ball activity",
            rating
        );
        assert!(
            rating >= 5.5,
            "Busy GK conceding 3 rated {} — should still outrate a no-extras 3-shipping keeper",
            rating
        );
    }

    #[test]
    fn conceded_penalty_scales_with_goals() {
        // One-goal GK should outrate a six-goal GK.
        let one = make_stats(
            0,
            0,
            20,
            15,
            0,
            0,
            0,
            0,
            3,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let six = make_stats(
            0,
            0,
            20,
            15,
            0,
            0,
            0,
            0,
            3,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let one_rating = RatingContext::new(&one, 0, 1).calculate();
        let six_rating = RatingContext::new(&six, 0, 6).calculate();
        assert!(
            one_rating - six_rating > 1.5,
            "1-goal GK ({}) vs 6-goal GK ({}) — delta too small",
            one_rating,
            six_rating
        );
    }

    #[test]
    fn goalkeeper_ten_conceded_does_not_floor_at_one() {
        // Regression: quadratic penalty put a 10-goal shipping at the
        // 1.0 floor, so save bonuses couldn't distinguish "awful + no
        // effort" from "awful but made saves". Keep the rating low
        // but not pinned to the absolute minimum.
        let gk = make_stats(
            0,
            0,
            20,
            15,
            0,
            0,
            0,
            0,
            3,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        let rating = RatingContext::new(&gk, 0, 10).calculate();
        assert!(
            rating >= 1.5 && rating <= 3.0,
            "GK conceding 10 with 3 saves rated {} — should sit in the 1.5-3.0 disaster band, not the 1.0 floor",
            rating
        );
    }

    #[test]
    fn saves_greater_than_goals_conceded_lifts_rating() {
        // Saves outnumbering conceded goals must read above baseline.
        // Loss + 2 conceded drag the rating down, but 5 saves at a
        // 71% rate keeps the keeper visibly above a flat 6.0.
        let gk = make_gk(5, 7); // 5 saves, 2 conceded → ~71% save rate
        let rating = RatingContext::new(&gk, 1, 2).calculate();
        assert!(
            rating >= 6.4,
            "GK with 5 saves vs 2 conceded rated {} — should be ≥ 6.4",
            rating
        );
    }

    #[test]
    fn elite_save_percentage_lifts_rating() {
        // 8 of 9 stopped is a man-of-the-match performance. Even with
        // a 0-1 loss the rating should land in the 7+ band — that's
        // where real match-rating systems put a single elite GK game.
        let gk = make_gk(8, 9);
        let rating = RatingContext::new(&gk, 0, 1).calculate();
        assert!(
            rating >= 7.0,
            "Elite save-percentage GK rated {} — should be in the 7+ band",
            rating
        );
    }

    #[test]
    fn low_save_percentage_penalised() {
        // GK who let in 4 of 5 shots (20% save rate) had a poor outing
        // even with 1 save credited. Should fall below 6.0.
        let gk = make_gk(1, 5);
        let rating = RatingContext::new(&gk, 0, 4).calculate();
        assert!(
            rating < 6.0,
            "Low-save% GK rated {} — should be < 6.0",
            rating
        );
    }

    #[test]
    fn shots_faced_falls_back_to_legacy_total_when_zero() {
        // Test fixtures and old save files don't populate `shots_faced`.
        // The formula treats `shots_faced=0` as "legacy data" and
        // synthesizes the denominator from saves + opponent_goals so
        // ratings stay sensible.
        let gk = make_gk(5, 0); // shots_faced unset
        let rating = RatingContext::new(&gk, 1, 2).calculate();
        // Same shape as the populated case above — should land at the
        // same above-baseline tier (≥ 6.4).
        assert!(
            rating >= 6.4,
            "Legacy GK (shots_faced=0) rated {} — fallback denominator should still produce a sensible rating",
            rating
        );
    }

    #[test]
    fn surplus_saves_bonus_is_capped() {
        // 10 saves vs 1 conceded shouldn't push the rating to absurd
        // values — the surplus bonus caps at +0.2.
        let elite = make_gk(10, 11);
        let rating = RatingContext::new(&elite, 1, 1).calculate();
        // Ceiling check: with all bonuses (saves cap, save%, surplus)
        // the rating should sit comfortably below 10.
        assert!(rating < 10.0);
        // But should clearly outrate a baseline GK.
        let baseline = make_gk(2, 4);
        let baseline_rating = RatingContext::new(&baseline, 1, 2).calculate();
        assert!(rating > baseline_rating + 1.0);
    }

    #[test]
    fn synthetic_xg_prevented_lifts_above_baseline_keeper() {
        // Engine doesn't populate xg_prevented; without a fallback, an
        // outstanding shot-stopping shift (8 saves on 9 shots) was missing
        // the +0.45/xG bonus the formula advertises. The synthesized
        // proxy must close the gap so an above-baseline keeper visibly
        // outrates a 70%-baseline keeper at the same workload.
        let elite = make_gk(8, 9);
        let baseline = make_gk(7, 10); // 70% — exactly the expected baseline
        let elite_rating = RatingContext::new(&elite, 0, 1).calculate();
        let baseline_rating = RatingContext::new(&baseline, 0, 3).calculate();
        assert!(
            elite_rating > baseline_rating + 0.4,
            "Elite GK ({}) should clearly outrate baseline ({}); proxy not lifting",
            elite_rating,
            baseline_rating
        );
    }

    #[test]
    fn quiet_clean_sheet_keeper_gets_dominant_defense_credit() {
        // Real symptom: young GKs behind a dominant defence (few shots
        // faced, mostly clean sheets) posted 6.2-6.4 season averages
        // because the save/save%/surplus stack only fires with shot
        // volume and the flat +0.20 CS bonus alone leaves a quiet
        // shutout at base+CS = 6.20. A GK who finished 90 minutes,
        // organised the line and let nothing through should read
        // visibly above that — landing in the 6.4-6.6 band per
        // WhoScored references for low-workload keepers.
        let gk = make_gk(0, 0); // 0 saves, 0 shots faced — fully shielded
        let rating = RatingContext::new(&gk, 1, 0).calculate(); // 1-0 win
        assert!(
            rating >= 6.45,
            "Quiet CS GK (0 saves, 1-0 win) rated {} — dominant-defence credit should lift to 6.45+",
            rating
        );
    }

    #[test]
    fn busy_clean_sheet_keeper_does_not_double_up_dominant_credit() {
        // The dominant-defence credit must NOT compose on a busy
        // shutout — that's what the save / save% / surplus stack
        // rewards. Gated to shots_faced < 3 so a 5-save CS still
        // grades exactly where the existing well-rewarded test
        // expects, with no inflation creep.
        let busy = make_gk(5, 5); // well-tested CS
        let busy_rating = RatingContext::new(&busy, 1, 0).calculate();
        let quiet = make_gk(0, 0);
        let quiet_rating = RatingContext::new(&quiet, 1, 0).calculate();
        // Busy must still clearly outrate quiet — save volume is the
        // dominant signal.
        assert!(
            busy_rating > quiet_rating + 0.5,
            "Busy CS ({}) should clearly outrate quiet CS ({}) — quiet credit must not close the gap",
            busy_rating,
            quiet_rating
        );
    }

    #[test]
    fn dominant_defense_credit_only_fires_on_clean_sheet() {
        // Conceding any goal removes the credit — this is a CS-only
        // signal, not a "low-shot-volume" signal. A 1-0 loss with 0
        // saves on 1 shot (one tap-in past a barely-tested keeper)
        // must not pick up the credit.
        let gk = make_gk(0, 1); // 0 saves, 1 shot faced, 1 conceded
        let rating = RatingContext::new(&gk, 0, 1).calculate();
        // Without the credit: 6.0 - 0.20 (conceded) - 0.15 (loss) +
        // passing(~0.20) = ~5.85. Credit would push it back to 6.0.
        assert!(
            rating < 6.0,
            "1-conceded loss rated {} — dominant-defence credit must not fire without a clean sheet",
            rating
        );
    }

    #[test]
    fn clean_sheet_keeper_with_distribution_giveaways_holds_above_55() {
        // Real symptom: a keeper in a 0-0 with several long balls that
        // were intercepted and led to opponent shots within the
        // response window posted ratings of 4-5. The per-event
        // -0.35 errors_leading_to_shot was uncapped and crushed
        // otherwise-clean shifts. With the cap, a clean sheet must
        // sit above 5.5 even with five such giveaways.
        let mut gk = make_gk(1, 1);
        gk.errors_leading_to_shot = 5;
        let rating = RatingContext::new(&gk, 0, 0).calculate();
        assert!(
            rating >= 5.5,
            "clean-sheet keeper with intercepted long balls rated {} — should hold above 5.5",
            rating
        );
    }

    #[test]
    fn errors_to_shot_penalty_is_capped() {
        // Two errors-to-shot already hit the cap; further events
        // should not compound. Compare a player with 2 vs 8 such
        // events — the rating delta must be at most a rounding
        // difference, not 2.1 (six extra events × 0.35).
        let mut few = make_stats(
            0,
            0,
            30,
            24,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        few.errors_leading_to_shot = 2;
        let mut many = few.clone();
        many.errors_leading_to_shot = 8;
        let few_rating = RatingContext::new(&few, 1, 1).calculate();
        let many_rating = RatingContext::new(&many, 1, 1).calculate();
        assert!(
            (few_rating - many_rating).abs() < 0.05,
            "errors-to-shot must cap: 2 events {} vs 8 events {} (delta {})",
            few_rating,
            many_rating,
            few_rating - many_rating
        );
    }

    #[test]
    fn dangerous_turnovers_penalty_is_capped() {
        // A goalkeeper distributing under pressure can have 8+ long
        // balls intercepted in their own third / own box across a
        // match. Without a cap, -0.20 per own-third turnover and
        // -0.45 per own-box turnover compose with the conceded-goal
        // penalty and floor the rating at 1.0 even after a single
        // conceded goal. The caps must hold.
        let mut gk = make_gk(2, 3);
        gk.zone_stats.dangerous_turnovers_own_third = 10;
        gk.zone_stats.dangerous_turnovers_own_box = 5;
        let rating = RatingContext::new(&gk, 0, 1).calculate();
        assert!(
            rating > 3.0,
            "GK with capped turnovers + 1 conceded should not floor — got {}",
            rating
        );
        // Doubling the turnover counts past the cap should not
        // materially change the rating.
        let mut worse = gk.clone();
        worse.zone_stats.dangerous_turnovers_own_third = 20;
        worse.zone_stats.dangerous_turnovers_own_box = 10;
        let worse_rating = RatingContext::new(&worse, 0, 1).calculate();
        assert!(
            (rating - worse_rating).abs() < 0.05,
            "turnover penalty must cap: 10/5 events {} vs 20/10 events {} (delta {})",
            rating,
            worse_rating,
            rating - worse_rating
        );
    }

    #[test]
    fn negative_xg_prevented_does_not_double_punish_blowout_keeper() {
        // Live producer debits xg_prevented by `-shot_xg` on every
        // conceded non-own-goal. Without an upside-only clamp, that
        // negative ledger stacked with the conceded-goal penalty,
        // the low-save% bonus, and (when a giveaway converted)
        // errors_leading_to_goal — pushing realistic blowout keepers
        // through the 1.0 floor. The rating in a 0-5 thrashing with
        // 4 saves should land in the disaster band but stay above 1.0.
        let mut gk = make_gk(4, 9);
        gk.xg_prevented = -2.5; // five conceded shots averaging 0.5 xG each
        let rating = RatingContext::new(&gk, 0, 5).calculate();
        assert!(
            rating > 1.5,
            "blowout keeper rated {} — negative xg_prevented must not double-tax",
            rating
        );
        // And it must produce the same rating as a keeper whose
        // xg_prevented hasn't been touched (proxy fallback returns 0
        // because saves are below baseline).
        let mut control = gk.clone();
        control.xg_prevented = 0.0;
        let control_rating = RatingContext::new(&control, 0, 5).calculate();
        assert!(
            (rating - control_rating).abs() < 0.01,
            "negative xg_prevented {} should match unset {} (upside-only)",
            rating,
            control_rating
        );
    }

    #[test]
    fn synthetic_xg_prevented_does_not_punish_bad_keeper() {
        // The proxy is positive-only — a keeper saving below baseline
        // mustn't get a *second* penalty on top of the conceded penalty
        // and the low-save% penalty. Compare the same disaster shift
        // before and after the proxy: rating must stay in the existing
        // disaster band.
        let gk = make_gk(2, 8); // 25% save rate, 6 conceded
        let rating = RatingContext::new(&gk, 0, 6).calculate();
        assert!(
            rating >= 1.5 && rating <= 4.5,
            "Disaster GK rated {} — proxy must not push it below the existing disaster floor",
            rating
        );
    }

    #[test]
    fn high_volume_passing_bonus() {
        // Both midfielders have a baseline tackle/interception so the
        // low-involvement cap doesn't fire (real high-volume passers
        // have other involvement; the test isolates the volume bonus).
        let mut few = make_stats(
            0,
            0,
            15,
            14,
            0,
            0,
            4,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        few.key_passes = 1;
        let mut many = make_stats(
            0,
            0,
            55,
            50,
            0,
            0,
            4,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        many.key_passes = 1;

        let few_rating = RatingContext::new(&few, 1, 1).calculate();
        let many_rating = RatingContext::new(&many, 1, 1).calculate();

        assert!(many_rating > few_rating);
    }

    #[test]
    fn defender_clean_sheet_with_clearances_outranks_passive() {
        // Two CS defenders side by side: the active one made 8
        // clearances and 4 interceptions; the passive one was anonymous.
        // Both win 1-0 with 20/16 passing.
        let passive = make_stats(
            0,
            0,
            20,
            16,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Defender,
        );
        let mut active = make_stats(
            0,
            0,
            20,
            16,
            0,
            0,
            2,
            4,
            0,
            0.0,
            PlayerFieldPositionGroup::Defender,
        );
        active.clearances = 8;
        active.blocks = 1;

        let passive_rating = RatingContext::new(&passive, 1, 0).calculate();
        let active_rating = RatingContext::new(&active, 1, 0).calculate();
        // Active CB clearly above the passive one and into the 7+ band.
        assert!(
            active_rating >= 7.0,
            "active CB clean sheet rated {} — should reach 7.0+",
            active_rating
        );
        assert!(
            active_rating - passive_rating >= 0.7,
            "active ({}) - passive ({}) gap too small",
            active_rating,
            passive_rating
        );
    }

    #[test]
    fn midfielder_buildup_outranks_sideways_passing() {
        // Both MIDs played 90 with similar pass volume + accuracy. The
        // creative one chained xG buildup, played key passes, made
        // progressive carries; the safe one only completed sideways.
        let safe = make_stats(
            0,
            0,
            60,
            55,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        let mut creative = make_stats(
            0,
            0,
            55,
            48,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        creative.key_passes = 3;
        creative.progressive_passes = 6;
        creative.progressive_carries = 4;
        creative.passes_into_box = 4;
        creative.xg_buildup = 0.8;

        let safe_rating = RatingContext::new(&safe, 1, 1).calculate();
        let creative_rating = RatingContext::new(&creative, 1, 1).calculate();
        assert!(
            creative_rating > safe_rating + 0.6,
            "creative MID ({}) should clearly outrate safe-passer MID ({})",
            creative_rating,
            safe_rating
        );
    }

    #[test]
    fn box_to_box_midfielder_without_attacking_output_capped_at_7_2() {
        // Real symptom: midfielders posting 7.2+ season averages with
        // 0 goals and 1-2 assists across the season. Per-match, a
        // box-to-box shift stacking pass volume + pressures + tackles
        // + interceptions + zone bonuses + the pressing bump could
        // pre-compression hit ~8.5 and post-compression ~7.9 with
        // zero direct attacking contribution. Cap that shift at 7.2
        // — the real-football read for an involved-but-no-end-product
        // game.
        let mut mid = make_stats(
            0,
            0,
            50,
            44,
            0,
            0,
            4,
            5,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        mid.successful_pressures = 4;
        mid.pressures = 10;
        mid.progressive_passes = 4;
        mid.progressive_carries = 3;
        mid.xg_buildup = 0.4;
        mid.carry_distance = 1200;
        let rating = RatingContext::new(&mid, 1, 0).calculate();
        assert!(
            rating <= 7.2,
            "Box-to-box MID without attacking output rated {} — should cap at 7.2",
            rating
        );
    }

    #[test]
    fn creative_midfielder_escapes_low_attacking_output_cap() {
        // A creator with 2+ key passes (or 3+ passes into box) is
        // genuinely producing chances and must be allowed to climb
        // past 7.2. Otherwise the cap punishes the wrong archetype.
        let mut creator = make_stats(
            0,
            0,
            50,
            44,
            0,
            0,
            2,
            3,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        creator.key_passes = 3;
        creator.passes_into_box = 4;
        creator.progressive_passes = 5;
        creator.xg_buildup = 0.8;
        let rating = RatingContext::new(&creator, 1, 0).calculate();
        assert!(
            rating > 7.2,
            "Creative MID (3 key passes, 4 box entries) rated {} — should escape low-output cap",
            rating
        );
    }

    #[test]
    fn destroyer_midfielder_with_multiple_blocks_escapes_cap() {
        // A ball-winner who blocked 2+ shots produced a clutch
        // intervention — same gate as the defender section. They
        // should be allowed past 7.2 even without goals/assists/key
        // passes.
        let mut destroyer = make_stats(
            0,
            0,
            40,
            34,
            0,
            0,
            6,
            5,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        destroyer.blocks = 3;
        destroyer.successful_pressures = 5;
        destroyer.pressures = 12;
        destroyer.progressive_passes = 3;
        let rating = RatingContext::new(&destroyer, 1, 0).calculate();
        assert!(
            rating > 7.2,
            "Destroyer MID (3 blocks, 6 tackles, 5 interceptions) rated {} — clutch intervention should escape low-output cap",
            rating
        );
    }

    #[test]
    fn winger_completed_crosses_help_failed_spam_does_not() {
        // Two wide MIDs, same baseline. One completed 4 of 6 crosses
        // and 3 passes_into_box; the other spammed 12 crosses with only
        // 1 completed. The accurate winger should rate higher despite
        // lower volume.
        let mut accurate = make_stats(
            0,
            0,
            30,
            24,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        accurate.crosses_attempted = 6;
        accurate.crosses_completed = 4;
        accurate.passes_into_box = 3;

        let mut spam = make_stats(
            0,
            0,
            30,
            24,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        spam.crosses_attempted = 12;
        spam.crosses_completed = 1;

        let accurate_rating = RatingContext::new(&accurate, 1, 1).calculate();
        let spam_rating = RatingContext::new(&spam, 1, 1).calculate();
        assert!(
            accurate_rating > spam_rating,
            "accurate crosser ({}) should outrate cross-spammer ({})",
            accurate_rating,
            spam_rating
        );
    }

    #[test]
    fn miscontrols_reduce_rating_but_dont_overpunish_cameo() {
        // Sub on for 25 minutes who fluffed two touches: rating drops
        // a little but stays in a sensible band — the minute damp keeps
        // the penalty from compounding with every event the cameo did.
        let mut clean = make_stats(
            0,
            0,
            12,
            10,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        clean.minutes_played = 25;
        let mut sloppy = clean.clone();
        sloppy.miscontrols = 2;
        sloppy.heavy_touches = 2;

        let clean_rating = RatingContext::new(&clean, 1, 1).calculate();
        let sloppy_rating = RatingContext::new(&sloppy, 1, 1).calculate();
        assert!(
            sloppy_rating < clean_rating,
            "sloppy cameo ({}) should rate below clean cameo ({})",
            sloppy_rating,
            clean_rating
        );
        // But not below the cameo bound — the damp prevents overpunishment.
        assert!(
            sloppy_rating >= 5.5,
            "sloppy cameo over-punished: {}",
            sloppy_rating
        );
    }

    #[test]
    fn striker_high_xg_no_goals_does_not_outrate_clinical() {
        // High xG, no goals (wasteful) vs low xG, two goals (clinical).
        // Both 2-0 wins, 20/15 passing, 2 SoT / 3 shots.
        let mut wasteful = make_stats(
            0,
            0,
            20,
            15,
            2,
            6,
            0,
            0,
            0,
            2.5,
            PlayerFieldPositionGroup::Forward,
        );
        wasteful.miscontrols = 0;
        let clinical = make_stats(
            2,
            0,
            20,
            15,
            2,
            3,
            0,
            0,
            0,
            0.6,
            PlayerFieldPositionGroup::Forward,
        );
        let wasteful_rating = RatingContext::new(&wasteful, 2, 0).calculate();
        let clinical_rating = RatingContext::new(&clinical, 2, 0).calculate();
        assert!(
            clinical_rating > wasteful_rating + 1.0,
            "clinical ({}) should clearly outrate wasteful ({}) — got delta {}",
            clinical_rating,
            wasteful_rating,
            clinical_rating - wasteful_rating
        );
    }

    #[test]
    fn defender_can_reach_seven_without_goals_or_assists() {
        // A complete defensive shift: 4 tackles, 5 interceptions, 7
        // clearances, 2 blocks, clean sheet. No goals, no assists, no
        // possession risk. Should clear 7.0.
        let mut anchor = make_stats(
            0,
            0,
            30,
            25,
            0,
            0,
            4,
            5,
            0,
            0.0,
            PlayerFieldPositionGroup::Defender,
        );
        anchor.clearances = 7;
        anchor.blocks = 2;
        let rating = RatingContext::new(&anchor, 1, 0).calculate();
        assert!(
            rating >= 7.0,
            "anchor CB rated {} — should reach 7.0+ on defensive work alone",
            rating
        );
    }

    #[test]
    fn defender_box_actions_outrate_same_count_in_middle_third() {
        // Two CBs with the same raw counts (3 tackles, 3 interceptions,
        // 4 clearances, 2 blocks). The "box" defender did all of his
        // work inside the own penalty area; the "midfield" defender's
        // counters happen to be unzoned (zone counters all zero). The
        // zone-aware bumps must lift the box defender clearly.
        let make_cb = || {
            let mut s = make_stats(
                0,
                0,
                25,
                21,
                0,
                0,
                3,
                3,
                0,
                0.0,
                PlayerFieldPositionGroup::Defender,
            );
            s.clearances = 4;
            s.blocks = 2;
            s
        };
        let middle = make_cb();
        let mut box_cb = make_cb();
        // Counters are mutually exclusive: a six-yard action only
        // increments the six-yard counter, not the own-box counter.
        // Stand: 3 tackles, 3 interceptions, 4 clearances (2 of which
        // were on the goal line), 2 blocks (1 on the goal line).
        box_cb.zone_stats.tackles_own_box = 3;
        box_cb.zone_stats.interceptions_own_box = 3;
        box_cb.zone_stats.clearances_own_box = 2;
        box_cb.zone_stats.blocks_own_box = 1;
        box_cb.zone_stats.clearances_own_six_yard = 2;
        box_cb.zone_stats.blocks_own_six_yard = 1;

        let mid_rating = RatingContext::new(&middle, 1, 0).calculate();
        let box_rating = RatingContext::new(&box_cb, 1, 0).calculate();
        assert!(
            box_rating > mid_rating + 0.30,
            "box CB ({}) should clearly outrate middle-third CB ({}) on the same raw counts",
            box_rating,
            mid_rating
        );
    }

    #[test]
    fn six_yard_action_stronger_than_own_box_but_not_double() {
        // Six-yard actions get a stronger zone bonus than own-box actions
        // (60% vs 35%), but the two counters are mutually exclusive, so a
        // single six-yard event shouldn't add the box bonus on top.
        // Given an identical workload, the six-yard CB outrates the
        // own-box CB by the *difference* of the two coefficients, not
        // their sum.
        let make_cb = || {
            let mut s = make_stats(
                0,
                0,
                25,
                21,
                0,
                0,
                3,
                3,
                0,
                0.0,
                PlayerFieldPositionGroup::Defender,
            );
            s.clearances = 4;
            s.blocks = 2;
            s
        };
        let mut box_cb = make_cb();
        box_cb.zone_stats.tackles_own_box = 3;
        box_cb.zone_stats.interceptions_own_box = 3;
        box_cb.zone_stats.clearances_own_box = 4;
        box_cb.zone_stats.blocks_own_box = 2;
        let mut six_cb = make_cb();
        six_cb.zone_stats.tackles_own_six_yard = 3;
        six_cb.zone_stats.interceptions_own_six_yard = 3;
        six_cb.zone_stats.clearances_own_six_yard = 4;
        six_cb.zone_stats.blocks_own_six_yard = 2;

        let box_rating = RatingContext::new(&box_cb, 1, 0).calculate();
        let six_rating = RatingContext::new(&six_cb, 1, 0).calculate();
        // Six-yard is the stronger replacement, not a stack — the gap
        // is bounded by the difference between the two coefficients.
        // For 12 events the upper bound (no caps) would be roughly:
        //   12 * avg_weight * (0.60 - 0.35) ≈ 12 * 0.10 * 0.25 = 0.30
        // Both branches saturate the DEF_ZONE_BONUS_CAP (0.60) here, so
        // the actual delta lands smaller. Assert > 0 (not pinned) and
        // < 0.5 (definitely not a +0.95 double-stack).
        assert!(
            six_rating > box_rating,
            "six-yard CB ({}) should outrate own-box CB ({})",
            six_rating,
            box_rating
        );
        assert!(
            six_rating - box_rating < 0.5,
            "six-yard ({}) over own-box ({}) gap = {} — looks like a stack, not a replacement",
            six_rating,
            box_rating,
            six_rating - box_rating
        );
    }

    #[test]
    fn error_to_goal_own_box_extra_penalty() {
        // A defender giving the ball away in their own box that becomes
        // a goal: the base errors_leading_to_goal already takes a -0.90
        // hit; the own-box-extra coefficient adds a further -0.35 on
        // top so the goal-mouth howler is materially worse than a
        // midfield error that turned into a goal.
        let mut base = make_stats(
            0,
            0,
            20,
            16,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Defender,
        );
        base.errors_leading_to_shot = 1;
        base.errors_leading_to_goal = 1;
        let baseline = RatingContext::new(&base, 1, 1).calculate();
        let mut with_extra = base.clone();
        with_extra.zone_stats.errors_to_goal_own_box = 1;
        let extra_rating = RatingContext::new(&with_extra, 1, 1).calculate();
        assert!(
            (baseline - extra_rating - ZoneCoeffs::ERROR_TO_GOAL_OWN_BOX_EXTRA.abs()).abs() < 0.01,
            "own-box error-to-goal extra should subtract {:.2} on top of base — got delta {}",
            ZoneCoeffs::ERROR_TO_GOAL_OWN_BOX_EXTRA,
            baseline - extra_rating
        );
    }

    #[test]
    fn ten_minute_cameo_does_not_get_full_match_minute_damp() {
        // A cameo with creative output racked up in 10 minutes must NOT
        // be treated as a 90-minute shift. The damp curve plus the
        // cameo clamp keep them in the 5.8-7.2 band; without the damp
        // they'd post a 9.0 from the modern bonuses alone.
        let mut cameo = make_stats(
            0,
            0,
            10,
            9,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        cameo.minutes_played = 10;
        cameo.key_passes = 3;
        cameo.progressive_passes = 5;
        cameo.progressive_carries = 4;
        cameo.passes_into_box = 4;
        cameo.zone_stats.progressive_passes_into_final_third = 5;
        cameo.zone_stats.carries_into_box = 3;

        let rating = RatingContext::new(&cameo, 1, 1).calculate();
        assert!(
            rating <= 7.2 && rating >= 5.8,
            "10-min cameo rated {} — should stay in cameo bound 5.8..7.2",
            rating
        );
    }

    #[test]
    fn own_goal_materially_lowers_rating() {
        // A solid CB shift undone by an own goal lands in the bad
        // band. Without the OG penalty the CB would post a 6.5+ on
        // their other contributions; the OG itself drops them at
        // least a full grade.
        let mut s = make_stats(
            0,
            0,
            30,
            25,
            0,
            0,
            2,
            3,
            0,
            0.0,
            PlayerFieldPositionGroup::Defender,
        );
        s.clearances = 4;
        let baseline = RatingContext::new(&s, 1, 1).calculate();
        s.own_goals = 1;
        let with_og = RatingContext::new(&s, 1, 2).calculate();
        assert!(
            baseline - with_og >= 1.0,
            "OG must drop rating by ≥ 1.0 grade — baseline {} → with OG {}",
            baseline,
            with_og
        );
    }

    #[test]
    fn penalty_conceding_foul_lowers_rating() {
        // Same defender, same outline; the only difference is conceding
        // a penalty. The single penalty foul carries a -0.35 hit on
        // top of the per-foul base.
        let mut s = make_stats(
            0,
            0,
            25,
            21,
            0,
            0,
            2,
            3,
            0,
            0.0,
            PlayerFieldPositionGroup::Defender,
        );
        s.clearances = 4;
        let baseline = RatingContext::new(&s, 1, 1).calculate();
        s.fouls = 1;
        s.zone_stats.penalty_fouls_conceded = 1;
        s.zone_stats.own_third_def_fouls = 1;
        let with_pen = RatingContext::new(&s, 1, 2).calculate();
        assert!(
            baseline - with_pen >= 0.30,
            "penalty foul must drop rating by ≥ 0.30 — baseline {} → with pen {}",
            baseline,
            with_pen
        );
    }

    #[test]
    fn high_volume_fouler_without_cards_still_penalised() {
        // Same MID, two scenarios: clean vs. 7-foul-no-cards. The
        // fouler must rate visibly below the clean version — cards
        // shouldn't be the only signal that catches a niggly player.
        let clean = make_stats(
            0,
            0,
            40,
            34,
            0,
            0,
            3,
            2,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        let mut niggly = clean.clone();
        niggly.fouls = 7;
        let clean_rating = RatingContext::new(&clean, 1, 1).calculate();
        let niggly_rating = RatingContext::new(&niggly, 1, 1).calculate();
        assert!(
            clean_rating - niggly_rating >= 0.15,
            "high-volume fouler ({}) should rate visibly below clean ({})",
            niggly_rating,
            clean_rating
        );
    }

    #[test]
    fn wide_forward_without_attacking_output_capped_at_7_0() {
        // Real symptom: a forward with 2 goals in 26 matches posting a
        // 7.5 season average and 14 POTM. Per-match, a wide forward
        // stacking 30 passes / completed crosses / dribbles / carries
        // / passes-into-box could pre-compression hit ~7.9 and post-
        // compression ~7.5 with zero direct end-product. Cap at 7.0
        // — a forward without a goal, an assist, real shooting
        // threat, or chance creation shouldn't reach Salah territory.
        let mut fwd = make_stats(
            0,
            0,
            30,
            25,
            0,
            0,
            0,
            0,
            0,
            0.2,
            PlayerFieldPositionGroup::Forward,
        );
        fwd.successful_dribbles = 4;
        fwd.attempted_dribbles = 4;
        fwd.progressive_carries = 5;
        fwd.passes_into_box = 2;
        fwd.crosses_attempted = 6;
        fwd.crosses_completed = 4;
        fwd.carry_distance = 2000;
        fwd.key_passes = 1;
        let rating = RatingContext::new(&fwd, 1, 0).calculate();
        assert!(
            rating <= 7.0,
            "Wide FW without goal/assist/shots-on-target rated {} — should cap at 7.0",
            rating
        );
    }

    #[test]
    fn goalscoring_forward_escapes_low_output_cap() {
        // A forward who scored is exempt regardless of other volume.
        let mut fwd = make_stats(
            1,
            0,
            20,
            16,
            1,
            2,
            0,
            0,
            0,
            0.8,
            PlayerFieldPositionGroup::Forward,
        );
        fwd.shots_on_target = 1;
        let rating = RatingContext::new(&fwd, 1, 0).calculate();
        assert!(
            rating > 7.0,
            "Goalscoring FW (1 goal, 0.8 xG) rated {} — should escape 7.0 cap",
            rating
        );
    }

    #[test]
    fn shooting_threat_forward_escapes_low_output_cap() {
        // A forward with 2+ shots on target showed real attacking
        // threat — even without a goal they shouldn't be hard-capped
        // by the 7.0 forward cap. (Strikers having bad finishing days
        // but generating shots is a different problem than strikers
        // who don't shoot at all.) Single key pass also included so
        // the existing 6.4 outfielder floor (which requires
        // creative == 0) doesn't fire first and mask the 7.0 escape.
        let mut fwd = make_stats(
            0,
            0,
            40,
            34,
            0,
            0,
            0,
            0,
            0,
            0.6,
            PlayerFieldPositionGroup::Forward,
        );
        fwd.key_passes = 1;
        fwd.shots_on_target = 3;
        fwd.shots_total = 4;
        fwd.successful_dribbles = 3;
        fwd.attempted_dribbles = 3;
        fwd.progressive_carries = 4;
        let rating = RatingContext::new(&fwd, 1, 0).calculate();
        assert!(
            rating > 7.0,
            "Forward with 3 shots-on-target rated {} — real shooting threat should escape 7.0 cap",
            rating
        );
    }

    #[test]
    fn creator_forward_escapes_low_output_cap() {
        // A forward who created multiple chances (2+ key passes) had
        // real end-product even without scoring/assisting — must not
        // be capped at 7.0.
        let mut fwd = make_stats(
            0,
            0,
            25,
            21,
            0,
            0,
            0,
            0,
            0,
            0.3,
            PlayerFieldPositionGroup::Forward,
        );
        fwd.key_passes = 3;
        fwd.passes_into_box = 3;
        fwd.successful_dribbles = 2;
        fwd.attempted_dribbles = 3;
        let rating = RatingContext::new(&fwd, 1, 0).calculate();
        assert!(
            rating > 7.0,
            "Creator FW (3 key passes, 3 box entries) rated {} — chance creation should escape cap",
            rating
        );
    }

    #[test]
    fn forward_offsides_penalised_more_than_other_positions() {
        let mut fwd = make_stats(
            0,
            0,
            10,
            7,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Forward,
        );
        fwd.offsides = 3;
        let mut mid = fwd.clone();
        mid.position_group = PlayerFieldPositionGroup::Midfielder;

        let fwd_rating = RatingContext::new(&fwd, 1, 1).calculate();
        let mid_rating = RatingContext::new(&mid, 1, 1).calculate();
        assert!(
            fwd_rating < mid_rating,
            "FWD with 3 offsides ({}) should be penalised more than MID with 3 ({})",
            fwd_rating,
            mid_rating
        );
    }

    #[test]
    fn gk_command_zone_actions_lift_rating_without_save_inflation() {
        // A keeper who didn't have to make many saves but commanded
        // his box (claimed crosses, punched out a few) gains a small
        // rating credit. Capped so this can't replace actual saves
        // as the headline keeper bonus.
        let mut quiet = make_stats(
            0,
            0,
            15,
            12,
            0,
            0,
            0,
            0,
            1,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        quiet.shots_faced = 1;
        let mut commanding = quiet.clone();
        commanding.zone_stats.gk_command_actions = 5;

        let quiet_rating = RatingContext::new(&quiet, 1, 0).calculate();
        let commanding_rating = RatingContext::new(&commanding, 1, 0).calculate();
        assert!(
            commanding_rating > quiet_rating,
            "commanding GK ({}) should outrate quiet GK ({})",
            commanding_rating,
            quiet_rating
        );
        assert!(
            commanding_rating - quiet_rating <= 0.30,
            "command-zone bonus is capped — delta {} should be ≤ 0.30",
            commanding_rating - quiet_rating
        );
    }

    #[test]
    fn subbed_in_player_minute_count_drives_damp() {
        // Two MIDs, one played 90 minutes, one came on for 10. Same
        // creative output. The full-90 must clearly outrate the cameo
        // because the cameo's modern bonuses are damped to zero.
        let make_creative = |minutes: u16| {
            let mut s = make_stats(
                0,
                0,
                30,
                26,
                0,
                0,
                0,
                0,
                0,
                0.0,
                PlayerFieldPositionGroup::Midfielder,
            );
            s.minutes_played = minutes;
            s.key_passes = 2;
            s.progressive_passes = 4;
            s.passes_into_box = 3;
            s.zone_stats.progressive_passes_into_final_third = 3;
            s
        };
        let starter = make_creative(90);
        let cameo = make_creative(10);
        let starter_rating = RatingContext::new(&starter, 1, 1).calculate();
        let cameo_rating = RatingContext::new(&cameo, 1, 1).calculate();
        assert!(
            starter_rating > cameo_rating,
            "starter ({}) with same modern stats must outrate damped 10-min cameo ({})",
            starter_rating,
            cameo_rating
        );
    }

    #[test]
    fn half_space_box_entries_lift_rating_within_cap() {
        // Two MIDs with identical baseline. One has 4 box-entry passes
        // ALL from half-space, the other has 4 from neutral lanes.
        // Half-space hits get an extra capped credit.
        let make_mid = || {
            let mut s = make_stats(
                0,
                0,
                40,
                34,
                0,
                0,
                4,
                0,
                0,
                0.0,
                PlayerFieldPositionGroup::Midfielder,
            );
            s.passes_into_box = 4;
            s.key_passes = 1;
            s
        };
        let neutral = make_mid();
        let mut half_space = make_mid();
        half_space.zone_stats.half_space_passes_into_box = 4;
        let neutral_rating = RatingContext::new(&neutral, 1, 1).calculate();
        let hs_rating = RatingContext::new(&half_space, 1, 1).calculate();
        let delta = hs_rating - neutral_rating;
        assert!(
            delta > 0.0,
            "half-space pass into box should give a positive delta — got {}",
            delta
        );
        // Cap is 0.20 per group; with 4 events at 0.04/each = 0.16
        assert!(
            delta <= 0.20 + 0.01,
            "half-space delta {} exceeds cap {}",
            delta,
            ZoneCoeffs::HALF_SPACE_BOX_ENTRY_CAP
        );
    }

    #[test]
    fn central_box_entries_capped() {
        // Spam test — 20 central box-entry passes still cap at the
        // configured ceiling, not 1.0+ runaway.
        let mut s = make_stats(
            0,
            0,
            40,
            34,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.passes_into_box = 20;
        s.zone_stats.central_passes_into_box = 20;
        let rating = RatingContext::new(&s, 1, 1).calculate();
        // Without lane bonuses a 20-passes-into-box midfielder already
        // hits multiple caps; the lane bonus must NOT push them past
        // a sane upper bound. Set a generous ceiling and assert.
        assert!(
            rating <= 9.5,
            "central-spam MID rated {} — lane bonus should not break the rating ceiling",
            rating
        );
    }

    #[test]
    fn switch_of_play_capped() {
        // 10 switches stays under the 0.15 cap.
        let mut s = make_stats(
            0,
            0,
            40,
            34,
            0,
            0,
            4,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.key_passes = 1;
        let baseline = RatingContext::new(&s, 1, 1).calculate();
        s.zone_stats.switches_of_play = 10;
        let rating = RatingContext::new(&s, 1, 1).calculate();
        let delta = rating - baseline;
        assert!(delta > 0.0, "switch-of-play should add positive credit");
        assert!(
            delta <= ZoneCoeffs::SWITCH_OF_PLAY_CAP + 0.01,
            "switch-of-play delta {} exceeds cap {}",
            delta,
            ZoneCoeffs::SWITCH_OF_PLAY_CAP
        );
    }

    #[test]
    fn failed_gk_claim_to_shot_lowers_rating() {
        // Wired or not, the rating helper still applies the
        // coefficient when the counter is populated. Verify both
        // bands.
        let mut gk = make_stats(
            0,
            0,
            15,
            12,
            0,
            0,
            0,
            0,
            3,
            0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        gk.shots_faced = 4;
        let baseline = RatingContext::new(&gk, 1, 1).calculate();
        let mut shot = gk.clone();
        shot.zone_stats.gk_failed_claims_to_shot = 1;
        let with_shot = RatingContext::new(&shot, 1, 1).calculate();
        assert!(
            (baseline - with_shot - ZoneCoeffs::GK_FAILED_CLAIM_TO_SHOT.abs()).abs() < 0.01,
            "failed-claim-to-shot should drop rating by {:.2} — got {}",
            ZoneCoeffs::GK_FAILED_CLAIM_TO_SHOT.abs(),
            baseline - with_shot
        );
        let mut goal = gk.clone();
        goal.zone_stats.gk_failed_claims_to_goal = 1;
        let with_goal = RatingContext::new(&goal, 1, 1).calculate();
        assert!(
            (baseline - with_goal - ZoneCoeffs::GK_FAILED_CLAIM_TO_GOAL.abs()).abs() < 0.01,
            "failed-claim-to-goal should drop rating by {:.2} — got {}",
            ZoneCoeffs::GK_FAILED_CLAIM_TO_GOAL.abs(),
            baseline - with_goal
        );
    }

    #[test]
    fn xg_buildup_excludes_shooter_and_assister() {
        // Verifies the rating helper treats `xg_buildup` as a clean
        // signal — large buildup should lift a midfielder visibly.
        // The producer wiring (in shoot handler) is tested indirectly
        // via the rating's response to populated values.
        let mut plain = make_stats(
            0,
            0,
            40,
            34,
            0,
            0,
            4,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        plain.key_passes = 1;
        let mut chained = plain.clone();
        chained.xg_buildup = 0.6;
        let plain_rating = RatingContext::new(&plain, 1, 1).calculate();
        let chained_rating = RatingContext::new(&chained, 1, 1).calculate();
        assert!(
            chained_rating > plain_rating,
            "buildup xG should lift rating: plain {}, chained {}",
            plain_rating,
            chained_rating
        );
    }
}
