use std::collections::HashMap;
use crate::club::player::contract::ContractBonusType;
use crate::club::team::team_talks::{apply_team_talk, MatchPhase, TeamTalkContext, TeamTalkTone};
use crate::club::StaffPosition;
use crate::continent::competitions::{CHAMPIONS_LEAGUE_ID, EUROPA_LEAGUE_ID, CONFERENCE_LEAGUE_ID};
use crate::r#match::engine::result::MatchResultRaw;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::MatchResult;
use crate::simulator::SimulatorData;
use crate::HappinessEventType;
use super::LeagueResult;

impl LeagueResult {
    pub(super) fn process_match_events(result: &mut MatchResult, data: &mut SimulatorData) {
        let details = match &result.details {
            Some(d) => d,
            None => return,
        };

        // Look up match type flags before mutable borrows
        // Continental cup matches (CL/EL/etc.) use a reserved ID range starting at 900_000_000
        let is_cup = result.league_id >= 900_000_000;
        let is_friendly = if is_cup {
            false
        } else {
            data.league(result.league_id)
                .map(|l| l.friendly)
                .unwrap_or(false)
        };

        // Helper macro to select the correct statistics field
        macro_rules! stats {
            ($player:expr) => {
                if is_cup { &mut $player.cup_statistics }
                else if is_friendly { &mut $player.friendly_statistics }
                else { &mut $player.statistics }
            };
        }

        // Mark players as played (main squad) or played_subs (substitutes)
        for player_id in &details.left_team_players.main {
            if let Some(player) = data.player_mut(*player_id) {
                stats!(player).played += 1;
            }
        }
        for player_id in &details.left_team_players.substitutes_used {
            if let Some(player) = data.player_mut(*player_id) {
                stats!(player).played_subs += 1;
            }
        }
        for player_id in &details.right_team_players.main {
            if let Some(player) = data.player_mut(*player_id) {
                stats!(player).played += 1;
            }
        }
        for player_id in &details.right_team_players.substitutes_used {
            if let Some(player) = data.player_mut(*player_id) {
                stats!(player).played_subs += 1;
            }
        }

        // Goals and assists from score details
        for detail in &result.score.details {
            match detail.stat_type {
                MatchStatisticType::Goal => {
                    if let Some(player) = data.player_mut(detail.player_id) {
                        stats!(player).goals += 1;
                    }
                }
                MatchStatisticType::Assist => {
                    if let Some(player) = data.player_mut(detail.player_id) {
                        stats!(player).assists += 1;
                    }
                }
                // Cards and fouls are tracked on per-player match statistics,
                // not in score details — ignored here.
                MatchStatisticType::YellowCard
                | MatchStatisticType::RedCard
                | MatchStatisticType::Foul => {}
            }
        }

        // Per-player stats (shots, passes, tackles, rating)
        let mut best_rating: f32 = 0.0;
        let mut best_player_id: Option<u32> = None;

        for (player_id, stats_data) in &details.player_stats {
            if let Some(player) = data.player_mut(*player_id) {
                let s = stats!(player);
                s.shots_on_target += stats_data.shots_on_target as f32;
                s.tackling += stats_data.tackles as f32;
                if stats_data.passes_attempted > 0 {
                    let match_pct = (stats_data.passes_completed as f32 / stats_data.passes_attempted as f32 * 100.0) as u8;
                    let games = s.played + s.played_subs;
                    if games <= 1 {
                        s.passes = match_pct;
                    } else {
                        let prev = s.passes as f32;
                        s.passes = ((prev * (games - 1) as f32 + match_pct as f32) / games as f32) as u8;
                    }
                }

                // Cards accumulate onto the season stat (saturating — u8).
                s.yellow_cards = s.yellow_cards.saturating_add(stats_data.yellow_cards as u8);
                s.red_cards = s.red_cards.saturating_add(stats_data.red_cards as u8);

                // Update running average rating
                let games = s.played + s.played_subs;
                if games <= 1 {
                    s.average_rating = stats_data.match_rating;
                } else {
                    let prev = s.average_rating;
                    s.average_rating =
                        (prev * (games - 1) as f32 + stats_data.match_rating) / games as f32;
                }

                // Track best rating for player of the match
                if stats_data.match_rating > best_rating {
                    best_rating = stats_data.match_rating;
                    best_player_id = Some(*player_id);
                }
            }
        }

        // Award player of the match
        if let Some(motm_id) = best_player_id {
            if let Some(player) = data.player_mut(motm_id) {
                stats!(player).player_of_the_match += 1;
                player.happiness.add_event(HappinessEventType::PlayerOfTheMatch, 4.0);
            }
        }

        // Goalkeeper stats: conceded goals and clean sheets
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();
        let home_team_id = result.score.home_team.team_id;

        // Find starting goalkeepers by checking main squad players' positions
        for &gk_id in details.left_team_players.main.iter() {
            if let Some(player) = data.player_mut(gk_id) {
                if player.position().is_goalkeeper() {
                    let goals_against = if details.left_team_players.team_id == home_team_id {
                        away_goals
                    } else {
                        home_goals
                    };
                    stats!(player).conceded += goals_against as u16;
                    if goals_against == 0 {
                        stats!(player).clean_sheets += 1;
                    }
                }
            }
        }
        for &gk_id in details.right_team_players.main.iter() {
            if let Some(player) = data.player_mut(gk_id) {
                if player.position().is_goalkeeper() {
                    let goals_against = if details.right_team_players.team_id == home_team_id {
                        away_goals
                    } else {
                        home_goals
                    };
                    stats!(player).conceded += goals_against as u16;
                    if goals_against == 0 {
                        stats!(player).clean_sheets += 1;
                    }
                }
            }
        }

        // Process loan match fees for official matches.
        // The parent club pays the borrowing club for each appearance.
        if !is_friendly {
            Self::process_loan_match_fees(details, data);
            // Contract clause payouts (appearance / goal / clean-sheet bonuses)
            Self::process_contract_bonuses(result, details, data);
        }

        // Post-match full-time team talks. Choose tone based on match
        // outcome vs pre-match expectations (rep gap), then apply a
        // DressingRoomSpeech event to every player who featured. Uses the
        // real-Player objects via player_mut() so morale actually moves.
        Self::apply_full_time_team_talks(result, details, data);

        // Individual debriefs driven by match rating. <6.3 → criticism,
        // >=7.5 → encouragement. Skips friendlies (low-stakes) and
        // substitutes who barely featured.
        if !is_friendly {
            Self::apply_post_match_debriefs(details, data);
        }

        // Apply physical effects from match participation (always, regardless of friendly flag)
        Self::apply_post_match_physical_effects(details, data);

        // Update player reputations based on match performance
        //
        // Continental competitions (CL/EL/Conference) use reserved league_id >= 900_000_000:
        //   Champions League:    900_000_001
        //   Europa League:       900_000_002
        //   Conference League:   900_000_003
        //
        // These get special reputation weights — especially for world reputation,
        // since playing in European competition is the primary driver of global recognition.
        let (league_weight, world_weight) = if result.league_id == CHAMPIONS_LEAGUE_ID {
            // Champions League: highest prestige, massive world reputation boost
            (1.5, 1.2)
        } else if result.league_id == EUROPA_LEAGUE_ID {
            // Europa League: high prestige
            (1.3, 0.8)
        } else if result.league_id == CONFERENCE_LEAGUE_ID {
            // Conference League: moderate prestige
            (1.1, 0.5)
        } else if is_cup {
            // Other cup competitions
            (1.0, 0.3)
        } else {
            let league_reputation = data.league(result.league_id)
                .map(|l| l.reputation)
                .unwrap_or(500) as f32;
            let w = (league_reputation / 1000.0 + 0.5).clamp(0.5, 1.5);
            (w, 0.2)
        };

        for (player_id, stats_data) in &details.player_stats {
            let rating_delta = (stats_data.match_rating - 6.0) * 20.0;
            let goal_bonus = stats_data.goals.min(3) as f32 * 15.0;
            let assist_bonus = stats_data.assists.min(3) as f32 * 8.0;
            let motm_bonus = if best_player_id == Some(*player_id) { 25.0 } else { 0.0 };
            let raw_delta = rating_delta + goal_bonus + assist_bonus + motm_bonus;

            if is_friendly {
                let home_delta = (raw_delta * 0.4 * league_weight) as i16;
                if let Some(player) = data.player_mut(*player_id) {
                    player.player_attributes.update_reputation(0, home_delta, 0);
                }
            } else {
                let current_delta = (raw_delta * league_weight) as i16;
                let home_delta = (raw_delta * 0.6 * league_weight) as i16;
                let world_delta = (raw_delta * world_weight * league_weight) as i16;
                if let Some(player) = data.player_mut(*player_id) {
                    player.player_attributes.update_reputation(current_delta, home_delta, world_delta);
                }
            }
        }

        // Save PoM to match result
        if let Some(details_mut) = &mut result.details {
            details_mut.player_of_the_match_id = best_player_id;
        }
    }

