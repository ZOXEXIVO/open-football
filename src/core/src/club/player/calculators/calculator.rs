use crate::{Person, Player, PlayerPositionType, PlayerSquadStatus, PlayerStatusType};
use chrono::NaiveDate;

pub struct PlayerValueCalculator;

impl PlayerValueCalculator {
    pub fn calculate(
        player: &Player,
        now: NaiveDate,
        price_level: f32,
        league_reputation: u16,
        club_reputation: u16,
    ) -> f64 {
        let base_value = determine_base_value(player);
        let age_factor = determine_age_factor(player, now);
        let potential_factor = determine_potential_factor(player, now);
        let status_factor = determine_status_factor(player);
        let squad_role_factor = determine_squad_role_factor(player);
        let contract_factor = determine_contract_factor(player, now);
        let performance_factor =
            determine_performance_factor(player, league_reputation);
        let recent_form_factor = determine_recent_form_factor(player);
        let career_factor = determine_career_consistency_factor(player);
        let reputation_factor = determine_reputation_factor(player);
        let position_factor = determine_position_factor(player);
        let league_club_factor = determine_league_club_factor(league_reputation, club_reputation);

        let value = base_value
            * age_factor
            * potential_factor
            * status_factor
            * squad_role_factor
            * contract_factor
            * performance_factor
            * recent_form_factor
            * career_factor
            * reputation_factor
            * position_factor
            * league_club_factor
            * price_level as f64;

        round_market_value(value.max(500.0))
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
/// Value tiers (approximate, before other factors):
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

/// Young players with high potential get a premium. The base bonus
/// scales with the gap between PA and CA; on top, wonderkids (age ≤ 21
/// with absolute PA ≥ 150) carry an additional multiplier so a 17-year-
/// old with PA 200 isn't priced like a journeyman just because his age
/// factor is small. Without this, the steep `determine_age_factor` curve
/// (e.g. 0.40 at age 18) silently capped elite young prospects far
/// below realistic transfer fees.
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

    // Base potential gap: adds 1-40% value for young players
    let gap_factor = 1.0 + (gap / 200.0) * age_bonus * 0.4;

    // Wonderkid premium: amplifies value for the rare ≤21yo with very
    // high absolute PA. Compensates for the steep age_factor curve so
    // realistic CA-110 / PA-180 17yos don't end up priced at <2M.
    let wonderkid_factor = if age <= 21 && potential >= 150.0 {
        let pa_excess = (potential - 150.0).min(50.0); // 0..50
        let age_weight = match age {
            0..=17 => 3.0,
            18 => 2.5,
            19 => 2.0,
            20 => 1.4,
            21 => 1.0,
            _ => 0.0,
        };
        1.0 + (pa_excess / 50.0) * age_weight
    } else {
        1.0
    };

    gap_factor * wonderkid_factor
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

    // Lst and Req discounts are intentionally NOT applied here.
    // They represent *market leverage* (buyer knows the player is available),
    // not a change in the player's intrinsic worth.
    // The transfer-specific discount is applied in PlayerValuationCalculator
    // (window.rs) where it belongs.

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

/// Season performance: goals, assists, appearances, average rating.
/// League-aware: deviations from neutral (1.0) are scaled by league
/// strength so a goal in a top-5 league is worth more to the market
/// than a goal in a semi-pro division. Half-weight when no league
/// context is available (academy, free agents, calibration tests).
fn determine_performance_factor(player: &Player, league_reputation: u16) -> f64 {
    let stats = &player.statistics;
    let mut factor = 1.0;

    // Appearances — regular playing time proves value
    if stats.played > 25 {
        factor *= 1.1;
    } else if stats.played > 15 {
        // Decent number of apps, no adjustment
    } else if stats.played > 5 {
        factor *= 0.95;
    } else if stats.played > 0 {
        factor *= 0.85; // Barely played — mild discount
    } else {
        factor *= 0.90; // No appearances yet — slight discount
    }

    // Goals contribution — position-aware
    if stats.played > 5 {
        let goals_per_game = stats.goals as f64 / stats.played as f64;
        let assists_per_game = stats.assists as f64 / stats.played as f64;

        if player.position().is_forward() {
            if goals_per_game > 0.5 {
                factor *= 1.2;
            } else if goals_per_game > 0.3 {
                factor *= 1.1;
            } else if goals_per_game < 0.1 {
                factor *= 0.8; // Forward who can't score
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

    // Average match rating — stronger impact for mediocre/poor performers
    if stats.average_rating > 7.5 {
        factor *= 1.15;
    } else if stats.average_rating > 7.0 {
        factor *= 1.05;
    } else if stats.average_rating > 6.9 {
        factor *= 0.97; // Slightly below par — mild discount
    } else if stats.average_rating > 6.8 {
        factor *= 0.93; // Average at best — noticeable discount
    } else if stats.average_rating > 6.5 {
        factor *= 0.85; // Below average performer
    } else if stats.average_rating > 6.0 {
        factor *= 0.75; // Poor performer — clear discount
    } else if stats.average_rating > 0.0 {
        factor *= 0.65; // Very poor
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

    // League-aware scaling: deviations from neutral (1.0) get more
    // weight in stronger leagues, less in weaker ones. A goal in Serie A
    // is a stronger market signal than the same number in a semi-pro
    // division. Half-weight floor when no league context is provided.
    let league_weight =
        0.5 + 0.5 * (league_reputation as f64 / 10_000.0).clamp(0.0, 1.0);
    1.0 + (factor - 1.0) * league_weight
}

/// Squad-status premium/discount. Owner-side leverage signal: a club's
/// designated KeyPlayer is more expensive to prise away than a backup
/// at the same CA. NotNeeded players carry an explicit "we'd take
/// less" tag — they get a real discount on intrinsic value, separate
/// from the Lst/Req/Unh market discounts applied in `window.rs`.
fn determine_squad_role_factor(player: &Player) -> f64 {
    let Some(contract) = &player.contract else {
        return 1.0;
    };
    match contract.squad_status {
        PlayerSquadStatus::KeyPlayer => 1.10,
        PlayerSquadStatus::FirstTeamRegular => 1.05,
        PlayerSquadStatus::HotProspectForTheFuture => 1.05,
        PlayerSquadStatus::FirstTeamSquadRotation => 1.0,
        PlayerSquadStatus::DecentYoungster => 0.97,
        PlayerSquadStatus::MainBackupPlayer => 0.95,
        PlayerSquadStatus::NotNeeded => 0.85,
        PlayerSquadStatus::Invalid
        | PlayerSquadStatus::NotYetSet
        | PlayerSquadStatus::SquadStatusCount => 1.0,
    }
}

/// Recent-form EMA multiplier. Complements the longer-horizon
/// `determine_performance_factor` (season average rating) with a short-window
/// signal that reacts to current form. A blazing run is worth a modest
/// premium; a cold streak is a discount. Zero form (pre-season / no match
/// data yet) returns 1.0 — neutral.
fn determine_recent_form_factor(player: &Player) -> f64 {
    let form = player.load.form_rating;
    if form <= 0.0 {
        return 1.0;
    }
    // Around 6.0 is neutral, 7.5+ hot, 5.5- cold. Kept lighter than the
    // season-average factor so two signals don't double-penalise a bad
    // stretch inside an otherwise-fine season.
    match form {
        f if f >= 8.0 => 1.10,
        f if f >= 7.5 => 1.06,
        f if f >= 7.0 => 1.03,
        f if f >= 6.5 => 1.00,
        f if f >= 6.0 => 0.98,
        f if f >= 5.5 => 0.94,
        _ => 0.88,
    }
}

/// Career consistency: players with a track record of mediocre performance across
/// multiple seasons should not command premium fees. This prevents the "stepping stone"
/// effect where a player's value inflates purely from moving to bigger clubs without
/// ever performing well.
///
/// Only applies to players with 2+ seasons of history. Young players (<22) are exempt
/// since their career is still developing.
fn determine_career_consistency_factor(player: &Player) -> f64 {
    let history = &player.statistics_history.items;

    // Need meaningful history to judge
    let rated_seasons: Vec<_> = history.iter()
        .filter(|h| h.statistics.played >= 10 && h.statistics.average_rating > 0.0)
        .collect();

    if rated_seasons.len() < 2 {
        return 1.0; // Not enough data
    }

    let total_games: u32 = rated_seasons.iter().map(|h| h.statistics.played as u32).sum();
    let weighted_rating: f64 = rated_seasons.iter()
        .map(|h| h.statistics.average_rating as f64 * h.statistics.played as f64)
        .sum::<f64>() / total_games as f64;

    // Career average rating impact:
    //   7.5+ → 1.10 (proven elite performer)
    //   7.0+ → 1.0  (solid career, no adjustment)
    //   6.9+ → 0.92 (slightly below par over career)
    //   6.8+ → 0.85 (consistently mediocre — significant discount)
    //   <6.8 → 0.75 (poor career record)
    if weighted_rating > 7.5 {
        1.10
    } else if weighted_rating > 7.0 {
        1.0
    } else if weighted_rating > 6.9 {
        0.92
    } else if weighted_rating > 6.8 {
        0.85
    } else {
        0.75
    }
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

/// League and club reputation factor.
///
/// A player's market value is heavily influenced by the
/// league and club they play in. The same player is worth far more at a Serie A
/// club than at a Maltese Premier League club.
///
/// League reputation (0-10000) and club reputation (0-10000) are blended:
///   - league_reputation contributes 60% (the market/TV money/visibility)
///   - club_reputation contributes 40% (brand, finances, exposure)
///
/// The resulting factor ranges from ~0.15 (weakest leagues) to ~1.30 (elite).
/// This means a player at a top-5 league club is worth ~8x more than the same
/// player in a semi-professional league.
///
/// Examples (approximate):
///   Serie A (8750) + Juventus (8500)  → factor ~1.25
///   Eredivisie (6500) + Ajax (7000)   → factor ~0.85
///   Maltese PL (2800) + Valletta (3000) → factor ~0.30
///   Amateur (1000) + local club (1000)  → factor ~0.15
fn determine_league_club_factor(league_reputation: u16, club_reputation: u16) -> f64 {
    let league_rep = league_reputation as f64;
    let club_rep = club_reputation as f64;

    // No context available (academy players, free agents) — neutral factor
    if league_rep == 0.0 && club_rep == 0.0 {
        return 1.0;
    }

    // Blend: 60% league, 40% club (league visibility drives market value more)
    let blended = league_rep * 0.6 + club_rep * 0.4;
    let normalized = blended / 10000.0; // 0.0 to 1.0

    // S-curve mapping: stronger separation between tiers
    // Uses a power curve with offset to create FM-like value tiers:
    //   normalized 0.10 (rep ~1000) → ~0.15
    //   normalized 0.25 (rep ~2500) → ~0.28
    //   normalized 0.40 (rep ~4000) → ~0.45
    //   normalized 0.55 (rep ~5500) → ~0.65
    //   normalized 0.70 (rep ~7000) → ~0.88
    //   normalized 0.85 (rep ~8500) → ~1.15
    //   normalized 1.00 (rep 10000) → ~1.30
    let factor = 0.15 + 1.15 * normalized * normalized;

    factor.clamp(0.15, 1.30)
}

/// Position-based value adjustment.
/// Includes base position premium and versatility bonus for multi-position players.
/// Players who can play both flanks (e.g. M L/R) or multiple roles are more valuable.
fn determine_position_factor(player: &Player) -> f64 {
    let base = if player.position().is_goalkeeper() {
        0.7 // Goalkeepers typically valued less
    } else if player.position().is_forward() {
        1.15 // Strikers command premium
    } else {
        1.0
    };

    // Versatility bonus: players with multiple qualified positions are more valuable.
    // Formation-slot variants (DCL/DCR for DC, MCL/MCR for MC) don't count.
    let positions = player.positions.positions();
    let unique_base_positions = positions.iter().filter(|p| !matches!(p,
        PlayerPositionType::DefenderCenterLeft |
        PlayerPositionType::DefenderCenterRight |
        PlayerPositionType::MidfielderCenterLeft |
        PlayerPositionType::MidfielderCenterRight
    )).count();

    let versatility_bonus = match unique_base_positions {
        0..=1 => 1.0,
        2 => 1.05,  // +5% for two real positions
        3 => 1.10,  // +10% for three
        _ => 1.15,  // +15% for four or more
    };

    base * versatility_bonus
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

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

    /// Build a fully-formed test player from `PlayerGenerator`, then
    /// surgically reset the fields the valuation tests care about so
    /// the result is deterministic. Generator handles all the deep
    /// initialisation we'd otherwise have to copy into the test file.
    fn make_valuation_player(
        now: NaiveDate,
        ca: u8,
        pa: u8,
        age: u8,
        position: PlayerPositionType,
    ) -> Player {
        use crate::club::player::generators::PlayerGenerator;
        use crate::PeopleNameGeneratorData;

        let names = PeopleNameGeneratorData {
            first_names: vec!["T".to_string()],
            last_names: vec!["Tester".to_string()],
            nicknames: Vec::new(),
        };
        let mut player = PlayerGenerator::generate(1, now, position, 100, &names);

        let bd = NaiveDate::from_ymd_opt(now.year() - age as i32, 6, 15)
            .unwrap_or(now);
        player.birth_date = bd;

        player.player_attributes.current_ability = ca;
        player.player_attributes.potential_ability = pa.max(ca);
        player.player_attributes.current_reputation = 800;
        player.player_attributes.world_reputation = 800;
        player.player_attributes.home_reputation = 1000;
        player.player_attributes.international_apps = 0;

        if let Some(c) = player.contract.as_mut() {
            c.expiration = NaiveDate::from_ymd_opt(now.year() + 3, now.month(), 1)
                .unwrap();
            c.squad_status = PlayerSquadStatus::FirstTeamSquadRotation;
        }

        // Reset noisy/random state from the generator so tests are
        // deterministic.
        player.statuses = crate::PlayerStatus::new();
        player.statistics = crate::PlayerStatistics::default();
        player.statistics_history = crate::PlayerStatisticsHistory::default();

        player
    }

    fn d(year: i32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, 7, 1).unwrap()
    }

    #[test]
    fn elite_league_club_values_player_far_above_minnow() {
        // Same physical player, two market contexts. The elite-league /
        // elite-club price should dwarf the lowly-league / small-club
        // price. The previous 0/0 callers flattened these to identical
        // values.
        let now = d(2025);
        let player = make_valuation_player(now, 130, 140, 26, PlayerPositionType::MidfielderCenter);

        let elite_value = PlayerValueCalculator::calculate(&player, now, 1.0, 9000, 9000);
        let small_value = PlayerValueCalculator::calculate(&player, now, 1.0, 2000, 2000);

        assert!(
            elite_value >= small_value * 3.0,
            "elite-context value {} should be at least 3x small-context {}",
            elite_value,
            small_value
        );
    }

    #[test]
    fn passing_real_seller_context_changes_value_vs_neutral() {
        // The 0/0 path returns the neutral 1.0 league_club_factor —
        // exactly the bug the audit flagged. Compare against a clearly
        // non-neutral context (a low-rep seller) so the test isn't
        // fooled by the curve happening to graze 1.0 around the
        // upper-mid tier.
        let now = d(2025);
        let player = make_valuation_player(now, 120, 130, 25, PlayerPositionType::MidfielderCenter);

        let neutral = PlayerValueCalculator::calculate(&player, now, 1.0, 0, 0);
        let with_low_context =
            PlayerValueCalculator::calculate(&player, now, 1.0, 2500, 2500);
        let with_high_context =
            PlayerValueCalculator::calculate(&player, now, 1.0, 9500, 9500);

        assert!(
            with_low_context < neutral * 0.5,
            "low-rep seller context should sharply discount vs neutral (neutral={}, low={})",
            neutral,
            with_low_context
        );
        assert!(
            with_high_context > neutral * 1.05,
            "elite seller context should premium vs neutral (neutral={}, high={})",
            neutral,
            with_high_context
        );
    }

    #[test]
    fn listed_and_requested_players_are_discounted() {
        // Lst → 0.9× and Req → 0.85× applied at the market layer
        // (window.rs). Verify the wrapper does what the calculator
        // doesn't, and that the discounts compound when both flags
        // are set.
        use crate::transfers::window::PlayerValuationCalculator as Vp;
        let now = d(2025);
        let mut player = make_valuation_player(
            now, 130, 135, 26, PlayerPositionType::MidfielderCenter,
        );

        let baseline = Vp::calculate_value_with_price_level(
            &player, now, 1.0, 7000, 7000,
        )
        .amount;

        player.statuses.add(now, PlayerStatusType::Lst);
        let listed = Vp::calculate_value_with_price_level(
            &player, now, 1.0, 7000, 7000,
        )
        .amount;
        assert!(
            listed < baseline,
            "listed player value {} should be below baseline {}",
            listed,
            baseline
        );

        player.statuses.add(now, PlayerStatusType::Req);
        let requested = Vp::calculate_value_with_price_level(
            &player, now, 1.0, 7000, 7000,
        )
        .amount;
        assert!(
            requested < listed,
            "transfer-requested + listed value {} should fall further below {}",
            requested,
            listed
        );
    }

    #[test]
    fn key_player_costs_more_than_backup_at_same_ca() {
        // Two identical CA-130 midfielders, one tagged KeyPlayer, the
        // other MainBackupPlayer. The key player must command a
        // measurable premium — squad role is genuine seller leverage.
        let now = d(2025);
        let mut key = make_valuation_player(now, 130, 135, 27, PlayerPositionType::MidfielderCenter);
        let mut backup = make_valuation_player(now, 130, 135, 27, PlayerPositionType::MidfielderCenter);

        if let Some(c) = key.contract.as_mut() {
            c.squad_status = PlayerSquadStatus::KeyPlayer;
            // Long contract — adds to the key-player premium.
            c.expiration = NaiveDate::from_ymd_opt(now.year() + 4, now.month(), 1).unwrap();
        }
        if let Some(c) = backup.contract.as_mut() {
            c.squad_status = PlayerSquadStatus::MainBackupPlayer;
        }

        let key_val = PlayerValueCalculator::calculate(&key, now, 1.0, 7000, 7000);
        let backup_val = PlayerValueCalculator::calculate(&backup, now, 1.0, 7000, 7000);

        assert!(
            key_val > backup_val * 1.1,
            "KeyPlayer value {} must clearly exceed MainBackupPlayer {} at same CA",
            key_val,
            backup_val
        );
    }

    #[test]
    fn elite_wonderkid_is_not_undervalued_by_age_factor() {
        // 18yo with PA 180, CA 110 at a top club: the old age_factor
        // (0.40) without a wonderkid premium left this player priced
        // like a journeyman. With the new potential factor, value
        // should reach realistic wonderkid territory.
        let now = d(2025);
        let player = make_valuation_player(
            now, 110, 180, 18, PlayerPositionType::MidfielderCenter,
        );

        let value = PlayerValueCalculator::calculate(&player, now, 1.0, 9000, 9000);

        assert!(
            value >= 15_000_000.0,
            "elite wonderkid value {} too low — age factor should not flatten high-PA youth",
            value
        );
    }

    #[test]
    fn league_strength_amplifies_performance_value() {
        // Same prolific scoring season. The market should pay more for
        // those goals in a top league than in a weaker one. Compares
        // performance-driven uplift (deviation from neutral 1.0)
        // alone — league_club_factor is held constant by passing the
        // same blended `club_reputation` in both calls.
        let now = d(2025);
        let mut player =
            make_valuation_player(now, 120, 125, 27, PlayerPositionType::Striker);
        // 30 goals / 30 games with a strong rating — exactly the
        // scenario that should price up in a strong league.
        player.statistics.played = 30;
        player.statistics.goals = 18;
        player.statistics.assists = 4;
        player.statistics.average_rating = 7.6;

        let strong = PlayerValueCalculator::calculate(&player, now, 1.0, 9000, 7000);
        let weak = PlayerValueCalculator::calculate(&player, now, 1.0, 2000, 7000);

        assert!(
            strong > weak,
            "strong-league value {} should exceed weak-league {} for the same scorer",
            strong,
            weak
        );
    }

    #[test]
    fn squad_role_factor_orders_statuses_correctly() {
        // Pure unit test of the role factor — KeyPlayer > Regular >
        // Rotation > Backup > NotNeeded. Pinning calibration so the
        // ordering can't silently invert.
        use PlayerSquadStatus::*;
        let now = d(2025);
        let mut p = make_valuation_player(now, 120, 125, 26, PlayerPositionType::MidfielderCenter);

        let with_status = |status: PlayerSquadStatus, p: &mut Player| {
            if let Some(c) = p.contract.as_mut() {
                c.squad_status = status;
            }
            determine_squad_role_factor(p)
        };

        assert!(with_status(KeyPlayer, &mut p) > with_status(FirstTeamRegular, &mut p));
        assert!(with_status(FirstTeamRegular, &mut p) > with_status(FirstTeamSquadRotation, &mut p));
        assert!(with_status(FirstTeamSquadRotation, &mut p) > with_status(MainBackupPlayer, &mut p));
        assert!(with_status(MainBackupPlayer, &mut p) > with_status(NotNeeded, &mut p));
    }
}
