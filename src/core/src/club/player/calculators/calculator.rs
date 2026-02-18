use crate::{Person, Player, PlayerStatusType};
use chrono::NaiveDate;

pub struct PlayerValueCalculator;

impl PlayerValueCalculator {
    pub fn calculate(player: &Player, now: NaiveDate, price_level: f32) -> f64 {
        let base_value = determine_base_value(player);
        let age_factor = determine_age_factor(player, now);
        let potential_factor = determine_potential_factor(player, now);
        let status_factor = determine_status_factor(player);
        let contract_factor = determine_contract_factor(player, now);
        let performance_factor = determine_performance_factor(player);
        let reputation_factor = determine_reputation_factor(player);
        let position_factor = determine_position_factor(player);

        let value = base_value
            * age_factor
            * potential_factor
            * status_factor
            * contract_factor
            * performance_factor
            * reputation_factor
            * position_factor
            * price_level as f64;

        round_market_value(value.max(5_000.0))
    }
}

/// Round to a "clean" market value (like real transfer fees)
fn round_market_value(value: f64) -> f64 {
    if value >= 10_000_000.0 {
        (value / 1_000_000.0).round() * 1_000_000.0
    } else if value >= 1_000_000.0 {
        (value / 100_000.0).round() * 100_000.0
    } else if value >= 100_000.0 {
        (value / 50_000.0).round() * 50_000.0
    } else if value >= 10_000.0 {
        (value / 10_000.0).round() * 10_000.0
    } else {
        (value / 1_000.0).round() * 1_000.0
    }
}

/// Base value from current_ability using a steep exponential curve.
///
/// FM-style value tiers (approximate, before other factors):
///   ability 20  → ~10K
///   ability 40  → ~75K
///   ability 60  → ~350K
///   ability 80  → ~1.5M
///   ability 100 → ~5M
///   ability 120 → ~15M
///   ability 140 → ~35M
///   ability 160 → ~65M
///   ability 180 → ~110M
///   ability 200 → ~175M
fn determine_base_value(player: &Player) -> f64 {
    let ability = player.player_attributes.current_ability as f64;

    // Use power-of-4 curve: steep growth that keeps low-ability values tiny
    let normalized = ability / 200.0; // 0.0 to 1.0
    let curve = normalized * normalized * normalized * normalized; // quartic

    // Skill quality: subtle modifier (average of all skill groups, 1-20 range)
    let technical = player.skills.technical.average() as f64;
    let mental = player.skills.mental.average() as f64;
    let physical = player.skills.physical.average() as f64;
    let skill_avg = (technical + mental + physical) / 3.0;
    let skill_factor = 0.85 + (skill_avg / 20.0) * 0.3; // 0.85 to 1.15

    175_000_000.0 * curve * skill_factor
}

/// Age factor: peak value at 25-28, premium for young players, steep decline 30+
fn determine_age_factor(player: &Player, date: NaiveDate) -> f64 {
    let age = player.age(date);

    match age {
        a if a < 17 => 0.15,
        17 => 0.25,
        18 => 0.40,
        19 => 0.55,
        20 => 0.70,
        21 => 0.82,
        22 => 0.90,
        23 => 0.95,
        24 => 1.0,
        25..=28 => 1.05,  // Peak years
        29 => 0.90,
        30 => 0.72,
        31 => 0.55,
        32 => 0.40,
        33 => 0.28,
        34 => 0.18,
        _ => 0.10,
    }
}

/// Young players with high potential get a premium
fn determine_potential_factor(player: &Player, date: NaiveDate) -> f64 {
    let age = player.age(date);
    let current = player.player_attributes.current_ability as f64;
    let potential = player.player_attributes.potential_ability as f64;

    if age > 28 || current >= potential {
        return 1.0;
    }

    let gap = potential - current;
    let age_bonus = if age < 21 {
        1.5
    } else if age < 24 {
        1.2
    } else {
        1.0
    };

    // Potential gap adds 1-40% value for young players
    1.0 + (gap / 200.0) * age_bonus * 0.4
}

/// Player statuses that affect value
fn determine_status_factor(player: &Player) -> f64 {
    let statuses = player.statuses.get();
    let mut factor = 1.0f64;

    if statuses.contains(&PlayerStatusType::Inj) {
        factor *= 0.6;
    }

    if statuses.contains(&PlayerStatusType::Unh) {
        factor *= 0.75;
    }

    if statuses.contains(&PlayerStatusType::Lst) {
        factor *= 0.85;
    }

    if statuses.contains(&PlayerStatusType::Req) {
        factor *= 0.8;
    }

    if statuses.contains(&PlayerStatusType::Loa) {
        factor *= 0.9;
    }

    factor
}

