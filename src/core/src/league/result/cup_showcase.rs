//! Domestic-cup breakout scouting.
//!
//! A cup tie is the one fixture where a lower-league or low-reputation
//! player gets measured against a clearly stronger club in front of the
//! whole country. When such a player turns in a standout performance,
//! scouts and recruitment departments should take note — the classic
//! "cup hero" who earns a move on the back of one giant-killing night.
//!
//! This module is the sibling of `match_events::record_match_scouting_memory`:
//! it runs *after* the post-match reputation updates and converts the raw
//! `details.player_stats` of a domestic-cup match into either
//! `ScoutPlayerMonitoring` rows (when a recipient club has real scouting
//! staff) or broad `KnownPlayerMemory` awareness (when it does not).
//!
//! The scoring math is split across small zero-sized namespaces so the
//! trigger thresholds (`Showcase`), the scout-facing numbers
//! (`ShowcaseAssessment`), the recipient roll (`RecipientSelection`), and
//! the per-club capability reads (`ClubScoutingFacts`) stay unit testable
//! without standing up a full `LeagueProcessAccess` world.

use std::cmp::Ordering;

use chrono::NaiveDate;

use super::LeagueResult;
use super::data_access::LeagueProcessAccess;
use crate::Person;
use crate::club::Club;
use crate::club::StaffPosition;
use crate::club::staff::perception::PotentialEstimator;
use crate::r#match::{FieldSquad, MatchResultRaw};
use crate::transfers::pipeline::scouting_config::ScoutingConfig;
use crate::transfers::pipeline::{KnownPlayerMemory, ScoutMonitoringSource, TransferRequestStatus};
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::IntegerUtils;
use crate::{PlayerFieldPositionGroup, PlayerPositionType};

// ============================================================
// Tuning constants
// ============================================================

/// Match rating a weaker-side player must clear before a cup tie counts
/// as a showcase at all. Below this, the performance wasn't a story.
const MIN_SHOWCASE_RATING: f32 = 7.4;
/// World reputation above which a player is "already famous" — appearing
/// in a cup tie isn't a discovery for them, so only clubs already
/// monitoring them get to refresh. (Player reputation is on the same
/// 0..~9500 curve as `derive_reputation_from_ability`, so ~4000 is a
/// well-established top-flight name.)
const FAME_DISCOVERY_CEILING: i16 = 4000;
/// At most this many players per cup tie become showcases — the very best
/// performers only, sorted by showcase score.
const MAX_CANDIDATES_PER_MATCH: usize = 3;
/// At most this many clubs pick up interest in a single showcase player.
const MAX_RECIPIENTS_PER_PLAYER: usize = 4;
/// Beyond the opponent and clubs with a matching transfer need, only this
/// many of the highest-reputation domestic clubs get a roll — keeps a
/// single cup upset from spamming the entire division.
const TOP_REP_POOL: usize = 6;
/// A brand-new monitoring row can't open above this confidence off one
/// match. Repeated showcases (which carry existing monitoring) bypass the
/// cap so a player can climb to `ReportReady` over several cup nights.
const FIRST_MATCH_CONFIDENCE_CAP: f32 = 0.58;

// ============================================================
// Domain enums & inputs
// ============================================================

/// How clear the reputation mismatch was. Drives both candidate
/// eligibility and the underdog flavour of downstream copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShowcaseBand {
    /// `ratio <= 0.75` — a normal underdog showcase.
    Normal,
    /// `ratio <= 0.55` — a strong underdog showcase.
    StrongUnderdog,
    /// `ratio <= 0.40` — a giant-killing showcase.
    GiantKilling,
}

/// Trigger tier derived from the showcase score. Sets the opening
/// confidence of a fresh monitoring row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShowcaseTier {
    Normal,
    Strong,
    Elite,
}

/// The weaker side's result in the tie — feeds `result_bonus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WeakerResult {
    Win,
    Draw,
    NarrowLoss,
    HeavyLoss,
}

/// Match-stat inputs for one weaker-side player, distilled from
/// `details.player_stats` plus a little match context.
struct ShowcaseStatLine {
    match_rating: f32,
    goals: u16,
    assists: u16,
    minutes_played: u16,
    age: u8,
    /// Defender/GK on a clean sheet.
    defensive_clean_sheet: bool,
    is_motm: bool,
    red_card: bool,
    own_goal_or_error: bool,
    weaker_result: WeakerResult,
}

// ============================================================
// Showcase — trigger scoring
// ============================================================

/// The cup-showcase trigger model: turns the reputation gap and a player's
/// stat line into a band, score, and tier.
struct Showcase;

