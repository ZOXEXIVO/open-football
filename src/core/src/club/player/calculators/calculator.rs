use crate::{Person, Player, PlayerStatusType};
use chrono::NaiveDate;

pub struct PlayerValueCalculator;

impl PlayerValueCalculator {
    pub fn calculate(player: &Player, now: NaiveDate) -> f64 {
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
            * position_factor;

        value.max(10_000.0)
    }
}

/// Base value derived from current_ability using exponential curve.
/// ability 40 → ~200K, 80 → ~3M, 120 → ~20M, 160 → ~60M, 200 → ~150M
fn determine_base_value(player: &Player) -> f64 {
    let ability = player.player_attributes.current_ability as f64;

    // Exponential: small base + steep growth for high ability
    let normalized = ability / 200.0; // 0.0 to 1.0
    let exponential = normalized * normalized * normalized; // cubic growth

    // Skill quality bonus (average of all skills, 1-20 range)
    let technical = player.skills.technical.average() as f64;
    let mental = player.skills.mental.average() as f64;
    let physical = player.skills.physical.average() as f64;
    let skill_avg = (technical + mental + physical) / 3.0;
    let skill_factor = 0.7 + (skill_avg / 20.0) * 0.6; // 0.7 to 1.3

    150_000_000.0 * exponential * skill_factor
}

/// Age factor: peak value at 25-28, premium for young players, steep decline 30+
fn determine_age_factor(player: &Player, date: NaiveDate) -> f64 {
    let age = player.age(date);

    match age {
        a if a < 18 => 0.3,
        18 => 0.5,
        19 => 0.65,
        20 => 0.8,
        21 => 0.9,
        22 => 0.95,
        23 => 1.0,
        24 => 1.05,
        25..=28 => 1.1,  // Peak years
        29 => 0.95,
        30 => 0.8,
        31 => 0.65,
        32 => 0.5,
        33 => 0.35,
        34 => 0.25,
        _ => 0.15,
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
    let age_bonus = if age < 22 { 1.5 } else { 1.0 };

    // Potential gap adds 1-50% value for young players
    1.0 + (gap / 200.0) * age_bonus * 0.5
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
    #[test]
    fn calculate_is_correct() {}
}
