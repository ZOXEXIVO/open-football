use crate::PlayerFieldPositionGroup;

/// Calculate a match rating (1.0 - 10.0, base 6.0)
pub fn calculate_match_rating(
    goals: u16,
    assists: u16,
    passes_attempted: u16,
    passes_completed: u16,
    shots_on_target: u16,
    shots_total: u16,
    tackles: u16,
    team_goals: u8,
    opponent_goals: u8,
    position_group: PlayerFieldPositionGroup,
) -> f32 {
    let mut rating: f32 = 6.0;

    // Goals: +1.0 each, capped at +3.0
    rating += (goals as f32 * 1.0).min(3.0);

    // Assists: +0.5 each, capped at +1.5
    rating += (assists as f32 * 0.5).min(1.5);

    // Pass completion bonus/penalty
    if passes_attempted > 5 {
        let pass_pct = passes_completed as f32 / passes_attempted as f32;
        // 70% = neutral, 90%+ = +0.4, below 50% = -0.4
        let pass_bonus = (pass_pct - 0.70) * 2.0;
        rating += pass_bonus.clamp(-0.4, 0.5);
    }

    // Shooting accuracy (only meaningful if shots taken)
    if shots_total > 0 {
        let shot_accuracy = shots_on_target as f32 / shots_total as f32;
        let shot_bonus = (shot_accuracy - 0.4) * 0.6;
        rating += shot_bonus.clamp(-0.2, 0.3);
    }

    // Defensive contribution - tackles
    // Weighted more for defenders/defensive midfielders
    let tackle_weight = match position_group {
        PlayerFieldPositionGroup::Defender => 0.12,
        PlayerFieldPositionGroup::Midfielder => 0.08,
        _ => 0.05,
    };
    rating += (tackles as f32 * tackle_weight).min(0.5);

    // Team result
    if team_goals > opponent_goals {
        rating += 0.3; // Win bonus
    } else if team_goals < opponent_goals {
        rating -= 0.2; // Loss penalty
    }

    // Clean sheet bonus for defenders and goalkeepers
    if opponent_goals == 0 {
        match position_group {
            PlayerFieldPositionGroup::Goalkeeper => rating += 0.8,
            PlayerFieldPositionGroup::Defender => rating += 0.4,
            PlayerFieldPositionGroup::Midfielder => rating += 0.1,
            _ => {}
        }
    }

    // Conceding many goals penalty for defenders/GK
    if opponent_goals >= 3 {
        match position_group {
            PlayerFieldPositionGroup::Goalkeeper => rating -= 0.5,
            PlayerFieldPositionGroup::Defender => rating -= 0.3,
            _ => {}
        }
    }

    rating.clamp(1.0, 10.0)
}