impl Showcase {
    /// Classify the reputation mismatch. `None` means the tie was too even
    /// to produce a lower-league showcase (an equal-reputation top-flight
    /// match never triggers this path).
    fn band_for_ratio(ratio: f32) -> Option<ShowcaseBand> {
        if ratio <= 0.40 {
            Some(ShowcaseBand::GiantKilling)
        } else if ratio <= 0.55 {
            Some(ShowcaseBand::StrongUnderdog)
        } else if ratio <= 0.75 {
            Some(ShowcaseBand::Normal)
        } else {
            None
        }
    }

    /// Reputation-gap reward, scaled to `[0.0, 1.20]`. Bigger gap = bigger
    /// reward for performing against it.
    fn underdog_bonus(player_team_rep: f32, opponent_rep: f32) -> f32 {
        if opponent_rep <= 0.0 {
            return 0.0;
        }
        (((opponent_rep - player_team_rep) / opponent_rep).clamp(0.0, 1.0)) * 1.20
    }

    /// Combine rating, context, and performance into a single showcase
    /// score.
    fn score(line: &ShowcaseStatLine, underdog_bonus: f32) -> f32 {
        let rating_score = ((line.match_rating - 7.0) * 1.25).clamp(-1.0, 2.25);

        let result_bonus = match line.weaker_result {
            WeakerResult::Win => 0.60,
            WeakerResult::Draw => 0.30,
            WeakerResult::NarrowLoss => 0.15,
            WeakerResult::HeavyLoss => 0.0,
        };

        let age_bonus = if line.age <= 21 {
            0.35
        } else if line.age <= 24 {
            0.20
        } else if line.age >= 29 {
            -0.15
        } else {
            0.0
        };

        let mut performance = 0.45 * line.goals as f32 + 0.35 * line.assists as f32;
        if line.defensive_clean_sheet {
            performance += 0.25;
        }
        if line.minutes_played >= 75 {
            performance += 0.20;
        }
        if line.is_motm {
            performance += 0.50;
        }
        if line.red_card {
            performance -= 2.00;
        }
        if line.own_goal_or_error {
            performance -= 1.00;
        }

        rating_score + underdog_bonus + result_bonus + age_bonus + performance
    }

    /// Map a showcase score onto a trigger tier. `None` below the floor.
    fn tier(score: f32) -> Option<ShowcaseTier> {
        if score >= 4.0 {
            Some(ShowcaseTier::Elite)
        } else if score >= 3.2 {
            Some(ShowcaseTier::Strong)
        } else if score >= 2.2 {
            Some(ShowcaseTier::Normal)
        } else {
            None
        }
    }
}

// ============================================================
// ShowcaseAssessment — scout-facing numbers
// ============================================================

/// Translates a triggered showcase into the numbers a scout dossier
/// carries: perceived ability/potential bonuses and the confidence ramp.
struct ShowcaseAssessment;

impl ShowcaseAssessment {
    /// Visible-ability bonus a standout cup rating adds on top of the
    /// scout's position-ability read.
    fn rating_ability_bonus(match_rating: f32) -> i32 {
        if match_rating >= 8.5 {
            12
        } else if match_rating >= 8.0 {
            8
        } else if match_rating >= 7.4 {
            4
        } else {
            0
        }
    }

    /// Small "this kid could be special" lift to perceived potential for
    /// the youngest discoveries.
    fn youth_potential_bonus(age: u8) -> i32 {
        if age <= 21 {
            5
        } else if age <= 24 {
            3
        } else {
            0
        }
    }

    /// Opening confidence for a tier, before any first-match cap or repeat
    /// ramp is applied.
    fn tier_initial_confidence(tier: ShowcaseTier) -> f32 {
        match tier {
            ShowcaseTier::Normal => 0.30,
            ShowcaseTier::Strong => 0.42,
            ShowcaseTier::Elite => 0.52,
        }
    }

    /// Confidence to record this observation with.
    ///
    /// * First sighting (`existing == None`): the tier opener, capped at
    ///   `FIRST_MATCH_CONFIDENCE_CAP` — one cup match creates interest, not
    ///   a meeting-ready dossier.
    /// * Repeat sighting: the existing confidence (or tier opener,
    ///   whichever is higher) lifted by a ramp that grows with prior match
    ///   count, so a player who keeps doing it climbs past
    ///   `MEETING_READY_CONFIDENCE` and becomes `ReportReady`.
    fn confidence(tier: ShowcaseTier, existing: Option<(f32, u16)>) -> f32 {
        let initial = Self::tier_initial_confidence(tier);
        match existing {
            None => initial.min(FIRST_MATCH_CONFIDENCE_CAP),
            Some((current, matches_watched)) => {
                let ramp = 0.10 + 0.04 * matches_watched as f32;
                (current.max(initial) + ramp).min(0.95)
            }
        }
    }
}

