use crate::r#match::PlayerMatchEndStats;
use crate::PlayerFieldPositionGroup;

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

    // Interceptions — reading the game is valuable, especially for defenders
    let interception_weight = match pos {
        PlayerFieldPositionGroup::Defender => 0.15,
        PlayerFieldPositionGroup::Midfielder => 0.10,
        _ => 0.06,
    };
    rating += (stats.interceptions as f32 * interception_weight).min(0.6);

    // ── Goalkeeper saves ─────────────────────────────────────────────────

    if pos == PlayerFieldPositionGroup::Goalkeeper {
        // Each save is a tangible contribution: +0.15 per save, cap +1.5
        let save_bonus = (stats.saves as f32 * 0.15).min(1.5);
        rating += save_bonus;

        // Busy keeper who kept a clean sheet — exceptional performance
        if stats.saves >= 5 && opponent_goals == 0 {
            rating += 0.3;
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
            PlayerFieldPositionGroup::Goalkeeper => rating += 0.5,
            PlayerFieldPositionGroup::Defender => rating += 0.3,
            PlayerFieldPositionGroup::Midfielder => rating += 0.1,
            _ => {}
        }
    }

    // ── Conceding many goals penalty ─────────────────────────────────────

    if opponent_goals >= 3 {
        match pos {
            PlayerFieldPositionGroup::Goalkeeper => rating -= 0.5,
            PlayerFieldPositionGroup::Defender => rating -= 0.3,
            _ => {}
        }
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
            match_rating: 0.0,
            xg,
            position_group,
            fouls: 0,
            yellow_cards: 0,
            red_cards: 0,
        }
    }

    #[test]
    fn base_rating_is_six() {
        // Forward with no events, 1-1 draw → pure base rating of 6.0
        let stats = make_stats(0, 0, 0, 0, 0, 0, 0, 0, 0, 0.0, PlayerFieldPositionGroup::Forward);
        let rating = calculate_match_rating(&stats, 1, 1);
        assert!((rating - 6.0).abs() < f32::EPSILON);
    }

    #[test]
    fn goals_add_up_to_cap() {
        let stats = make_stats(5, 0, 0, 0, 0, 0, 0, 0, 0, 0.0, PlayerFieldPositionGroup::Forward);
        let rating = calculate_match_rating(&stats, 5, 0);
        // goals capped at +3.0, plus win bonus +0.3, clean sheet not applicable for forward
        assert!(rating >= 9.0);
    }

    #[test]
    fn goalkeeper_saves_matter() {
        let quiet_gk = make_stats(0, 0, 15, 12, 0, 0, 0, 0, 1, 0.0, PlayerFieldPositionGroup::Goalkeeper);
        let busy_gk = make_stats(0, 0, 15, 12, 0, 0, 0, 0, 8, 0.0, PlayerFieldPositionGroup::Goalkeeper);

        let quiet_rating = calculate_match_rating(&quiet_gk, 1, 0);
        let busy_rating = calculate_match_rating(&busy_gk, 1, 0);

        // Busy GK with 8 saves should rate significantly higher
        assert!(busy_rating - quiet_rating > 1.0);
    }

    #[test]
    fn interceptions_boost_defender_rating() {
        let passive = make_stats(0, 0, 20, 16, 0, 0, 0, 0, 0, 0.0, PlayerFieldPositionGroup::Defender);
        let active = make_stats(0, 0, 20, 16, 0, 0, 3, 4, 0, 0.0, PlayerFieldPositionGroup::Defender);

        let passive_rating = calculate_match_rating(&passive, 1, 1);
        let active_rating = calculate_match_rating(&active, 1, 1);

        assert!(active_rating > passive_rating);
        assert!(active_rating - passive_rating > 0.8);
    }

    #[test]
    fn rating_clamped_to_range() {
        // Worst case
        let bad = make_stats(0, 0, 20, 5, 0, 5, 0, 0, 0, 0.0, PlayerFieldPositionGroup::Goalkeeper);
        let rating = calculate_match_rating(&bad, 0, 5);
        assert!(rating >= 1.0);
        assert!(rating <= 10.0);

        // Best case
        let great = make_stats(5, 3, 60, 57, 5, 5, 5, 5, 10, 1.0, PlayerFieldPositionGroup::Goalkeeper);
        let rating = calculate_match_rating(&great, 5, 0);
        assert!(rating >= 1.0);
        assert!(rating <= 10.0);
    }

    #[test]
    fn clinical_finisher_bonus() {
        // Player with 2 goals from 0.8 xG (clinical)
        let clinical = make_stats(2, 0, 20, 15, 2, 3, 0, 0, 0, 0.8, PlayerFieldPositionGroup::Forward);
        // Player with 2 goals from 2.0 xG (expected)
        let expected = make_stats(2, 0, 20, 15, 2, 3, 0, 0, 0, 2.0, PlayerFieldPositionGroup::Forward);

        let clinical_rating = calculate_match_rating(&clinical, 2, 0);
        let expected_rating = calculate_match_rating(&expected, 2, 0);

        assert!(clinical_rating > expected_rating);
    }

    #[test]
    fn high_volume_passing_bonus() {
        // Few passes, good accuracy
        let few = make_stats(0, 0, 15, 14, 0, 0, 0, 0, 0, 0.0, PlayerFieldPositionGroup::Midfielder);
        // Many passes, good accuracy
        let many = make_stats(0, 0, 55, 50, 0, 0, 0, 0, 0, 0.0, PlayerFieldPositionGroup::Midfielder);

        let few_rating = calculate_match_rating(&few, 1, 1);
        let many_rating = calculate_match_rating(&many, 1, 1);

        assert!(many_rating > few_rating);
    }
}
