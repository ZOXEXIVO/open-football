use super::LeagueResult;
use super::data_access::LeagueProcessAccess;
use crate::PlayerFieldPositionGroup;
use crate::PlayerPositionType;
use crate::club::StaffPosition;
use crate::club::player::contract::ContractBonusType;
use crate::club::player::events::discipline::YELLOW_CARD_BAN_THRESHOLD;
use crate::club::player::events::{MatchOutcome, MatchParticipation};
use crate::club::team::reputation::{
    CompetitionType as RepCompetition, MatchOutcome as RepOutcome,
};
use crate::club::team::{
    LeadershipCandidate, MatchPhase, MatchdayLeadership, TeamTalkContext, TeamTalkTone,
    apply_team_talk_dated,
};
use crate::continent::competitions::{
    CHAMPIONS_LEAGUE_ID, CONFERENCE_LEAGUE_ID, COPA_LIBERTADORES_ID, EUROPA_LEAGUE_ID,
};
use crate::r#match::engine::result::MatchResultRaw;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{FieldSquad, MatchResult};
use crate::transfers::pipeline::KnownPlayerMemory;
use crate::transfers::window::PlayerValuationCalculator;
use chrono::Datelike;
use chrono::NaiveDate;
use std::cmp::Ordering;
use std::collections::HashMap;

impl LeagueResult {
    pub(super) fn process_match_events<D: LeagueProcessAccess>(
        result: &mut MatchResult,
        data: &mut D,
    ) {
        let details = match &result.details {
            Some(d) => d,
            None => return,
        };

        // Continental cups don't have a `League` row; everything else is
        // classified by the league's own `is_cup` flag. The previous
        // `league_id >= 900_000_000` heuristic miscategorised domestic
        // leagues whose generated IDs landed above that threshold (e.g.
        // Russian Second Division ~2.0e9), routing their matches into the
        // cup statistics bucket.
        let is_continental_cup = matches!(
            result.league_id,
            CHAMPIONS_LEAGUE_ID | EUROPA_LEAGUE_ID | CONFERENCE_LEAGUE_ID | COPA_LIBERTADORES_ID
        );
        let (is_cup, is_friendly) = if is_continental_cup {
            (true, false)
        } else {
            data.league(result.league_id)
                .map(|l| (l.is_cup, l.friendly))
                .unwrap_or((false, false))
        };

        // Players inside their post-transfer settlement window play at a
        // reduced level. Dampened rating feeds into season averages, POM
        // selection, debriefs, and reputation.
        let now_date = data.date().date();
        let effective_ratings = compute_effective_ratings(details, data, now_date);
        let best_player_id = pick_player_of_the_match(details, &effective_ratings);

        let (league_weight, world_weight) = reputation_weights(result, is_cup, data);

        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();
        let home_team_id = result.score.home_team.team_id;
        let away_team_id = result.score.away_team.team_id;

        // Rivalry detection: walk team → club on both sides and check whether
        // either club lists the other as a rival. Either-direction is a derby.
        let home_club = data.team(home_team_id).map(|t| t.club_id);
        let away_club = data.team(away_team_id).map(|t| t.club_id);
        let is_derby = match (home_club, away_club) {
            (Some(h), Some(a)) => {
                let h_vs_a = data.club(h).map(|c| c.is_rival(a)).unwrap_or(false);
                let a_vs_h = data.club(a).map(|c| c.is_rival(h)).unwrap_or(false);
                h_vs_a || a_vs_h
            }
            _ => false,
        };

        // Per-player match reaction — Player owns all bookkeeping.
        for side in [&details.left_team_players, &details.right_team_players] {
            let (scored, conceded) = if side.team_id == home_team_id {
                (home_goals, away_goals)
            } else {
                (away_goals, home_goals)
            };
            let (team_won, team_lost) = (scored > conceded, scored < conceded);
            dispatch_match_outcomes(
                side,
                scored,
                conceded,
                details,
                data,
                &effective_ratings,
                best_player_id,
                is_friendly,
                is_cup,
                &result.league_slug,
                league_weight,
                world_weight,
                is_derby,
                team_won,
                team_lost,
            );
        }

        Self::record_match_scouting_memory(
            details,
            data,
            is_friendly,
            now_date,
            home_team_id,
            away_team_id,
        );

        // Post-match social fallout — bonds, friction, admiration, envy.
        // Only fires for competitive matches where the dressing-room stakes
        // are real; friendlies don't move relationships.
        if !is_friendly {
            Self::apply_match_relationship_updates(
                result,
                details,
                data,
                now_date,
                home_team_id,
                best_player_id,
            );
        }

        if !is_friendly {
            Self::process_loan_match_fees(details, data);
            Self::process_contract_bonuses(result, details, data);
        }

        Self::apply_full_time_team_talks(result, details, data);
        Self::apply_post_match_physical_effects(details, data, is_friendly);
        Self::apply_post_match_reputation(result, data, is_friendly, is_cup);

        // Domestic-cup breakout scouting: a lower-league hero who shines
        // against a stronger club in a cup tie becomes visible to scouts.
        // Gated to domestic cups only — friendlies and continental cups
        // (which already have world-stage exposure) are excluded.
        if is_cup && !is_friendly && !is_continental_cup {
            Self::record_domestic_cup_showcase_scouting(
                details,
                data,
                now_date,
                home_team_id,
                away_team_id,
                home_goals,
                away_goals,
                best_player_id,
            );
        }

        // Disciplinary fallout — apply yellow / red card stats to each
        // player who featured, and serve a match for any banned player
        // on either side who didn't appear (their team played without
        // them, so the suspension counter ticks down).
        if !is_friendly {
            Self::apply_post_match_discipline(result, details, data);
        }

        if let Some(details_mut) = &mut result.details {
            details_mut.player_of_the_match_id = best_player_id;
        }
    }

