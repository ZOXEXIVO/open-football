use crate::PlayerFieldPositionGroup;
use crate::r#match::PlayerMatchEndStats;

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

    // Goals: +1.0 each, capped at +3.0
    rating += (stats.goals as f32 * 1.0).min(3.0);

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

    // ── xG-based finishing quality ───────────────────────────────────────

    if stats.xg > 0.5 {
        let xg_delta = stats.goals as f32 - stats.xg;
        if xg_delta > 0.0 {
            // Clinical finisher — scored more than expected
            rating += (xg_delta * 0.15).min(0.3);
        } else if stats.goals == 0 && stats.xg > 1.0 {
            // Unlucky — created good chances but didn't convert
            rating += 0.1;
        }
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

    // Blocks count more for defenders, less for attackers.
    let block_w = match pos {
        PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper => 0.10,
        PlayerFieldPositionGroup::Midfielder => 0.07,
        _ => 0.04,
    };
    rating += (stats.blocks as f32 * block_w).min(0.5) * minute_damp;

    // Errors carry weight regardless of position. Errors-to-goal are the
    // most damaging individual events in the rating, sitting just below a
    // red card.
    rating -= stats.errors_leading_to_shot as f32 * 0.35;
    rating -= stats.errors_leading_to_goal as f32 * 0.90;

    // Cards — applied here in the post-pass so they affect the final
    // figure in the same place as errors.
    rating -= stats.yellow_cards as f32 * 0.15;
    rating -= stats.red_cards as f32 * 1.50;

    // GK xG-prevented — positive means above-expectation shot stopping.
    // Clamped so a single well-saved shot can't outweigh actual goals
    // shipped.
    if pos == PlayerFieldPositionGroup::Goalkeeper {
        rating += (stats.xg_prevented * 0.45).clamp(-1.2, 1.4);
    }

    // Cameo bound: a player who came on for under 15 minutes can't post
    // worse than 5.8 or better than 7.2 unless they did something
    // exceptional (goal, red, error-to-goal). The exceptional-event
    // exemption keeps a 90th-minute winner posting an 8+.
    let exceptional = stats.goals > 0
        || stats.red_cards > 0
        || stats.errors_leading_to_goal > 0;
    if stats.minutes_played < 15 && stats.minutes_played > 0 && !exceptional {
        rating = rating.clamp(5.8, 7.2);
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
            blocks: 0,
            clearances: 0,
            errors_leading_to_shot: 0,
            errors_leading_to_goal: 0,
            xg_prevented: 0.0,
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
        let stats = make_stats(
            5,
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
        let rating = calculate_match_rating(&stats, 5, 0);
        // goals capped at +3.0, plus win bonus +0.3, clean sheet not applicable for forward
        assert!(rating >= 9.0);
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
    fn high_volume_passing_bonus() {
        // Few passes, good accuracy
        let few = make_stats(
            0,
            0,
            15,
            14,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        // Many passes, good accuracy
        let many = make_stats(
            0,
            0,
            55,
            50,
            0,
            0,
            0,
            0,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );

        let few_rating = calculate_match_rating(&few, 1, 1);
        let many_rating = calculate_match_rating(&many, 1, 1);

        assert!(many_rating > few_rating);
    }
}
