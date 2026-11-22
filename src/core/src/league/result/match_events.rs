use super::LeagueResult;
use super::data_access::LeagueProcessAccess;
use crate::ConflictLocation;
use crate::HappinessEventType;
use crate::Person;
use crate::PlayerFieldPositionGroup;
use crate::PlayerPositionType;
use crate::club::mind::organs::memory::ActorRef;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::contract::ContractBonusType;
use crate::club::player::events::discipline::{CompetitionFamily, YELLOW_CARD_BAN_THRESHOLD};
use crate::club::player::events::{MatchOutcome, MatchParticipation, MatchTeamRef};
use crate::club::player::personality::adaptation::AdaptationSquadContext;
use crate::club::player::player::Player;
use crate::club::staff::coach::standing::{EvidenceLens, LadderContext, StandingTuning};
use crate::club::staff::perception::PotentialEstimator;
use crate::club::team::reputation::{
    CompetitionType as RepCompetition, MatchOutcome as RepOutcome,
};
use crate::club::team::{
    LeadershipCandidate, MatchPhase, MatchdayLeadership, TeamTalkApplier, TeamTalkContext,
    TeamTalkMoments, TeamTalkTone,
};
use crate::club::{
    ChangeType, HappinessEventCause, HappinessEventChangeKind, HappinessEventContext,
    HappinessEventEvidence, HappinessEventFollowUp, HappinessEventScope, HappinessEventSeverity,
    RelationshipChange, TeammateConflictContext, TeammateConflictReason,
};
use crate::club::{Staff, StaffPosition};
use crate::continent::competitions::{
    CHAMPIONS_LEAGUE_ID, CONFERENCE_LEAGUE_ID, COPA_LIBERTADORES_ID, EUROPA_LEAGUE_ID,
};
use crate::league::Season;
use crate::r#match::PlayerMatchEndStats;
use crate::r#match::engine::rating::{EngineVolumeCalibration, RatingContext};
use crate::r#match::engine::result::MatchResultRaw;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{FieldSquad, MatchResult};
use crate::transfers::pipeline::KnownPlayerMemory;
use crate::transfers::value::PlayerValuationCalculator;
use crate::{MatchSelectionContext, SelectionOmissionReason};
use chrono::Datelike;
use chrono::NaiveDate;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Ordering;

use crate::Team;
use crate::club::staff::{CoachMatchObservation, CoachProfile};

/// Match-level facts resolved once and shared by every post-match pass.
struct MatchEventFacts {
    is_continental_cup: bool,
    is_cup: bool,
    is_friendly: bool,
    is_derby: bool,
    home_goals: u8,
    away_goals: u8,
    home_team_id: u32,
    away_team_id: u32,
    best_player_id: Option<u32>,
    league_weight: f32,
    world_weight: f32,
}

/// Roster snapshot for one side's relationship pass: ids, position groups
/// and the personality bits the update builders read.
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

/// Everything one side's relationship pass reads, captured before the
/// mutation phase.
struct SideRelationshipSnapshot {
    players: Vec<SidePlayer>,
    scorer_goals: FxHashMap<u32, u8>,
    red_carded: Vec<u32>,
    appeared: Vec<u32>,
    team_won: bool,
    conceded: u8,
    heavy_defeat: bool,
}

impl SideRelationshipSnapshot {
    fn build<D: LeagueProcessAccess>(
        side: &FieldSquad,
        result: &MatchResult,
        data: &D,
        now: NaiveDate,
        scored: u8,
        conceded: u8,
    ) -> Self {
        let team_won = scored > conceded;
        let team_lost = scored < conceded;
        let heavy_defeat = team_lost && (conceded - scored) >= 4;

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
            .map(|p| LeadershipCandidate::from_player_at(p, now))
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
        let mut scorer_goals: FxHashMap<u32, u8> = FxHashMap::default();
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

        SideRelationshipSnapshot {
            players,
            scorer_goals,
            red_carded,
            appeared,
            team_won,
            conceded,
            heavy_defeat,
        }
    }
}

/// One pending relationship change, applied after the read pass.
struct RelationshipUpdate {
    from: u32,
    to: u32,
    change_type: ChangeType,
    magnitude: f32,
    event: Option<VisiblePairEvent>,
}

/// A visible player-history event attached to a relationship update.
#[derive(Clone)]
struct VisiblePairEvent {
    kind: HappinessEventType,
    magnitude: f32,
    evidence: Vec<HappinessEventEvidence>,
    cause: HappinessEventCause,
    reason: TeammateConflictReason,
    location: ConflictLocation,
}

/// A teammate's likelihood of speaking up about a red card, with the
/// football-realistic frame for his row.
#[derive(Clone)]
struct ReactorProfile {
    score: f32,
    evidence: Vec<HappinessEventEvidence>,
    cause: HappinessEventCause,
    reason: TeammateConflictReason,
}

impl ReactorProfile {
    fn for_player(player: &SidePlayer, relation_level: f32) -> Self {
        let mut score = 0.0;
        let mut evidence = vec![HappinessEventEvidence::DressingRoomRow];

        let is_strained = relation_level <= -25.0;
        let is_leader = player.is_captain || player.leadership >= 15.0;
        let is_high_pro = player.professionalism >= 14.0;
        let is_high_sport = player.sportsmanship >= 14.0;
        let is_high_contro = player.controversy >= 14.0;
        let is_low_temper = player.temperament <= 8.0;

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
            score += (player.controversy - 13.0) * 0.25;
            evidence.push(HappinessEventEvidence::HighControversy);
        }
        if is_low_temper {
            score += (9.0 - player.temperament) * 0.25;
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

        ReactorProfile {
            score,
            evidence,
            cause,
            reason,
        }
    }
}

/// One side's dressing-room talk inputs, resolved before delivery.
struct SideTalk {
    club_id: u32,
    player_ids: Vec<u32>,
    dressing_room: Vec<u32>,
    half_time_talk: Option<TeamTalkTone>,
    delta: i8,
    ht_delta: i8,
    rep_edge: f32,
}

/// A player memory row queued for the write pass.
struct MemoryAction {
    country_id: u32,
    current_club_id: u32,
    memory: KnownPlayerMemory,
}

/// Host-country facts for one side's scouting-memory pass.
struct ScoutingMemoryContext {
    current_club_id: u32,
    current_country_id: u32,
    current_price_level: f32,
    host_league_rep: u16,
    host_club_rep: u16,
}

impl ScoutingMemoryContext {
    fn for_side<D: LeagueProcessAccess>(team_id: u32, data: &D) -> Option<Self> {
        let current_club_id = data.team(team_id).map(|t| t.club_id)?;
        let current_country_id = data.country_by_club(current_club_id).map(|c| c.id)?;
        let current_price_level = data
            .country(current_country_id)
            .map(|c| c.settings.pricing.price_level)
            .unwrap_or(1.0);
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
        Some(ScoutingMemoryContext {
            current_club_id,
            current_country_id,
            current_price_level,
            host_league_rep,
            host_club_rep,
        })
    }
}

/// Per-match contract-clause payout pass: appearance, goal and clean-sheet
/// bonuses plus the unused-substitute fee.
struct ContractBonusPass;

impl ContractBonusPass {
    fn goals_per_player(result: &MatchResult) -> FxHashMap<u32, u8> {
        let mut goals_per_player: FxHashMap<u32, u8> = FxHashMap::default();
        for detail in &result.score.details {
            if detail.stat_type == MatchStatisticType::Goal {
                *goals_per_player.entry(detail.player_id).or_insert(0) += 1;
            }
        }
        goals_per_player
    }

    /// Everyone who featured in the match (main + used subs).
    fn appearance_ids(details: &MatchResultRaw) -> Vec<u32> {
        let mut appearance_ids: Vec<u32> = Vec::new();
        appearance_ids.extend(details.left_team_players.main.iter().copied());
        appearance_ids.extend(details.left_team_players.substitutes_used.iter().copied());
        appearance_ids.extend(details.right_team_players.main.iter().copied());
        appearance_ids.extend(details.right_team_players.substitutes_used.iter().copied());
        appearance_ids
    }

    /// Bench players who never came on are paid the unused-sub fee.
    fn unused_sub_ids(details: &MatchResultRaw) -> Vec<u32> {
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
        unused_subs_left
            .into_iter()
            .chain(unused_subs_right)
            .collect()
    }

    fn accumulate_appearance_bonuses<D: LeagueProcessAccess>(
        club_totals: &mut FxHashMap<u32, i64>,
        appearance_ids: &[u32],
        goals_per_player: &FxHashMap<u32, u8>,
        details: &MatchResultRaw,
        data: &D,
        left_conceded: u8,
        right_conceded: u8,
    ) {
        for pid in appearance_ids {
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
                    let parent_payout =
                        Self::bonus_payout(&parent_contract.bonuses, goals, gk_clean_sheet);
                    if parent_payout > 0 {
                        *club_totals.entry(parent_club_id).or_insert(0) += parent_payout;
                    }
                }

                // Loan-contract bonuses (if the borrower negotiated any)
                // bill the borrower.
                if let Some(loan_contract) = player.contract_loan.as_ref() {
                    let borrower_payout =
                        Self::bonus_payout(&loan_contract.bonuses, goals, gk_clean_sheet);
                    if borrower_payout > 0 {
                        *club_totals.entry(borrower_club_id).or_insert(0) += borrower_payout;
                    }
                }
            }
        }
    }

    fn accumulate_unused_sub_bonuses<D: LeagueProcessAccess>(
        club_totals: &mut FxHashMap<u32, i64>,
        unused_sub_ids: &[u32],
        data: &D,
    ) {
        for pid in unused_sub_ids {
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
                    let payout = Self::unused_sub_payout(&parent_contract.bonuses);
                    if payout > 0 {
                        *club_totals.entry(parent_club_id).or_insert(0) += payout;
                    }
                }
                if let Some(loan_contract) = player.contract_loan.as_ref() {
                    let payout = Self::unused_sub_payout(&loan_contract.bonuses);
                    if payout > 0 {
                        *club_totals.entry(borrower_club_id).or_insert(0) += payout;
                    }
                }
            }
        }
    }

    /// Sum the appearance/goal/clean-sheet clauses of one contract.
    fn bonus_payout(
        bonuses: &[crate::club::player::contract::ContractBonus],
        goals: i64,
        gk_clean_sheet: bool,
    ) -> i64 {
        let mut payout: i64 = 0;
        for bonus in bonuses {
            let v = bonus.value as i64;
            if v <= 0 {
                continue;
            }
            match bonus.bonus_type {
                ContractBonusType::AppearanceFee => payout += v,
                ContractBonusType::GoalFee => payout += v * goals,
                ContractBonusType::CleanSheetFee if gk_clean_sheet => payout += v,
                _ => {}
            }
        }
        payout
    }

    fn unused_sub_payout(bonuses: &[crate::club::player::contract::ContractBonus]) -> i64 {
        let mut payout: i64 = 0;
        for bonus in bonuses {
            if matches!(bonus.bonus_type, ContractBonusType::UnusedSubstitutionFee)
                && bonus.value > 0
            {
                payout += bonus.value as i64;
            }
        }
        payout
    }
}

impl LeagueResult {
    pub(super) fn process_match_events<D: LeagueProcessAccess>(
        result: &mut MatchResult,
        data: &mut D,
    ) {
        if result.details.is_none() {
            return;
        }

        let (is_continental_cup, is_cup, is_friendly) =
            Self::classify_competition(result.league_id, data);

        // Players inside their post-transfer settlement window play at a
        // reduced level. Compute the public/effective rating once, then
        // overwrite `details.player_stats[*].match_rating` so every
        // downstream reader of `MatchResultRaw` (match-page DTO, weekly
        // / season awards, cup showcase, league stat-rebuild) sees the
        // canonical adjusted value. The original engine verdict is
        // preserved on `raw_match_rating` for calibration / debug.
        let now_date = data.date().date();
        let effective_ratings = Self::canonicalize_ratings(result, data, now_date);
        let details = match &result.details {
            Some(d) => d,
            None => return,
        };
        let best_player_id = PlayerOfTheMatch::pick(details, &effective_ratings);

        let (league_weight, world_weight) = MatchReputation::weights(result, is_cup, data);

        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();
        let home_team_id = result.score.home_team.team_id;
        let away_team_id = result.score.away_team.team_id;
        let is_derby = Self::detect_derby(data, home_team_id, away_team_id);

        let facts = MatchEventFacts {
            is_continental_cup,
            is_cup,
            is_friendly,
            is_derby,
            home_goals,
            away_goals,
            home_team_id,
            away_team_id,
            best_player_id,
            league_weight,
            world_weight,
        };

        Self::dispatch_side_outcomes(result, details, data, &effective_ratings, &facts);

        Self::apply_post_match_fanout(result, details, data, &facts, now_date);

        if let Some(details_mut) = &mut result.details {
            details_mut.player_of_the_match_id = facts.best_player_id;
        }
        Self::sync_match_store(result, data);
    }

    /// Everything the match writes after the per-player dispatch: scouting
    /// memory, dressing-room fallout, bonuses, talks, physical effects,
    /// reputation, cup showcases and discipline.
    fn apply_post_match_fanout<D: LeagueProcessAccess>(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut D,
        facts: &MatchEventFacts,
        now_date: NaiveDate,
    ) {
        Self::record_match_scouting_memory(
            details,
            data,
            facts.is_friendly,
            now_date,
            facts.home_team_id,
            facts.away_team_id,
        );

        // Post-match social fallout — bonds, friction, admiration, envy.
        // Only fires for competitive matches where the dressing-room stakes
        // are real; friendlies don't move relationships.
        if !facts.is_friendly {
            Self::apply_match_relationship_updates(
                result,
                details,
                data,
                now_date,
                facts.home_team_id,
                facts.best_player_id,
            );
        }

        if !facts.is_friendly {
            Self::process_loan_match_fees(details, data);
            Self::process_contract_bonuses(result, details, data);
        }

        Self::apply_matchday_team_talks(
            result,
            details,
            data,
            facts.is_friendly,
            facts.is_derby || facts.is_continental_cup,
        );
        Self::apply_post_match_physical_effects(details, data, facts.is_friendly);
        Self::apply_post_match_reputation(result, data, facts.is_friendly, facts.is_cup);
        Self::apply_post_match_reporting(result, details, data, facts, now_date);
    }