// ============================================================
// RecipientSelection — who picks up interest
// ============================================================

/// One candidate recipient club, reduced to the facts the selection needs.
/// Whether the club *acts* is independent of whether it has scouting staff
/// — staff only decides monitoring vs. a known-player memory afterwards.
struct RecipientClub {
    is_opponent: bool,
    has_need: bool,
    already_monitors: bool,
}

/// The recipient model: per-club probability and the capped, priority-
/// ordered selection of who acts on a showcase.
struct RecipientSelection;

impl RecipientSelection {
    /// Probability that a given recipient club acts on a showcase. Clamped
    /// so a single cup tie is never a sure thing and never quite hopeless.
    fn probability(
        score: f32,
        underdog_bonus: f32,
        has_need: bool,
        is_opponent: bool,
        age: u8,
    ) -> f32 {
        let mut p = 0.10 + score * 0.06 + underdog_bonus * 0.15;
        if has_need {
            p += 0.15;
        }
        if is_opponent {
            p += 0.10;
        }
        if age <= 23 {
            p += 0.08;
        }
        p.clamp(0.05, 0.65)
    }

    /// Decide which recipients (by index into `recipients`) act on a
    /// showcase.
    ///
    /// Recipients are visited in priority order (opponent, then clubs with
    /// a matching transfer need, then the highest-reputation clubs). Each
    /// is rolled independently; the first `MAX_RECIPIENTS_PER_PLAYER` that
    /// pass win. Already-famous players only refresh clubs that are already
    /// monitoring them — a household name in a cup tie isn't a discovery.
    fn select(
        score: f32,
        underdog_bonus: f32,
        age: u8,
        is_famous: bool,
        recipients: &[RecipientClub],
        roll: &mut impl FnMut(f32) -> bool,
    ) -> Vec<usize> {
        let mut acted = Vec::new();
        for (i, r) in recipients.iter().enumerate() {
            if acted.len() >= MAX_RECIPIENTS_PER_PLAYER {
                break;
            }
            if is_famous && !r.already_monitors {
                continue;
            }
            let p = Self::probability(score, underdog_bonus, r.has_need, r.is_opponent, age);
            if roll(p) {
                acted.push(i);
            }
        }
        acted
    }
}

// ============================================================
// ClubScoutingFacts — per-club capability reads
// ============================================================

/// Reads the scouting-relevant facts off a `Club` during the read pass.
struct ClubScoutingFacts;

impl ClubScoutingFacts {
    /// Best (highest judging-ability) dedicated Scout / ChiefScout at a
    /// club, or `None` if the club has no real scouting staff. The Manager
    /// fallback used elsewhere deliberately does *not* count here: only a
    /// proper scouting department opens a monitoring row; everyone else
    /// just remembers the player.
    fn best_scout_id(club: &Club) -> Option<u32> {
        club.teams
            .teams
            .iter()
            .flat_map(|t| t.staffs.iter())
            .filter(|s| {
                matches!(
                    s.contract.as_ref().map(|c| &c.position),
                    Some(StaffPosition::Scout) | Some(StaffPosition::ChiefScout)
                )
            })
            .max_by_key(|s| s.staff_attributes.knowledge.judging_player_ability)
            .map(|s| s.id)
    }

    /// Position groups a club currently has an open transfer need for.
    fn active_need_groups(club: &Club) -> Vec<PlayerFieldPositionGroup> {
        let mut groups = Vec::new();
        for request in &club.transfer_plan.transfer_requests {
            if matches!(
                request.status,
                TransferRequestStatus::Pending
                    | TransferRequestStatus::ScoutingActive
                    | TransferRequestStatus::Shortlisted
                    | TransferRequestStatus::Negotiating
            ) {
                let group = request.position.position_group();
                if !groups.contains(&group) {
                    groups.push(group);
                }
            }
        }
        groups
    }
}

// ============================================================
// Orchestrator
// ============================================================

/// Per-candidate facts carried from the read pass into recipient
/// selection and the write pass.
struct ShowcaseCandidate {
    player_id: u32,
    position: PlayerPositionType,
    position_group: PlayerFieldPositionGroup,
    age: u8,
    score: f32,
    tier: ShowcaseTier,
    assessed_ability: u8,
    assessed_potential: u8,
    estimated_value: f64,
    is_injured: bool,
    determination: f32,
    contract_months: i16,
    world_reputation: i16,
    is_famous: bool,
}

/// A monitoring row to write in the mutable pass.
struct MonitorAction {
    club_id: u32,
    scout_id: u32,
    player_id: u32,
    tier: ShowcaseTier,
    assessed_ability: u8,
    assessed_potential: u8,
    estimated_value: f64,
    is_injured: bool,
    determination: f32,
    age: u8,
    contract_months: i16,
    world_reputation: i16,
}