    /// Clubs learn about players who appear in their domestic football.
    /// This is especially important for foreign loanees: once they return
    /// home, active country-local scouting can no longer see them, but clubs
    /// should still remember the player by id and profile.
    fn record_match_scouting_memory<D: LeagueProcessAccess>(
        details: &MatchResultRaw,
        data: &mut D,
        is_friendly: bool,
        date: NaiveDate,
        home_team_id: u32,
        away_team_id: u32,
    ) {
        struct MemoryAction {
            country_id: u32,
            current_club_id: u32,
            memory: KnownPlayerMemory,
        }

        let mut actions: Vec<MemoryAction> = Vec::new();

        for side in [&details.left_team_players, &details.right_team_players] {
            let current_club_id = match data.team(side.team_id).map(|t| t.club_id) {
                Some(id) => id,
                None => continue,
            };
            let current_country_id = match data.country_by_club(current_club_id).map(|c| c.id) {
                Some(id) => id,
                None => continue,
            };
            let current_price_level = data
                .country(current_country_id)
                .map(|c| c.settings.pricing.price_level)
                .unwrap_or(1.0);

            let appeared: Vec<u32> = side
                .main
                .iter()
                .copied()
                .chain(side.substitutes_used.iter().copied())
                .collect();

            for player_id in appeared {
                let player = match data.player(player_id) {
                    Some(p) => p,
                    None => continue,
                };

                let foreign_loan_exposure = player
                    .contract_loan
                    .as_ref()
                    .and_then(|loan| loan.loan_from_club_id)
                    .and_then(|parent_club_id| {
                        data.country_by_club(parent_club_id)
                            .map(|parent_country| parent_country.id != current_country_id)
                    })
                    .unwrap_or(false);

                // Regular domestic players are discovered by the normal
                // scouting pipeline. This memory path is for players whose
                // stay in the country is temporary or otherwise easy to lose.
                if !foreign_loan_exposure {
                    continue;
                }

                let stats = match details.player_stats.get(&player_id) {
                    Some(stats) => stats,
                    None => continue,
                };
                let skill_ability = player
                    .skills
                    .calculate_ability_for_position(player.position());
                let rating_bonus = if stats.match_rating >= 7.5 {
                    5
                } else if stats.match_rating >= 7.0 {
                    3
                } else if stats.match_rating >= 6.5 {
                    1
                } else if stats.match_rating < 5.5 {
                    -3
                } else {
                    0
                };
                let assessed_ability = (skill_ability as i32 + rating_bonus).clamp(1, 200) as u8;
                let assessed_potential = player
                    .player_attributes
                    .potential_ability
                    .max(assessed_ability);
                // Use the host club's blended reputation as the seller
                // proxy for memory snapshots — the player is appearing
                // in this country, so the local league/club context is
                // the right anchor for the buyer's mental price tag.
                let (host_league_rep, host_club_rep) = data
                    .country(current_country_id)
                    .and_then(|country| {
                        country
                            .clubs
                            .iter()
                            .find(|c| c.id == current_club_id)
                            .map(|club| PlayerValuationCalculator::seller_context(country, club))
                    })
                    .unwrap_or((0, 0));
                let estimated_value = PlayerValuationCalculator::calculate_value_with_price_level(
                    player,
                    date,
                    current_price_level,
                    host_league_rep,
                    host_club_rep,
                )
                .amount;

                actions.push(MemoryAction {
                    country_id: current_country_id,
                    current_club_id,
                    memory: KnownPlayerMemory {
                        player_id,
                        last_known_club_id: current_club_id,
                        last_known_country_id: current_country_id,
                        position: player.position(),
                        position_group: player.position().position_group(),
                        assessed_ability,
                        assessed_potential,
                        confidence: if is_friendly { 0.28 } else { 0.48 },
                        estimated_fee: estimated_value,
                        last_seen: date,
                        official_appearances_seen: if is_friendly { 0 } else { 1 },
                        friendly_appearances_seen: if is_friendly { 1 } else { 0 },
                    },
                });
            }
        }

        for action in actions {
            if let Some(country) = data.country_mut(action.country_id) {
                for club in &mut country.clubs {
                    if club.id == action.current_club_id
                        || club.teams.iter().any(|team| team.id == home_team_id)
                        || club.teams.iter().any(|team| team.id == away_team_id)
                    {
                        club.transfer_plan
                            .remember_known_player(action.memory.clone());
                    }
                }
            }
        }
    }