    /// Process loan match fees: parent club pays borrowing club per official appearance.
    /// Collects (parent_club_id, borrowing_club_id, fee) for all loan players who appeared,
    /// then applies the financial transactions.
    /// Pay out per-match contract clause triggers:
    ///   AppearanceFee  → flat bonus per player who played
    ///   GoalFee        → flat bonus × goals scored in this match
    ///   CleanSheetFee  → flat bonus for GK on a clean sheet
    ///
    /// Bonuses are charged to the employing club as a player-wage expense.
    fn process_contract_bonuses(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut SimulatorData,
    ) {
        // Count this match's goals per player.
        let mut goals_per_player: HashMap<u32, u8> = HashMap::new();
        for detail in &result.score.details {
            if detail.stat_type == MatchStatisticType::Goal {
                *goals_per_player.entry(detail.player_id).or_insert(0) += 1;
            }
        }

        // Everyone who featured in the match (main + used subs).
        let mut appearance_ids: Vec<u32> = Vec::new();
        appearance_ids.extend(details.left_team_players.main.iter().copied());
        appearance_ids.extend(details.left_team_players.substitutes_used.iter().copied());
        appearance_ids.extend(details.right_team_players.main.iter().copied());
        appearance_ids.extend(details.right_team_players.substitutes_used.iter().copied());

        // Which GKs started on which team + did they keep a clean sheet?
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();
        let home_team_id = result.score.home_team.team_id;
        let left_team_id = details.left_team_players.team_id;
        let left_conceded = if left_team_id == home_team_id { away_goals } else { home_goals };
        let right_conceded = if left_team_id == home_team_id { home_goals } else { away_goals };

        // Pass 1 (read): compute (club_id, total_payout) aggregates without holding
        // a mutable borrow of a club while reading another player.
        let mut club_totals: HashMap<u32, i64> = HashMap::new();
        for pid in &appearance_ids {
            if let Some(player) = data.player(*pid) {
                // Player's current club comes from the player index (updated by
                // the simulator whenever a transfer moves someone).
                let club_id = match data
                    .indexes
                    .as_ref()
                    .and_then(|i| i.get_player_location(*pid))
                {
                    Some((_, _, club_id, _)) => club_id,
                    None => continue,
                };
                let contract = match player.contract.as_ref() {
                    Some(c) => c,
                    None => continue,
                };

                // Determine clean sheet eligibility for a GK
                let is_gk = player.position().is_goalkeeper();
                let on_left = details.left_team_players.main.contains(pid);
                let on_right = details.right_team_players.main.contains(pid);
                let gk_clean_sheet = is_gk && (
                    (on_left && left_conceded == 0) ||
                    (on_right && right_conceded == 0)
                );

                let goals = goals_per_player.get(pid).copied().unwrap_or(0) as i64;

                let mut payout: i64 = 0;
                for bonus in &contract.bonuses {
                    let v = bonus.value as i64;
                    if v <= 0 { continue; }
                    match bonus.bonus_type {
                        ContractBonusType::AppearanceFee => payout += v,
                        ContractBonusType::GoalFee => payout += v * goals,
                        ContractBonusType::CleanSheetFee if gk_clean_sheet => payout += v,
                        _ => {}
                    }
                }
                if payout > 0 {
                    *club_totals.entry(club_id).or_insert(0) += payout;
                }
            }
        }

        // Pass 2 (write): charge each club once.
        for (club_id, amount) in club_totals {
            if let Some(club) = data.club_mut(club_id) {
                club.finance.balance.push_expense_player_wages(amount);
            }
        }
    }