/// A known-player memory to write in the mutable pass.
struct MemoryAction {
    club_id: u32,
    memory: KnownPlayerMemory,
}

impl LeagueResult {
    /// Convert a finished domestic-cup match into breakout scouting
    /// interest. Caller gates this to domestic cups (`is_cup &&
    /// !is_friendly` and not a continental competition); friendlies and
    /// continental cups never reach here.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_domestic_cup_showcase_scouting<D: LeagueProcessAccess>(
        details: &MatchResultRaw,
        data: &mut D,
        now: NaiveDate,
        home_team_id: u32,
        away_team_id: u32,
        home_goals: u8,
        away_goals: u8,
        best_player_id: Option<u32>,
    ) {
        // ── Reputation gap: only a clear underdog tie qualifies ───────
        let home_rep = data
            .team(home_team_id)
            .map(|t| t.reputation.market_value_score() as f32)
            .unwrap_or(0.0);
        let away_rep = data
            .team(away_team_id)
            .map(|t| t.reputation.market_value_score() as f32)
            .unwrap_or(0.0);
        if home_rep <= 0.0 || away_rep <= 0.0 {
            return;
        }

        let (
            weaker_team_id,
            weaker_rep,
            weaker_goals,
            opponent_team_id,
            opponent_rep,
            opponent_goals,
        ) = if home_rep <= away_rep {
            (
                home_team_id,
                home_rep,
                home_goals,
                away_team_id,
                away_rep,
                away_goals,
            )
        } else {
            (
                away_team_id,
                away_rep,
                away_goals,
                home_team_id,
                home_rep,
                home_goals,
            )
        };

        if Showcase::band_for_ratio(weaker_rep / opponent_rep).is_none() {
            return;
        }
        let underdog = Showcase::underdog_bonus(weaker_rep, opponent_rep);

        let weaker_result = if weaker_goals > opponent_goals {
            WeakerResult::Win
        } else if weaker_goals == opponent_goals {
            WeakerResult::Draw
        } else if opponent_goals - weaker_goals == 1 {
            WeakerResult::NarrowLoss
        } else {
            WeakerResult::HeavyLoss
        };
        let clean_sheet = opponent_goals == 0;

        // The weaker side's match squad — only its players can be heroes.
        let weaker_side: &FieldSquad = if details.left_team_players.team_id == weaker_team_id {
            &details.left_team_players
        } else if details.right_team_players.team_id == weaker_team_id {
            &details.right_team_players
        } else {
            return;
        };
        let appeared: Vec<u32> = weaker_side
            .main
            .iter()
            .copied()
            .chain(weaker_side.substitutes_used.iter().copied())
            .collect();
        if appeared.is_empty() {
            return;
        }

        let weaker_club_id = match data.team(weaker_team_id).map(|t| t.club_id) {
            Some(id) => id,
            None => return,
        };
        let opponent_club_id = match data.team(opponent_team_id).map(|t| t.club_id) {
            Some(id) => id,
            None => return,
        };
        let country_id = match data.country_by_club(weaker_club_id).map(|c| c.id) {
            Some(id) => id,
            None => return,
        };

        let mut monitor_actions: Vec<MonitorAction> = Vec::new();
        let mut memory_actions: Vec<MemoryAction> = Vec::new();

        // ── Read pass: one shared borrow of the country resolves
        //    candidates, recipient facts, and the probability rolls.
        {
            let country = match data.country(country_id) {
                Some(c) => c,
                None => return,
            };
            let price_level = country.settings.pricing.price_level;
            let weaker_club = match country.clubs.iter().find(|c| c.id == weaker_club_id) {
                Some(c) => c,
                None => return,
            };
            let (seller_league_rep, seller_club_rep) =
                PlayerValuationCalculator::seller_context(country, weaker_club);
            let weaker_team = match weaker_club
                .teams
                .teams
                .iter()
                .find(|t| t.id == weaker_team_id)
            {
                Some(t) => t,
                None => return,
            };

            // Recipient-club facts, computed once and shared across candidates.
            struct ClubFacts {
                club_id: u32,
                rep: f32,
                scout_id: Option<u32>,
                need_groups: Vec<PlayerFieldPositionGroup>,
            }
            let mut club_facts: Vec<ClubFacts> = Vec::new();
            for club in &country.clubs {
                if club.id == weaker_club_id {
                    continue; // a club doesn't "discover" its own player
                }
                let rep = club
                    .teams
                    .main()
                    .map(|t| t.reputation.market_value_score() as f32)
                    .unwrap_or(0.0);
                club_facts.push(ClubFacts {
                    club_id: club.id,
                    rep,
                    scout_id: ClubScoutingFacts::best_scout_id(club),
                    need_groups: ClubScoutingFacts::active_need_groups(club),
                });
            }

            // Build the candidate set from the weaker side's performers.
            let mut candidates: Vec<ShowcaseCandidate> = Vec::new();
            for pid in &appeared {
                let stats = match details.player_stats.get(pid) {
                    Some(s) => s,
                    None => continue,
                };
                if stats.match_rating < MIN_SHOWCASE_RATING {
                    continue;
                }
                let player = match weaker_team.players.players.iter().find(|p| p.id == *pid) {
                    Some(p) => p,
                    None => continue,
                };
                let position = player.position();
                let position_group = position.position_group();
                let age = player.age(now);

                let defensive_clean_sheet = clean_sheet
                    && matches!(
                        position_group,
                        PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper
                    );
                let line = ShowcaseStatLine {
                    match_rating: stats.match_rating,
                    goals: stats.goals,
                    assists: stats.assists,
                    minutes_played: stats.minutes_played,
                    age,
                    defensive_clean_sheet,
                    is_motm: best_player_id == Some(*pid),
                    red_card: stats.red_cards > 0,
                    own_goal_or_error: stats.own_goals > 0 || stats.errors_leading_to_goal > 0,
                    weaker_result,
                };
                let score = Showcase::score(&line, underdog);
                let tier = match Showcase::tier(score) {
                    Some(t) => t,
                    None => continue,
                };

                let skill_ability = player.skills.calculate_ability_for_position(position) as i32;
                let assessed_ability = (skill_ability
                    + ShowcaseAssessment::rating_ability_bonus(stats.match_rating))
                .clamp(1, 200) as u8;
                // Scouts in the stands can't see biological PA — the
                // assessment anchors on the observable ceiling (visible
                // ability + age/mentals projection) instead.
                let observable_ceiling = PotentialEstimator::observable_ceiling(player, now) as i32;
                let assessed_potential = (observable_ceiling.max(assessed_ability as i32)
                    + ShowcaseAssessment::youth_potential_bonus(age))
                .clamp(1, 200) as u8;

                let estimated_value = PlayerValuationCalculator::calculate_value_with_price_level(
                    player,
                    now,
                    price_level,
                    seller_league_rep,
                    seller_club_rep,
                )
                .amount;

                let contract_months = player
                    .contract
                    .as_ref()
                    .map(|c| {
                        ((c.expiration - now).num_days().max(0) / 30).min(i16::MAX as i64) as i16
                    })
                    .unwrap_or(0);

                candidates.push(ShowcaseCandidate {
                    player_id: *pid,
                    position,
                    position_group,
                    age,
                    score,
                    tier,
                    assessed_ability,
                    assessed_potential,
                    estimated_value,
                    is_injured: player.player_attributes.is_injured,
                    determination: player.skills.mental.determination,
                    contract_months,
                    world_reputation: player.player_attributes.world_reputation,
                    is_famous: player.player_attributes.world_reputation >= FAME_DISCOVERY_CEILING,
                });
            }

            // Strongest showcases first, then cap per match.
            candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
            candidates.truncate(MAX_CANDIDATES_PER_MATCH);

            for cand in &candidates {
                // Order recipients: opponent, then matching-need clubs by
                // reputation, then the remaining top-reputation clubs.
                let mut ordered: Vec<&ClubFacts> = Vec::new();
                if let Some(opp) = club_facts.iter().find(|cf| cf.club_id == opponent_club_id) {
                    ordered.push(opp);
                }
                let mut need: Vec<&ClubFacts> = club_facts
                    .iter()
                    .filter(|cf| {
                        cf.club_id != opponent_club_id
                            && cf.need_groups.contains(&cand.position_group)
                    })
                    .collect();
                need.sort_by(|a, b| b.rep.partial_cmp(&a.rep).unwrap_or(Ordering::Equal));
                ordered.extend(need);
                let mut rest: Vec<&ClubFacts> = club_facts
                    .iter()
                    .filter(|cf| {
                        cf.club_id != opponent_club_id
                            && !cf.need_groups.contains(&cand.position_group)
                    })
                    .collect();
                rest.sort_by(|a, b| b.rep.partial_cmp(&a.rep).unwrap_or(Ordering::Equal));
                rest.truncate(TOP_REP_POOL);
                ordered.extend(rest);

                let recipient_rows: Vec<RecipientClub> = ordered
                    .iter()
                    .map(|cf| {
                        let already_monitors = country
                            .clubs
                            .iter()
                            .find(|c| c.id == cf.club_id)
                            .map(|c| {
                                !c.transfer_plan
                                    .monitorings_for_player(cand.player_id)
                                    .is_empty()
                            })
                            .unwrap_or(false);
                        RecipientClub {
                            is_opponent: cf.club_id == opponent_club_id,
                            has_need: cf.need_groups.contains(&cand.position_group),
                            already_monitors,
                        }
                    })
                    .collect();

                let mut roll = |p: f32| (IntegerUtils::random(0, 10_000) as f32 / 10_000.0) < p;
                let acted = RecipientSelection::select(
                    cand.score,
                    underdog,
                    cand.age,
                    cand.is_famous,
                    &recipient_rows,
                    &mut roll,
                );

                for idx in acted {
                    let cf = ordered[idx];
                    match cf.scout_id {
                        Some(scout_id) => monitor_actions.push(MonitorAction {
                            club_id: cf.club_id,
                            scout_id,
                            player_id: cand.player_id,
                            tier: cand.tier,
                            assessed_ability: cand.assessed_ability,
                            assessed_potential: cand.assessed_potential,
                            estimated_value: cand.estimated_value,
                            is_injured: cand.is_injured,
                            determination: cand.determination,
                            age: cand.age,
                            contract_months: cand.contract_months,
                            world_reputation: cand.world_reputation,
                        }),
                        None => memory_actions.push(MemoryAction {
                            club_id: cf.club_id,
                            memory: KnownPlayerMemory {
                                player_id: cand.player_id,
                                last_known_club_id: weaker_club_id,
                                last_known_country_id: country_id,
                                position: cand.position,
                                position_group: cand.position_group,
                                assessed_ability: cand.assessed_ability,
                                assessed_potential: cand.assessed_potential,
                                confidence: ShowcaseAssessment::tier_initial_confidence(cand.tier),
                                estimated_fee: cand.estimated_value,
                                last_seen: now,
                                official_appearances_seen: 1,
                                friendly_appearances_seen: 0,
                            },
                        }),
                    }
                }
            }
        }

        // ── Write pass: apply monitoring rows and known-player memories.
        let config = ScoutingConfig::default();
        for action in monitor_actions {
            if let Some(club) = data.club_mut(action.club_id) {
                let buyer_world_rep = club
                    .teams
                    .main()
                    .map(|t| t.reputation.world as i16)
                    .unwrap_or(0);
                let risk_flags = config.risk_flags_for(
                    action.is_injured,
                    action.determination,
                    action.age,
                    action.contract_months,
                    action.world_reputation,
                    buyer_world_rep,
                );
                // Repeat showcases bypass the first-match confidence cap and
                // ramp toward ReportReady — read the live row to find out.
                let existing = club
                    .transfer_plan
                    .find_monitoring(action.scout_id, action.player_id)
                    .map(|m| (m.confidence, m.matches_watched));
                let confidence = ShowcaseAssessment::confidence(action.tier, existing);
                club.transfer_plan.upsert_monitoring(
                    action.scout_id,
                    action.player_id,
                    ScoutMonitoringSource::MatchStandout,
                    None,
                    None,
                    None,
                    action.assessed_ability,
                    action.assessed_potential,
                    confidence,
                    1.0,
                    action.estimated_value,
                    risk_flags,
                    now,
                    true,
                );
            }
        }
        for action in memory_actions {
            if let Some(club) = data.club_mut(action.club_id) {
                club.transfer_plan.remember_known_player(action.memory);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_line() -> ShowcaseStatLine {
        ShowcaseStatLine {
            match_rating: 8.0,
            goals: 0,
            assists: 0,
            minutes_played: 90,
            age: 25,
            defensive_clean_sheet: false,
            is_motm: false,
            red_card: false,
            own_goal_or_error: false,
            weaker_result: WeakerResult::NarrowLoss,
        }
    }

    #[test]
    fn band_only_triggers_for_clear_underdogs() {
        assert_eq!(
            Showcase::band_for_ratio(0.40),
            Some(ShowcaseBand::GiantKilling)
        );
        assert_eq!(
            Showcase::band_for_ratio(0.50),
            Some(ShowcaseBand::StrongUnderdog)
        );
        assert_eq!(
            Showcase::band_for_ratio(0.55),
            Some(ShowcaseBand::StrongUnderdog)
        );
        assert_eq!(Showcase::band_for_ratio(0.70), Some(ShowcaseBand::Normal));
        assert_eq!(Showcase::band_for_ratio(0.75), Some(ShowcaseBand::Normal));
        // Equal-reputation / mild-gap top-flight matches never trigger.
        assert_eq!(Showcase::band_for_ratio(0.80), None);
        assert_eq!(Showcase::band_for_ratio(1.0), None);
    }

    #[test]
    fn underdog_bonus_scales_with_gap() {
        // No gap → no bonus.
        assert!(Showcase::underdog_bonus(1000.0, 1000.0).abs() < 1e-6);
        // Heavy gap → near the 1.20 ceiling.
        let big = Showcase::underdog_bonus(200.0, 1000.0);
        assert!((big - 0.96).abs() < 1e-4, "got {big}");
        // Guard against divide-by-zero.
        assert_eq!(Showcase::underdog_bonus(0.0, 0.0), 0.0);
    }

    #[test]
    fn lower_rep_goal_scorer_clears_showcase_threshold() {
        // ratio 0.30 → giant-killing; rep 300 vs 1000.
        let underdog = Showcase::underdog_bonus(300.0, 1000.0); // 0.84
        let line = ShowcaseStatLine {
            match_rating: 8.3,
            goals: 1,
            age: 20,
            weaker_result: WeakerResult::Win,
            ..base_line()
        };
        let score = Showcase::score(&line, underdog);
        // rating 1.625 + underdog 0.84 + win 0.60 + age 0.35 + goal 0.45 + 75' 0.20
        assert!(score >= 4.0, "expected elite, got {score}");
        assert_eq!(Showcase::tier(score), Some(ShowcaseTier::Elite));
    }

    #[test]
    fn poor_rating_produces_no_showcase() {
        // 6.9 is below the candidate gate entirely.
        assert!(6.9 < MIN_SHOWCASE_RATING);
        // And even a 6.9 line in a giant-killing win stays below the floor.
        let underdog = Showcase::underdog_bonus(300.0, 1000.0);
        let line = ShowcaseStatLine {
            match_rating: 6.9,
            weaker_result: WeakerResult::Win,
            age: 20,
            ..base_line()
        };
        assert_eq!(Showcase::tier(Showcase::score(&line, underdog)), None);
    }

    #[test]
    fn red_card_and_own_goal_sink_the_score() {
        let underdog = Showcase::underdog_bonus(300.0, 1000.0);
        let line = ShowcaseStatLine {
            match_rating: 8.3,
            goals: 1,
            red_card: true,
            own_goal_or_error: true,
            weaker_result: WeakerResult::Win,
            age: 20,
            ..base_line()
        };
        // The -2.00 red and -1.00 error pull a strong night under the floor.
        assert_eq!(Showcase::tier(Showcase::score(&line, underdog)), None);
    }

    #[test]
    fn assessment_bonuses_follow_rating_and_age() {
        assert_eq!(ShowcaseAssessment::rating_ability_bonus(7.4), 4);
        assert_eq!(ShowcaseAssessment::rating_ability_bonus(7.99), 4);
        assert_eq!(ShowcaseAssessment::rating_ability_bonus(8.0), 8);
        assert_eq!(ShowcaseAssessment::rating_ability_bonus(8.49), 8);
        assert_eq!(ShowcaseAssessment::rating_ability_bonus(8.5), 12);
        assert_eq!(ShowcaseAssessment::rating_ability_bonus(7.0), 0);

        assert_eq!(ShowcaseAssessment::youth_potential_bonus(21), 5);
        assert_eq!(ShowcaseAssessment::youth_potential_bonus(24), 3);
        assert_eq!(ShowcaseAssessment::youth_potential_bonus(25), 0);
    }

    #[test]
    fn recipient_probability_is_clamped_and_layered() {
        // Floor.
        let low = RecipientSelection::probability(0.0, 0.0, false, false, 30);
        assert!(low >= 0.05 && low <= 0.65);
        // Ceiling — a huge elite score with every bonus saturates at 0.65.
        let high = RecipientSelection::probability(6.0, 1.20, true, true, 20);
        assert!((high - 0.65).abs() < 1e-6, "got {high}");
        // Each layer adds interest.
        let plain = RecipientSelection::probability(2.5, 0.5, false, false, 30);
        let with_need = RecipientSelection::probability(2.5, 0.5, true, false, 30);
        assert!(with_need > plain);
        let with_opp = RecipientSelection::probability(2.5, 0.5, false, true, 30);
        assert!(with_opp > plain);
        let with_youth = RecipientSelection::probability(2.5, 0.5, false, false, 22);
        assert!(with_youth > plain);
    }

    #[test]
    fn confidence_caps_first_match_then_ramps_to_report_ready() {
        // First sighting of an elite showcase opens at the tier value and
        // never above the first-match cap.
        let first = ShowcaseAssessment::confidence(ShowcaseTier::Elite, None);
        assert!((first - 0.52).abs() < 1e-6);
        assert!(first <= FIRST_MATCH_CONFIDENCE_CAP);
        // A repeat sighting lifts existing confidence past the meeting
        // threshold (0.6).
        let repeat = ShowcaseAssessment::confidence(ShowcaseTier::Elite, Some((0.52, 1)));
        assert!(repeat >= 0.6, "got {repeat}");
        // Normal-tier repeats climb more slowly but still get there.
        let r1 = ShowcaseAssessment::confidence(ShowcaseTier::Normal, None); // 0.30
        let r2 = ShowcaseAssessment::confidence(ShowcaseTier::Normal, Some((r1, 1))); // ~0.44
        let r3 = ShowcaseAssessment::confidence(ShowcaseTier::Normal, Some((r2, 2))); // ~0.62
        assert!(r2 < 0.6 && r3 >= 0.6, "r2 {r2}, r3 {r3}");
    }

    fn recipient(is_opponent: bool, has_need: bool, monitors: bool) -> RecipientClub {
        RecipientClub {
            is_opponent,
            has_need,
            already_monitors: monitors,
        }
    }

    #[test]
    fn recipient_selection_caps_at_four() {
        let recipients: Vec<RecipientClub> =
            (0..10).map(|_| recipient(false, false, false)).collect();
        let mut always = |_p: f32| true;
        let acted = RecipientSelection::select(5.0, 1.0, 20, false, &recipients, &mut always);
        assert_eq!(acted.len(), MAX_RECIPIENTS_PER_PLAYER);
    }

    #[test]
    fn famous_player_only_refreshes_existing_monitors() {
        // Opponent (no existing monitoring) + a club that already monitors.
        let recipients = vec![
            recipient(true, false, false), // opponent, not monitoring
            recipient(false, false, true), // already monitors
        ];
        let mut always = |_p: f32| true;
        let acted = RecipientSelection::select(5.0, 1.0, 20, true, &recipients, &mut always);
        // Only the already-monitoring club acts.
        assert_eq!(acted, vec![1]);
    }

    #[test]
    fn non_famous_player_can_be_freshly_discovered() {
        let recipients = vec![recipient(true, false, false), recipient(false, true, false)];
        let mut always = |_p: f32| true;
        let acted = RecipientSelection::select(5.0, 1.0, 20, false, &recipients, &mut always);
        assert_eq!(acted, vec![0, 1]);
    }

    #[test]
    fn recipients_respect_the_roll_outcome() {
        let recipients = vec![recipient(true, false, false)];
        let mut never = |_p: f32| false;
        let acted = RecipientSelection::select(5.0, 1.0, 20, false, &recipients, &mut never);
        assert!(acted.is_empty());
    }

    /// End-to-end of the apply step against a real `ClubTransferPlan`:
    /// the first showcase opens a capped `MatchStandout` row; a second
    /// showcase of the same player by the same scout ramps confidence
    /// past the meeting threshold and flips the row to `ReportReady`.
    #[test]
    fn repeated_cup_showcase_creates_then_promotes_monitoring() {
        use crate::transfers::pipeline::{
            ClubTransferPlan, ScoutMonitoringStatus, ScoutPlayerMonitoring,
        };

        let date = NaiveDate::from_ymd_opt(2026, 5, 27).unwrap();
        let mut plan = ClubTransferPlan::new();
        let scout = 7u32;
        let player = 42u32;

        let apply = |plan: &mut ClubTransferPlan| {
            let existing = plan
                .find_monitoring(scout, player)
                .map(|m| (m.confidence, m.matches_watched));
            let confidence = ShowcaseAssessment::confidence(ShowcaseTier::Elite, existing);
            plan.upsert_monitoring(
                scout,
                player,
                ScoutMonitoringSource::MatchStandout,
                None,
                None,
                None,
                120,
                130,
                confidence,
                1.0,
                1_000_000.0,
                vec![],
                date,
                true,
            );
        };

        // First cup showcase → a fresh MatchStandout row, capped + Active.
        apply(&mut plan);
        {
            let m = plan.find_monitoring(scout, player).unwrap();
            assert_eq!(m.source, ScoutMonitoringSource::MatchStandout);
            assert_eq!(m.matches_watched, 1);
            assert!(m.confidence <= FIRST_MATCH_CONFIDENCE_CAP);
            assert_eq!(m.status, ScoutMonitoringStatus::Active);
        }

        // Second cup showcase → same row, more matches watched, ReportReady.
        apply(&mut plan);
        let m = plan.find_monitoring(scout, player).unwrap();
        assert_eq!(
            plan.scout_monitoring.len(),
            1,
            "must not spawn a duplicate row"
        );
        assert_eq!(m.matches_watched, 2);
        assert!(m.confidence >= ScoutPlayerMonitoring::MEETING_READY_CONFIDENCE);
        assert_eq!(m.status, ScoutMonitoringStatus::ReportReady);
    }
}