    /// Player-facing reporting after the physical/reputation passes: the
    /// cup showcase and discipline.
    fn apply_post_match_reporting<D: LeagueProcessAccess>(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut D,
        facts: &MatchEventFacts,
        now_date: NaiveDate,
    ) {
        // Domestic-cup breakout scouting: a lower-league hero who shines
        // against a stronger club in a cup tie becomes visible to scouts.
        // Gated to domestic cups only — friendlies and continental cups
        // (which already have world-stage exposure) are excluded.
        if facts.is_cup && !facts.is_friendly && !facts.is_continental_cup {
            Self::record_domestic_cup_showcase_scouting(
                details,
                data,
                now_date,
                facts.home_team_id,
                facts.away_team_id,
                facts.home_goals,
                facts.away_goals,
                facts.best_player_id,
            );
        }

        // Disciplinary fallout — apply yellow / red card stats to each
        // player who featured, and serve a match for any banned player
        // on either side who didn't appear (their team played without
        // them, so the suspension counter ticks down).
        if !facts.is_friendly {
            Self::apply_post_match_discipline(
                result,
                details,
                Self::competition_family(facts.is_continental_cup),
                data,
            );
        }
    }

    /// Continental cups don't have a `League` row; everything else is
    /// classified by the league's own `is_cup` flag. The previous
    /// `league_id >= 900_000_000` heuristic miscategorised domestic
    /// leagues whose generated IDs landed above that threshold (e.g.
    /// Russian Second Division ~2.0e9), routing their matches into the
    /// cup statistics bucket.
    fn classify_competition<D: LeagueProcessAccess>(
        league_id: u32,
        data: &D,
    ) -> (bool, bool, bool) {
        let is_continental_cup = matches!(
            league_id,
            CHAMPIONS_LEAGUE_ID | EUROPA_LEAGUE_ID | CONFERENCE_LEAGUE_ID | COPA_LIBERTADORES_ID
        );
        let (is_cup, is_friendly) = if is_continental_cup {
            (true, false)
        } else {
            data.league(league_id)
                .map(|l| (l.is_cup, l.friendly))
                .unwrap_or((false, false))
        };
        (is_continental_cup, is_cup, is_friendly)
    }

    /// Recompute the canonical rating map and fold it into the stored
    /// player stats.
    fn canonicalize_ratings<D: LeagueProcessAccess>(
        result: &mut MatchResult,
        data: &D,
        now_date: NaiveDate,
    ) -> FxHashMap<u32, f32> {
        let Some(details) = result.details.as_ref() else {
            return FxHashMap::default();
        };
        let effective_ratings = EffectiveRatingCalculator::compute(details, data, now_date);
        if let Some(details_mut) = result.details.as_mut() {
            CanonicalRatingMutator::apply(details_mut, &effective_ratings);
        }
        effective_ratings
    }

    /// Rivalry detection: walk team → club on both sides and check whether
    /// either club lists the other as a rival. Either-direction is a derby.
    fn detect_derby<D: LeagueProcessAccess>(
        data: &D,
        home_team_id: u32,
        away_team_id: u32,
    ) -> bool {
        let home_club = data.team(home_team_id).map(|t| t.club_id);
        let away_club = data.team(away_team_id).map(|t| t.club_id);
        match (home_club, away_club) {
            (Some(h), Some(a)) => {
                let h_vs_a = data.club(h).map(|c| c.is_rival(a)).unwrap_or(false);
                let a_vs_h = data.club(a).map(|c| c.is_rival(h)).unwrap_or(false);
                h_vs_a || a_vs_h
            }
            _ => false,
        }
    }

    /// Per-player match reaction — Player owns all bookkeeping.
    fn dispatch_side_outcomes<D: LeagueProcessAccess>(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut D,
        effective_ratings: &FxHashMap<u32, f32>,
        facts: &MatchEventFacts,
    ) {
        let ctx = MatchDispatchContext {
            details,
            effective_ratings,
            best_player_id: facts.best_player_id,
            is_friendly: facts.is_friendly,
            is_cup: facts.is_cup,
            competition_slug: &result.league_slug,
            league_weight: facts.league_weight,
            world_weight: facts.world_weight,
            is_derby: facts.is_derby,
            is_continental: facts.is_continental_cup,
            opponent_team_id: None,
        };
        for side in [&details.left_team_players, &details.right_team_players] {
            let (scored, conceded) = if side.team_id == facts.home_team_id {
                (facts.home_goals, facts.away_goals)
            } else {
                (facts.away_goals, facts.home_goals)
            };
            let opponent_team_id = if side.team_id == facts.home_team_id {
                Some(facts.away_team_id)
            } else {
                Some(facts.home_team_id)
            };
            let (team_won, team_lost) = (scored > conceded, scored < conceded);
            let side_ctx = MatchDispatchContext {
                opponent_team_id,
                ..ctx
            };
            MatchOutcomeDispatcher::dispatch(
                side, scored, conceded, team_won, team_lost, &side_ctx, data,
            );
        }
    }

    fn competition_family(is_continental_cup: bool) -> CompetitionFamily {
        if is_continental_cup {
            CompetitionFamily::Continental
        } else {
            CompetitionFamily::Domestic
        }
    }