    /// Feed the completed match into both teams' `TeamReputation`. Friendlies
    /// don't drift reputation; cups and league games do, with competition
    /// weighting handled inside `process_weekly_update`.
    fn apply_post_match_reputation<D: LeagueProcessAccess>(
        result: &MatchResult,
        data: &mut D,
        is_friendly: bool,
        is_cup: bool,
    ) {
        if is_friendly {
            return;
        }

        let home_team_id = result.score.home_team.team_id;
        let away_team_id = result.score.away_team.team_id;
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();

        let home_rep = data
            .team(home_team_id)
            .map(|t| overall_score_to_u16(t.reputation.overall_score()))
            .unwrap_or(0);
        let away_rep = data
            .team(away_team_id)
            .map(|t| overall_score_to_u16(t.reputation.overall_score()))
            .unwrap_or(0);

        let (home_outcome, away_outcome) = match home_goals.cmp(&away_goals) {
            Ordering::Greater => (RepOutcome::Win, RepOutcome::Loss),
            Ordering::Less => (RepOutcome::Loss, RepOutcome::Win),
            Ordering::Equal => (RepOutcome::Draw, RepOutcome::Draw),
        };

        let comp = match (result.league_id, is_cup) {
            (
                CHAMPIONS_LEAGUE_ID | EUROPA_LEAGUE_ID | CONFERENCE_LEAGUE_ID
                | COPA_LIBERTADORES_ID,
                _,
            ) => RepCompetition::ContinentalCup,
            (_, true) => RepCompetition::DomesticCup,
            _ => RepCompetition::League,
        };

        let (home_pos, away_pos, total_teams) =
            league_standings(result.league_id, home_team_id, away_team_id, data);
        let date = data.date().date();

        if let Some(team) = data.team_mut(home_team_id) {
            team.on_match_completed(
                home_outcome,
                away_rep,
                comp.clone(),
                home_pos,
                total_teams,
                date,
            );
        }
        if let Some(team) = data.team_mut(away_team_id) {
            team.on_match_completed(away_outcome, home_rep, comp, away_pos, total_teams, date);
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
    fn process_contract_bonuses<D: LeagueProcessAccess>(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut D,
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

        // Bench players who never came on are paid the unused-sub fee.
        let unused_subs_left: Vec<u32> = details
            .left_team_players
            .substitutes
            .iter()
            .copied()
            .filter(|id| !details.left_team_players.substitutes_used.contains(id))
            .collect();
        let unused_subs_right: Vec<u32> = details
            .right_team_players
            .substitutes
            .iter()
            .copied()
            .filter(|id| !details.right_team_players.substitutes_used.contains(id))
            .collect();
        let unused_sub_ids: Vec<u32> = unused_subs_left
            .into_iter()
            .chain(unused_subs_right.into_iter())
            .collect();

        // Which GKs started on which team + did they keep a clean sheet?
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();
        let home_team_id = result.score.home_team.team_id;
        let left_team_id = details.left_team_players.team_id;
        let left_conceded = if left_team_id == home_team_id {
            away_goals
        } else {
            home_goals
        };
        let right_conceded = if left_team_id == home_team_id {
            home_goals
        } else {
            away_goals
        };

        // Pass 1 (read): compute (club_id, total_payout) aggregates without holding
        // a mutable borrow of a club while reading another player.
        //
        // Bonus payer ownership for loanees:
        //   - Bonuses written on the *parent* contract are owed by the
        //     parent club. The borrower can negotiate its own bonuses on
        //     `contract_loan` and those bill the borrower. A loanee
        //     scoring 10 goals at the borrower must NOT charge the
        //     borrower for a goal bonus the parent agreed to.
        //   - Permanent players: contract bonuses billed to current club
        //     as before.
        let mut club_totals: HashMap<u32, i64> = HashMap::new();
        for pid in &appearance_ids {
            if let Some(player) = data.player(*pid) {
                // Permanent-deal payer (current club from index). Loaned
                // players' borrower id is the same lookup result; we
                // override below for parent-contract bonuses.
                let borrower_club_id =
                    match data.indexes().and_then(|i| i.get_player_location(*pid)) {
                        Some((_, _, club_id, _)) => club_id,
                        None => continue,
                    };

                // Determine clean sheet eligibility for a GK
                let is_gk = player.position().is_goalkeeper();
                let on_left = details.left_team_players.main.contains(pid);
                let on_right = details.right_team_players.main.contains(pid);
                let gk_clean_sheet =
                    is_gk && ((on_left && left_conceded == 0) || (on_right && right_conceded == 0));

                let goals = goals_per_player.get(pid).copied().unwrap_or(0) as i64;

                // Parent contract bonuses → parent club. Pull the parent
                // club id off the loan contract so cross-country loans
                // route correctly.
                if let Some(parent_contract) = player.contract.as_ref() {
                    let parent_club_id = player
                        .contract_loan
                        .as_ref()
                        .and_then(|l| l.loan_from_club_id)
                        .unwrap_or(borrower_club_id);
                    let mut parent_payout: i64 = 0;
                    for bonus in &parent_contract.bonuses {
                        let v = bonus.value as i64;
                        if v <= 0 {
                            continue;
                        }
                        match bonus.bonus_type {
                            ContractBonusType::AppearanceFee => parent_payout += v,
                            ContractBonusType::GoalFee => parent_payout += v * goals,
                            ContractBonusType::CleanSheetFee if gk_clean_sheet => {
                                parent_payout += v
                            }
                            _ => {}
                        }
                    }
                    if parent_payout > 0 {
                        *club_totals.entry(parent_club_id).or_insert(0) += parent_payout;
                    }
                }

                // Loan-contract bonuses (if the borrower negotiated any)
                // bill the borrower.
                if let Some(loan_contract) = player.contract_loan.as_ref() {
                    let mut borrower_payout: i64 = 0;
                    for bonus in &loan_contract.bonuses {
                        let v = bonus.value as i64;
                        if v <= 0 {
                            continue;
                        }
                        match bonus.bonus_type {
                            ContractBonusType::AppearanceFee => borrower_payout += v,
                            ContractBonusType::GoalFee => borrower_payout += v * goals,
                            ContractBonusType::CleanSheetFee if gk_clean_sheet => {
                                borrower_payout += v
                            }
                            _ => {}
                        }
                    }
                    if borrower_payout > 0 {
                        *club_totals.entry(borrower_club_id).or_insert(0) += borrower_payout;
                    }
                }
            }
        }

        // Unused-substitute fee: bench players who didn't get on the
        // pitch are still paid their negotiated showup fee. Same payer
        // routing as the in-play bonuses — parent contract bills the
        // parent, loan contract bills the borrower.
        for pid in &unused_sub_ids {
            if let Some(player) = data.player(*pid) {
                let borrower_club_id =
                    match data.indexes().and_then(|i| i.get_player_location(*pid)) {
                        Some((_, _, club_id, _)) => club_id,
                        None => continue,
                    };
                if let Some(parent_contract) = player.contract.as_ref() {
                    let parent_club_id = player
                        .contract_loan
                        .as_ref()
                        .and_then(|l| l.loan_from_club_id)
                        .unwrap_or(borrower_club_id);
                    let mut payout: i64 = 0;
                    for bonus in &parent_contract.bonuses {
                        if matches!(bonus.bonus_type, ContractBonusType::UnusedSubstitutionFee)
                            && bonus.value > 0
                        {
                            payout += bonus.value as i64;
                        }
                    }
                    if payout > 0 {
                        *club_totals.entry(parent_club_id).or_insert(0) += payout;
                    }
                }
                if let Some(loan_contract) = player.contract_loan.as_ref() {
                    let mut payout: i64 = 0;
                    for bonus in &loan_contract.bonuses {
                        if matches!(bonus.bonus_type, ContractBonusType::UnusedSubstitutionFee)
                            && bonus.value > 0
                        {
                            payout += bonus.value as i64;
                        }
                    }
                    if payout > 0 {
                        *club_totals.entry(borrower_club_id).or_insert(0) += payout;
                    }
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
    /// `club::team::talks::team_talks::apply_team_talk`.
    fn apply_full_time_team_talks<D: LeagueProcessAccess>(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut D,
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
                .indexes()
                .and_then(|i| i.get_player_location(*pids.first().unwrap()))
                .map(|(_, _, c, _)| c)
                .unwrap_or(0);
            sides.push(SideTalk {
                club_id,
                player_ids: pids,
                delta,
            });
        }

        let now = data.date().date();
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
                    apply_team_talk_dated(
                        single.iter_mut(),
                        manager_clone.as_ref(),
                        tone,
                        ctx,
                        Some(now),
                    );
                }
            }
        }
    }

    /// Post-match relationship updates. The match itself is the most
    /// emotionally loaded moment in a player's week — a clean sheet, a
    /// heavy defeat, a red card all leave traces in the dressing room.
    /// We update underlying relations and emit at most two visible
    /// happiness events per player per match, so the player history
    /// surfaces meaningful incidents without overwhelming readers.
    fn apply_match_relationship_updates<D: LeagueProcessAccess>(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut D,
        now: NaiveDate,
        home_team_id: u32,
        best_player_id: Option<u32>,
    ) {
        use crate::ConflictLocation;
        use crate::club::ChangeType;
        use crate::club::HappinessEventCause;
        use crate::club::HappinessEventChangeKind;
        use crate::club::HappinessEventContext;
        use crate::club::HappinessEventEvidence;
        use crate::club::HappinessEventFollowUp;
        use crate::club::HappinessEventScope;
        use crate::club::HappinessEventSeverity;
        use crate::club::HappinessEventType;
        use crate::club::RelationshipChange;
        use crate::club::TeammateConflictContext;
        use crate::club::TeammateConflictReason;
        use crate::r#match::player::statistics::MatchStatisticType;

        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();

        for side in [&details.left_team_players, &details.right_team_players] {
            let (scored, conceded) = if side.team_id == home_team_id {
                (home_goals, away_goals)
            } else {
                (away_goals, home_goals)
            };
            let team_won = scored > conceded;
            let team_lost = scored < conceded;
            let heavy_defeat = team_lost && (conceded - scored) >= 4;

            // Roster snapshot — we need ids + position groups + a couple of
            // personality bits. Walk once.
            struct SidePlayer {
                id: u32,
                group: PlayerFieldPositionGroup,
                position: PlayerPositionType,
                temperament: f32,
                controversy: f32,
                professionalism: f32,
                sportsmanship: f32,
                leadership: f32,
                is_captain: bool,
            }

            let appeared: Vec<u32> = side
                .main
                .iter()
                .copied()
                .chain(side.substitutes_used.iter().copied())
                .collect();

            // Matchday armband — the captain who actually started, not the
            // persistent club captain who may have been rotated out. Honour
            // the club hierarchy where it overlaps the kickoff XI
            // (`side.main`), otherwise the best on-pitch leader. This keeps the
            // leadership events below attached to a player who appeared, and
            // never fires a captain event for a benched club captain.
            let (persistent_captain, persistent_vice) = data
                .team(side.team_id)
                .map(|t| (t.captain_id, t.vice_captain_id))
                .unwrap_or((None, None));
            let xi_candidates: Vec<LeadershipCandidate> = side
                .main
                .iter()
                .filter_map(|id| data.player(*id))
                .map(LeadershipCandidate::from_player)
                .collect();
            let team_captain_id =
                MatchdayLeadership::resolve(persistent_captain, persistent_vice, &xi_candidates)
                    .captain_id;

            let mut players: Vec<SidePlayer> = Vec::new();
            for pid in &appeared {
                if let Some(p) = data.player(*pid) {
                    players.push(SidePlayer {
                        id: *pid,
                        group: p.position().position_group(),
                        position: p.position(),
                        temperament: p.attributes.temperament,
                        controversy: p.attributes.controversy,
                        professionalism: p.attributes.professionalism,
                        sportsmanship: p.attributes.sportsmanship,
                        leadership: p.skills.mental.leadership,
                        is_captain: Some(*pid) == team_captain_id,
                    });
                }
            }

            // Goal scorers and red-card recipients — built from the match
            // detail stream. We don't have an assister↔scorer mapping in
            // the current stat model, so the assist↔scorer cooperation
            // bonus is omitted here (kept for a future model upgrade).
            let mut scorer_goals: HashMap<u32, u8> = HashMap::new();
            let mut red_carded: Vec<u32> = Vec::new();
            for d in &result.score.details {
                if !appeared.contains(&d.player_id) {
                    continue;
                }
                match d.stat_type {
                    MatchStatisticType::Goal => {
                        *scorer_goals.entry(d.player_id).or_insert(0) += 1;
                    }
                    MatchStatisticType::RedCard => {
                        red_carded.push(d.player_id);
                    }
                    _ => {}
                }
            }

            // Track per-player visible event count so we cap at 2.
            let mut event_budget: HashMap<u32, u8> = HashMap::new();

            // Pending updates — applied after the read pass.
            struct Update {
                from: u32,
                to: u32,
                change_type: ChangeType,
                magnitude: f32,
                event: Option<VisiblePairEvent>,
            }

            #[derive(Clone)]
            struct VisiblePairEvent {
                kind: HappinessEventType,
                magnitude: f32,
                evidence: Vec<HappinessEventEvidence>,
                cause: HappinessEventCause,
                reason: TeammateConflictReason,
                location: ConflictLocation,
            }
            let mut updates: Vec<Update> = Vec::new();

            // ── Clean sheet bonds: GK ↔ CBs, CB ↔ CB ────────────────
            if conceded == 0 {
                let gk_ids: Vec<u32> = players
                    .iter()
                    .filter(|p| p.group == PlayerFieldPositionGroup::Goalkeeper)
                    .map(|p| p.id)
                    .collect();
                let cb_ids: Vec<u32> = players
                    .iter()
                    .filter(|p| {
                        matches!(
                            p.position,
                            PlayerPositionType::DefenderCenter
                                | PlayerPositionType::DefenderCenterLeft
                                | PlayerPositionType::DefenderCenterRight
                        )
                    })
                    .map(|p| p.id)
                    .collect();
                for &gk in &gk_ids {
                    for &cb in &cb_ids {
                        updates.push(Update {
                            from: gk,
                            to: cb,
                            change_type: ChangeType::MatchCooperation,
                            magnitude: 0.12,
                            event: None,
                        });
                        updates.push(Update {
                            from: cb,
                            to: gk,
                            change_type: ChangeType::MatchCooperation,
                            magnitude: 0.12,
                            event: None,
                        });
                    }
                }
                for i in 0..cb_ids.len() {
                    for j in (i + 1)..cb_ids.len() {
                        updates.push(Update {
                            from: cb_ids[i],
                            to: cb_ids[j],
                            change_type: ChangeType::MatchCooperation,
                            magnitude: 0.15,
                            event: None,
                        });
                        updates.push(Update {
                            from: cb_ids[j],
                            to: cb_ids[i],
                            change_type: ChangeType::MatchCooperation,
                            magnitude: 0.15,
                            event: None,
                        });
                    }
                }
            }

            // ── Heavy defeat: captain steps in or implodes ──────────
            if heavy_defeat {
                let captain = players.iter().find(|p| p.is_captain);
                if let Some(cap) = captain {
                    if cap.professionalism < 8.0 {
                        // Captain takes it out on dressing room.
                        for p in players.iter().filter(|p| p.id != cap.id) {
                            updates.push(Update {
                                from: cap.id,
                                to: p.id,
                                change_type: ChangeType::PersonalConflict,
                                magnitude: 0.15,
                                event: None,
                            });
                        }
                    } else {
                        // Captain consoles the soft-temperament players.
                        for p in players
                            .iter()
                            .filter(|p| p.id != cap.id && p.temperament <= 8.0)
                        {
                            updates.push(Update {
                                from: p.id,
                                to: cap.id,
                                change_type: ChangeType::PersonalSupport,
                                magnitude: 0.10,
                                event: None,
                            });
                        }
                    }
                }
            }

            // ── Red card: friction within the same unit ─────────────
            //
            // Simulation truth: every same-unit teammate accumulates a
            // small `PersonalConflict` update against the offender — the
            // back-four had to play with ten, the midfield got
            // overloaded, etc. Visible player-history rows are scarcer:
            // we pick the 1-2 teammates whose personality / standing
            // would realistically make them speak up, and classify the
            // event so each reactor's row reads as a specific football
            // moment (captain's leadership challenge vs. a pro's
            // standards reaction vs. plain tactical blame) rather than
            // a copy-paste "argued with teammate" line.
            for &offender in &red_carded {
                let group = match players.iter().find(|p| p.id == offender) {
                    Some(p) => p.group,
                    None => continue,
                };

                #[derive(Clone)]
                struct ReactorProfile {
                    score: f32,
                    evidence: Vec<HappinessEventEvidence>,
                    cause: HappinessEventCause,
                    reason: TeammateConflictReason,
                }

                let mut visible_reactors: Vec<(u32, ReactorProfile)> = players
                    .iter()
                    .filter(|p| p.id != offender && p.group == group)
                    .map(|p| {
                        let rel = data
                            .player(p.id)
                            .and_then(|player| player.relations.get_player(offender));
                        let relation_level = rel.map(|r| r.level).unwrap_or(0.0);

                        let mut score = 0.0;
                        let mut evidence = vec![HappinessEventEvidence::DressingRoomRow];

                        let is_strained = relation_level <= -25.0;
                        let is_leader = p.is_captain || p.leadership >= 15.0;
                        let is_high_pro = p.professionalism >= 14.0;
                        let is_high_sport = p.sportsmanship >= 14.0;
                        let is_high_contro = p.controversy >= 14.0;
                        let is_low_temper = p.temperament <= 8.0;

                        if is_strained {
                            score += 2.0;
                            evidence.push(HappinessEventEvidence::AlreadyStrainedRelationship);
                        }
                        if is_leader {
                            score += 1.8;
                            evidence.push(HappinessEventEvidence::CaptainOrLeaderInfluence);
                        }
                        if is_high_pro {
                            score += 1.1;
                            evidence.push(HappinessEventEvidence::HighProfessionalism);
                        }
                        if is_high_sport {
                            score += 0.8;
                        }
                        if is_high_contro {
                            score += (p.controversy - 13.0) * 0.25;
                            evidence.push(HappinessEventEvidence::HighControversy);
                        }
                        if is_low_temper {
                            score += (9.0 - p.temperament) * 0.25;
                            evidence.push(HappinessEventEvidence::LowTemperament);
                        }

                        // Pick the football-realistic frame for this
                        // reactor's row. Order matters: a captain's
                        // public callout outranks a pro's standards
                        // reaction, which outranks pre-existing friction,
                        // which outranks the generic "we played with
                        // ten" tactical complaint. Each variant maps a
                        // closed (cause, reason) pair so renderers pick
                        // distinct copy.
                        let (cause, reason) = if is_leader {
                            (
                                HappinessEventCause::LeadershipDispute,
                                TeammateConflictReason::LeadershipChallenge,
                            )
                        } else if is_high_pro || is_high_sport {
                            (
                                HappinessEventCause::TrainingFriction,
                                TeammateConflictReason::TrainingStandards,
                            )
                        } else if is_strained {
                            (
                                HappinessEventCause::PersonalityClash,
                                TeammateConflictReason::PersonalityClash,
                            )
                        } else {
                            (
                                HappinessEventCause::TacticalDisagreement,
                                TeammateConflictReason::TacticalBlame,
                            )
                        };

                        (
                            p.id,
                            ReactorProfile {
                                score,
                                evidence,
                                cause,
                                reason,
                            },
                        )
                    })
                    .collect();

                // Rank by score desc, threshold by 1.5, cap at 2. A
                // vanilla red card with vanilla teammates produces zero
                // visible rows — the offender takes the (separate) card
                // event, the dressing room simmers in the simulation
                // layer, and the feed stays quiet.
                visible_reactors
                    .sort_by(|a, b| b.1.score.partial_cmp(&a.1.score).unwrap_or(Ordering::Equal));
                let visible_reactors: HashMap<u32, ReactorProfile> = visible_reactors
                    .into_iter()
                    .filter(|(_, prof)| prof.score >= 1.5)
                    .take(2)
                    .collect();

                for p in players
                    .iter()
                    .filter(|p| p.id != offender && p.group == group)
                {
                    updates.push(Update {
                        from: p.id,
                        to: offender,
                        change_type: ChangeType::PersonalConflict,
                        magnitude: 0.20,
                        event: visible_reactors.get(&p.id).map(|prof| VisiblePairEvent {
                            kind: HappinessEventType::ConflictWithTeammate,
                            magnitude: -1.5,
                            evidence: prof.evidence.clone(),
                            cause: prof.cause,
                            reason: prof.reason,
                            location: ConflictLocation::DressingRoom,
                        }),
                    });
                }
            }

            // ── Player of the match: admiration vs envy ────────────
            if let Some(motm) = best_player_id {
                if appeared.contains(&motm) {
                    let motm_controversy = players
                        .iter()
                        .find(|p| p.id == motm)
                        .map(|p| p.controversy)
                        .unwrap_or(10.0);
                    for p in players.iter().filter(|p| p.id != motm) {
                        let same_group = p.group
                            == players
                                .iter()
                                .find(|q| q.id == motm)
                                .map(|q| q.group)
                                .unwrap_or(p.group);
                        if p.controversy <= 11.0 {
                            updates.push(Update {
                                from: p.id,
                                to: motm,
                                change_type: ChangeType::ReputationAdmiration,
                                magnitude: 0.10,
                                event: None,
                            });
                        } else if p.controversy >= 14.0 && motm_controversy >= 11.0 && same_group {
                            updates.push(Update {
                                from: p.id,
                                to: motm,
                                change_type: ChangeType::ReputationTension,
                                magnitude: 0.10,
                                event: None,
                            });
                        }
                    }
                }
            }

            // ── Goal scorer admiration (single-direction proxy) ─────
            // Without an assist↔scorer pairing in the stat stream we
            // approximate the cooperation lift with a generic admiration
            // signal from teammates toward each scorer. Caps at 2 goals
            // per scorer to avoid runaway updates.
            for (scorer, goals) in &scorer_goals {
                let n = (*goals).min(2) as f32;
                for p in players.iter().filter(|p| p.id != *scorer) {
                    updates.push(Update {
                        from: p.id,
                        to: *scorer,
                        change_type: ChangeType::ReputationAdmiration,
                        magnitude: 0.08 * n,
                        event: None,
                    });
                }
            }

            // ── Apply updates with cooldown / event budget ──────────
            // Sort by `from` so the player borrow happens once per
            // unique source. Each match accumulates ~30–50 updates
            // spread across ~11 players; without grouping we'd take
            // ~30–50 mutable borrows. With grouping we take ~11.
            updates.sort_by_key(|u| u.from);
            let mut i = 0;
            while i < updates.len() {
                let from = updates[i].from;
                let block_end = updates[i..]
                    .iter()
                    .position(|u| u.from != from)
                    .map(|p| i + p)
                    .unwrap_or(updates.len());

                if let Some(player) = data.player_mut(from) {
                    for upd in &updates[i..block_end] {
                        let relation_before = upd.event.as_ref().map(|_| {
                            player
                                .relations
                                .get_player(upd.to)
                                .map(|r| (r.level, r.trust, r.friendship, r.professional_respect))
                                .unwrap_or((0.0, 50.0, 30.0, 50.0))
                        });
                        let change_type = upd.change_type.clone();
                        let signed = match change_type {
                            ChangeType::PersonalConflict
                            | ChangeType::TrainingFriction
                            | ChangeType::CompetitionRivalry
                            | ChangeType::ReputationTension => -upd.magnitude.abs(),
                            _ => upd.magnitude.abs(),
                        };
                        let change = if signed >= 0.0 {
                            RelationshipChange::positive(change_type, signed.abs())
                        } else {
                            RelationshipChange::negative(change_type, signed.abs())
                        };
                        player
                            .relations
                            .update_player_relationship(upd.to, change, now);
                        if let Some(event) = upd.event.clone() {
                            let slot = event_budget.entry(upd.from).or_insert(0);
                            if *slot < 2 {
                                let (level_before, trust, friendship, professional_respect) =
                                    relation_before.unwrap_or((0.0, 50.0, 30.0, 50.0));
                                let level_after = player
                                    .relations
                                    .get_player(upd.to)
                                    .map(|r| r.level)
                                    .unwrap_or(level_before);

                                let mut evidence = event.evidence.clone();
                                if level_before <= -25.0
                                    && !evidence.contains(
                                        &HappinessEventEvidence::AlreadyStrainedRelationship,
                                    )
                                {
                                    evidence
                                        .push(HappinessEventEvidence::AlreadyStrainedRelationship);
                                } else if level_before.abs() < 25.0
                                    && !evidence.contains(&HappinessEventEvidence::WeakExistingBond)
                                {
                                    evidence.push(HappinessEventEvidence::WeakExistingBond);
                                }
                                if trust <= 35.0
                                    && !evidence.contains(&HappinessEventEvidence::LowTrust)
                                {
                                    evidence.push(HappinessEventEvidence::LowTrust);
                                }
                                if friendship <= 25.0
                                    && !evidence.contains(&HappinessEventEvidence::LowFriendship)
                                {
                                    evidence.push(HappinessEventEvidence::LowFriendship);
                                }
                                if professional_respect <= 35.0
                                    && !evidence
                                        .contains(&HappinessEventEvidence::LowProfessionalRespect)
                                {
                                    evidence.push(HappinessEventEvidence::LowProfessionalRespect);
                                }
                                if player.happiness.has_recent_event_with_partner(
                                    &event.kind,
                                    upd.to,
                                    90,
                                ) && !evidence
                                    .contains(&HappinessEventEvidence::RepeatedIncident)
                                {
                                    evidence.push(HappinessEventEvidence::RepeatedIncident);
                                }

                                let follow_up = if level_before <= -25.0
                                    || evidence.contains(&HappinessEventEvidence::RepeatedIncident)
                                {
                                    HappinessEventFollowUp::DressingRoomDamageRisk
                                } else {
                                    HappinessEventFollowUp::LikelyToSettle
                                };
                                let context = HappinessEventContext::new(
                                    event.cause,
                                    HappinessEventSeverity::from_magnitude(event.magnitude),
                                    HappinessEventScope::DressingRoom,
                                )
                                .with_relationship_levels(level_before, level_after)
                                .with_relationship_axes(trust, friendship, professional_respect)
                                .with_change_kind(HappinessEventChangeKind::from_change_type(
                                    &upd.change_type,
                                ))
                                .with_evidence_iter(evidence)
                                .with_follow_up(follow_up)
                                .with_teammate_conflict_context(TeammateConflictContext::new(
                                    event.reason,
                                    event.location,
                                ));

                                if player
                                    .happiness
                                    .add_event_with_partner_context_and_cooldown(
                                        event.kind,
                                        event.magnitude,
                                        upd.to,
                                        context,
                                        45,
                                    )
                                {
                                    *slot += 1;
                                }
                            }
                        }
                    }
                }
                i = block_end;
            }

            // Win/loss generic team-mate cooperation lift — softer signal
            // shared by every player on the winning side. Friction lift
            // skipped on losses; the captain block above captured the
            // emotional payload for heavy defeats. A team that just wins
            // narrowly doesn't accumulate dressing-room damage.
            //
            // One mutable borrow per player; the inner loop fans out the
            // pairing updates so a winning XI does ~11 lookups instead
            // of ~110.
            if team_won {
                let n = players.len();
                for i in 0..n {
                    if let Some(player) = data.player_mut(players[i].id) {
                        for j in 0..n {
                            if i == j {
                                continue;
                            }
                            player.relations.update_with_type(
                                players[j].id,
                                0.05,
                                ChangeType::MatchCooperation,
                                now,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Apply card / suspension consequences from a finished competitive
    /// match. Walks every featuring player and updates their per-player
    /// suspension counter; then serves one match against any player on
    /// either team who carried a ban into this fixture and did not
    /// appear (i.e. they sat out — the ban ticks down). Friendlies are
    /// excluded by the caller — friendly cards don't ban a player from
    /// the next competitive match in this model.
    fn apply_post_match_discipline<D: LeagueProcessAccess>(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut D,
    ) {
        // Pull the league's accumulation threshold up-front so we don't
        // hold a borrow on `data` while mutating players. Continental
        // and friendly leagues fall back to the standard FIFA rule.
        let yellow_threshold = data
            .league(result.league_id)
            .map(|l| l.regulations.yellow_card_ban_threshold)
            .unwrap_or(YELLOW_CARD_BAN_THRESHOLD);

        // 1) Process cards for every player who has stats this match.
        let card_entries: Vec<(u32, u8, u8)> = details
            .player_stats
            .iter()
            .filter(|(_, s)| s.yellow_cards > 0 || s.red_cards > 0)
            .map(|(pid, s)| (*pid, s.yellow_cards as u8, s.red_cards as u8))
            .collect();

        let mut new_suspensions: Vec<u32> = Vec::new();
        for (pid, yellows, reds) in card_entries {
            if let Some(player) = data.player_mut(pid) {
                let added = player.on_match_disciplinary_result(yellows, reds, yellow_threshold);
                if added > 0 {
                    new_suspensions.push(pid);
                }
            }
        }

        // 2) Decrement existing bans for absent players. "Absent" means
        // the player is on either team but does NOT appear in the
        // FieldSquad of their side this match. The card pass above
        // sets `is_banned = true` *for this match* if a red happened —
        // those players are not absent (they were on the field), so we
        // exclude them via `new_suspensions` to avoid immediately
        // serving the suspension they just earned.
        let teams = [
            (
                details.left_team_players.team_id,
                &details.left_team_players,
            ),
            (
                details.right_team_players.team_id,
                &details.right_team_players,
            ),
        ];

        // Collect banned-player ids per team without holding a borrow
        // across the mutation pass.
        let mut absent_banned: Vec<u32> = Vec::new();
        for (team_id, side) in teams {
            let Some(team) = data.team(team_id) else {
                continue;
            };
            for player in &team.players.players {
                if !player.player_attributes.is_banned {
                    continue;
                }
                if new_suspensions.contains(&player.id) {
                    continue;
                }
                if side.main.contains(&player.id)
                    || side.substitutes.contains(&player.id)
                    || side.substitutes_used.contains(&player.id)
                {
                    continue;
                }
                absent_banned.push(player.id);
            }
        }

        for pid in absent_banned {
            if let Some(player) = data.player_mut(pid) {
                player.serve_suspension_match();
            }
            // Mirror on the league-side analytics record.
            if let Some(league) = data.league_mut(result.league_id) {
                let _ = league.regulations.record_suspension_served(pid);
            }
        }
    }

    fn process_loan_match_fees<D: LeagueProcessAccess>(details: &MatchResultRaw, data: &mut D) {
        // Collect fee transfers: (parent_club_id, borrowing_club_id, fee)
        let mut fee_transfers: Vec<(u32, u32, u32)> = Vec::new();

        let all_players = details
            .left_team_players
            .main
            .iter()
            .chain(details.left_team_players.substitutes_used.iter())
            .chain(details.right_team_players.main.iter())
            .chain(details.right_team_players.substitutes_used.iter());

        for &player_id in all_players {
            if let Some(player) = data.player(player_id) {
                if let Some(ref loan) = player.contract_loan {
                    if let (Some(fee), Some(parent_id), Some(borrowing_id)) = (
                        loan.loan_match_fee,
                        loan.loan_from_club_id,
                        loan.loan_to_club_id,
                    ) {
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

fn compute_effective_ratings<D: LeagueProcessAccess>(
    details: &MatchResultRaw,
    data: &D,
    now: NaiveDate,
) -> HashMap<u32, f32> {
    let mut out = HashMap::with_capacity(details.player_stats.len());
    for (player_id, stats) in &details.player_stats {
        let location = data
            .indexes()
            .and_then(|i| i.get_player_location(*player_id));
        let country_code = location
            .and_then(|(_, country_id, _, _)| data.country_info().get(&country_id))
            .map(|ci| ci.code.clone())
            .unwrap_or_default();
        let club_rep = location
            .and_then(|(_, _, _, team_id)| data.team(team_id))
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.0);
        let mult = data
            .player(*player_id)
            .map(|p| p.settlement_form_multiplier(now, &country_code, club_rep))
            .unwrap_or(1.0);

        // Personality shape of the rating — tuned so that average players
        // fall in [stats.match_rating ± 0.5] with consistency/temperament
        // modulating the variance. A `consistency=18` player barely moves
        // off baseline; `consistency=4` swings wildly. `important_matches`
        // lifts/drops the rating in big fixtures (league = baseline = no
        // effect; we'd need match-importance context here to use it fully,
        // so for now it contributes a small always-on bonus scaled by the
        // opponent's reputation relative to ours — a real "big-game"
        // player.)
        let (consistency, big_match, temperament, chemistry) = data
            .player(*player_id)
            .map(|p| {
                (
                    p.attributes.consistency,
                    p.attributes.important_matches,
                    p.attributes.temperament,
                    p.relations.get_team_chemistry(),
                )
            })
            .unwrap_or((10.0, 10.0, 10.0, 50.0));

        // Anchor settlement form around the neutral baseline (6.0)
        // and only dampen the upside. Multiplying the absolute rating
        // compressed the entire 1..10 band toward 0, so a settling
        // keeper who posted a baseline "did-nothing" 6.0 was scaled to
        // 5.1 purely from the multiplier — and a clean-sheet 7.0
        // landed at 5.95. The multiplier represents reduced peak
        // effectiveness, so above-baseline shifts get the deviation
        // dampened; below-baseline shifts pass through (the bad-day
        // signal already reflects how the player performed and
        // shouldn't be artificially softened by their settling
        // status either).
        const BASELINE: f32 = 6.0;
        let mut adjusted = if stats.match_rating > BASELINE {
            BASELINE + (stats.match_rating - BASELINE) * mult
        } else {
            stats.match_rating
        };

        // Team chemistry shifts individual performance. Neutral at 50;
        // ±2.5% of baseline rating at the extremes. Not huge — the lion's
        // share of a performance is on the player — but a dysfunctional
        // dressing room measurably drags everyone down and a tight squad
        // gets a small lift.
        let chem_shift = ((chemistry - 50.0) / 50.0).clamp(-1.0, 1.0) * 0.15;
        adjusted += chem_shift;

        // Consistency drives match-to-match volatility. A high-consistency
        // player drifts LESS from their stat-derived rating; a low-
        // consistency ("streaky") player swings widely match-to-match —
        // one excellent game, three quiet ones. Real football: a Finishing-7
        // / Consistency-8 striker has the *occasional* hot match but
        // can't string two 8.0s in a row.
        //
        // Previous implementation keyed the seed only on `player_id`, so
        // a streaky player got the SAME swing every single game (a static
        // bias compounding in one direction across the season). Combining
        // with the match date produces a different swing per fixture —
        // genuine volatility that averages toward zero over many apps.
        //
        // Band widened to ±0.6 max because the old ±0.4 wasn't enough to
        // produce the "scored 2 in match A, anonymous in match B" shape
        // that's the hallmark of low-consistency players.
        let variance_band = (1.0 - (consistency / 20.0)).clamp(0.0, 1.0) * 0.6;
        if variance_band > 0.01 {
            // Per-match deterministic seed: player_id + date so the same
            // player gets a different swing every fixture, but a given
            // (player, date) pair is reproducible for tests.
            let date_seed = now.num_days_from_ce() as f32;
            let seed = ((*player_id as f32 * 0.618033) + (date_seed * 0.381966)).fract();
            let swing = (seed - 0.5) * 2.0 * variance_band;
            adjusted += swing;
        }

        // Low-temperament players drop a touch when the game slipped away
        // (stat rating below 6 already) — they let it affect them more.
        if stats.match_rating < 6.0 && temperament < 10.0 {
            let drop = ((10.0 - temperament) / 10.0) * 0.25;
            adjusted -= drop;
        }

        // Big-match personality: small baseline lift in cup fixtures for
        // high `important_matches`. The caller passes these ratings into
        // the MatchOutcome that already knows is_cup — but we can't see
        // that here, so the effect is modest and always-on as a proxy.
        if big_match >= 15.0 {
            adjusted += 0.15;
        } else if big_match <= 5.0 {
            adjusted -= 0.1;
        }

        out.insert(*player_id, adjusted.clamp(1.0, 10.0));
    }
    out
}

fn pick_player_of_the_match(
    details: &MatchResultRaw,
    effective_ratings: &HashMap<u32, f32>,
) -> Option<u32> {
    let mut best_rating = 0.0_f32;
    let mut best = None;
    for (player_id, stats) in &details.player_stats {
        let r = *effective_ratings
            .get(player_id)
            .unwrap_or(&stats.match_rating);
        if r > best_rating {
            best_rating = r;
            best = Some(*player_id);
        }
    }
    best
}

fn reputation_weights<D: LeagueProcessAccess>(
    result: &MatchResult,
    is_cup: bool,
    data: &D,
) -> (f32, f32) {
    if result.league_id == CHAMPIONS_LEAGUE_ID {
        (1.5, 1.2)
    } else if result.league_id == COPA_LIBERTADORES_ID {
        (1.45, 1.0)
    } else if result.league_id == EUROPA_LEAGUE_ID {
        (1.3, 0.8)
    } else if result.league_id == CONFERENCE_LEAGUE_ID {
        (1.1, 0.5)
    } else if is_cup {
        (1.0, 0.3)
    } else {
        let league_reputation = data
            .league(result.league_id)
            .map(|l| l.reputation)
            .unwrap_or(500) as f32;
        let w = (league_reputation / 1000.0 + 0.5).clamp(0.5, 1.5);
        (w, 0.2)
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch_match_outcomes<D: LeagueProcessAccess>(
    side: &FieldSquad,
    team_scored: u8,
    team_conceded: u8,
    details: &MatchResultRaw,
    data: &mut D,
    effective_ratings: &HashMap<u32, f32>,
    best_player_id: Option<u32>,
    is_friendly: bool,
    is_cup: bool,
    competition_slug: &str,
    league_weight: f32,
    world_weight: f32,
    is_derby: bool,
    team_won: bool,
    team_lost: bool,
) {
    let starter_ids: Vec<u32> = side.main.iter().copied().collect();
    let sub_ids: Vec<u32> = side.substitutes_used.iter().copied().collect();
    let all_ids: Vec<(u32, MatchParticipation)> = starter_ids
        .iter()
        .map(|id| (*id, MatchParticipation::Starter))
        .chain(
            sub_ids
                .iter()
                .map(|id| (*id, MatchParticipation::Substitute)),
        )
        .collect();

    for (pid, participation) in all_ids {
        let stats = match details.player_stats.get(&pid) {
            Some(s) => s,
            None => continue,
        };
        let effective = *effective_ratings.get(&pid).unwrap_or(&stats.match_rating);
        let is_motm = best_player_id == Some(pid);
        if let Some(player) = data.player_mut(pid) {
            player.on_match_played(&MatchOutcome {
                stats,
                effective_rating: effective,
                participation,
                is_friendly,
                is_cup,
                competition_slug,
                is_motm,
                team_goals_for: team_scored,
                team_goals_against: team_conceded,
                league_weight,
                world_weight,
                is_derby,
                team_won,
                team_lost,
            });
        }
    }

    // Unused substitutes feel the snub as well, even though they didn't
    // feature in player_stats. Prefer the structured drop event when
    // the squad selector attached a `MatchSelectionContext` for this
    // player so the events feed reads "Lost out to Marco Silva..."
    // instead of "Dropped from match squad".
    for &pid in &side.substitutes {
        if side.substitutes_used.contains(&pid) {
            continue;
        }
        let ctx = side
            .selection_omissions
            .iter()
            .find(|o| o.player_id == pid)
            .map(|o| o.context.clone());
        if let Some(player) = data.player_mut(pid) {
            match ctx {
                Some(c) => player.on_match_dropped_with_context(c),
                None => player.on_match_dropped(),
            }
        }
    }

    // Players left out of the matchday squad entirely also feel the
    // snub. They never reached `side.main` / `side.substitutes`, so
    // the squad selector flagged them upstream — fire the contextual
    // drop event here.
    for omitted in &side.selection_omissions {
        if side.main.contains(&omitted.player_id) || side.substitutes.contains(&omitted.player_id) {
            continue;
        }
        if let Some(player) = data.player_mut(omitted.player_id) {
            player.on_match_dropped_with_context(omitted.context.clone());
        }
    }
}

/// Compact the overall reputation score (0.0–1.0) into the u16 scale the
/// `TeamReputation` drift model expects (same 0–10_000 basis as `home`/
/// `national`/`world`).
fn overall_score_to_u16(score: f32) -> u16 {
    (score * 10_000.0).clamp(0.0, 10_000.0) as u16
}

/// Look up league-table positions for both teams and the total number of
/// teams. Cup and continental competitions fall back to 1/1/1 — the
/// position factor is meaningless there but still needs a valid denominator.
fn league_standings<D: LeagueProcessAccess>(
    league_id: u32,
    home_team_id: u32,
    away_team_id: u32,
    data: &D,
) -> (u8, u8, u8) {
    let league = match data.league(league_id) {
        Some(l) => l,
        None => return (1, 1, 1),
    };
    let rows = &league.table.rows;
    if rows.is_empty() {
        return (1, 1, 1);
    }
    let total = rows.len().min(u8::MAX as usize) as u8;
    let position = |team_id: u32| -> u8 {
        let pts = rows
            .iter()
            .find(|r| r.team_id == team_id)
            .map(|r| r.points)
            .unwrap_or(0);
        let ahead = rows.iter().filter(|r| r.points > pts).count();
        (ahead + 1).min(u8::MAX as usize) as u8
    };
    (position(home_team_id), position(away_team_id), total)
}
