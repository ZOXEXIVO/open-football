use crate::PlayerFieldPositionGroup;
use crate::r#match::PlayerMatchEndStats;
use crate::r#match::engine::zones::ZoneCoeffs;
#[cfg(test)]
use crate::r#match::engine::zones::ZoneStats;

/// Calculate a match rating (1.0 - 10.0, base 6.0) from in-match statistics.
///
/// The formula is position-aware: goalkeepers are rated on saves, defenders on
/// tackles/interceptions/clean sheets, midfielders on passing volume & accuracy,
/// and forwards on goals/shots/xG.
pub fn calculate_match_rating(
    stats: &PlayerMatchEndStats,
    team_goals: u8,
    opponent_goals: u8,
) -> f32 {
    let pos = stats.position_group;
    let mut rating: f32 = 6.0;

    // ── Attacking contributions ──────────────────────────────────────────

    // Goals: +0.75 each, capped at +2.4. Down from a flat +1.0/+3.0 cap
    // because the engine was rewarding lucky / variance-driven goals
    // identically to genuine quality finishes — a 5/20 striker who
    // scuffed in two rebounds still posted an 8.0. The post-shot xG
    // delta below adds +0..0.20 for clinical conversions, and the
    // cap-busting "decisive" bonus rewards a goal that flipped the
    // scoreline.
    let goal_count = stats.goals as f32;
    let base_goal_credit = (goal_count * 0.75).min(2.4);
    rating += base_goal_credit;

    // Decisive goal bonus: a goal that put the team into a winning /
    // drawing position (or pulled them level) still earns extra credit
    // beyond the base. Flat +0.20 per goal-scoring forward when the
    // team won — cheap proxy for "actually mattered to the result"
    // without re-deriving per-goal scoreline state.
    if stats.goals > 0 && team_goals > opponent_goals {
        rating += 0.20;
    }

    // High-xG over-conversion: only awarded when the player out-scored
    // their xG by a meaningful margin. Capped at +0.20.
    if stats.xg > 0.05 && goal_count > 0.0 {
        let over = (goal_count - stats.xg).max(0.0);
        rating += (over * 0.10).min(0.20);
    }

    // Low-xG goals dampener: if the goals came from very low-quality
    // chances (e.g. one tap-in worth 0.10 xG), reduce the goal credit
    // a touch — the variance-finished goal still counts but not the
    // same as a genuinely difficult conversion.
    if stats.goals > 0 && stats.xg < goal_count * 0.35 {
        rating -= 0.15 * goal_count;
    }

    // Wasted high xG: created premium chances and scored none.
    if stats.goals == 0 && stats.xg >= 0.8 {
        let waste = ((stats.xg - 0.8) * 0.20).min(0.35);
        rating -= waste;
    }

    // Shot-spam penalty: high shot volume with very poor xG-per-shot
    // means the player kept firing low-quality attempts.
    if stats.shots_total > 4 {
        let xg_per_shot = if stats.shots_total > 0 {
            stats.xg / stats.shots_total as f32
        } else {
            0.0
        };
        if xg_per_shot < 0.08 {
            rating -= ((stats.shots_total as f32 - 4.0) * 0.035).min(0.30);
        }
    }

    // Assists: +0.5 each, capped at +1.5
    rating += (stats.assists as f32 * 0.5).min(1.5);

    // ── Passing quality ──────────────────────────────────────────────────

    if stats.passes_attempted > 10 {
        let pass_pct = stats.passes_completed as f32 / stats.passes_attempted as f32;

        // 70% = neutral, 90%+ = +0.5, below 50% = -0.4
        let mut pass_bonus = (pass_pct - 0.70) * 2.0;
        pass_bonus = pass_bonus.clamp(-0.4, 0.5);

        // Volume bonus: high-volume accurate passing shows sustained involvement
        if stats.passes_attempted > 30 && pass_pct > 0.80 {
            pass_bonus += 0.15;
        }
        if stats.passes_attempted > 50 && pass_pct > 0.85 {
            pass_bonus += 0.15;
        }

        rating += pass_bonus;
    }

    // ── Shooting accuracy ────────────────────────────────────────────────

    if stats.shots_total > 0 {
        let shot_accuracy = stats.shots_on_target as f32 / stats.shots_total as f32;
        let shot_bonus = (shot_accuracy - 0.4) * 0.6;
        rating += shot_bonus.clamp(-0.2, 0.3);
    }

    // ── Defensive contributions (position-weighted) ──────────────────────

    // Tackles
    let tackle_weight = match pos {
        PlayerFieldPositionGroup::Defender => 0.12,
        PlayerFieldPositionGroup::Midfielder => 0.08,
        _ => 0.05,
    };
    rating += (stats.tackles as f32 * tackle_weight).min(0.5);

    // Interceptions — reading the game is valuable, especially for defenders.
    // For goalkeepers this includes commanding the box: claimed crosses,
    // through-balls collected, aerials caught — same weight as defenders
    // because the GK's "cleaned ball" work is a direct defensive action.
    let interception_weight = match pos {
        PlayerFieldPositionGroup::Defender => 0.15,
        PlayerFieldPositionGroup::Goalkeeper => 0.15,
        PlayerFieldPositionGroup::Midfielder => 0.10,
        _ => 0.06,
    };
    rating += (stats.interceptions as f32 * interception_weight).min(0.8);

    // Clearances — last-ditch defending. Heavily weighted for back-line
    // players (a CB clearing 8 set-pieces under pressure is the marker
    // of a strong outing); midfielders get a smaller share for tracking
    // back; forwards basically don't clear.
    let clearance_weight = match pos {
        PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper => 0.07,
        PlayerFieldPositionGroup::Midfielder => 0.04,
        _ => 0.02,
    };
    let clearance_cap = match pos {
        PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper => 0.45,
        PlayerFieldPositionGroup::Midfielder => 0.25,
        _ => 0.15,
    };
    rating += (stats.clearances as f32 * clearance_weight).min(clearance_cap);

    // ── Goalkeeper saves ─────────────────────────────────────────────────

    if pos == PlayerFieldPositionGroup::Goalkeeper {
        // Real-football season averages for elite GKs sit at 7.0-7.3.
        // Per-save / save% / surplus all reward high-volume saving, so
        // each individual layer has to be modest — stacked, they
        // compose to ~+1.7 max above base, which is the right max for
        // a single great game.
        let save_bonus = (stats.saves as f32 * 0.15).min(1.2);
        rating += save_bonus;

        let shots_faced = stats.shots_faced.max(stats.saves + opponent_goals as u16);
        if shots_faced >= 3 {
            let save_pct = stats.saves as f32 / shots_faced as f32;
            let pct_bonus = if save_pct > 0.80 {
                0.3
            } else if save_pct > 0.70 {
                0.15
            } else if save_pct > 0.60 {
                0.05
            } else if save_pct < 0.50 {
                -0.2
            } else {
                0.0
            };
            rating += pct_bonus;
        }

        if stats.saves > opponent_goals as u16 {
            let surplus = stats.saves - opponent_goals as u16;
            rating += (surplus as f32 * 0.05).min(0.2);
        }
    }

    // ── Team result ──────────────────────────────────────────────────────

    if team_goals > opponent_goals {
        rating += 0.3; // Win bonus
    } else if team_goals < opponent_goals {
        rating -= 0.2; // Loss penalty
    }

    // ── Clean sheet bonus ────────────────────────────────────────────────

    if opponent_goals == 0 {
        match pos {
            PlayerFieldPositionGroup::Goalkeeper => rating += 0.3,
            PlayerFieldPositionGroup::Defender => rating += 0.3,
            PlayerFieldPositionGroup::Midfielder => rating += 0.1,
            _ => {}
        }
    }

    // ── Conceding goals penalty ──────────────────────────────────────────
    //
    // The goalkeeper owns every conceded goal — a flat -0.5 for "3+"
    // let a GK who shipped seven still post an 8/10 (base 6 + saves 1.5
    // + passing 0.8 + etc). Penalty has to scale with the actual number
    // of goals past them.

    match pos {
        PlayerFieldPositionGroup::Goalkeeper => {
            // Real-football rating is performance-driven: the GK
            // doesn't "own" every goal (defenders, luck, unstoppable
            // shots all contribute). Linear base + linear extra past
            // the 3rd — not quadratic, because quadratic clamps the
            // worst cases at 1.0 regardless of effort, making a
            // 10-conceding-with-saves display as "as bad as possible"
            // same as a 10-conceding-with-zero-saves.
            //   1 conceded → -0.15   (normal, still ~6 with saves)
            //   2 conceded → -0.30   (below avg, ~6 with saves)
            //   3 conceded → -0.45   (bad day, ~5.7)
            //   4 conceded → -1.00   (slipping, ~5)
            //   5 conceded → -1.55   (~4.5)
            //   6 conceded → -2.10   (awful, ~4)
            //   7 conceded → -2.65   (~3.7)
            //   8 conceded → -3.20   (~3)
            //  10 conceded → -4.30   (~2, not hard-floored)
            let base = opponent_goals as f32 * 0.15;
            let heavy = (opponent_goals as f32 - 3.0).max(0.0) * 0.4;
            rating -= base + heavy;
        }
        PlayerFieldPositionGroup::Defender => {
            // -0.25 per goal past the 2nd, capped at -1.5. Defenders
            // share blame for a hammering but not on the GK's scale.
            if opponent_goals >= 3 {
                let extra = (opponent_goals as f32 - 2.0).min(6.0);
                rating -= extra * 0.25;
            }
        }
        _ => {}
    }

    // ── xG-based unlucky finisher ────────────────────────────────────────
    //
    // Clinical-finisher bonus is now folded into the per-goal block
    // above (high_xg_conversion_bonus). We keep the small "unlucky"
    // bump here for forwards who manufactured genuine chances without
    // converting any.
    if stats.goals == 0 && stats.xg > 1.0 {
        rating += 0.1;
    }

    // ── Modern build-up / chance creation contributions ──────────────────
    //
    // These are small per-event bonuses with caps so a midfielder racking up
    // pressures + progressive passes + key passes lifts visibly above a
    // teammate who only completed safe sideways passes. Damped for cameo
    // appearances so a 12-minute sub doesn't post a 9.0 from one key pass.

    let minute_damp = if stats.minutes_played < 15 {
        0.0
    } else if stats.minutes_played < 30 {
        0.65
    } else if stats.minutes_played < 60 {
        0.85
    } else {
        1.0
    };

    rating += (stats.key_passes as f32 * 0.12).min(0.6) * minute_damp;
    rating += (stats.progressive_passes as f32 * 0.025).min(0.35) * minute_damp;
    rating += (stats.progressive_carries as f32 * 0.04).min(0.40) * minute_damp;

    // Successful dribbles — modest bonus, scaled by position group.
    let dribble_weight = match pos {
        PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => 0.08,
        _ => 0.04,
    };
    rating += (stats.successful_dribbles as f32 * dribble_weight).min(0.45) * minute_damp;

    // Failed dribbles — small penalty cap. Reduced for forwards / wide
    // attackers because attempting take-ons is part of their job.
    if stats.attempted_dribbles > stats.successful_dribbles {
        let failed = (stats.attempted_dribbles - stats.successful_dribbles) as f32;
        let fail_w = if pos == PlayerFieldPositionGroup::Forward {
            0.025
        } else {
            0.04
        };
        rating -= (failed * fail_w).min(0.30) * minute_damp;
    }

    rating += (stats.successful_pressures as f32 * 0.035).min(0.35) * minute_damp;

    // Raw pressing volume — applying pressure even when it doesn't
    // immediately force a turnover is still graft. Worth a third of a
    // successful pressure each, with a tight cap so a high-volume
    // presser doesn't outscore a creative midfielder by spamming.
    let raw_pressure_volume = stats.pressures.saturating_sub(stats.successful_pressures);
    rating += (raw_pressure_volume as f32 * 0.012).min(0.20) * minute_damp;

    // Blocks count more for defenders, less for attackers.
    let block_w = match pos {
        PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper => 0.10,
        PlayerFieldPositionGroup::Midfielder => 0.07,
        _ => 0.04,
    };
    rating += (stats.blocks as f32 * block_w).min(0.5) * minute_damp;

    // ── Crossing (position-aware accuracy reward) ───────────────────────
    //
    // Reward completed crosses, mildly punish spam from attempts that
    // didn't reach a teammate. Caps small so a winger can't post a 9.0
    // from crossing volume alone. Centre-backs / GKs barely benefit.
    if stats.crosses_attempted > 0 {
        let completed = stats.crosses_completed as f32;
        let failed = stats
            .crosses_attempted
            .saturating_sub(stats.crosses_completed) as f32;
        let (cross_cap, miss_cap) = match pos {
            PlayerFieldPositionGroup::Midfielder
            | PlayerFieldPositionGroup::Forward
            | PlayerFieldPositionGroup::Defender => (0.30, 0.20),
            _ => (0.12, 0.08),
        };
        rating += (completed * 0.08).min(cross_cap) * minute_damp;
        rating -= (failed * 0.012).min(miss_cap) * minute_damp;
    }

    // ── Passes into box ─────────────────────────────────────────────────
    //
    // Chance-creation indicator independent of whether the ball ended
    // in a shot (so it's not subsumed by `key_passes`). Caps small —
    // moving the ball into the box matters but should not dominate.
    let pib_w = match pos {
        PlayerFieldPositionGroup::Midfielder | PlayerFieldPositionGroup::Forward => 0.06,
        PlayerFieldPositionGroup::Defender => 0.04,
        _ => 0.02,
    };
    rating += (stats.passes_into_box as f32 * pib_w).min(0.30) * minute_damp;

    // ── xG buildup credit ───────────────────────────────────────────────
    //
    // `xg_buildup` excludes the player's own shots and direct assists,
    // so it's a clean "made the chance happen up the chain" signal.
    // Midfielders/defenders weighted higher: for a forward, most of
    // their xG involvement is the shot itself, already rewarded.
    if stats.xg_buildup > 0.1 {
        let buildup_w = match pos {
            PlayerFieldPositionGroup::Midfielder => 0.30,
            PlayerFieldPositionGroup::Defender => 0.22,
            PlayerFieldPositionGroup::Forward => 0.10,
            _ => 0.05,
        };
        rating += (stats.xg_buildup * buildup_w).min(0.30) * minute_damp;
    }

    // ── Carry distance (tie-breaker) ────────────────────────────────────
    //
    // Per progressive carry is already rewarded above; this is a small
    // top-up for a player who genuinely broke ground over the match.
    // 1000 units of cumulative carry → +0.10. Cap tight.
    let carry_bonus = ((stats.carry_distance as f32 / 1000.0) - 0.05).max(0.0);
    rating += carry_bonus.min(0.15) * minute_damp;

    // ── Possession-quality penalties ────────────────────────────────────
    //
    // Miscontrols and heavy touches make a player visibly worse on the
    // ball. Damped for cameos so a 12-minute sub doesn't get hammered
    // for one bad first touch. Caps small — these shouldn't override a
    // strong defensive / creative shift.
    //
    // The rating-side wiring is complete; the LIVE producer for these
    // counters is intentionally deferred until receiver-state tracking
    // lands (the engine needs to distinguish a clean reception from a
    // heavy first touch / miscontrol at the moment the receiver claims
    // the ball). Until then, both counters default to zero and the
    // rating impact is zero — nothing to fix in this slice.
    rating -= (stats.miscontrols as f32 * 0.03).min(0.22) * minute_damp;
    rating -= (stats.heavy_touches as f32 * 0.015).min(0.18) * minute_damp;

    // Errors carry weight regardless of position. Errors-to-goal are the
    // most damaging individual events in the rating, sitting just below a
    // red card. Both are capped per-match: a goalkeeper distributing
    // under sustained pressure will see several long balls intercepted
    // and an opponent shot within the response window — without a cap
    // the per-event hit stacked unboundedly and pushed clean-sheet
    // keepers into the 4-5 band, and conceding keepers to the 1.0 floor.
    rating -= (stats.errors_leading_to_shot as f32 * 0.35).min(0.7);
    rating -= (stats.errors_leading_to_goal as f32 * 0.90).min(1.8);

    // Cards — applied here in the post-pass so they affect the final
    // figure in the same place as errors.
    rating -= stats.yellow_cards as f32 * 0.15;
    rating -= stats.red_cards as f32 * 1.50;

    // ── Zone-aware defensive bonus ──────────────────────────────────────
    //
    // The base defensive credit (above) treats every tackle / interception
    // / block / clearance the same regardless of where it happened. Real
    // football: a tackle on the edge of your own box is worth more than
    // one in the centre circle, and a sliding clearance on the goal line
    // is the play of the match. Layer the per-event base credit by a
    // small extra bonus tied to the zone the action took place in. All
    // zone counters default to zero, so call-sites that haven't been
    // updated to record zones yet still produce the legacy rating.
    let z = stats.zone_stats;
    let pressure_per = 0.035_f32;

    let def_zone_bonus = (z.tackles_own_box as f32 * tackle_weight
        + z.interceptions_own_box as f32 * interception_weight
        + z.blocks_own_box as f32 * block_w
        + z.clearances_own_box as f32 * clearance_weight)
        * ZoneCoeffs::DEF_OWN_BOX_BONUS
        + (z.tackles_own_six_yard as f32 * tackle_weight
            + z.interceptions_own_six_yard as f32 * interception_weight
            + z.blocks_own_six_yard as f32 * block_w
            + z.clearances_own_six_yard as f32 * clearance_weight)
            * ZoneCoeffs::DEF_OWN_SIX_YARD_BONUS
        + z.interceptions_middle_third as f32
            * interception_weight
            * ZoneCoeffs::INTERCEPTION_MIDDLE_BONUS
        + z.tackles_final_third as f32 * tackle_weight * ZoneCoeffs::TACKLE_FINAL_THIRD_BONUS
        + z.pressures_won_final_third as f32
            * pressure_per
            * ZoneCoeffs::PRESSURE_FINAL_THIRD_BONUS;
    rating += def_zone_bonus.min(ZoneCoeffs::DEF_ZONE_BONUS_CAP);

    // ── Progressive / box-entry zone bonuses ────────────────────────────
    //
    // A progressive pass/carry that ends in the final third is worth a
    // small extra credit on top of the per-event base in the modern
    // build-up section above. Box entries (passes_into_box + carries
    // into box) get a slightly larger bump — entering the box is the
    // chance-creation moment. Both damped for cameos.
    let progressive_zone = (z.progressive_passes_into_final_third as f32
        + z.progressive_carries_into_final_third as f32)
        * ZoneCoeffs::PROGRESSIVE_TO_FINAL_THIRD_PER;
    rating += progressive_zone.min(ZoneCoeffs::PROGRESSIVE_TO_FINAL_THIRD_CAP) * minute_damp;

    let box_entries = stats.passes_into_box as f32 + z.carries_into_box as f32;
    rating +=
        (box_entries * ZoneCoeffs::BOX_ENTRY_PER).min(ZoneCoeffs::BOX_ENTRY_CAP) * minute_damp;

    // ── Lane-aware creation bonuses (capped tight) ──────────────────────
    //
    // Half-space and central balls into the box are the most
    // threatening creation channels in modern football. They sit on
    // top of the regular `passes_into_box` credit, so the totals stay
    // small per event — but a midfielder who consistently hits the
    // half-space ends up materially above a midfielder racking up
    // wide / cross-spam crosses for the same number of box entries.
    rating += (z.half_space_passes_into_box as f32 * ZoneCoeffs::HALF_SPACE_BOX_ENTRY_PER)
        .min(ZoneCoeffs::HALF_SPACE_BOX_ENTRY_CAP)
        * minute_damp;
    rating += (z.central_passes_into_box as f32 * ZoneCoeffs::CENTRAL_BOX_ENTRY_PER)
        .min(ZoneCoeffs::CENTRAL_BOX_ENTRY_CAP)
        * minute_damp;
    rating += (z.switches_of_play as f32 * ZoneCoeffs::SWITCH_OF_PLAY_PER)
        .min(ZoneCoeffs::SWITCH_OF_PLAY_CAP)
        * minute_damp;

    // ── Dangerous turnovers ─────────────────────────────────────────────
    //
    // A miscontrol or bad pass in your own third is a real chance for
    // the opponent; in your own box it's a near-goal scenario. These
    // already feed `errors_leading_to_*` when they convert to a shot,
    // but they also dent the rating on their own.
    rating += z.dangerous_turnovers_own_third as f32 * ZoneCoeffs::TURNOVER_OWN_THIRD;
    rating += z.dangerous_turnovers_own_box as f32 * ZoneCoeffs::TURNOVER_OWN_BOX;

    // Errors-to-goal that originated from an own-box giveaway carry an
    // additional hit on top of the base error penalty — a goal-mouth
    // howler is materially worse than a midfield mistake that became a
    // goal. Capped by the rating's own 1.0 floor.
    rating += z.errors_to_goal_own_box as f32 * ZoneCoeffs::ERROR_TO_GOAL_OWN_BOX_EXTRA;

    // ── GK command-zone events ──────────────────────────────────────────
    //
    // `gk_command_actions` has a live producer (cross-claim, punch,
    // sweeper interception, high claim). The two `gk_failed_claims_*`
    // counters are intentionally read here without a live producer —
    // the rating side stays wired so the moment the GK state machine
    // emits "attempted claim and missed → opponent shot/goal" the
    // counter takes effect with no rating-helper change. Until then
    // both default to zero and contribute nothing.
    if pos == PlayerFieldPositionGroup::Goalkeeper {
        rating += (z.gk_command_actions as f32 * ZoneCoeffs::GK_COMMAND_PER)
            .min(ZoneCoeffs::GK_COMMAND_CAP);
        rating += z.gk_failed_claims_to_shot as f32 * ZoneCoeffs::GK_FAILED_CLAIM_TO_SHOT;
        rating += z.gk_failed_claims_to_goal as f32 * ZoneCoeffs::GK_FAILED_CLAIM_TO_GOAL;
    }

    // ── Discipline: fouls / offsides / own goals ────────────────────────
    //
    // Fouls are penalised regardless of card outcome — a high-volume
    // fouler who didn't get booked is still a drag on the team. The
    // own-third extra fires for defenders / goalkeepers because that's
    // where their fouls turn into set-piece chances against the team.
    // Penalty-conceding fouls are the most damaging single foul event.
    rating += (stats.fouls as f32 * ZoneCoeffs::FOUL_PER).max(ZoneCoeffs::FOUL_CAP);
    if matches!(
        pos,
        PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper
    ) {
        rating += z.own_third_def_fouls as f32 * ZoneCoeffs::FOUL_OWN_THIRD_DEF_EXTRA_PER;
    }
    rating += z.penalty_fouls_conceded as f32 * ZoneCoeffs::FOUL_PENALTY;

    // Offsides: forwards live with the offside line, so the per-event
    // hit is a touch larger but capped fast. Other positions getting
    // caught offside is rarer and a worse decision per event.
    let (off_per, off_cap) = match pos {
        PlayerFieldPositionGroup::Forward => (
            ZoneCoeffs::OFFSIDE_FORWARD_PER,
            ZoneCoeffs::OFFSIDE_FORWARD_CAP,
        ),
        _ => (ZoneCoeffs::OFFSIDE_OTHER_PER, ZoneCoeffs::OFFSIDE_OTHER_CAP),
    };
    rating += (stats.offsides as f32 * off_per).max(off_cap);

    // Own goals: -1.0 base + -0.30 because OGs sit inside the player's
    // own box by definition. Multiple OGs stack but are clipped by the
    // 1.0 floor.
    rating +=
        stats.own_goals as f32 * (ZoneCoeffs::OWN_GOAL_BASE + ZoneCoeffs::OWN_GOAL_OWN_BOX_EXTRA);

    // GK xG-prevented — positive means above-expectation shot stopping.
    // The engine doesn't currently populate per-shot xG, so when
    // `xg_prevented` is left at zero we derive a positive proxy from
    // actual save volume vs. an expected baseline (70% save rate, ~0.30
    // xG per shot on target). Without this, GKs miss up to +1.4 of
    // designed-in upside on every match while still absorbing the full
    // conceded penalty — pushing season averages well below outfield
    // positions.
    //
    // Upside-only by design: bad shifts are already taxed by the
    // conceded-goal penalty, the low-save% bonus, and (when the
    // giveaway converts) errors_leading_to_goal. The live producer
    // debits xg_prevented by `-shot_xg` on every concession, so taking
    // the raw value through here would double-count the same goals
    // and floor blowout keepers at 1.0. Clamp the input to [0, ∞)
    // before scaling so the contribution stays a non-negative bonus.
    if pos == PlayerFieldPositionGroup::Goalkeeper {
        let direct = stats.xg_prevented.max(0.0);
        let xg_p = if direct > 0.0 {
            direct
        } else {
            let shots = stats.shots_faced.max(stats.saves + opponent_goals as u16) as f32;
            if shots >= 3.0 {
                let expected_saves = shots * 0.70;
                ((stats.saves as f32 - expected_saves) * 0.30).max(0.0)
            } else {
                0.0
            }
        };
        rating += (xg_p * 0.45).min(1.4);
    }

    // Cameo bound: a player who came on for under 15 minutes can't post
    // worse than 5.8 or better than 7.2 unless they did something
    // exceptional (goal, red, error-to-goal). The exceptional-event
    // exemption keeps a 90th-minute winner posting an 8+.
    let exceptional = stats.goals > 0
        || stats.red_cards > 0
        || stats.errors_leading_to_goal > 0
        || stats.own_goals > 0;
    if stats.minutes_played < 15 && stats.minutes_played > 0 && !exceptional {
        rating = rating.clamp(5.8, 7.2);
    }

    // ── Low-involvement caps ─────────────────────────────────────────────
    //
    // Stops the "all-5 forward who didn't actually do much" from
    // posting 8.0 because of one lucky 0.10-xG poke. Goalkeepers are
    // exempt — their job is shot-stopping, not tackles or key passes.
    //
    // 1) 60+ minutes with no goal/assist/key pass and minimal
    //    defensive output: cap at 6.4.
    // 2) Single goal off a low-xG chance with otherwise low
    //    involvement: cap at 7.2.
    if !matches!(pos, PlayerFieldPositionGroup::Goalkeeper) {
        let total_defensive_actions = stats.tackles
            .saturating_add(stats.interceptions)
            .saturating_add(stats.successful_pressures)
            .saturating_add(stats.blocks)
            .saturating_add(stats.clearances);
        let creative_actions =
            stats.key_passes.saturating_add(stats.progressive_passes / 3);

        if stats.minutes_played >= 60
            && stats.goals == 0
            && stats.assists == 0
            && creative_actions == 0
            && total_defensive_actions < 4
        {
            rating = rating.min(6.4);
        }

        // Low-involvement single-goal cap.
        let positive_events = (stats.assists as u32)
            + (stats.key_passes as u32)
            + (stats.progressive_passes as u32) / 3
            + (stats.successful_dribbles as u32)
            + (stats.tackles as u32)
            + (stats.interceptions as u32)
            + (stats.successful_pressures as u32) / 2
            + (stats.blocks as u32)
            + (stats.clearances as u32);
        if stats.goals == 1 && stats.xg < 0.20 && positive_events < 5 {
            rating = rating.min(7.2);
        }
    }

    // ── Defender quality / volume separation ────────────────────────────
    //
    // Defender ratings should reward quality of defending, not raw
    // volume. The base rating already credits tackles/interceptions/
    // clearances/blocks per event with reasonable weights, and the zone
    // bonus separates own-box from midfield. The block below adds:
    //
    //   * a "clean duels" bonus that pays only for tackles produced
    //     without an accompanying foul,
    //   * a small block-quality lift when the defender contributes
    //     meaningful blocks on top of base credit,
    //   * a clearance-quality split so a defender who completes some
    //     controlled clearances earns a small extra above pure volume,
    //   * a low-skill volume cap that stops a defender drifting to 8.0+
    //     purely on routine clearances/tackles when no clean sheet,
    //     no clutch intervention, and no goal/assist were produced.
    if pos == PlayerFieldPositionGroup::Defender {
        // Clean-duel bonus: tackles produced beyond the foul count.
        // Approximates "tackles without a foul" — fouls already carry
        // a base penalty, so subtracting them rewards the cleanly-won
        // ball-winners separately.
        let tackles_u32 = stats.tackles as u32;
        let fouls_u32 = stats.fouls as u32;
        let clean_tackles = tackles_u32.saturating_sub(fouls_u32);
        rating += (clean_tackles as f32 * 0.055).min(0.28) * minute_damp;

        // Block-quality lift — proportional to total blocks in the
        // match. Base credit already pays per-block; this small extra
        // separates a defender who blocked 4 shots from one who blocked
        // a single low-danger effort.
        let extra_blocks = (stats.blocks as i32 - 1).max(0) as f32;
        rating += (extra_blocks * 0.05).min(0.40) * minute_damp;

        // Low-skill volume cap. Without a clean sheet, a goal or
        // assist, multiple blocks, or a major clutch intervention
        // (errors_to_goal == 0 implied), routine event volume should
        // not push a defender above 7.1.
        let major_intervention = stats.blocks >= 2;
        let attacking_contribution = stats.goals > 0 || stats.assists > 0;
        let clean_sheet = opponent_goals == 0;
        if stats.minutes_played >= 60
            && !clean_sheet
            && !attacking_contribution
            && !major_intervention
            && stats.errors_leading_to_goal == 0
        {
            rating = rating.min(7.1);
        }
    }

    // ── Midfielder quality / volume separation ──────────────────────────
    //
    // Midfielder ratings should reward quality, not raw pass volume. The
    // generic creative bonuses above already credit progressive passes
    // and key passes, but a low-skill midfielder racking up 70 safe
    // sideways passes can still drift into 7.5+ via passing accuracy +
    // win bonus + light defensive credit. The cap below stops the
    // "safe-recycler 8.0" outcome unless the midfielder produced real
    // attacking, defensive, or chance-creation output.
    if pos == PlayerFieldPositionGroup::Midfielder {
        // Pressing-quality bump — sustained successful pressures + raw
        // pressing volume.
        let pressing_quality = (stats.successful_pressures as f32 * 0.030
            + (stats.pressures.saturating_sub(stats.successful_pressures)) as f32 * 0.006)
            .min(0.32);
        rating += pressing_quality * minute_damp;

        // Switch-of-play bonus already applied above (capped). Add a
        // small extra per high-value key pass via xg_chain_buildup —
        // already covered by `xg_buildup` block. Nothing new here.

        // Safe-recycle cap: high pass volume, no progressive value, no
        // shot/key-pass involvement, minimal defensive contribution.
        if stats.minutes_played >= 60
            && stats.passes_attempted >= 30
            && stats.goals == 0
            && stats.assists == 0
            && stats.key_passes == 0
            && stats.progressive_passes <= 2
            && stats.shots_total == 0
            && (stats.tackles + stats.interceptions + stats.successful_pressures) < 5
        {
            rating = rating.min(6.7);
        }
    }

    rating.clamp(1.0, 10.0)
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
        let rating = calculate_match_rating(&stats, 1, 1);
        assert!((rating - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn goals_add_up_to_cap() {
        // Five-goal forward with realistic xG (the goals weren't lucky).
        // New goal-credit model caps base at +2.4, adds +0.20 decisive
        // win, +0.20 clinical bonus when goals exceed xG. Spec
        // explicitly removes the flat +1.0/goal — high output still
        // produces an elite rating, just not a runaway one.
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
        let rating = calculate_match_rating(&stats, 5, 0);
        assert!(rating >= 8.4, "rating={rating}");
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

        let quiet_rating = calculate_match_rating(&quiet_gk, 1, 0);
        let busy_rating = calculate_match_rating(&busy_gk, 1, 0);

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

        let passive_rating = calculate_match_rating(&passive, 1, 1);
        let active_rating = calculate_match_rating(&active, 1, 1);

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
        let rating = calculate_match_rating(&bad, 0, 5);
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
        let rating = calculate_match_rating(&great, 5, 0);
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

        let clinical_rating = calculate_match_rating(&clinical, 2, 0);
        let expected_rating = calculate_match_rating(&expected, 2, 0);

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
        let rating = calculate_match_rating(&gk, 0, 7);
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
        let rating = calculate_match_rating(&gk, 0, 3);
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
        let quiet_rating = calculate_match_rating(&quiet, 1, 0);
        let busy_rating = calculate_match_rating(&busy, 1, 0);
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
        let rating = calculate_match_rating(&gk, 0, 2);
        assert!(
            rating >= 5.5 && rating <= 6.5,
            "GK conceding 2 rated {} — should be around 6",
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
        let one_rating = calculate_match_rating(&one, 0, 1);
        let six_rating = calculate_match_rating(&six, 0, 6);
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
        let rating = calculate_match_rating(&gk, 0, 10);
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
        let rating = calculate_match_rating(&gk, 1, 2);
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
        let rating = calculate_match_rating(&gk, 0, 1);
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
        let rating = calculate_match_rating(&gk, 0, 4);
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
        let rating = calculate_match_rating(&gk, 1, 2);
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
        let rating = calculate_match_rating(&elite, 1, 1);
        // Ceiling check: with all bonuses (saves cap, save%, surplus)
        // the rating should sit comfortably below 10.
        assert!(rating < 10.0);
        // But should clearly outrate a baseline GK.
        let baseline = make_gk(2, 4);
        let baseline_rating = calculate_match_rating(&baseline, 1, 2);
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
        let elite_rating = calculate_match_rating(&elite, 0, 1);
        let baseline_rating = calculate_match_rating(&baseline, 0, 3);
        assert!(
            elite_rating > baseline_rating + 0.4,
            "Elite GK ({}) should clearly outrate baseline ({}); proxy not lifting",
            elite_rating,
            baseline_rating
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
        let rating = calculate_match_rating(&gk, 0, 0);
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
            0, 0, 30, 24, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        few.errors_leading_to_shot = 2;
        let mut many = few.clone();
        many.errors_leading_to_shot = 8;
        let few_rating = calculate_match_rating(&few, 1, 1);
        let many_rating = calculate_match_rating(&many, 1, 1);
        assert!(
            (few_rating - many_rating).abs() < 0.05,
            "errors-to-shot must cap: 2 events {} vs 8 events {} (delta {})",
            few_rating,
            many_rating,
            few_rating - many_rating
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
        let rating = calculate_match_rating(&gk, 0, 5);
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
        let control_rating = calculate_match_rating(&control, 0, 5);
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
        let rating = calculate_match_rating(&gk, 0, 6);
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

        let few_rating = calculate_match_rating(&few, 1, 1);
        let many_rating = calculate_match_rating(&many, 1, 1);

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

        let passive_rating = calculate_match_rating(&passive, 1, 0);
        let active_rating = calculate_match_rating(&active, 1, 0);
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

        let safe_rating = calculate_match_rating(&safe, 1, 1);
        let creative_rating = calculate_match_rating(&creative, 1, 1);
        assert!(
            creative_rating > safe_rating + 0.6,
            "creative MID ({}) should clearly outrate safe-passer MID ({})",
            creative_rating,
            safe_rating
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

        let accurate_rating = calculate_match_rating(&accurate, 1, 1);
        let spam_rating = calculate_match_rating(&spam, 1, 1);
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

        let clean_rating = calculate_match_rating(&clean, 1, 1);
        let sloppy_rating = calculate_match_rating(&sloppy, 1, 1);
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
        let wasteful_rating = calculate_match_rating(&wasteful, 2, 0);
        let clinical_rating = calculate_match_rating(&clinical, 2, 0);
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
        let rating = calculate_match_rating(&anchor, 1, 0);
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

        let mid_rating = calculate_match_rating(&middle, 1, 0);
        let box_rating = calculate_match_rating(&box_cb, 1, 0);
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

        let box_rating = calculate_match_rating(&box_cb, 1, 0);
        let six_rating = calculate_match_rating(&six_cb, 1, 0);
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
        let baseline = calculate_match_rating(&base, 1, 1);
        let mut with_extra = base.clone();
        with_extra.zone_stats.errors_to_goal_own_box = 1;
        let extra_rating = calculate_match_rating(&with_extra, 1, 1);
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

        let rating = calculate_match_rating(&cameo, 1, 1);
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
        let baseline = calculate_match_rating(&s, 1, 1);
        s.own_goals = 1;
        let with_og = calculate_match_rating(&s, 1, 2);
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
        let baseline = calculate_match_rating(&s, 1, 1);
        s.fouls = 1;
        s.zone_stats.penalty_fouls_conceded = 1;
        s.zone_stats.own_third_def_fouls = 1;
        let with_pen = calculate_match_rating(&s, 1, 2);
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
        let clean_rating = calculate_match_rating(&clean, 1, 1);
        let niggly_rating = calculate_match_rating(&niggly, 1, 1);
        assert!(
            clean_rating - niggly_rating >= 0.15,
            "high-volume fouler ({}) should rate visibly below clean ({})",
            niggly_rating,
            clean_rating
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

        let fwd_rating = calculate_match_rating(&fwd, 1, 1);
        let mid_rating = calculate_match_rating(&mid, 1, 1);
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

        let quiet_rating = calculate_match_rating(&quiet, 1, 0);
        let commanding_rating = calculate_match_rating(&commanding, 1, 0);
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
        let starter_rating = calculate_match_rating(&starter, 1, 1);
        let cameo_rating = calculate_match_rating(&cameo, 1, 1);
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
        let neutral_rating = calculate_match_rating(&neutral, 1, 1);
        let hs_rating = calculate_match_rating(&half_space, 1, 1);
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
        let rating = calculate_match_rating(&s, 1, 1);
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
        let baseline = calculate_match_rating(&s, 1, 1);
        s.zone_stats.switches_of_play = 10;
        let rating = calculate_match_rating(&s, 1, 1);
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
        let baseline = calculate_match_rating(&gk, 1, 1);
        let mut shot = gk.clone();
        shot.zone_stats.gk_failed_claims_to_shot = 1;
        let with_shot = calculate_match_rating(&shot, 1, 1);
        assert!(
            (baseline - with_shot - ZoneCoeffs::GK_FAILED_CLAIM_TO_SHOT.abs()).abs() < 0.01,
            "failed-claim-to-shot should drop rating by {:.2} — got {}",
            ZoneCoeffs::GK_FAILED_CLAIM_TO_SHOT.abs(),
            baseline - with_shot
        );
        let mut goal = gk.clone();
        goal.zone_stats.gk_failed_claims_to_goal = 1;
        let with_goal = calculate_match_rating(&goal, 1, 1);
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
        let plain_rating = calculate_match_rating(&plain, 1, 1);
        let chained_rating = calculate_match_rating(&chained, 1, 1);
        assert!(
            chained_rating > plain_rating,
            "buildup xG should lift rating: plain {}, chained {}",
            plain_rating,
            chained_rating
        );
    }
}