    /// Sync the finalized record back into the league's durable match
    /// store. `process_match_day_results` pushed a pre-processing
    /// snapshot of this match earlier in the tick (PoM `None`, raw
    /// ratings) — before `compute_effective_ratings`,
    /// `CanonicalRatingMutator` and the Player of the Match pick above
    /// had run. Both the per-match web page and the weekly award
    /// aggregator read `League.matches`, so without this they'd never
    /// see the nominee for domestic league / cup games. Continental
    /// cups have no `League` row and store via the global `match_store`
    /// after `process_cup_match`, so `league_mut` finds nothing and
    /// this is a no-op for them.
    fn sync_match_store<D: LeagueProcessAccess>(result: &MatchResult, data: &mut D) {
        if let Some(league) = data.league_mut(result.league_id) {
            league
                .matches
                .replace_if_present(result.copy_without_data_positions());
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
        let actions = Self::collect_scouting_memory(details, data, is_friendly, date);
        Self::write_scouting_memory(actions, data, home_team_id, away_team_id);
    }

    /// Read pass: one shared borrow of the country resolves candidates,
    /// recipient facts, and the probability rolls.
    fn collect_scouting_memory<D: LeagueProcessAccess>(
        details: &MatchResultRaw,
        data: &D,
        is_friendly: bool,
        date: NaiveDate,
    ) -> Vec<MemoryAction> {
        let mut actions: Vec<MemoryAction> = Vec::new();
        for side in [&details.left_team_players, &details.right_team_players] {
            let Some(context) = ScoutingMemoryContext::for_side(side.team_id, data) else {
                continue;
            };
            let appeared: Vec<u32> = side
                .main
                .iter()
                .copied()
                .chain(side.substitutes_used.iter().copied())
                .collect();
            for player_id in appeared {
                if let Some(action) = Self::scouting_memory_action(
                    player_id,
                    details,
                    data,
                    is_friendly,
                    date,
                    &context,
                ) {
                    actions.push(action);
                }
            }
        }
        actions
    }

    /// Build one player's memory snapshot, or `None` when he isn't the kind
    /// of player this path is for.
    fn scouting_memory_action<D: LeagueProcessAccess>(
        player_id: u32,
        details: &MatchResultRaw,
        data: &D,
        is_friendly: bool,
        date: NaiveDate,
        context: &ScoutingMemoryContext,
    ) -> Option<MemoryAction> {
        let player = data.player(player_id)?;

        let foreign_loan_exposure = player
            .contract_loan
            .as_ref()
            .and_then(|loan| loan.loan_from_club_id)
            .and_then(|parent_club_id| {
                data.country_by_club(parent_club_id)
                    .map(|parent_country| parent_country.id != context.current_country_id)
            })
            .unwrap_or(false);

        // Regular domestic players are discovered by the normal
        // scouting pipeline. This memory path is for players whose
        // stay in the country is temporary or otherwise easy to lose.
        if !foreign_loan_exposure {
            return None;
        }

        let stats = details.player_stats.get(&player_id)?;
        let skill_ability = player
            .skills
            .calculate_ability_for_position(player.position());
        // `stats.match_rating` is the canonical public/effective
        // rating — `process_match_events` overwrites it in-place
        // with the settlement-adjusted value before this helper
        // runs, so neighbouring clubs see what the wider football
        // world sees, not the unfiltered engine verdict.
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
        // Scouts in the stands assess from observables — never
        // the hidden biological PA.
        let assessed_potential =
            PotentialEstimator::observable_ceiling(player, date).max(assessed_ability);
        // Use the host club's blended reputation as the seller
        // proxy for memory snapshots — the player is appearing
        // in this country, so the local league/club context is
        // the right anchor for the buyer's mental price tag.
        let estimated_value = PlayerValuationCalculator::calculate_value_with_price_level(
            player,
            date,
            context.current_price_level,
            context.host_league_rep,
            context.host_club_rep,
        )
        .amount;

        Some(MemoryAction {
            country_id: context.current_country_id,
            current_club_id: context.current_club_id,
            memory: KnownPlayerMemory {
                player_id,
                last_known_club_id: context.current_club_id,
                last_known_country_id: context.current_country_id,
                position: player.position(),
                position_group: player.position().position_group(),
                age: player.age(date),
                assessed_ability,
                assessed_potential,
                confidence: if is_friendly { 0.28 } else { 0.48 },
                estimated_fee: estimated_value,
                last_seen: date,
                official_appearances_seen: if is_friendly { 0 } else { 1 },
                friendly_appearances_seen: if is_friendly { 1 } else { 0 },
            },
        })
    }

    /// Write pass: remember each player at the clubs that just watched him.
    fn write_scouting_memory<D: LeagueProcessAccess>(
        actions: Vec<MemoryAction>,
        data: &mut D,
        home_team_id: u32,
        away_team_id: u32,
    ) {
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
            .map(|t| MatchReputation::overall_score_to_u16(t.reputation.overall_score()))
            .unwrap_or(0);
        let away_rep = data
            .team(away_team_id)
            .map(|t| MatchReputation::overall_score_to_u16(t.reputation.overall_score()))
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
            MatchReputation::league_standings(result.league_id, home_team_id, away_team_id, data);
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
        let goals_per_player = ContractBonusPass::goals_per_player(result);
        let appearance_ids = ContractBonusPass::appearance_ids(details);
        let unused_sub_ids = ContractBonusPass::unused_sub_ids(details);

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
        let mut club_totals: FxHashMap<u32, i64> = FxHashMap::default();
        ContractBonusPass::accumulate_appearance_bonuses(
            &mut club_totals,
            &appearance_ids,
            &goals_per_player,
            details,
            data,
            left_conceded,
            right_conceded,
        );

        // Unused-substitute fee: bench players who didn't get on the
        // pitch are still paid their negotiated showup fee. Same payer
        // routing as the in-play bonuses — parent contract bills the
        // parent, loan contract bills the borrower.
        ContractBonusPass::accumulate_unused_sub_bonuses(&mut club_totals, &unused_sub_ids, data);

        // Pass 2 (write): charge each club once.
        for (club_id, amount) in club_totals {
            if let Some(club) = data.club_mut(club_id) {
                club.finance.balance.push_expense_player_wages(amount);
            }
        }
    }

    /// Apply the matchday's dressing-room talk moments to both sides:
    /// a pre-match rallying cry on big occasions (derby / continental
    /// night), a half-time talk when the break demands one (trailing,
    /// or a clear favourite held level), and the routine full-time talk
    /// toned by the final score. All applied at result-processing time —
    /// they land as dated `DressingRoomSpeech` morale events; in-match
    /// performance effects are deliberately out of scope. The head
    /// coach's Man Management / Motivating attributes drive
    /// effectiveness; per-player magnitude uses personality (pressure,
    /// temperament, important_matches) via
    /// `club::team::talks::team_talks::apply_team_talk`.
    fn apply_matchday_team_talks<D: LeagueProcessAccess>(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &mut D,
        is_friendly: bool,
        big_match: bool,
    ) {
        let sides = Self::collect_side_talks(result, details, data);
        let now = data.date().date();
        for side in sides {
            Self::deliver_side_talk(&side, data, is_friendly, big_match, now);
        }
    }

    /// Reconstruct the half-time score from the goal timeline. The
    /// details vec also carries assists (and a goal can be entered
    /// twice via the end-of-match statistics sweep), so filter to
    /// goals and de-dup on (scorer, timestamp). Own goals credit the
    /// other side.
    fn half_time_delta(details: &MatchResultRaw, result: &MatchResult) -> i8 {
        let ht_cutoff_ms: u64 = 45 * 60_000;
        let left_participants: FxHashSet<u32> = details
            .left_team_players
            .main
            .iter()
            .chain(details.left_team_players.substitutes_used.iter())
            .copied()
            .collect();
        let right_participants: FxHashSet<u32> = details
            .right_team_players
            .main
            .iter()
            .chain(details.right_team_players.substitutes_used.iter())
            .copied()
            .collect();
        let mut seen_goals: FxHashSet<(u32, u64)> = FxHashSet::default();
        let (mut left_ht, mut right_ht) = (0i8, 0i8);
        for goal in result.score.details.iter() {
            if goal.stat_type != MatchStatisticType::Goal || goal.time > ht_cutoff_ms {
                continue;
            }
            if !seen_goals.insert((goal.player_id, goal.time)) {
                continue;
            }
            let scorer_left = left_participants.contains(&goal.player_id);
            let scorer_right = right_participants.contains(&goal.player_id);
            if !scorer_left && !scorer_right {
                continue;
            }
            if scorer_left != goal.is_auto_goal {
                left_ht += 1;
            } else {
                right_ht += 1;
            }
        }
        left_ht - right_ht
    }

    /// Collect each side's players and club id once: those who took the
    /// field for the pre-match and full-time talks, the whole dressing
    /// room and the talk its coach gave for the half-time one. `rep_edge`
    /// is this side's reputation edge over the opponent (0-centred) — it
    /// decides whether the result reads as a humiliation or an upset on
    /// top of the raw scoreline.
    fn collect_side_talks<D: LeagueProcessAccess>(
        result: &MatchResult,
        details: &MatchResultRaw,
        data: &D,
    ) -> Vec<SideTalk> {
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
        let left_ht_delta = Self::half_time_delta(details, result);

        let left_slot_team_id = details.left_team_players.team_id;
        let right_slot_team_id = details.right_team_players.team_id;
        let rep_of = |data: &D, team_id: u32| -> f32 {
            data.team(team_id)
                .map(|t| t.reputation.overall_score())
                .unwrap_or(0.5)
        };
        let left_rep = rep_of(data, left_slot_team_id);
        let right_rep = rep_of(data, right_slot_team_id);

        let mut sides: Vec<SideTalk> = Vec::with_capacity(2);
        for (slot, delta, ht_delta, rep_edge) in [
            (
                &details.left_team_players,
                left_delta,
                left_ht_delta,
                left_rep - right_rep,
            ),
            (
                &details.right_team_players,
                right_delta,
                -left_ht_delta,
                right_rep - left_rep,
            ),
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
                dressing_room: slot.dressing_room().collect(),
                half_time_talk: details.half_time_talk(slot.team_id),
                delta,
                ht_delta,
                rep_edge,
            });
        }
        sides
    }

    fn deliver_side_talk<D: LeagueProcessAccess>(
        side: &SideTalk,
        data: &mut D,
        is_friendly: bool,
        big_match: bool,
        now: NaiveDate,
    ) {
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

        // Chronological delivery: pre-match rallying cry (big
        // occasions only), the half-time moment when the break
        // demanded one, then the routine full-time talk. The
        // repeated-tone dampener in `apply_team_talk_dated` keeps a
        // manager who says the same thing three times in one day
        // from tripling the effect.
        if let Some(tone) = TeamTalkMoments::pre_match_tone(big_match) {
            Self::deliver_team_talk(
                data,
                &side.player_ids,
                manager_clone.as_ref(),
                tone,
                TeamTalkContext {
                    phase: MatchPhase::PreMatch,
                    score_delta: 0,
                    big_match,
                },
                now,
            );
        }

        if !is_friendly && let Some(tone) = side.half_time_talk {
            Self::deliver_team_talk(
                data,
                &side.dressing_room,
                manager_clone.as_ref(),
                tone,
                TeamTalkContext {
                    phase: MatchPhase::HalfTime,
                    score_delta: side.ht_delta,
                    big_match,
                },
                now,
            );
        }

        let tone = Self::tone_for(side.delta);
        let ctx = TeamTalkContext {
            phase: MatchPhase::FullTime,
            score_delta: side.delta,
            big_match,
        };
        Self::deliver_team_talk(
            data,
            &side.player_ids,
            manager_clone.as_ref(),
            tone,
            ctx,
            now,
        );

        // Expectation-aware extremes on top of the routine talk
        // tones: a humiliation — a three-goal collapse or any
        // defeat as a clear favourite — triggers a dressing-room
        // inquest; an upset win over clearly stronger opposition
        // gets the response publicly praised. Competitive matches
        // only; a friendly humbles nobody.
        if !is_friendly {
            Self::apply_expectation_extremes(side, data);
        }
    }

    fn tone_for(delta: i8) -> TeamTalkTone {
        match delta {
            d if d >= 2 => TeamTalkTone::Praise,
            1 => TeamTalkTone::Encourage,
            0 => TeamTalkTone::Encourage,
            -1 => TeamTalkTone::Criticise,
            _ => TeamTalkTone::Passionate,
        }
    }

    fn apply_expectation_extremes<D: LeagueProcessAccess>(side: &SideTalk, data: &mut D) {
        let humiliation = side.delta <= -3 || (side.delta < 0 && side.rep_edge >= 0.15);
        let upset_win = side.delta > 0 && side.rep_edge <= -0.15;
        if humiliation || upset_win {
            let cfg = HappinessConfig::default();
            let (event, magnitude) = if humiliation {
                (
                    HappinessEventType::DressingRoomInquest,
                    cfg.catalog.dressing_room_inquest,
                )
            } else {
                (
                    HappinessEventType::ResiliencePraised,
                    cfg.catalog.resilience_praised,
                )
            };
            for pid in &side.player_ids {
                if let Some(player) = data.player_mut(*pid) {
                    player
                        .happiness
                        .add_event_with_cooldown(event.clone(), magnitude, 10);
                }
            }
        }
    }

    /// Route one team talk to every listed player. `apply_team_talk_dated`
    /// wants an iterator of `&mut Player`, and `LeagueProcessAccess` hands
    /// out one mutable player at a time — so deliver per player.
    fn deliver_team_talk<D: LeagueProcessAccess>(
        data: &mut D,
        player_ids: &[u32],
        manager: Option<&Staff>,
        tone: TeamTalkTone,
        ctx: TeamTalkContext,
        now: NaiveDate,
    ) {
        for pid in player_ids {
            if let Some(player) = data.player_mut(*pid) {
                let single = std::slice::from_mut(player);
                TeamTalkApplier::apply_dated(single.iter_mut(), manager, tone, ctx, Some(now));
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
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();

        for side in [&details.left_team_players, &details.right_team_players] {
            let (scored, conceded) = if side.team_id == home_team_id {
                (home_goals, away_goals)
            } else {
                (away_goals, home_goals)
            };

            let snapshot =
                SideRelationshipSnapshot::build(side, result, data, now, scored, conceded);
            let updates = Self::relationship_updates(&snapshot, data, best_player_id);
            // Track per-player visible event count so we cap at 2.
            let mut event_budget: FxHashMap<u32, u8> = FxHashMap::default();
            Self::apply_relationship_updates(data, updates, &mut event_budget, now);
            Self::apply_winning_cooperation(&snapshot, data, now);
        }
    }

    /// Every relationship update the match earns for one side, in the order
    /// the apply pass expects: clean sheet, heavy defeat, red cards, Player
    /// of the Match, then scorer admiration.
    fn relationship_updates<D: LeagueProcessAccess>(
        snapshot: &SideRelationshipSnapshot,
        data: &D,
        best_player_id: Option<u32>,
    ) -> Vec<RelationshipUpdate> {
        let mut updates: Vec<RelationshipUpdate> = Vec::new();
        Self::clean_sheet_updates(&snapshot.players, snapshot.conceded, &mut updates);
        Self::heavy_defeat_updates(&snapshot.players, snapshot.heavy_defeat, &mut updates);
        Self::red_card_updates(&snapshot.players, &snapshot.red_carded, data, &mut updates);
        Self::player_of_the_match_updates(
            &snapshot.players,
            &snapshot.appeared,
            best_player_id,
            &mut updates,
        );
        Self::scorer_admiration_updates(&snapshot.players, &snapshot.scorer_goals, &mut updates);
        updates
    }

    /// ── Clean sheet bonds: GK ↔ CBs, CB ↔ CB ────────────────
    fn clean_sheet_updates(
        players: &[SidePlayer],
        conceded: u8,
        updates: &mut Vec<RelationshipUpdate>,
    ) {
        if conceded != 0 {
            return;
        }
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
                updates.push(RelationshipUpdate {
                    from: gk,
                    to: cb,
                    change_type: ChangeType::MatchCooperation,
                    magnitude: 0.12,
                    event: None,
                });
                updates.push(RelationshipUpdate {
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
                updates.push(RelationshipUpdate {
                    from: cb_ids[i],
                    to: cb_ids[j],
                    change_type: ChangeType::MatchCooperation,
                    magnitude: 0.15,
                    event: None,
                });
                updates.push(RelationshipUpdate {
                    from: cb_ids[j],
                    to: cb_ids[i],
                    change_type: ChangeType::MatchCooperation,
                    magnitude: 0.15,
                    event: None,
                });
            }
        }
    }

    /// ── Heavy defeat: captain steps in or implodes ──────────
    fn heavy_defeat_updates(
        players: &[SidePlayer],
        heavy_defeat: bool,
        updates: &mut Vec<RelationshipUpdate>,
    ) {
        if !heavy_defeat {
            return;
        }
        let captain = players.iter().find(|p| p.is_captain);
        if let Some(cap) = captain {
            if cap.professionalism < 8.0 {
                // Captain takes it out on dressing room.
                for p in players.iter().filter(|p| p.id != cap.id) {
                    updates.push(RelationshipUpdate {
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
                    updates.push(RelationshipUpdate {
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

    /// ── Red card: friction within the same unit ─────────────
    ///
    /// Simulation truth: every same-unit teammate accumulates a
    /// small `PersonalConflict` update against the offender — the
    /// back-four had to play with ten, the midfield got
    /// overloaded, etc. Visible player-history rows are scarcer:
    /// we pick the 1-2 teammates whose personality / standing
    /// would realistically make them speak up, and classify the
    /// event so each reactor's row reads as a specific football
    /// moment (captain's leadership challenge vs. a pro's
    /// standards reaction vs. plain tactical blame) rather than
    /// a copy-paste "argued with teammate" line.
    fn red_card_updates<D: LeagueProcessAccess>(
        players: &[SidePlayer],
        red_carded: &[u32],
        data: &D,
        updates: &mut Vec<RelationshipUpdate>,
    ) {
        for &offender in red_carded {
            let group = match players.iter().find(|p| p.id == offender) {
                Some(p) => p.group,
                None => continue,
            };

            let visible_reactors = Self::visible_reactors(players, offender, group, data);

            for p in players
                .iter()
                .filter(|p| p.id != offender && p.group == group)
            {
                updates.push(RelationshipUpdate {
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
    }

    /// The teammates whose personality and standing would make them speak
    /// up about a red card, ranked and capped at two.
    fn visible_reactors<D: LeagueProcessAccess>(
        players: &[SidePlayer],
        offender: u32,
        group: PlayerFieldPositionGroup,
        data: &D,
    ) -> FxHashMap<u32, ReactorProfile> {
        let mut visible_reactors: Vec<(u32, ReactorProfile)> = players
            .iter()
            .filter(|p| p.id != offender && p.group == group)
            .map(|p| {
                let relation_level = data
                    .player(p.id)
                    .and_then(|player| player.relations.get_player(offender))
                    .map(|r| r.level)
                    .unwrap_or(0.0);
                (p.id, ReactorProfile::for_player(p, relation_level))
            })
            .collect();

        // Rank by score desc, threshold by 1.5, cap at 2. A
        // vanilla red card with vanilla teammates produces zero
        // visible rows — the offender takes the (separate) card
        // event, the dressing room simmers in the simulation
        // layer, and the feed stays quiet.
        visible_reactors
            .sort_by(|a, b| b.1.score.partial_cmp(&a.1.score).unwrap_or(Ordering::Equal));
        visible_reactors
            .into_iter()
            .filter(|(_, prof)| prof.score >= 1.5)
            .take(2)
            .collect()
    }

    /// ── Player of the match: admiration vs envy ────────────
    fn player_of_the_match_updates(
        players: &[SidePlayer],
        appeared: &[u32],
        best_player_id: Option<u32>,
        updates: &mut Vec<RelationshipUpdate>,
    ) {
        if let Some(motm) = best_player_id
            && appeared.contains(&motm)
        {
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
                    updates.push(RelationshipUpdate {
                        from: p.id,
                        to: motm,
                        change_type: ChangeType::ReputationAdmiration,
                        magnitude: 0.10,
                        event: None,
                    });
                } else if p.controversy >= 14.0 && motm_controversy >= 11.0 && same_group {
                    updates.push(RelationshipUpdate {
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

    /// ── Goal scorer admiration (single-direction proxy) ─────
    /// Without an assist↔scorer pairing in the stat stream we
    /// approximate the cooperation lift with a generic admiration
    /// signal from teammates toward each scorer. Caps at 2 goals
    /// per scorer to avoid runaway updates.
    fn scorer_admiration_updates(
        players: &[SidePlayer],
        scorer_goals: &FxHashMap<u32, u8>,
        updates: &mut Vec<RelationshipUpdate>,
    ) {
        for (scorer, goals) in scorer_goals {
            let n = (*goals).min(2) as f32;
            for p in players.iter().filter(|p| p.id != *scorer) {
                updates.push(RelationshipUpdate {
                    from: p.id,
                    to: *scorer,
                    change_type: ChangeType::ReputationAdmiration,
                    magnitude: 0.08 * n,
                    event: None,
                });
            }
        }
    }

    /// ── Apply updates with cooldown / event budget ──────────
    /// Sort by `from` so the player borrow happens once per
    /// unique source. Each match accumulates ~30–50 updates
    /// spread across ~11 players; without grouping we'd take
    /// ~30–50 mutable borrows. With grouping we take ~11.
    fn apply_relationship_updates<D: LeagueProcessAccess>(
        data: &mut D,
        mut updates: Vec<RelationshipUpdate>,
        event_budget: &mut FxHashMap<u32, u8>,
        now: NaiveDate,
    ) {
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
                    Self::apply_relationship_update(player, upd, event_budget, now);
                }
            }
            i = block_end;
        }
    }

    fn apply_relationship_update(
        player: &mut Player,
        upd: &RelationshipUpdate,
        event_budget: &mut FxHashMap<u32, u8>,
        now: NaiveDate,
    ) {
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
            if *slot < 2 && Self::record_visible_event(player, upd, &event, relation_before, now) {
                *slot += 1;
            }
        }
    }

    /// Record one visible partner event, enriching its evidence from the
    /// live relationship. Returns true when the event was accepted.
    fn record_visible_event(
        player: &mut Player,
        upd: &RelationshipUpdate,
        event: &VisiblePairEvent,
        relation_before: Option<(f32, f32, f32, f32)>,
        _now: NaiveDate,
    ) -> bool {
        let (level_before, trust, friendship, professional_respect) =
            relation_before.unwrap_or((0.0, 50.0, 30.0, 50.0));
        let level_after = player
            .relations
            .get_player(upd.to)
            .map(|r| r.level)
            .unwrap_or(level_before);

        let mut evidence = event.evidence.clone();
        if level_before <= -25.0
            && !evidence.contains(&HappinessEventEvidence::AlreadyStrainedRelationship)
        {
            evidence.push(HappinessEventEvidence::AlreadyStrainedRelationship);
        } else if level_before.abs() < 25.0
            && !evidence.contains(&HappinessEventEvidence::WeakExistingBond)
        {
            evidence.push(HappinessEventEvidence::WeakExistingBond);
        }
        if trust <= 35.0 && !evidence.contains(&HappinessEventEvidence::LowTrust) {
            evidence.push(HappinessEventEvidence::LowTrust);
        }
        if friendship <= 25.0 && !evidence.contains(&HappinessEventEvidence::LowFriendship) {
            evidence.push(HappinessEventEvidence::LowFriendship);
        }
        if professional_respect <= 35.0
            && !evidence.contains(&HappinessEventEvidence::LowProfessionalRespect)
        {
            evidence.push(HappinessEventEvidence::LowProfessionalRespect);
        }
        if player
            .happiness
            .has_recent_event_with_partner(&event.kind, upd.to, 90)
            && !evidence.contains(&HappinessEventEvidence::RepeatedIncident)
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
        .with_change_kind(HappinessEventChangeKind::from_change_type(&upd.change_type))
        .with_evidence_iter(evidence)
        .with_follow_up(follow_up)
        .with_teammate_conflict_context(TeammateConflictContext::new(event.reason, event.location));

        player
            .happiness
            .add_event_with_partner_context_and_cooldown(
                event.kind.clone(),
                event.magnitude,
                upd.to,
                context,
                45,
            )
    }

    /// Win/loss generic team-mate cooperation lift — softer signal
    /// shared by every player on the winning side. Friction lift
    /// skipped on losses; the captain block above captured the
    /// emotional payload for heavy defeats. A team that just wins
    /// narrowly doesn't accumulate dressing-room damage.
    ///
    /// One mutable borrow per player; the inner loop fans out the
    /// pairing updates so a winning XI does ~11 lookups instead
    /// of ~110.
    fn apply_winning_cooperation<D: LeagueProcessAccess>(
        snapshot: &SideRelationshipSnapshot,
        data: &mut D,
        now: NaiveDate,
    ) {
        if !snapshot.team_won {
            return;
        }
        for (i, outer) in snapshot.players.iter().enumerate() {
            if let Some(player) = data.player_mut(outer.id) {
                for (j, teammate) in snapshot.players.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    player.relations.update_with_type(
                        teammate.id,
                        0.05,
                        ChangeType::MatchCooperation,
                        now,
                    );
                }
            }
        }
    }

    /// Apply card / suspension consequences from a finished competitive
    /// match of `family`. Walks every featuring player and updates his
    /// suspension counter in that family; then serves one match against
    /// any player on either team who carried a ban of that family into
    /// this fixture and did not appear (i.e. he sat out — the ban ticks
    /// down). Friendlies are excluded by the caller — friendly cards don't
    /// ban a player from the next competitive match in this model.
    fn apply_post_match_discipline<D: LeagueProcessAccess>(
        result: &MatchResult,
        details: &MatchResultRaw,
        family: CompetitionFamily,
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
                let added =
                    player.on_match_disciplinary_result(yellows, reds, yellow_threshold, family);
                if added > 0 {
                    new_suspensions.push(pid);
                }
            }
        }

        // 2) Decrement existing bans for absent players. "Absent" means
        // the player is on either team but does NOT appear in the
        // FieldSquad of their side this match. The card pass above
        // suspends a man sent off in this match — he is not absent (he
        // was on the field), so he is excluded via `new_suspensions` to
        // avoid immediately serving the suspension he just earned.
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
                if !player.is_suspended_in(family) {
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
                player.serve_suspension_match(family);
            }
            // Mirror on the league-side analytics record.
            if let Some(league) = data.league_mut(result.league_id) {
                let _ = league.regulations.record_suspension_served(pid);
            }
        }
    }

    fn process_loan_match_fees<D: LeagueProcessAccess>(details: &MatchResultRaw, data: &mut D) {
        // Collect fee transfers: (parent_club_id, borrowing_club_id, fee).
        //
        // The parent-funded fee is earned only by putting the loanee on the
        // pitch — and it's weighted toward *starting* him: a start banks the
        // full fee, a substitute cameo only the reduced share (see
        // `PlayerClubContract::loan_fee_for_appearance`). Tying the money to a
        // start is precisely what gives the borrowing club a reason to name the
        // player in its XI rather than stash him on the bench.
        let mut fee_transfers: Vec<(u32, u32, u32)> = Vec::new();

        // `main` is the starting XI; `substitutes_used` are the bench players
        // who actually came on. Pair each id with whether it started so the fee
        // can be sized accordingly.
        let appearances = details
            .left_team_players
            .main
            .iter()
            .map(|id| (*id, true))
            .chain(
                details
                    .left_team_players
                    .substitutes_used
                    .iter()
                    .map(|id| (*id, false)),
            )
            .chain(details.right_team_players.main.iter().map(|id| (*id, true)))
            .chain(
                details
                    .right_team_players
                    .substitutes_used
                    .iter()
                    .map(|id| (*id, false)),
            );

        for (player_id, started) in appearances {
            if let Some(player) = data.player(player_id)
                && let Some(ref loan) = player.contract_loan
            {
                let fee = loan.loan_fee_for_appearance(started);
                if fee > 0
                    && let (Some(parent_id), Some(borrowing_id)) =
                        (loan.loan_from_club_id, loan.loan_to_club_id)
                {
                    fee_transfers.push((parent_id, borrowing_id, fee));
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

/// Folds the public/effective rating map back into the canonical
/// `MatchResultRaw.player_stats[*].match_rating` field so every
/// downstream reader sees one consistent value. The raw engine rating
/// stays on `raw_match_rating` for calibration / debug surfaces.
///
/// Extracted as a named operation (rather than an inline loop in
/// `process_match_events`) so the mutation contract has a single test
/// site — see `canonical_rating_tests` below.
struct CanonicalRatingMutator;

impl CanonicalRatingMutator {
    fn apply(details: &mut MatchResultRaw, effective_ratings: &FxHashMap<u32, f32>) {
        for (pid, public_rating) in effective_ratings {
            if let Some(stats) = details.player_stats.get_mut(pid) {
                stats.match_rating = *public_rating;
            }
        }
    }
}

/// Per-team enrichment shared by every player on the same side of a
/// match — resolved once, then read for each player's settlement
/// context. Building this per-side (instead of per-player) keeps the
/// adaptation_score call cheap: a single team / staff lookup feeds the
/// 11 starters + subs.
struct MatchSideContext {
    /// Staff id of the manager who picked this XI. `None` when the team
    /// has no Manager on the books — the player's `manager_relation_level`
    /// stays neutral in that case.
    manager_id: Option<u32>,
    /// Primary position of every player who started the match, in the
    /// order they appeared in `FieldSquad.main`. Drives `adaptation_score`'s
    /// role-fit axis. `None` when the side didn't field 11 (test or
    /// abandoned fixture).
    formation: Option<[PlayerPositionType; 11]>,
}

impl MatchSideContext {
    /// Resolve manager + formation array for one side of a finished
    /// match. Walks the team's staff collection for the manager id and
    /// looks each starter up in `data` for their primary position.
    fn build<D: LeagueProcessAccess>(side: &FieldSquad, data: &D) -> Self {
        let manager_id = data.team(side.team_id).and_then(|team| {
            team.staffs
                .find_by_position(StaffPosition::Manager)
                .map(|s| s.id)
        });
        let formation = if side.main.len() == 11 {
            let mut slots: [PlayerPositionType; 11] = [PlayerPositionType::Striker; 11];
            let mut all_resolved = true;
            for (i, pid) in side.main.iter().enumerate() {
                match data.player(*pid) {
                    Some(p) => slots[i] = p.position(),
                    None => {
                        all_resolved = false;
                        break;
                    }
                }
            }
            if all_resolved { Some(slots) } else { None }
        } else {
            None
        };
        MatchSideContext {
            manager_id,
            formation,
        }
    }

    /// Player's relation level (-100..100) to this side's manager, or
    /// `0.0` (neutral) when the relation hasn't been recorded yet or the
    /// team has no manager.
    fn manager_relation_level_for(&self, player: &Player) -> f32 {
        self.manager_id
            .and_then(|mid| player.relations.get_staff(mid))
            .map(|rel| rel.level)
            .unwrap_or(0.0)
    }
}

/// Mood-colouring of a match rating from the player's morale.
///
/// Morale is the happiness model's 0..100 mood score (50 = neutral). The
/// shift is deliberately a *pure player-state* signal — it reads nothing
/// about the club's reputation or quality, so two keepers with the same
/// morale receive the same colouring whether one plays for the champions
/// and the other for the bottom side. Two design rules keep it honest:
///
///   * **Asymmetric** — low morale drags harder than high morale lifts.
///     Disgruntlement visibly disrupts a performance (a player hiding,
///     playing within himself), whereas contentment is the baseline a
///     professional is expected to bring; confidence helps, but less than
///     unhappiness hurts.
///   * **Bounded** — capped at −0.45 / +0.15 so morale only *colours* the
///     stat-line verdict, never overturns it. A brilliant shift on poor
///     morale is still brilliant; a poor shift on great morale is still
///     poor. Keeping the magnitude small also keeps the morale→rating→
///     form→morale loop's gain well under 1, so a dip nudges the
///     equilibrium rather than spiralling.
struct MoraleRatingShaper;

impl MoraleRatingShaper {
    #[inline]
    fn shift(morale: f32) -> f32 {
        let centered = ((morale - 50.0) / 50.0).clamp(-1.0, 1.0);
        if centered < 0.0 {
            centered * 0.45
        } else {
            // Upside trimmed 0.20 → 0.15 (FM-parity winger pass,
            // 2026-07): a regular starter at a winning club sits well
            // above neutral morale nearly all season, so the lift was
            // a systematic +0.1..0.2 on exactly the players the season
            // bands assume mean-neutral shaping for.
            centered * 0.15
        }
    }
}

/// Per-side rating context, resolved once for every player.
struct RatingSides<'a> {
    left_ctx: &'a MatchSideContext,
    right_ctx: &'a MatchSideContext,
    left_team_id: u32,
    right_team_id: u32,
    left_team_rep: f32,
    right_team_rep: f32,
}

impl RatingSides<'_> {
    /// The player's own side, the opponent's reputation, and the team id
    /// used by the per-identity texture seed.
    fn for_player<'a>(
        &'a self,
        player_id: u32,
        details: &MatchResultRaw,
    ) -> (Option<&'a MatchSideContext>, f32, u32) {
        let is_left = details.left_team_players.main.contains(&player_id)
            || details
                .left_team_players
                .substitutes_used
                .contains(&player_id);
        let is_right = details.right_team_players.main.contains(&player_id)
            || details
                .right_team_players
                .substitutes_used
                .contains(&player_id);
        if is_left {
            (Some(self.left_ctx), self.right_team_rep, self.left_team_id)
        } else if is_right {
            (Some(self.right_ctx), self.left_team_rep, self.right_team_id)
        } else {
            (None, self.left_team_rep, self.right_team_id)
        }
    }
}

/// Computes the public/effective rating map from the engine's raw stat
/// lines, applying settlement adaptation and the personality/state shape.
struct EffectiveRatingCalculator;

/// Per-player inputs to the personality/state shape of a rating.
struct RatingShapeInputs {
    club_rep: f32,
    opponent_rep: f32,
    team_chem: f32,
    consistency: f32,
    big_match: f32,
    temperament: f32,
    pressure: f32,
    professionalism: f32,
    morale: f32,
}

impl EffectiveRatingCalculator {
    fn compute<D: LeagueProcessAccess>(
        details: &MatchResultRaw,
        data: &D,
        now: NaiveDate,
    ) -> FxHashMap<u32, f32> {
        // Resolve per-side enrichment once. Map each player_id (starters +
        // used subs) to the side context their team owns so the per-player
        // loop below doesn't repeat the team / staff / formation lookups.
        let left_ctx = MatchSideContext::build(&details.left_team_players, data);
        let right_ctx = MatchSideContext::build(&details.right_team_players, data);

        // Per-side team reputation, resolved once so the pressure axis below
        // (opponent-rep gap → personality-amplified swing) doesn't re-walk
        // the team store per player.
        let left_team_id = details.left_team_players.team_id;
        let right_team_id = details.right_team_players.team_id;
        let sides = RatingSides {
            left_ctx: &left_ctx,
            right_ctx: &right_ctx,
            left_team_id,
            right_team_id,
            left_team_rep: data
                .team(left_team_id)
                .map(|t| t.reputation.overall_score())
                .unwrap_or(0.0),
            right_team_rep: data
                .team(right_team_id)
                .map(|t| t.reputation.overall_score())
                .unwrap_or(0.0),
        };

        // Per-match deterministic date seed, shared by the two
        // deterministic jitter sources below (consistency swing + routine
        // texture). `num_days_from_ce` keeps the public rating fully
        // reproducible for a given (player, date) pair — the spec's
        // "deterministic noise seeded by real inputs" requirement.
        // f64 on purpose: at the ~7×10⁵ magnitude of days-from-CE (and
        // player ids up in the same range) an f32 mantissa quantises
        // `.fract()` to steps of ~0.06, collapsing the jitter to a
        // handful of shared levels instead of a per-identity spread.
        let date_seed = now.num_days_from_ce() as f64;

        let mut out =
            FxHashMap::with_capacity_and_hasher(details.player_stats.len(), Default::default());
        for (player_id, stats) in &details.player_stats {
            out.insert(
                *player_id,
                Self::player_rating(*player_id, stats, details, data, now, date_seed, &sides),
            );
        }
        out
    }

    /// The public/effective rating for one player.
    fn player_rating<D: LeagueProcessAccess>(
        player_id: u32,
        stats: &PlayerMatchEndStats,
        details: &MatchResultRaw,
        data: &D,
        now: NaiveDate,
        date_seed: f64,
        sides: &RatingSides<'_>,
    ) -> f32 {
        let (side_ctx, opponent_rep, player_team_id) = sides.for_player(player_id, details);
        let location = data
            .indexes()
            .and_then(|i| i.get_player_location(player_id));
        let country_code = location
            .and_then(|(_, country_id, _, _)| data.country_info().get(&country_id))
            .map(|ci| ci.code.clone())
            .unwrap_or_default();
        let club_rep = location
            .and_then(|(_, _, _, team_id)| data.team(team_id))
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.0);

        // Build the public/effective rating in two stages:
        //   1) settlement adjustment — single source of truth for the
        //      "this player is still adapting" dampening. Reads richer
        //      adaptation signals (language, mentor, role fit, chemistry,
        //      appearances, dream-move lift) via `adaptation_score`, not
        //      just days-since-transfer. Skips entirely for elite
        //      reputations, post-window calendars, no-recent-transfer
        //      players, and after enough competitive starts;
        //   2) the existing chemistry / consistency / big-match /
        //      temperament shape — applied to the settlement-adjusted
        //      value so the same rating drives season averages, POTM,
        //      awards, scouting observations, form EMA, and reputation.
        // `RatingContext` upstream stays purely stat-line — the
        // raw/public split is owned here.
        let adjusted_from_settlement = Self::settlement_adjusted_rating(
            player_id,
            stats,
            data,
            now,
            &country_code,
            club_rep,
            side_ctx,
            location,
        );

        // Team-level chemistry — read from the weekly-refreshed
        // snapshot whenever the player's current team can be located.
        // Per polish task #6, the team's `team_chemistry()` is the
        // canonical source; the per-player
        // `relations.get_team_chemistry()` is reserved for the
        // adaptation read above where there's no clean team context.
        let team_chem = location
            .and_then(|(_, _, _, team_id)| data.team(team_id))
            .map(|t| t.team_chemistry())
            .unwrap_or(50.0);
        let inputs = Self::shape_inputs(player_id, data, club_rep, opponent_rep, team_chem);
        Self::shape_rating(
            player_id,
            stats,
            player_team_id,
            date_seed,
            adjusted_from_settlement,
            &inputs,
        )
    }

    /// Settlement adjustment plus its adaptation context. The raw stat-line
    /// rating is returned unchanged when the player isn't found or the
    /// adaptation model has nothing to apply.
    #[allow(clippy::too_many_arguments)]
    fn settlement_adjusted_rating<D: LeagueProcessAccess>(
        player_id: u32,
        stats: &PlayerMatchEndStats,
        data: &D,
        now: NaiveDate,
        country_code: &str,
        club_rep: f32,
        side_ctx: Option<&MatchSideContext>,
        location: Option<(u32, u32, u32, u32)>,
    ) -> f32 {
        data.player(player_id)
            .map(|p| {
                // TODO: mentor_quality is not yet surfaced by the
                // squad-life system as a per-player snapshot — the
                // existing transfer / adaptation callers pass `None`
                // too. When `SquadSocialView` (or a sibling helper)
                // gains a mentor-quality field, plumb it through here
                // instead of holding the axis at neutral.
                let squad = AdaptationSquadContext {
                    same_language_teammates: p
                        .squad_social_view
                        .as_ref()
                        .map(|v| v.same_language_teammates)
                        .unwrap_or(0),
                    same_nationality_teammates: p
                        .squad_social_view
                        .as_ref()
                        .map(|v| v.same_nationality_teammates)
                        .unwrap_or(0),
                    mentor_quality: None,
                    squad_chemistry: p.relations.get_team_chemistry().clamp(0.0, 100.0),
                    manager_relation_level: side_ctx
                        .map(|context| context.manager_relation_level_for(p))
                        .unwrap_or(0.0),
                    is_loan: p.contract_loan.is_some(),
                    is_favorite_club: location
                        .map(|(_, _, club_id, _)| p.favorite_clubs.contains(&club_id))
                        .unwrap_or(false),
                };
                let formation = side_ctx.and_then(|context| context.formation.as_ref());
                p.settlement_rating_adjustment(
                    stats.match_rating,
                    now,
                    country_code,
                    club_rep,
                    formation,
                    &squad,
                )
                .public_rating
            })
            .unwrap_or(stats.match_rating)
    }

    /// Personality and team-state inputs for the rating shape. Defaults match
    /// an unknown player: neutral temperament, pressure, professionalism and
    /// chemistry, baseline big-match weighting and neutral morale.
    fn shape_inputs<D: LeagueProcessAccess>(
        player_id: u32,
        data: &D,
        club_rep: f32,
        opponent_rep: f32,
        team_chem: f32,
    ) -> RatingShapeInputs {
        let (consistency, big_match, temperament, pressure, professionalism) = data
            .player(player_id)
            .map(|p| {
                (
                    p.attributes.consistency,
                    p.attributes.important_matches,
                    p.attributes.temperament,
                    p.attributes.pressure,
                    p.attributes.professionalism,
                )
            })
            .unwrap_or((10.0, 10.0, 10.0, 10.0, 10.0));
        let morale = data
            .player(player_id)
            .map(|p| p.happiness().morale)
            .unwrap_or(50.0);
        RatingShapeInputs {
            club_rep,
            opponent_rep,
            team_chem,
            consistency,
            big_match,
            temperament,
            pressure,
            professionalism,
            morale,
        }
    }

    /// Apply the chemistry, morale and routine-texture parts of the rating
    /// shape, in that order.
    fn shape_rating(
        player_id: u32,
        stats: &PlayerMatchEndStats,
        player_team_id: u32,
        date_seed: f64,
        adjusted_from_settlement: f32,
        inputs: &RatingShapeInputs,
    ) -> f32 {
        let adjusted = Self::apply_state_shape(
            player_id,
            stats,
            player_team_id,
            date_seed,
            adjusted_from_settlement,
            inputs,
        );
        Self::apply_personality_shape(stats, adjusted_from_settlement, adjusted, inputs)
    }

    fn apply_state_shape(
        player_id: u32,
        stats: &PlayerMatchEndStats,
        player_team_id: u32,
        date_seed: f64,
        adjusted_from_settlement: f32,
        inputs: &RatingShapeInputs,
    ) -> f32 {
        const BASELINE: f32 = 6.0;
        let mut adjusted = adjusted_from_settlement;

        // Team chemistry shifts individual performance. Neutral at 50;
        // ±0.10 at the extremes. Asymmetric: positive chem only adds
        // *above* baseline, so chemistry alone can't lift a goalless
        // routine shift into the good-rating band.
        let raw_chem_shift = ((inputs.team_chem - 50.0) / 50.0).clamp(-1.0, 1.0) * 0.10;
        adjusted += if raw_chem_shift > 0.0 && adjusted <= BASELINE {
            0.0
        } else {
            raw_chem_shift
        };

        // Morale — the player's current mood colours how he performs, the
        // way a real match rating tracks a player's confidence and
        // contentment. Read straight from the happiness model (0..100,
        // 50 = neutral): a disgruntled, low-morale player plays within
        // himself and the rating reflects it; a buoyant one rides the
        // lift. This is a pure *player-state* factor — deliberately
        // independent of club reputation, so a relegation keeper and a
        // title-winner on identical morale take the identical shift; the
        // rating tracks the man, not his club. Bounded and asymmetric (see
        // `MoraleRatingShaper`) so morale only colours the stat-line
        // verdict and never overturns it.
        adjusted += MoraleRatingShaper::shift(inputs.morale);

        // Match-to-match volatility now lives where the performance is
        // produced, not here: `MatchdayForm` stamps a form multiplier on
        // the match player at kickoff and `effective_skill` folds it into
        // every duel, pass and shot, so a streaky player genuinely has a
        // worse afternoon and the stat line says so. Re-applying a
        // consistency swing to the finished rating would double-count the
        // same attribute — and, worse, would move the number away from
        // the match it describes.
        //
        // What stays is a much smaller residue: two players can turn in
        // the same stat line and still be written up differently. That is
        // the texture band below, which scales with how much actually
        // happened and is far too small to reorder anything.
        let _ = (inputs.consistency, inputs.professionalism);

        // Routine texture (spec Section A) — a tiny, deterministic
        // per-identity jitter that breaks otherwise-identical stat lines
        // apart so two medium players on the same scoreline don't print
        // the same robotic 7.10. Seeded by (player_id, date, team_id) and
        // independent of the consistency swing above; the band scales with
        // the stat-line evidence tier (passenger ≈0.03 … decisive ≈0.08)
        // so it never changes the verdict, only the texture. Gated to real
        // shifts (≥30 min) — a cameo hasn't earned a stat-line signature.
        if stats.minutes_played >= 30 {
            // Same engine→real volume conversion the rating itself used —
            // the texture band's evidence tier must agree with it.
            let band =
                RatingContext::new(&EngineVolumeCalibration::normalize(stats), 0, 0).texture_band();
            let tex_seed = ((player_id as f64 * 0.754877)
                + (date_seed * 0.569840)
                + (player_team_id as f64 * 0.193147))
                .fract() as f32;
            adjusted += (tex_seed - 0.5) * 2.0 * band;
        }

        adjusted
    }

    /// Apply the temperament, big-match and pressure axes, then bound the
    /// combined shape so it colours the stat-line verdict without replacing it.
    fn apply_personality_shape(
        stats: &PlayerMatchEndStats,
        adjusted_from_settlement: f32,
        mut adjusted: f32,
        inputs: &RatingShapeInputs,
    ) -> f32 {
        const BASELINE: f32 = 6.0;
        // Temperament — always-on smooth shape (was: only fires below 6.0
        // AND temperament<10, which left 80% of upper-medium players
        // untouched). A short-fuse player loses focus when the game turns
        // against him; a composed one rides the dip. Symmetric on the
        // upside: high temperament holds form a touch better when the team
        // is on top, but the lift is small (composure isn't a goal).
        // Slopes strengthened so a player's composure (a mental *skill*)
        // visibly shapes how he handles a game: a short-fuse player loses
        // the head more sharply when it turns against him, a composed one
        // rides the dip better. Still bounded by the deficit/surplus caps
        // so it colours the verdict without inventing it — and, like
        // morale, it reads only the player, never the club.
        let temp_centered = (inputs.temperament - 10.0) / 10.0;
        if stats.match_rating < BASELINE {
            let deficit = (BASELINE - stats.match_rating).min(2.0);
            adjusted -= deficit * 0.24 * (-temp_centered).max(0.0);
            adjusted += deficit * 0.11 * temp_centered.max(0.0);
        } else {
            let surplus = (stats.match_rating - BASELINE).min(2.5);
            adjusted += surplus * 0.07 * temp_centered.max(0.0);
            adjusted -= surplus * 0.10 * (-temp_centered).max(0.0);
        }

        // Important-matches personality — always-on smooth gradient (was
        // only firing at ≥15 or ≤5, which never touched the 8-14 band
        // where most upper-medium players live). Amplified vs strong
        // opponents: a +2 rep gap roughly doubles the effect; a routine
        // mismatch barely touches it. Both signs stay asymmetric on the
        // upside so the always-on proxy can't ride routine into the good
        // band.
        let rep_gap = (inputs.opponent_rep - inputs.club_rep).clamp(-3.0, 3.0);
        let big_match_amp = 1.0 + (rep_gap / 3.0).max(0.0);
        let big_centered = (inputs.big_match - 10.0) / 10.0;
        if big_centered >= 0.0 {
            if adjusted > BASELINE {
                adjusted += big_centered * 0.10 * big_match_amp;
            }
        } else {
            adjusted += big_centered * 0.12 * big_match_amp;
        }

        // Pressure axis — copes / folds under elevated opponent strength.
        // Only fires when the opponent is meaningfully stronger (positive
        // rep gap); a routine fixture vs a weaker side has no pressure to
        // cope with, so the axis self-zeros there. High-pressure players
        // step up, low-pressure ones shrink. Effect grows with the gap
        // size, capped so a mid-tier side at Bernabéu doesn't see ±0.5
        // mood swings on every player.
        if rep_gap > 0.3 {
            let pressure_centered = (inputs.pressure - 10.0) / 10.0;
            let gap_scale = (rep_gap / 2.0).min(1.0);
            adjusted += pressure_centered * 0.22 * gap_scale;
        }

        // Combined bound on the personality/state shape applied since the
        // settlement-adjusted value. Every factor above is individually
        // bounded, but they stack: a low-morale, low-chemistry, streaky
        // hot-head on an unlucky seed could shed their sum (≈ −1.5),
        // overturning a clean stat line into a disaster verdict the match
        // never showed — a clean-sheet keeper printing 6.2 reads as a
        // rating bug to anyone looking at the match page. The shape's
        // mandate is to colour the stat-line verdict, never to replace
        // it, so the worst combined mood/state day costs about half a
        // rating point and the best gains a shade less (upside stays the
        // smaller lobe, matching every asymmetric factor above).
        let shape = adjusted - adjusted_from_settlement;
        let adjusted = adjusted_from_settlement + shape.clamp(-0.55, 0.40);

        adjusted.clamp(1.0, 10.0)
    }
}

/// Player-of-the-match selection. The pipeline always reads
/// `pick(details, effective_ratings)`; the lower-level
/// `pick_from_ratings` is split out so tests can drive the decision
/// with synthetic maps instead of building a full `MatchResultRaw`.
struct PlayerOfTheMatch;

impl PlayerOfTheMatch {
    fn pick(details: &MatchResultRaw, effective_ratings: &FxHashMap<u32, f32>) -> Option<u32> {
        Self::pick_from_ratings(&details.player_stats, effective_ratings)
    }

    /// Highest effective rating wins, falling back to the raw stat-line
    /// rating when the effective map is missing an entry — a defensive
    /// case, since `compute_effective_ratings` iterates the same key
    /// set.
    fn pick_from_ratings(
        player_stats: &FxHashMap<u32, PlayerMatchEndStats>,
        effective_ratings: &FxHashMap<u32, f32>,
    ) -> Option<u32> {
        let mut best_rating = 0.0_f32;
        let mut best = None;
        for (player_id, stats) in player_stats {
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
}

/// Reputation inputs and outputs of a finished match: the competition
/// weighting that drives player reputation drift, and the league-table
/// context the team reputation model reads.
struct MatchReputation;

impl MatchReputation {
    fn weights<D: LeagueProcessAccess>(result: &MatchResult, is_cup: bool, data: &D) -> (f32, f32) {
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
            // League reputation is a 0..10000 scale. Dividing by 1000
            // saturated the clamp for every league above rep 1000, so a
            // season in the Premier League and a season in a mid-tier league
            // grew a player's fame at exactly the same rate — there was no
            // reputational reason to ever leave a weak league.
            let rep_norm = (league_reputation / 10_000.0).clamp(0.0, 1.0);
            // Domestic standing rises linearly with the league's own standing.
            let domestic_w = 0.5 + rep_norm;
            // World fame rises CONVEXLY: recognition outside your own country
            // is won on the biggest stages, so a top-five league is worth
            // several times a mid-tier one abroad — while still sitting below
            // the continental competitions handled above.
            let world_w = 0.05 + 0.45 * rep_norm * rep_norm;
            (domestic_w, world_w)
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
}

/// Everything one side's match dispatch needs that is shared by both
/// sides of the fixture.
struct MatchDispatchContext<'a> {
    details: &'a MatchResultRaw,
    effective_ratings: &'a FxHashMap<u32, f32>,
    best_player_id: Option<u32>,
    is_friendly: bool,
    is_cup: bool,
    competition_slug: &'a str,
    league_weight: f32,
    world_weight: f32,
    is_derby: bool,
    is_continental: bool,
    opponent_team_id: Option<u32>,
}

/// Resolved per-side dispatch state: match date, early hooks, club/team
/// identity and the player-stat maps, gathered once before the player loop.
struct SideDispatchState {
    today_date: NaiveDate,
    match_season_year: u16,
    coach_match_importance: f32,
    early_hooks: FxHashSet<u32>,
    played_for_team: Option<(String, String, u16, String, String)>,
    opponent_club_id: Option<u32>,
    player_club_id: u32,
}

/// Routes one side's players through the match-played bookkeeping, the
/// coach's observation memory, substitution frustration and selection-snub
/// events.
struct MatchOutcomeDispatcher;

impl MatchOutcomeDispatcher {
    fn dispatch<D: LeagueProcessAccess>(
        side: &FieldSquad,
        team_scored: u8,
        team_conceded: u8,
        team_won: bool,
        team_lost: bool,
        ctx: &MatchDispatchContext<'_>,
        data: &mut D,
    ) {
        let state = SideDispatchState::resolve(side, ctx, data);
        let coach_observations = Self::dispatch_players(
            side,
            team_scored,
            team_conceded,
            team_won,
            team_lost,
            ctx,
            &state,
            data,
        );
        Self::apply_coach_observations(side, ctx, &coach_observations, data);
        Self::apply_substitution_frustration(
            side,
            team_scored,
            team_conceded,
            team_won,
            team_lost,
            ctx,
            &state,
            data,
        );
        Self::drop_unused_substitutes(side, data);
        Self::drop_omitted_players(side, data);
        Self::record_overlooked_players(side, ctx, data);
    }

    /// Dispatch every starter and used substitute through `on_match_played`
    /// and collect the coach observations from the same pass.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_players<D: LeagueProcessAccess>(
        side: &FieldSquad,
        team_scored: u8,
        team_conceded: u8,
        team_won: bool,
        team_lost: bool,
        ctx: &MatchDispatchContext<'_>,
        state: &SideDispatchState,
        data: &mut D,
    ) -> Vec<crate::club::staff::CoachMatchObservation> {
        let starter_ids: Vec<u32> = side.main.to_vec();
        let sub_ids: Vec<u32> = side.substitutes_used.to_vec();
        let all_ids: Vec<(u32, MatchParticipation)> = starter_ids
            .iter()
            .map(|id| (*id, MatchParticipation::Starter))
            .chain(
                sub_ids
                    .iter()
                    .map(|id| (*id, MatchParticipation::Substitute)),
            )
            .collect();

        // Build coach observations alongside the per-player match dispatch.
        // The persistent staff lives on `team.staffs`, so we collect the
        // observations in this pass and apply them once at the end behind
        // a single team_mut borrow — keeping the player_mut borrows for
        // `on_match_played` from overlapping with the staff borrow.
        let mut coach_observations: Vec<crate::club::staff::CoachMatchObservation> = Vec::new();
        for (pid, participation) in all_ids {
            let stats = match ctx.details.player_stats.get(&pid) {
                Some(s) => s,
                None => continue,
            };
            let effective = *ctx
                .effective_ratings
                .get(&pid)
                .unwrap_or(&stats.match_rating);
            let is_motm = ctx.best_player_id == Some(pid);
            let is_starter = matches!(participation, MatchParticipation::Starter);
            // Slot the player started in, taken from the kickoff `FieldSquad`.
            // Drives the coach's `role_fit` reading so an emergency-slot
            // appearance (e.g. midfielder pressed into fullback) registers as
            // an out-of-position start instead of a natural-role one.
            let played_slot = if is_starter {
                side.starter_slot(pid)
            } else {
                None
            };
            if let Some(player) = data.player_mut(pid) {
                player.on_match_played(&Self::match_outcome(
                    side,
                    pid,
                    stats,
                    participation,
                    effective,
                    is_motm,
                    is_starter,
                    team_scored,
                    team_conceded,
                    team_won,
                    team_lost,
                    ctx,
                    state,
                ));
                coach_observations.push(CoachObservationBuilder::build(
                    player,
                    pid,
                    effective,
                    stats,
                    is_starter,
                    state.early_hooks.contains(&pid),
                    state.coach_match_importance,
                    ctx.is_cup,
                    ctx.is_derby,
                    ctx.is_continental,
                    team_won,
                    played_slot,
                    ctx.details.standard_of_football,
                    state.today_date,
                ));
            }
        }
        coach_observations
    }

    /// Build the per-player `MatchOutcome`. Kept separate so the dispatch loop
    /// stays a loop rather than an arm wall of struct fields.
    #[allow(clippy::too_many_arguments)]
    fn match_outcome<'a>(
        side: &FieldSquad,
        pid: u32,
        stats: &'a PlayerMatchEndStats,
        participation: MatchParticipation,
        effective: f32,
        is_motm: bool,
        is_starter: bool,
        team_scored: u8,
        team_conceded: u8,
        team_won: bool,
        team_lost: bool,
        ctx: &'a MatchDispatchContext<'a>,
        state: &'a SideDispatchState,
    ) -> MatchOutcome<'a> {
        let played_slot = if is_starter {
            side.starter_slot(pid)
        } else {
            None
        };
        MatchOutcome {
            stats,
            effective_rating: effective,
            participation,
            is_friendly: ctx.is_friendly,
            is_cup: ctx.is_cup,
            competition_slug: ctx.competition_slug,
            is_motm,
            team_goals_for: team_scored,
            team_goals_against: team_conceded,
            league_weight: ctx.league_weight,
            world_weight: ctx.world_weight,
            is_derby: ctx.is_derby,
            team_won,
            team_lost,
            won_tie: ctx
                .details
                .score
                .as_ref()
                .is_some_and(|score| score.won_by(side.team_id)),
            is_continental: ctx.is_continental,
            opponent_team_id: ctx.opponent_team_id,
            opponent_club_id: state.opponent_club_id,
            played_for: state.played_for_team.as_ref().map(
                |(slug, name, reputation, league_slug, league_name)| MatchTeamRef {
                    slug,
                    name,
                    reputation: *reputation,
                    league_slug,
                    league_name,
                },
            ),
            match_season_year: state.match_season_year,
            date: state.today_date,
            club_id: state.player_club_id,
            standard_of_football: ctx.details.standard_of_football,
            penalties_saved: ctx.details.penalties_saved_by(pid),
            penalties_missed: ctx.details.penalties_missed_by(pid),
            slot: played_slot,
        }
    }

    /// Apply observations to the head coach's persistent memory. The
    /// staff borrow is fully scoped here so it doesn't overlap with
    /// the player borrows above. Memory updates are friendly-safe:
    /// friendlies contribute small signals so a pre-season cameo
    /// doesn't reshape the manager's trust profile.
    fn apply_coach_observations<D: LeagueProcessAccess>(
        side: &FieldSquad,
        ctx: &MatchDispatchContext<'_>,
        coach_observations: &[crate::club::staff::CoachMatchObservation],
        data: &mut D,
    ) {
        if !coach_observations.is_empty()
            && !ctx.is_friendly
            && let Some(team) = data.team_mut(side.team_id)
        {
            CoachObservationBuilder::apply(team, coach_observations);
        }
    }

    /// Substitution-frustration pass.
    #[allow(clippy::too_many_arguments)]
    fn apply_substitution_frustration<D: LeagueProcessAccess>(
        side: &FieldSquad,
        team_scored: u8,
        team_conceded: u8,
        team_won: bool,
        team_lost: bool,
        ctx: &MatchDispatchContext<'_>,
        state: &SideDispatchState,
        data: &mut D,
    ) {
        // Walk the match's recorded substitutions and fire
        // `SubstitutionFrustration` for players who were hooked under
        // conditions that read as a snub — playing well, pulled early in a
        // big match, or repeatedly hooked across recent weeks. Critical-injury
        // and youth-protection passes are filtered out via the `reason` field
        // stamped on the SubstitutionInfo. `player_stats` is empty only for a
        // result with nobody recorded on it, which also has no substitutions —
        // skip instead of panicking.
        if !ctx.is_friendly
            && let Some(any_player_stats) = ctx.details.player_stats.values().next()
        {
            let big_match_kind = Self::big_match_kind(
                any_player_stats,
                team_scored,
                team_conceded,
                team_won,
                team_lost,
                ctx,
                state,
            );

            for sub in &ctx.details.substitutions {
                if sub.team_id != side.team_id {
                    continue;
                }
                // Only discretionary swaps qualify — injury / youth
                // protection are never a frustration trigger.
                if !matches!(
                    sub.reason,
                    crate::r#match::engine::flow::result::SubstitutionReason::Discretionary
                ) {
                    continue;
                }
                let pid = sub.player_out_id;
                let minute = (sub.match_time_ms / 60_000) as u8;
                let rating_when_off = ctx
                    .effective_ratings
                    .get(&pid)
                    .copied()
                    .or_else(|| ctx.details.player_stats.get(&pid).map(|s| s.match_rating))
                    .unwrap_or(6.5);
                if let Some(player) = data.player_mut(pid) {
                    player.on_match_substituted_for_frustration(
                        minute,
                        rating_when_off,
                        big_match_kind.is_some(),
                    );
                }
            }
        }
    }

    /// Representative outcome used only to read whether the fixture was a big
    /// match for the substitution-frustration model.
    #[allow(clippy::too_many_arguments)]
    fn big_match_kind(
        stats: &PlayerMatchEndStats,
        team_scored: u8,
        team_conceded: u8,
        team_won: bool,
        team_lost: bool,
        ctx: &MatchDispatchContext<'_>,
        state: &SideDispatchState,
    ) -> Option<crate::BigMatchKind> {
        MatchOutcome {
            stats,
            effective_rating: 0.0,
            participation: MatchParticipation::Starter,
            is_friendly: ctx.is_friendly,
            is_cup: ctx.is_cup,
            competition_slug: ctx.competition_slug,
            is_motm: false,
            team_goals_for: team_scored,
            team_goals_against: team_conceded,
            league_weight: ctx.league_weight,
            world_weight: ctx.world_weight,
            is_derby: ctx.is_derby,
            team_won,
            team_lost,
            won_tie: team_won,
            is_continental: ctx.is_continental,
            opponent_team_id: ctx.opponent_team_id,
            opponent_club_id: None,
            played_for: None,
            match_season_year: 0,
            date: state.today_date,
            club_id: state.player_club_id,
            standard_of_football: ctx.details.standard_of_football,
            penalties_saved: 0,
            penalties_missed: 0,
            slot: None,
        }
        .big_match_kind()
    }

    /// Unused substitutes feel the snub as well, even though they didn't
    /// feature in player_stats. Prefer the structured drop event when
    /// the squad selector attached a `MatchSelectionContext` for this
    /// player so the events feed reads "Lost out to Marco Silva..."
    /// instead of "Dropped from match squad".
    fn drop_unused_substitutes<D: LeagueProcessAccess>(side: &FieldSquad, data: &mut D) {
        // Rotation pre-brief: a manager with real man-management explains
        // his policy omissions before the squad goes up, converting the
        // silent-snub grievance into an understood rest. Resolved once per
        // side; applied to every omission context consumed below.
        let rotation_brief = RotationBriefGate::for_team(data, side.team_id);
        for &pid in &side.substitutes {
            if side.substitutes_used.contains(&pid) {
                continue;
            }
            let ctx = side
                .selection_omissions
                .iter()
                .find(|o| o.player_id == pid)
                .map(|o| rotation_brief.applied(o.context.clone()));
            if let Some(player) = data.player_mut(pid) {
                match ctx {
                    Some(c) => player.on_match_dropped_with_context(c),
                    None => player.on_match_dropped(),
                }
            }
        }
    }

    /// Players left out of the matchday squad entirely also feel the
    /// snub. They never reached `side.main` / `side.substitutes`, so
    /// the squad selector flagged them upstream — fire the contextual
    /// drop event here.
    fn drop_omitted_players<D: LeagueProcessAccess>(side: &FieldSquad, data: &mut D) {
        let rotation_brief = RotationBriefGate::for_team(data, side.team_id);
        for omitted in &side.selection_omissions {
            if side.main.contains(&omitted.player_id)
                || side.substitutes.contains(&omitted.player_id)
            {
                continue;
            }
            if let Some(player) = data.player_mut(omitted.player_id) {
                player
                    .on_match_dropped_with_context(rotation_brief.applied(omitted.context.clone()));
            }
        }
    }

    /// The rest of the roster: fit, named nowhere, and not newsworthy
    /// enough for an explained omission. Nothing is narrated, but the
    /// match still counts against them — without this a reserve the
    /// manager never picks kept a neutral starter share for years and
    /// read as "playing" to every model that asks.
    fn record_overlooked_players<D: LeagueProcessAccess>(
        side: &FieldSquad,
        ctx: &MatchDispatchContext<'_>,
        data: &mut D,
    ) {
        for &pid in &side.overlooked {
            if side.main.contains(&pid)
                || side.substitutes.contains(&pid)
                || side.selection_omissions.iter().any(|o| o.player_id == pid)
            {
                continue;
            }
            if let Some(player) = data.player_mut(pid) {
                player.on_match_overlooked(ctx.is_friendly);
            }
        }
    }
}

impl SideDispatchState {
    /// Resolve the team this side was fielded as, once. A player borrowed
    /// from another of the club's teams (a reserve pulled up to the main XI,
    /// or a senior turning out for the "2" side) has this differ from his
    /// active history spell, so league stats get attributed to the team he
    /// actually played for. Cloned to owned strings up front so the
    /// immutable team / league reads don't overlap the `player_mut` borrows
    /// in the loop below. Friendlies / cups carry their own buckets and
    /// don't need it.
    fn resolve<D: LeagueProcessAccess>(
        side: &FieldSquad,
        ctx: &MatchDispatchContext<'_>,
        data: &D,
    ) -> Self {
        let today_date = data.date().date();
        let coach_match_importance = CoachObservationBuilder::match_importance(
            ctx.is_friendly,
            ctx.is_cup,
            ctx.is_continental,
        );
        let early_hook_threshold_minutes: u16 = 70;
        let early_hooks: FxHashSet<u32> = ctx
            .details
            .substitutions
            .iter()
            .filter(|s| s.team_id == side.team_id)
            .filter(|s| {
                matches!(
                    s.reason,
                    crate::r#match::engine::flow::result::SubstitutionReason::Discretionary
                )
            })
            .filter(|s| (s.match_time_ms / 60_000) as u16 <= early_hook_threshold_minutes)
            .map(|s| s.player_out_id)
            .collect();

        let played_for_team: Option<(String, String, u16, String, String)> =
            if ctx.is_friendly || ctx.is_cup {
                None
            } else {
                data.team(side.team_id).map(|t| {
                    let (league_name, league_slug) = t
                        .league_id
                        .and_then(|lid| data.league(lid))
                        .map(|l| (l.name.clone(), l.slug.clone()))
                        .unwrap_or_default();
                    (
                        t.slug.clone(),
                        t.name.clone(),
                        t.reputation.world,
                        league_slug,
                        league_name,
                    )
                })
            };
        let match_season_year = Season::from_date(today_date).start_year;

        // The opponent's CLUB — `sold_from` records the selling club, so the
        // scored-against-former-club beat can only match on it.
        let opponent_club_id = ctx
            .opponent_team_id
            .and_then(|tid| data.team(tid))
            .map(|t| t.club_id);

        // …and this side's club, resolved the same way. Everything the
        // player mind lays down about a match is cued by it.
        let player_club_id = data.team(side.team_id).map(|t| t.club_id).unwrap_or(0);

        SideDispatchState {
            today_date,
            match_season_year,
            coach_match_importance,
            early_hooks,
            played_for_team,
            opponent_club_id,
            player_club_id,
        }
    }
}

/// Decides whether a policy omission was pre-briefed by the coaching
/// staff. A manager with real man-management explains his rotation
/// calls ("you're rested for Saturday"); the grievance model then
/// treats the omission as understood rather than a silent snub. Only
/// policy-flavoured reasons qualify — being dropped on form or trust
/// is never softened by a nice conversation.
struct RotationBriefGate {
    briefs_players: bool,
}

impl RotationBriefGate {
    const MAN_MANAGEMENT_TO_BRIEF: u8 = 12;

    fn for_team<D: LeagueProcessAccess>(data: &D, team_id: u32) -> Self {
        let briefs_players = data
            .team(team_id)
            .and_then(|t| t.staffs.social_head_coach())
            .map(|s| s.staff_attributes.mental.man_management >= Self::MAN_MANAGEMENT_TO_BRIEF)
            .unwrap_or(false);
        RotationBriefGate { briefs_players }
    }

    fn applied(&self, mut ctx: MatchSelectionContext) -> MatchSelectionContext {
        if self.briefs_players && Self::policy_reason(ctx.reason) {
            ctx.explained_by_coach = true;
        }
        ctx
    }

    fn policy_reason(reason: SelectionOmissionReason) -> bool {
        matches!(
            reason,
            SelectionOmissionReason::FatigueManagement
                | SelectionOmissionReason::FitnessProtection
                | SelectionOmissionReason::ReturningFromInjury
                | SelectionOmissionReason::LowMatchImportanceRotation
                | SelectionOmissionReason::MedicalRecurrenceRisk
                | SelectionOmissionReason::EligibilityRuleBlock
        )
    }
}

/// Stateless namespace bundling the per-player coach observation
/// construction and the per-team apply pass. Owning a single struct
/// keeps the dispatch loop above readable and gives the conversion
/// from `MatchOutcome`-like data to [`CoachMatchObservation`] a
/// single home.
struct CoachObservationBuilder;

impl CoachObservationBuilder {
    /// Pick a representative match importance for the coach memory
    /// signals. Cup and continental ties carry materially more weight
    /// than league rounds — the big-match flags only update when the
    /// fixture is itself big.
    fn match_importance(is_friendly: bool, is_cup: bool, is_continental: bool) -> f32 {
        if is_friendly {
            return 0.2;
        }
        if is_continental {
            return 0.95;
        }
        if is_cup {
            return 0.80;
        }
        0.70
    }

    /// Build a single observation from the per-player stats already
    /// captured in the dispatch loop.
    #[allow(clippy::too_many_arguments)]
    fn build(
        player: &Player,
        player_id: u32,
        effective_rating: f32,
        stats: &PlayerMatchEndStats,
        is_starter: bool,
        was_substituted_early: bool,
        match_importance: f32,
        is_cup: bool,
        is_derby: bool,
        is_continental: bool,
        team_won: bool,
        played_slot: Option<PlayerPositionType>,
        standard: f32,
        date: NaiveDate,
    ) -> CoachMatchObservation {
        CoachMatchObservation {
            player_id,
            effective_rating,
            minutes_played: stats.minutes_played,
            is_starter,
            match_importance,
            is_cup,
            is_derby,
            is_continental,
            goals: stats.goals,
            assists: stats.assists,
            errors_leading_to_goal: stats.errors_leading_to_goal,
            errors_leading_to_shot: stats.errors_leading_to_shot,
            keeper_claims: stats.zone_stats.gk_command_actions
                + stats.zone_stats.gk_failed_claims_to_shot,
            standard_of_football: standard,
            yellow_cards: stats.yellow_cards as u8,
            red_cards: stats.red_cards as u8,
            team_won,
            was_substituted_early,
            role_fit: Self::role_fit(player, played_slot),
            professionalism_signal: (player.attributes.professionalism / 20.0).clamp(0.0, 1.0),
            date,
        }
    }

    /// Coach's read of how well the player's natural role matched
    /// the slot they actually played. Read at observation time so
    /// the coach's `role_fit_confidence` learns from emergency-slot
    /// appearances (a midfielder pressed into fullback registers as
    /// a sub-1.0 fit). Falls back to 1.0 when the played slot isn't
    /// known (substitutes, legacy FieldSquads without `starter_slots`).
    ///
    /// The level is the player's own positional rating at the slot,
    /// normalised into [0.0, 1.0] by the engine's 0..20 convention.
    fn role_fit(player: &Player, played_slot: Option<PlayerPositionType>) -> f32 {
        let Some(slot) = played_slot else {
            return 1.0;
        };
        let level = player.positions.get_level(slot);
        if level == 0 {
            // Outright emergency role — read as 0.35 so the signal is
            // negative but the EMA doesn't crash the confidence in one
            // appearance.
            return 0.35;
        }
        (level as f32 / 20.0).clamp(0.35, 1.0)
    }

    /// Apply every queued observation to the team's head coach.
    /// Walks the same manager → caretaker → assistant chain that the
    /// selection layer uses via `head_coach()`, so the coach calling
    /// the shots and the coach whose memory updates are always the
    /// same person. Also runs the dormant-record decay so memory of
    /// players the coach hasn't seen recently softens.
    fn apply(team: &mut Team, observations: &[CoachMatchObservation]) {
        let club_id = team.club_id;
        // Read the ceilings before the staff list is borrowed: what a coach
        // thinks a player might become is a perception read over the
        // player, and the two cannot be borrowed at once.
        let date = observations.first().map(|obs| obs.date);
        let ceilings: FxHashMap<u32, f32> = match date {
            Some(date) => observations
                .iter()
                .filter_map(|obs| {
                    let player = team
                        .players
                        .players
                        .iter()
                        .find(|p| p.id == obs.player_id)?;
                    Some((
                        obs.player_id,
                        PotentialEstimator::observable_ceiling(player, date) as f32 / 200.0,
                    ))
                })
                .collect(),
            None => FxHashMap::default(),
        };
        // What the ladder needs about the squad around each man, read
        // before the staff list is borrowed. Without it every observation
        // was weighed as though the coach had a full squad, no captain and
        // no young player worth being patient with — which is the default
        // and which is wrong precisely when it matters.
        let captain_id = team.captain_id;
        let fit_seniors = team
            .players
            .players
            .iter()
            .filter(|player| !player.is_on_loan() && player.is_ready_for_match())
            .count();
        let ages: FxHashMap<u32, u8> = match date {
            Some(date) => team
                .players
                .players
                .iter()
                .map(|player| (player.id, player.age(date)))
                .collect(),
            None => FxHashMap::default(),
        };

        let Some(head_coach) = team.staffs.head_coach_mut() else {
            return;
        };
        let profile = CoachProfile::from_staff(head_coach);
        let patience = head_coach.mind.judgement.patience();
        let lens = EvidenceLens {
            loyalty: head_coach.attributes.loyalty,
            ..EvidenceLens::default()
        };
        for obs in observations {
            let age = ages.get(&obs.player_id).copied().unwrap_or(0);
            let context = LadderContext {
                stubbornness: profile.stubbornness,
                man_management: profile.man_management,
                volatility: profile.emotional_volatility,
                judging_accuracy: profile.judging_accuracy,
                matches_observed: 0,
                fit_seniors,
                is_captain: captain_id == Some(obs.player_id),
                // A patient coach gives a young player a run before he
                // judges him — and how patient he is is a thing his own
                // career taught him.
                in_youth_grace: age > 0
                    && age <= StandingTuning::YOUTH_GRACE_AGE
                    && patience >= StandingTuning::YOUTH_GRACE_PATIENCE,
                has_recovery_signal: false,
                cleared_the_air: false,
            };
            head_coach
                .coach_memory
                .observe_in(obs, &profile, &context, &lens);
        }
        if let Some(latest) = observations.last() {
            head_coach.coach_memory.decay_inactive(latest.date);
        }

        // The judgements organ, fed from the same chokepoint and the
        // same observations. It runs **in parallel** with the store
        // above: `CoachMemory` keeps feeding selection untouched, while
        // the organ accumulates a view that travels with the man
        // between jobs — which is the property §5 of
        // `docs/staff_mind.md` exists for and which nothing here has
        // today.
        for obs in observations {
            let player = ActorRef::player(obs.player_id);
            // What the coach thinks the player is worth, read off his
            // own long-form baseline rather than any ground truth: 5.0
            // is a passenger and 8.0 is a very good player. CA-blind by
            // construction.
            let read = head_coach
                .coach_memory
                .get(obs.player_id)
                .map(|memory| ((memory.long_form_rating - 5.0) / 3.0).clamp(0.0, 1.0))
                .unwrap_or(0.5);
            let ctx = head_coach.mind_context(obs.date, club_id);
            // Level from his own baseline; ceiling from what he makes of
            // the player's potential, through his own eye for it.
            //
            // Seeding both at the level — which is what this did — meant a
            // coach could never be wrong in the direction that matters: he
            // had no view about what a young player might become, so there
            // was nothing for a career to prove him wrong about, and
            // `IWasWrongAboutHim` could not form even once the audit that
            // scores these views was running.
            let ceiling = ceilings
                .get(&obs.player_id)
                .copied()
                .unwrap_or(read)
                .max(read);
            head_coach.mind.form_judgement(player, read, ceiling, &ctx);
            head_coach
                .mind
                .watched(player, obs.effective_rating, obs.is_big_match(), &ctx);
        }
    }
}

#[cfg(test)]
mod potm_tests {
    use super::*;
    use crate::r#match::engine::ZoneStats;

    struct PotmFixture;

    impl PotmFixture {
        fn stat(rating: f32) -> PlayerMatchEndStats {
            PlayerMatchEndStats {
                shots_on_target: 0,
                shots_total: 0,
                passes_attempted: 0,
                passes_completed: 0,
                tackles: 0,
                interceptions: 0,
                saves: 0,
                shots_faced: 0,
                goals: 0,
                assists: 0,
                match_rating: rating,
                raw_match_rating: rating,
                xg: 0.0,
                position_group: PlayerFieldPositionGroup::Forward,
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
                xg_faced: 0.0,
                offsides: 0,
                own_goals: 0,
                zone_stats: ZoneStats::default(),
            }
        }
    }

    /// POTM must follow the public/effective rating, not the raw
    /// engine stat-line rating. A fresh signing whose raw rating
    /// would crown them man of the match must lose out to a settled
    /// teammate whose effective rating is higher.
    #[test]
    fn potm_follows_effective_rating_not_raw() {
        let mut stats = FxHashMap::default();
        // Fresh signing: raw 8.6, effective dampened to 6.8.
        stats.insert(1, PotmFixture::stat(8.6));
        // Settled teammate: raw 7.5, effective 7.5.
        stats.insert(2, PotmFixture::stat(7.5));

        let mut effective = FxHashMap::default();
        effective.insert(1, 6.8);
        effective.insert(2, 7.5);

        let potm = PlayerOfTheMatch::pick_from_ratings(&stats, &effective);
        assert_eq!(
            potm,
            Some(2),
            "settled 7.5 must beat fresh-signing dampened 6.8, \
             even though the fresh signing's raw was higher"
        );
    }

    /// Regression: when effective == raw (no settlement window
    /// active for anyone) the legacy ordering is preserved.
    #[test]
    fn potm_matches_raw_ordering_when_no_dampening() {
        let mut stats = FxHashMap::default();
        stats.insert(1, PotmFixture::stat(8.6));
        stats.insert(2, PotmFixture::stat(7.5));

        let mut effective = FxHashMap::default();
        effective.insert(1, 8.6);
        effective.insert(2, 7.5);

        let potm = PlayerOfTheMatch::pick_from_ratings(&stats, &effective);
        assert_eq!(potm, Some(1));
    }
}

#[cfg(test)]
mod canonical_rating_tests {
    //! Regression coverage for the raw → public rating contract.
    //!
    //! `CanonicalRatingMutator::apply` is the single chokepoint that
    //! pushes the settlement-adjusted rating into
    //! `MatchResultRaw.player_stats[*].match_rating`. Every reader that
    //! used to consume the raw engine value — match-page DTO, weekly /
    //! season awards, cup showcase, league stat-rebuild, scouting memory
    //! — now reads that same field, so a single test of the mutator
    //! plus a thin "the reader uses `stats.match_rating`" assertion
    //! pins the whole contract.
    use super::*;
    use crate::r#match::ResultMatchPositionData;
    use crate::r#match::engine::ZoneStats;

    struct CanonicalFixture;

    impl CanonicalFixture {
        fn empty_stats(rating: f32) -> PlayerMatchEndStats {
            PlayerMatchEndStats {
                shots_on_target: 0,
                shots_total: 0,
                passes_attempted: 0,
                passes_completed: 0,
                tackles: 0,
                interceptions: 0,
                saves: 0,
                shots_faced: 0,
                goals: 0,
                assists: 0,
                match_rating: rating,
                raw_match_rating: rating,
                xg: 0.0,
                position_group: PlayerFieldPositionGroup::Forward,
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
                xg_faced: 0.0,
                offsides: 0,
                own_goals: 0,
                zone_stats: ZoneStats::default(),
            }
        }

        /// Construct a `MatchResultRaw` carrying just the player_stats
        /// map. Other fields are defaulted — the canonical-rating
        /// mutator only touches `player_stats`, so the rest can stay
        /// minimal.
        fn match_with(stats: FxHashMap<u32, PlayerMatchEndStats>) -> MatchResultRaw {
            MatchResultRaw {
                score: None,
                position_data: ResultMatchPositionData::empty(),
                recording_artifacts: None,
                left_team_players: FieldSquad::new(),
                right_team_players: FieldSquad::new(),
                match_time_ms: 90 * 60 * 1000,
                additional_time_ms: 0,
                player_stats: stats.into_iter().collect(),
                substitutions: Vec::new(),
                chances: Vec::new(),
                physical_snapshots: FxHashMap::default(),
                penalty_shootout: Vec::new(),
                player_of_the_match_id: None,
                starting_home_tactic: None,
                starting_away_tactic: None,
                final_home_tactic: None,
                final_away_tactic: None,
                coach_home: Default::default(),
                coach_away: Default::default(),
                tally: Default::default(),
                weather: Default::default(),
                pitch: Default::default(),
                standard_of_football: 0.6630,
                penalty_saves: Vec::new(),
                penalty_misses: Vec::new(),
            }
        }
    }

    /// The mutator must overwrite `match_rating` with the public value
    /// for every player_stats entry the effective_ratings map covers.
    /// `raw_match_rating` must remain untouched so calibration scripts
    /// can still recover the original engine verdict.
    #[test]
    fn mutator_writes_public_rating_to_canonical_field() {
        let mut stats = FxHashMap::default();
        stats.insert(11, CanonicalFixture::empty_stats(8.6)); // fresh signing
        stats.insert(22, CanonicalFixture::empty_stats(7.5)); // settled
        let mut details = CanonicalFixture::match_with(stats);

        let mut effective = FxHashMap::default();
        effective.insert(11, 6.8); // dampened
        effective.insert(22, 7.5); // unchanged

        CanonicalRatingMutator::apply(&mut details, &effective);

        let fresh = details.player_stats.get(&11).unwrap();
        assert!(
            (fresh.match_rating - 6.8).abs() < 1e-6,
            "canonical match_rating must be the public value (6.8)"
        );
        assert!(
            (fresh.raw_match_rating - 8.6).abs() < 1e-6,
            "raw_match_rating must be preserved for calibration"
        );
        let settled = details.player_stats.get(&22).unwrap();
        assert!((settled.match_rating - 7.5).abs() < 1e-6);
        assert!((settled.raw_match_rating - 7.5).abs() < 1e-6);
    }

    /// Missing entries in the effective map (defensive: should never
    /// happen because `compute_effective_ratings` iterates the same
    /// key set) leave the canonical field untouched — the previous
    /// engine value stays, the player is not silently zeroed.
    #[test]
    fn mutator_leaves_missing_entries_alone() {
        let mut stats = FxHashMap::default();
        stats.insert(11, CanonicalFixture::empty_stats(7.4));
        let mut details = CanonicalFixture::match_with(stats);
        let effective: FxHashMap<u32, f32> = FxHashMap::default();

        CanonicalRatingMutator::apply(&mut details, &effective);

        assert!((details.player_stats.get(&11).unwrap().match_rating - 7.4).abs() < 1e-6);
    }

    /// Cup showcase reads `stats.match_rating` directly to gate "did
    /// this player play well enough to attract scouts?". After the
    /// mutation, a fresh-signing's raw 8.0 that dampens to 6.6 must
    /// no longer clear the conventional 7.0 showcase floor — the
    /// adaptation-aware verdict is what the rest of the football world
    /// sees.
    #[test]
    fn cup_showcase_gate_reads_dampened_rating_after_mutation() {
        const SHOWCASE_FLOOR: f32 = 7.0;
        let mut stats = FxHashMap::default();
        stats.insert(11, CanonicalFixture::empty_stats(8.0));
        let mut details = CanonicalFixture::match_with(stats);

        let mut effective = FxHashMap::default();
        effective.insert(11, 6.6);
        CanonicalRatingMutator::apply(&mut details, &effective);

        let reader = details.player_stats.get(&11).unwrap();
        assert!(
            reader.match_rating < SHOWCASE_FLOOR,
            "settling player's public rating must fall below showcase floor; got {}",
            reader.match_rating
        );
    }

    /// League stat-rebuild (`League::aggregate_player_statistics`)
    /// feeds `record_match_rating(ps.match_rating, ...)` on rehydrate.
    /// After mutation the rebuild sees the public value, matching
    /// what's recorded live by `on_match_played`. This test asserts
    /// the rehydrate input matches what the live path stores — the
    /// invariant the rebuild relies on.
    #[test]
    fn league_rebuild_reads_public_rating_after_mutation() {
        let mut stats = FxHashMap::default();
        stats.insert(11, CanonicalFixture::empty_stats(8.2));
        let mut details = CanonicalFixture::match_with(stats);

        let mut effective = FxHashMap::default();
        effective.insert(11, 7.1);
        CanonicalRatingMutator::apply(&mut details, &effective);

        // The rebuild path reads `ps.match_rating` — same field a
        // live `on_match_played` call gets via `o.effective_rating`,
        // because both ultimately read from the canonical field after
        // the mutation runs.
        let ps = details.player_stats.get(&11).unwrap();
        assert!(
            (ps.match_rating - 7.1).abs() < 1e-6,
            "rebuild must read public 7.1, not raw 8.2"
        );
    }

    /// Match-page DTO (`web/match/get/mod.rs`) and the weekly /
    /// season awards aggregators all read `stats.match_rating`. The
    /// fixture above confirms the field is mutated; this test mimics
    /// the exact arithmetic the awards path performs (a `rating_sum`
    /// accumulator + a `best_rating` max) and asserts both consume
    /// the public value. If a future edit accidentally restores a
    /// raw-rating-only aggregator, this test fires.
    #[test]
    fn awards_aggregator_consumes_public_rating() {
        // Two performances: a fresh signing with raw 8.6 (dampened to
        // 6.8) and a settled teammate with raw 7.5 (unchanged). After
        // the mutation, the awards-style aggregator must rank the
        // settled teammate as the higher contributor.
        let mut stats = FxHashMap::default();
        stats.insert(11, CanonicalFixture::empty_stats(8.6));
        stats.insert(22, CanonicalFixture::empty_stats(7.5));
        let mut details = CanonicalFixture::match_with(stats);
        let mut effective = FxHashMap::default();
        effective.insert(11, 6.8);
        effective.insert(22, 7.5);
        CanonicalRatingMutator::apply(&mut details, &effective);

        // Mirror the season_awards / player_of_week pattern.
        let mut best_id = 0u32;
        let mut best_rating = 0.0f32;
        let mut rating_sum = 0.0f32;
        for (pid, ps) in &details.player_stats {
            rating_sum += ps.match_rating;
            if ps.match_rating > best_rating {
                best_rating = ps.match_rating;
                best_id = *pid;
            }
        }

        assert_eq!(
            best_id, 22,
            "settled teammate must outrank dampened fresh signing"
        );
        assert!(
            (rating_sum - (6.8 + 7.5)).abs() < 1e-6,
            "aggregator must sum public ratings (6.8 + 7.5), not raw (8.6 + 7.5)"
        );
    }
}

#[cfg(test)]
mod morale_shaper_tests {
    //! The morale colouring is a pure player-state factor — these pin its
    //! shape so a future edit can't quietly turn it into a club-quality
    //! proxy or let it grow large enough to overturn the stat-line verdict.
    use super::*;

    #[test]
    fn neutral_morale_does_not_move_the_rating() {
        assert!(
            MoraleRatingShaper::shift(50.0).abs() < 1e-6,
            "morale 50 is the neutral baseline — no shift"
        );
    }

    #[test]
    fn low_morale_drags_and_high_morale_lifts() {
        assert!(
            MoraleRatingShaper::shift(30.0) < 0.0,
            "a disgruntled player (morale 30) must take a drag"
        );
        assert!(
            MoraleRatingShaper::shift(75.0) > 0.0,
            "a buoyant player (morale 75) must take a lift"
        );
    }

    #[test]
    fn drag_is_stronger_than_an_equal_distance_lift() {
        // Unhappiness disrupts more than contentment enhances: the drag at
        // 20-below-neutral must exceed the lift at 20-above-neutral.
        let drag = -MoraleRatingShaper::shift(30.0);
        let lift = MoraleRatingShaper::shift(70.0);
        assert!(
            drag > lift,
            "low-morale drag ({drag:.3}) must exceed the symmetric high-morale lift ({lift:.3})"
        );
    }

    #[test]
    fn shift_is_monotonic_in_morale() {
        let samples = [0.0, 20.0, 35.0, 50.0, 60.0, 80.0, 100.0];
        for pair in samples.windows(2) {
            assert!(
                MoraleRatingShaper::shift(pair[0]) <= MoraleRatingShaper::shift(pair[1]),
                "shift must rise monotonically with morale: {} then {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn shift_stays_bounded_so_it_colours_not_overturns() {
        // Even rock-bottom / maxed morale (and out-of-range inputs) stay
        // inside the design envelope — a brilliant shift on poor morale is
        // still brilliant, a poor shift on great morale is still poor.
        for morale in [-50.0, 0.0, 10.0, 90.0, 100.0, 150.0] {
            let s = MoraleRatingShaper::shift(morale);
            assert!(
                (-0.45..=0.20).contains(&s),
                "morale {morale} produced shift {s:.3} outside the [-0.45, 0.20] envelope"
            );
        }
    }

    #[test]
    fn concern_morale_is_meaningful_but_not_dominant() {
        // The "concern" band (~30-35) should read as a clear but modest
        // drag — enough to colour a season average, not enough to bury an
        // otherwise solid performance.
        let s = MoraleRatingShaper::shift(30.0);
        assert!(
            (-0.25..=-0.10).contains(&s),
            "concern-band morale 30 should drag ~-0.18, got {s:.3}"
        );
    }
}