    /// Apply a post-match full-time team talk to both sides. Tone is picked
    /// from the score outcome; the head coach's Man Management / Motivating
    /// attributes drive effectiveness. The actual magnitude-per-player uses
    /// personality (pressure, temperament, important_matches) via
    /// `club::team::team_talks::apply_team_talk`.
    /// After each competitive match, rate-based individual debriefs. Bad
    /// individual performances (rating <6.3) pick up a `ManagerCriticism`
    /// event; standout performances (>=7.5) pick up `ManagerEncouragement`.
    /// Already-ticking happiness-event machinery handles decay + morale
    /// ripple — we just feed the events in.
    fn apply_post_match_debriefs(
        details: &MatchResultRaw,
        data: &mut SimulatorData,
    ) {
        use crate::HappinessEventType;
        for (player_id, stats) in &details.player_stats {
            // Ignore barely-featured players (unreliable ratings). Match
            // rating is on a 1..10 scale; 0.0 means "no stats recorded".
            if stats.match_rating < 1.0 {
                continue;
            }

            if stats.match_rating < 6.3 {
                if let Some(player) = data.player_mut(*player_id) {
                    // Magnitude scales with how bad it was (6.3 → -2.0, 4.0 → -4.3).
                    let mag = -(2.0 + (6.3 - stats.match_rating).clamp(0.0, 3.0));
                    player.happiness.add_event(HappinessEventType::ManagerCriticism, mag);
                }
            } else if stats.match_rating >= 7.5 {
                if let Some(player) = data.player_mut(*player_id) {
                    let mag = 1.5 + (stats.match_rating - 7.5).clamp(0.0, 2.5);
                    player.happiness.add_event(HappinessEventType::ManagerEncouragement, mag);
                }
            }
        }
    }