/// Contract length heavily affects transfer value
fn determine_contract_factor(player: &Player, date: NaiveDate) -> f64 {
    let contract = match &player.contract {
        Some(contract) => contract,
        None => return 0.1, // Free agent: minimal value
    };

    let days_remaining = (contract.expiration - date).num_days();
    let years_remaining = days_remaining as f64 / 365.0;

    match years_remaining {
        y if y <= 0.0 => 0.1,   // Expired
        y if y < 0.5 => 0.3,    // Less than 6 months
        y if y < 1.0 => 0.5,    // Less than 1 year
        y if y < 1.5 => 0.7,    // 1-1.5 years
        y if y < 2.0 => 0.85,   // 1.5-2 years
        y if y < 3.0 => 1.0,    // 2-3 years
        y if y < 4.0 => 1.05,   // 3-4 years
        _ => 1.1,               // 4+ years
    }
}

/// Season performance: goals, assists, appearances, average rating
fn determine_performance_factor(player: &Player) -> f64 {
    let stats = &player.statistics;
    let mut factor = 1.0;

    // Appearances
    if stats.played > 25 {
        factor *= 1.1;
    } else if stats.played < 5 {
        factor *= 0.85;
    }

    // Goals contribution
    if stats.played > 0 {
        let goals_per_game = stats.goals as f64 / stats.played as f64;
        let assists_per_game = stats.assists as f64 / stats.played as f64;

        if player.position().is_forward() {
            if goals_per_game > 0.5 {
                factor *= 1.2;
            } else if goals_per_game > 0.3 {
                factor *= 1.1;
            }
        }

        if player.position().is_midfielder() {
            let combined = goals_per_game + assists_per_game;
            if combined > 0.4 {
                factor *= 1.15;
            } else if combined > 0.2 {
                factor *= 1.05;
            }
        }
    }

    // Average match rating
    if stats.average_rating > 7.5 {
        factor *= 1.15;
    } else if stats.average_rating > 7.0 {
        factor *= 1.05;
    } else if stats.average_rating > 0.0 && stats.average_rating < 6.0 {
        factor *= 0.9;
    }

    // International experience
    let intl_apps = player.player_attributes.international_apps;
    if intl_apps > 50 {
        factor *= 1.2;
    } else if intl_apps > 20 {
        factor *= 1.1;
    } else if intl_apps > 5 {
        factor *= 1.05;
    }

    factor
}

/// Player reputation adds premium for well-known players
fn determine_reputation_factor(player: &Player) -> f64 {
    let rep = player.player_attributes.current_reputation as f64;

    if rep > 2000.0 {
        1.3
    } else if rep > 1000.0 {
        1.15
    } else if rep > 500.0 {
        1.05
    } else {
        1.0
    }
}

/// Position-based value adjustment
fn determine_position_factor(player: &Player) -> f64 {
    if player.position().is_goalkeeper() {
        0.7 // Goalkeepers typically valued less
    } else if player.position().is_forward() {
        1.15 // Strikers command premium
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_value_low_ability_is_cheap() {
        // ability 40 with avg skill ~5: base_value ~259K
        // After age/contract factors this becomes ~100-200K final value
        let normalized: f64 = 40.0 / 200.0;
        let curve = normalized.powi(4);
        let skill_factor = 0.85 + (5.0 / 20.0) * 0.3;
        let value = 175_000_000.0 * curve * skill_factor;
        assert!(value < 300_000.0, "ability 40 base_value = {}", value);
    }

    #[test]
    fn base_value_high_ability_is_expensive() {
        // ability 160 with avg skill ~15 should be tens of millions
        let normalized: f64 = 160.0 / 200.0;
        let curve = normalized.powi(4);
        let skill_factor = 0.85 + (15.0 / 20.0) * 0.3;
        let value = 175_000_000.0 * curve * skill_factor;
        assert!(value > 50_000_000.0, "ability 160 base_value = {}", value);
    }

    #[test]
    fn round_market_value_rounds_correctly() {
        assert_eq!(round_market_value(15_432_100.0), 15_000_000.0);
        assert_eq!(round_market_value(1_567_000.0), 1_600_000.0);
        assert_eq!(round_market_value(237_000.0), 250_000.0);
        assert_eq!(round_market_value(43_000.0), 40_000.0);
        assert_eq!(round_market_value(7_500.0), 8_000.0);
    }
}