    fn apply_full_time_team_talks(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut SimulatorData,
    ) {
        let score = &result.score;
        let home_goals = score.home_team.get() as i8;
        let away_goals = score.away_team.get() as i8;
        let home_team_id = score.home_team.team_id;
        let left_team_id = details.left_team_players.team_id;

        // Map "left" / "right" match slots to score deltas.
        let left_delta: i8 = if left_team_id == home_team_id {
            home_goals - away_goals
        } else {
            away_goals - home_goals
        };
        let right_delta: i8 = -left_delta;

        let tone_for = |delta: i8| -> TeamTalkTone {
            match delta {
                d if d >= 2 => TeamTalkTone::Praise,
                1 => TeamTalkTone::Encourage,
                0 => TeamTalkTone::Encourage,
                -1 => TeamTalkTone::Criticise,
                _ => TeamTalkTone::Passionate,
            }
        };

        // Collect each side's player ids + club id once.
        struct SideTalk {
            club_id: u32,
            player_ids: Vec<u32>,
            delta: i8,
        }

        let mut sides: Vec<SideTalk> = Vec::with_capacity(2);
        for (slot, delta) in [
            (&details.left_team_players, left_delta),
            (&details.right_team_players, right_delta),
        ] {
            let mut pids: Vec<u32> = Vec::new();
            pids.extend(slot.main.iter().copied());
            pids.extend(slot.substitutes_used.iter().copied());
            if pids.is_empty() {
                continue;
            }
            // Resolve club id from the first player's index entry.
            let club_id = data
                .indexes
                .as_ref()
                .and_then(|i| i.get_player_location(*pids.first().unwrap()))
                .map(|(_, _, c, _)| c)
                .unwrap_or(0);
            sides.push(SideTalk { club_id, player_ids: pids, delta });
        }

        for side in sides {
            // Find the head coach (Manager) for this club. Scans each team's
            // staff collection via StaffCollection::find_by_position — the
            // first team that has a Manager on the books wins.
            let manager_ref = data.club(side.club_id).and_then(|club| {
                club.teams
                    .teams
                    .iter()
                    .find_map(|t| t.staffs.find_by_position(StaffPosition::Manager))
            });
            let manager_clone = manager_ref.cloned();

            let tone = tone_for(side.delta);
            let ctx = TeamTalkContext {
                phase: MatchPhase::FullTime,
                score_delta: side.delta,
                big_match: false, // cup/derby detection would slot in here
            };

            // Borrow each player mutably one at a time; apply_team_talk
            // needs &mut Player so we route through player_mut.
            for pid in &side.player_ids {
                if let Some(player) = data.player_mut(*pid) {
                    // apply_team_talk takes an iterator of &mut Player; use a
                    // single-element array for the per-player loop.
                    let single = std::slice::from_mut(player);
                    apply_team_talk(single.iter_mut(), manager_clone.as_ref(), tone, ctx);
                }
            }
        }
    }

    fn process_loan_match_fees(details: &MatchResultRaw, data: &mut SimulatorData) {
        // Collect fee transfers: (parent_club_id, borrowing_club_id, fee)
        let mut fee_transfers: Vec<(u32, u32, u32)> = Vec::new();

        let all_players = details.left_team_players.main.iter()
            .chain(details.left_team_players.substitutes_used.iter())
            .chain(details.right_team_players.main.iter())
            .chain(details.right_team_players.substitutes_used.iter());

        for &player_id in all_players {
            if let Some(player) = data.player(player_id) {
                if let Some(ref loan) = player.contract_loan {
                    if let (Some(fee), Some(parent_id), Some(borrowing_id)) =
                        (loan.loan_match_fee, loan.loan_from_club_id, loan.loan_to_club_id)
                    {
                        if fee > 0 {
                            fee_transfers.push((parent_id, borrowing_id, fee));
                        }
                    }
                }
            }
        }

        // Apply financial transactions
        for (parent_club_id, borrowing_club_id, fee) in fee_transfers {
            let amount = fee as i64;
            if let Some(parent_club) = data.club_mut(parent_club_id) {
                parent_club.finance.balance.push_expense_loan_fees(amount);
            }
            if let Some(borrowing_club) = data.club_mut(borrowing_club_id) {
                borrowing_club.finance.balance.push_income_loan_fees(amount);
            }
        }
    }
}
