mod balance;
mod bench_scenarios;
mod competitive;
pub(crate) mod helpers;
mod lines;
pub mod model;
mod omissions;
mod role_duty;
mod room;
mod rotation;
pub(crate) mod scoring;

#[cfg(test)]
mod tests;

use crate::club::staff::goalkeeping::KeeperSelectionBrief;
use crate::club::staff::{
    CoachDecisionEngine, CoachProfile, CoachSquadPlan, CoachStrategy, PlanReading, PlannedRole,
    ShareClimate, StrategyDeriver, StrategyInputs,
};
use crate::club::team::ClubWeek;
use crate::league::LeagueRegulations;
use crate::club::{ClubPhilosophy, PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::{Person, Player, Tactics, Team, TeamType};
use chrono::NaiveDate;
use log::debug;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;

use helpers::*;
use model::{EligibilityDecision, EligibilityEvaluator, FixtureEligibility, MatchSelectionGameModel};
use rotation::{DevelopmentSelection, DevelopmentStakes, MatchInvolvement};
use scoring::ScoringEngine;

use chrono::Utc;
pub use omissions::OmittedPlayer;
pub use lines::PitchLine;
pub use room::FixtureRoom;

pub struct SquadSelector;

pub struct PlayerSelectionResult {
    pub main_squad: Vec<MatchPlayer>,
    pub substitutes: Vec<MatchPlayer>,
    /// Important omissions — players the selector chose not to start or
    /// not to include in the matchday squad, where the omission deserves
    /// a structured explanation in the player-events feed (KeyPlayers
    /// missed, regulars demoted, force-selected players overlooked, …).
    /// Empty when nothing notable happened.
    pub omissions: Vec<OmittedPlayer>,
    /// Every fit player on the selecting team's own roster who was not
    /// named in the XI or on the bench — newsworthy or not. The
    /// omissions above explain the notable cases; this is the plain
    /// register the playing-time bookkeeping reads, so a reserve nobody
    /// ever picks still accrues the matches he was overlooked for.
    pub overlooked: Vec<u32>,
}

pub struct SelectionContext {
    pub is_friendly: bool,
    pub date: NaiveDate,
    /// What the fixture is worth to this side: 0.0 nothing, 1.0 a must-win.
    pub match_importance: f32,
    /// Club philosophy — tilts the starting XI selection. Populated by
    /// the match-day caller so a DevelopAndSell side actually puts the
    /// kids on the pitch in league games.
    pub philosophy: Option<ClubPhilosophy>,
    /// Which competition the match belongs to. Lets the selector apply a
    /// domestic-cup opportunity bias (rotate hard in early rounds, field a
    /// strong XI in semis/finals) instead of inferring everything from a
    /// `knockout` bool.
    pub competition: SelectionCompetition,
    /// Optional richer fixture model — opponent profile, environment,
    /// competition rules, squad state, coach policy. When `None` the
    /// selector synthesises a neutral default so existing callers
    /// behave exactly as before; richer match-day callers populate
    /// this with real opponent / weather / registration data.
    pub game_model: Option<MatchSelectionGameModel>,
    /// Development guests — ids of underutilized players visiting from an
    /// older club squad that has no fixtures of its own (a league-less U20
    /// side). Unlike ordinary reserve supplements, which are borrowed only
    /// when the roster can't fill a matchday squad, guests always join the
    /// rotation candidate pool: the whole point of the visit is match
    /// practice. They carry a fixed modest standing in the minutes plan
    /// instead of a season share (see `DevelopmentPlan`), so they break
    /// into the XI only when the team's own players are on or ahead of
    /// their planned appearances. Only the rotation selector reads this.
    pub development_guest_ids: Vec<u32>,
    /// The selecting team's actual played-match count this season, read
    /// from its league table by the matchday caller. The development
    /// minutes plan measures appearance deficits against the season
    /// length; when this is absent it falls back to the busiest-player
    /// estimate, which under-reads in an evenly rotated squad (the
    /// busiest player starts only about half the fixtures) — and against
    /// a too-short season own players build deficits too slowly to evict
    /// visiting guests. Only the rotation selector reads this.
    pub season_matches_played: Option<f32>,
    /// The goalkeeping department's word for this fixture — who the
    /// declared number one is, who the deputy is, and whether the
    /// specialist has asked for a particular keeper to be played. `None`
    /// when the club has never reviewed its keeper room, in which case
    /// goalkeeper selection behaves exactly as it did before the
    /// department existed. See [`crate::club::staff::goalkeeping`].
    pub keeper_brief: Option<KeeperSelectionBrief>,
    /// The club's week across its competitions.
    pub week: ClubWeek,
    /// How far football owed may move a shirt in this fixture, as the
    /// matchday caller read it from the stakes, the week and the coach.
    /// `None` reads the stakes alone.
    pub room: Option<f32>,
    /// The facts of the fixture a player's eligibility is asked against.
    pub eligibility: FixtureEligibility,
    /// Substitutes the competition lets a side name.
    pub bench_size: usize,
}

/// What a side's selection was told about the fixture, kept on the team
/// sheet so a result can be read against it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SelectionReading {
    pub importance: f32,
    pub room: f32,
    pub days_to_next: Option<u16>,
    pub next_importance: f32,
}

impl SelectionContext {
    pub fn reading(&self) -> SelectionReading {
        SelectionReading {
            importance: self.match_importance,
            room: self.room(),
            days_to_next: self.week.to_next.map(|days| days.clamp(0, 999) as u16),
            next_importance: self.week.next_importance,
        }
    }

    pub fn room(&self) -> f32 {
        self.room
            .unwrap_or(1.0 - self.match_importance)
            .clamp(0.0, 1.0)
    }

    /// How little room the fixture leaves.
    pub fn pressure(&self) -> f32 {
        1.0 - self.room()
    }
}

/// Competition context for squad selection. Replaces inferring the cup
/// rotation policy from a bare `knockout` flag — a `DomesticCup` tie
/// carries its bracket position and the two sides' reputations so the
/// selector can tell an early-round romp from a final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionCompetition {
    League,
    DomesticCup {
        /// 1-based round index within the bracket.
        round: u8,
        /// Total rounds needed to resolve the bracket (final == total).
        total_rounds: u8,
        /// Selecting side's reputation (market-value score).
        own_reputation: u16,
        /// Opponent's reputation (market-value score).
        opponent_reputation: u16,
    },
    ContinentalCup,
    Friendly,
}

impl Default for SelectionContext {
    fn default() -> Self {
        SelectionContext {
            is_friendly: false,
            date: Utc::now().date_naive(),
            match_importance: 0.7,
            philosophy: None,
            competition: SelectionCompetition::League,
            game_model: None,
            development_guest_ids: Vec::new(),
            season_matches_played: None,
            keeper_brief: None,
            week: ClubWeek::free(),
            room: None,
            eligibility: FixtureEligibility::default(),
            bench_size: usize::from(LeagueRegulations::BENCH_LOWER),
        }
    }
}

impl SquadSelector {
    /// Travelling as emergency cover does not automatically qualify a player
    /// to start. Protect the XI from development/status bonuses crossing a
    /// large ability gap, while retaining enough cover to field eleven.
    fn starting_pool<'a>(
        team: &Team,
        available: &[&'a Player],
        ctx: &SelectionContext,
        tactics: &Tactics,
    ) -> Vec<&'a Player> {
        if team.team_type != TeamType::Main || ctx.is_friendly {
            return available.to_vec();
        }
        let roster = team.players.players();
        let own_ids: FxHashSet<u32> = roster.iter().map(|p| p.id).collect();
        let band = (25.0 - 15.0 * ctx.pressure()).round() as u8;
        let mut starting = Vec::with_capacity(available.len());
        let mut emergency = Vec::new();
        for &player in available {
            if own_ids.contains(&player.id)
                || player.positions.is_goalkeeper()
                || player.is_force_match_selection
                || CallUpReadiness::meets_level(player, &roster, band)
            {
                starting.push(player);
            } else {
                emergency.push(player);
            }
        }
        let outfield = starting
            .iter()
            .filter(|p| !p.positions.is_goalkeeper())
            .count();
        // Without a keeper an outfielder must cover goal as well.
        let keeper_slots = usize::from(starting.iter().any(|p| p.positions.is_goalkeeper()));
        let needed = (DEFAULT_SQUAD_SIZE - keeper_slots).saturating_sub(outfield);
        emergency.sort_by(|a, b| {
            let merit = |p: &Player| {
                let slot = SlotFit::best_tactical_position(p, tactics);
                CallUpReadiness::level(p) as f32 * SlotFit::position_fit_score(p, slot) / 20.0
            };
            merit(b).total_cmp(&merit(a)).then_with(|| a.id.cmp(&b.id))
        });
        starting.extend(emergency.into_iter().take(needed));
        starting
    }

    /// The fixture's blocks apply before counting depth or borrowing
    /// reserves, and also to emergency keepers and manager-pinned players.
    fn eligible_under_rules(player: &Player, ctx: &SelectionContext) -> bool {
        !matches!(
            EligibilityEvaluator::evaluate(player, &ctx.eligibility, ctx.date, ctx.is_friendly),
            EligibilityDecision::HardBlocked { .. } | EligibilityDecision::Withheld { .. }
        )
    }

    /// A side that cannot name eleven with a goalkeeper among them takes
    /// back the men it was withholding.
    fn readmit_withheld<'a>(
        candidates: impl Iterator<Item = &'a Player>,
        ctx: &SelectionContext,
        available: &mut Vec<&'a Player>,
        available_ids: &mut FxHashSet<u32>,
    ) {
        let can_name_eleven = available.len() >= DEFAULT_SQUAD_SIZE
            && available.iter().any(|p| p.positions.is_goalkeeper());
        if can_name_eleven {
            return;
        }
        for player in candidates {
            let withheld = matches!(
                EligibilityEvaluator::evaluate(player, &ctx.eligibility, ctx.date, ctx.is_friendly),
                EligibilityDecision::Withheld { .. }
            );
            let fit = PlayerAvailability::is_available(player, ctx.is_friendly)
                || KeeperAvailability::is_fallback_available(player, ctx.is_friendly);
            if withheld && fit && available_ids.insert(player.id) {
                available.push(player);
            }
        }
    }

    // ========== PUBLIC API ==========

    pub fn select(team: &Team, staff: &Staff) -> PlayerSelectionResult {
        Self::select_with_reserves(team, staff, &[])
    }

    pub fn select_with_reserves(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
    ) -> PlayerSelectionResult {
        Self::select_with_context(team, staff, reserve_players, &SelectionContext::default())
    }

    pub fn select_with_context(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
        ctx: &SelectionContext,
    ) -> PlayerSelectionResult {
        let tactics = team.tactics();
        Self::select_with_tactics_context(team, staff, reserve_players, tactics.borrow(), ctx)
    }

    pub fn select_with_tactics_context(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
        tactics: &Tactics,
        ctx: &SelectionContext,
    ) -> PlayerSelectionResult {
        let is_main_team = team.team_type == TeamType::Main;
        let mut engine = ScoringEngine::from_staff_for_team(staff, is_main_team);
        engine.owning_roster = Some(team.players.iter().map(|p| p.id).collect());

        // Coach perception profile — read-only lens, derived once.
        let coach_profile = CoachProfile::from_staff(staff);

        // Force-selection is a Main-team pin: a flagged player is committed
        // to the senior XI, so non-Main squads must drop them from every
        // entry path — both their own roster (a B-team player flagged for
        // the first team) and the cross-team reserve pool.
        //
        // Track membership in a side `FxHashSet<u32>` so reserve / keeper-
        // fallback duplicate checks are O(1) instead of O(n) per probe.
        let team_player_count = team.players.players().len();
        let estimated = team_player_count + reserve_players.len();
        let mut available: Vec<&Player> = Vec::with_capacity(estimated);
        let mut available_ids: FxHashSet<u32> =
            FxHashSet::with_capacity_and_hasher(estimated, Default::default());
        for &p in team.players.players().iter() {
            if PlayerAvailability::is_available(p, ctx.is_friendly)
                && Self::eligible_under_rules(p, ctx)
                && (is_main_team || !p.is_force_match_selection)
                && available_ids.insert(p.id)
            {
                available.push(p);
            }
        }

        // Registration, not selection: a side in an age-limited competition
        // cannot name a senior in it, however short of bodies it is and
        // whichever squad he was borrowed from. The cap applies to the
        // borrowed pool only — a squad's own roster is the club's own
        // registration decision, kept by the passes that build it.
        let registration_cap = team.team_type.registration_age_cap();
        for &rp in reserve_players {
            if !is_main_team && rp.is_force_match_selection {
                continue;
            }
            if registration_cap.is_some_and(|cap| rp.age(ctx.date) > cap) {
                continue;
            }
            if PlayerAvailability::is_available(rp, ctx.is_friendly)
                && Self::eligible_under_rules(rp, ctx)
                && available_ids.insert(rp.id)
            {
                available.push(rp);
            }
        }

        // Keeper-fallback recovery. If the normal `is_available` filter
        // excluded every goalkeeper on the roster (all injured or on
        // international duty or low-condition), we'd then press an
        // outfielder into goal — but an outfielder has `Goalkeeping`
        // defaulted to zero, so their save rate is effectively 0% and
        // the team concedes 8-12 goals repeatably. Real football: if
        // all first-team keepers are unavailable, a walking-wounded /
        // low-condition keeper still starts over an outfielder. Add
        // them back here, skipping only keepers who physically can't
        // play (actively banned or absent).
        let has_fit_keeper = available.iter().any(|p| p.positions.is_goalkeeper());
        if !has_fit_keeper {
            for p in team.players.players().iter().copied() {
                // Re-admit only keepers who can physically play. The shared
                // helper skips injured / international-duty keepers and, in a
                // competitive match, banned ones — a low condition is the only
                // reason it relaxes. Force-selected players in non-Main squads
                // stay dropped (the Main-team pin is honoured elsewhere).
                if !KeeperAvailability::is_fallback_available(p, ctx.is_friendly)
                    || !Self::eligible_under_rules(p, ctx)
                {
                    continue;
                }
                if !is_main_team && p.is_force_match_selection {
                    continue;
                }
                if !available_ids.insert(p.id) {
                    continue;
                }
                available.push(p);
            }
        }

        Self::readmit_withheld(
            team.players
                .players()
                .into_iter()
                .filter(|p| is_main_team || !p.is_force_match_selection),
            ctx,
            &mut available,
            &mut available_ids,
        );

        let outfield_count = available
            .iter()
            .filter(|p| !p.positions.is_goalkeeper())
            .count();
        let gk_count = available.len() - outfield_count;

        if available.len() < DEFAULT_SQUAD_SIZE {
            let all_players = team.players.players();
            // Census splits genuine unavailability from market / near-transfer
            // status. The latter never blocks selection, so it's reported on a
            // separate line — a short squad is an availability problem, not a
            // transfer-list one.
            let census = SelectionStatusCensus::of(&all_players, ctx.is_friendly);

            log::debug!(
                "Squad selection for team {}: only {} available out of {} registered. \
                {} unavailable (blocks selection) — injured={}, international={}, low_condition={}, banned={}. \
                {} carry a market/near-transfer status (still selectable) — listed={}, loan_listed={}, requested={}, unhappy={}, bid_accepted={}, agreed_transfer={}. \
                ({} outfield, {} GK, {} reserves offered)",
                team.name,
                available.len(),
                all_players.len(),
                census.unavailable_total(),
                census.injured,
                census.international,
                census.low_condition,
                census.banned,
                census.market_total(),
                census.listed,
                census.loan_listed,
                census.requested,
                census.unhappy,
                census.bid_accepted,
                census.agreed_transfer,
                outfield_count,
                gk_count,
                reserve_players.len()
            );
        } else if available.len() < DEFAULT_SQUAD_SIZE + ctx.bench_size {
            debug!(
                "Squad selection for team {}: only {} available (need {} for full squad+bench)",
                team.name,
                available.len(),
                DEFAULT_SQUAD_SIZE + ctx.bench_size
            );
        }

        // Borrow the richer game model only when the caller actually
        // populated `ctx.game_model`. Conservative default: leave it
        // `None` so the new bounded additive terms collapse to zero —
        // existing callers see the same scoring they always have.
        // Match-day callers build a fixture-aware model with
        // `MatchSelectionGameModel::build_for_fixture` and store it on
        // the context before invoking the selector.
        let game_model_ref: Option<&MatchSelectionGameModel> = ctx.game_model.as_ref();

        // Coach decision engine — built once per side, after
        // `available` so the strategy reads real squad-depth and any
        // available cup-context signals (opponent reputation /
        // strength ratio). Personality + strategy is the lens; the
        // existing scoring engine still does the heavy lifting. The
        // fixture game model, when present, supplies the real opponent
        // strength ratio and the derby read; the cup-bracket fallback
        // keeps legacy callers unchanged.
        let strength_ratio = game_model_ref
            .map(|m| m.opponent_profile.strength_ratio)
            .unwrap_or_else(|| StrategyInputsBuilder::strength_ratio(&ctx.competition));
        let squad_depth = StrategyInputsBuilder::squad_depth(available.len(), ctx.bench_size);
        let coach_strategy = StrategyDeriver::derive(&StrategyInputs {
            profile: &coach_profile,
            philosophy: ctx.philosophy,
            match_importance: ctx.pressure(),
            is_friendly: ctx.is_friendly,
            is_cup: matches!(
                ctx.competition,
                SelectionCompetition::DomesticCup { .. } | SelectionCompetition::ContinentalCup
            ),
            is_continental: matches!(ctx.competition, SelectionCompetition::ContinentalCup),
            is_derby: game_model_ref.map(|m| m.is_derby()).unwrap_or(false),
            strength_ratio,
            squad_depth,
        });
        let coach_engine = CoachDecisionEngine::from_staff(staff, &coach_profile, coach_strategy)
            .with_owning_roster(engine.owning_roster.as_ref().unwrap());

        let succession_heirs: Vec<u32> = available
            .iter()
            .filter(|p| staff.squad_plan.role_of(p.id) == Some(PlannedRole::SuccessionHeir))
            .map(|p| p.id)
            .collect();

        let scx = competitive::SelectionScoringContext {
            staff,
            tactics,
            engine: &engine,
            date: ctx.date,
            is_friendly: ctx.is_friendly,
            match_importance: ctx.pressure(),
            stakes: ctx.match_importance,
            room: ctx.room(),
            bench_size: ctx.bench_size,
            coach: Some(&coach_engine),
            competition: ctx.competition,
            game_model: game_model_ref,
            succession_heirs: &succession_heirs,
            keeper_brief: ctx.keeper_brief,
        };

        let starting_pool = Self::starting_pool(team, &available, ctx, tactics);
        let main_squad = scx.select_starting_eleven(team.id, &starting_pool);

        let main_squad_ids: FxHashSet<u32> = main_squad.iter().map(|mp| mp.id).collect();
        let remaining: Vec<&Player> = available
            .iter()
            .filter(|p| !main_squad_ids.contains(&p.id))
            .copied()
            .collect();

        let mut substitutes = scx.select_substitutes(team.id, &remaining);

        if substitutes.is_empty() && !remaining.is_empty() {
            debug!(
                "Substitute selection produced empty bench with {} remaining players — force-populating",
                remaining.len()
            );
            for player in &remaining {
                if substitutes.len() >= ctx.bench_size {
                    break;
                }
                let pos = SlotFit::best_tactical_position(player, tactics);
                substitutes.push(MatchPlayer::from_player(
                    team.id,
                    player,
                    pos,
                    false,
                    Some(ctx.date),
                ));
            }
        }

        debug!(
            "Final squad: {} starters, {} subs",
            main_squad.len(),
            substitutes.len()
        );

        let omissions = omissions::OmissionBuilder {
            available: &available,
            main_squad: &main_squad,
            substitutes: &substitutes,
            staff,
            tactics,
            engine: &engine,
            date: ctx.date,
            is_friendly: ctx.is_friendly,
            match_importance: ctx.pressure(),
            room: ctx.room(),
            coach: Some(&coach_engine),
            competition: ctx.competition,
            game_model: game_model_ref,
            succession_heirs: &succession_heirs,
        }
        .build();

        let overlooked = Self::overlooked_on_roster(team, &available, &main_squad, &substitutes);

        PlayerSelectionResult {
            main_squad,
            substitutes,
            omissions,
            overlooked,
        }
    }

    /// The selecting team's own fit players who were left out altogether.
    /// Borrowed reserves are not "overlooked" by a squad that does not own
    /// them — their own side's fixtures keep their books.
    fn overlooked_on_roster(
        team: &Team,
        available: &[&Player],
        main_squad: &[MatchPlayer],
        substitutes: &[MatchPlayer],
    ) -> Vec<u32> {
        let named: FxHashSet<u32> = main_squad
            .iter()
            .chain(substitutes.iter())
            .map(|p| p.id)
            .collect();
        let own: FxHashSet<u32> = team.players.players().iter().map(|p| p.id).collect();
        available
            .iter()
            .map(|p| p.id)
            .filter(|id| own.contains(id) && !named.contains(id))
            .collect()
    }

    // ========== ROTATION SELECTION ==========

    pub fn select_for_rotation(team: &Team, staff: &Staff) -> PlayerSelectionResult {
        Self::select_for_rotation_with_reserves(team, staff, &[])
    }

    pub fn select_for_rotation_with_reserves(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
    ) -> PlayerSelectionResult {
        Self::select_for_rotation_with_context(
            team,
            staff,
            reserve_players,
            &SelectionContext {
                is_friendly: true,
                // A bare rotation call is a pure development fixture — the
                // default competitive importance (0.7) would read as real
                // stakes and suppress the minutes plan.
                match_importance: 0.1,
                ..SelectionContext::default()
            },
        )
    }

    pub fn select_for_rotation_with_context(
        team: &Team,
        staff: &Staff,
        reserve_players: &[&Player],
        ctx: &SelectionContext,
    ) -> PlayerSelectionResult {
        let tactics = team.tactics();
        let is_main_team = team.team_type == TeamType::Main;

        let team_player_count = team.players.players().len();
        let estimated = team_player_count + reserve_players.len();
        let mut available: Vec<&Player> = Vec::with_capacity(estimated);
        let mut available_ids: FxHashSet<u32> =
            FxHashSet::with_capacity_and_hasher(estimated, Default::default());
        for &p in team.players.players().iter() {
            if PlayerAvailability::is_available(p, ctx.is_friendly)
                && Self::eligible_under_rules(p, ctx)
                && (is_main_team || !p.is_force_match_selection)
                && available_ids.insert(p.id)
            {
                available.push(p);
            }
        }

        // Development guests join the pool unconditionally — the shortfall
        // gate below exists so a full roster isn't diluted by borrowed
        // seniors, but a guest is here precisely because his own squad has
        // no fixtures; withholding him until the roster runs short would
        // make the visit pointless at any well-stocked academy.
        // Registration, not selection — see the competitive selector.
        let registration_cap = team.team_type.registration_age_cap();
        let mut guest_ids: Vec<u32> = Vec::with_capacity(ctx.development_guest_ids.len());
        if !ctx.development_guest_ids.is_empty() {
            for &rp in reserve_players.iter() {
                if !ctx.development_guest_ids.contains(&rp.id) {
                    continue;
                }
                if !is_main_team && rp.is_force_match_selection {
                    continue;
                }
                if registration_cap.is_some_and(|cap| rp.age(ctx.date) > cap) {
                    continue;
                }
                if PlayerAvailability::is_available(rp, ctx.is_friendly)
                    && Self::eligible_under_rules(rp, ctx)
                    && available_ids.insert(rp.id)
                {
                    available.push(rp);
                    guest_ids.push(rp.id);
                }
            }
        }

        if available.len() < DEFAULT_SQUAD_SIZE + ctx.bench_size {
            let needed = (DEFAULT_SQUAD_SIZE + ctx.bench_size) - available.len();
            let mut supplements: Vec<&Player> = reserve_players
                .iter()
                .filter(|&&rp| {
                    if !is_main_team && rp.is_force_match_selection {
                        return false;
                    }
                    if registration_cap.is_some_and(|cap| rp.age(ctx.date) > cap) {
                        return false;
                    }
                    PlayerAvailability::is_available(rp, ctx.is_friendly)
                        && Self::eligible_under_rules(rp, ctx)
                        && !available_ids.contains(&rp.id)
                })
                .copied()
                .collect();

            supplements.sort_by(|a, b| {
                b.player_attributes
                    .days_since_last_match
                    .cmp(&a.player_attributes.days_since_last_match)
            });

            // Shortfall fill-ins hold guest standing too: they are not part
            // of this team's season plan, and their empty appearance ledgers
            // would otherwise read as season-scale deficits that start them
            // over the roster's own players. As guests they complete the
            // squad when there are holes but never displace an own player
            // who is on or behind his plan.
            for rp in supplements.into_iter().take(needed) {
                if available_ids.insert(rp.id) {
                    available.push(rp);
                    guest_ids.push(rp.id);
                }
            }

            if available.len() < DEFAULT_SQUAD_SIZE {
                debug!(
                    "Rotation selection for team {}: only {} available after borrowing ({} reserves offered)",
                    team.name,
                    available.len(),
                    reserve_players.len()
                );
            }
        }

        Self::readmit_withheld(
            team.players
                .players()
                .into_iter()
                .filter(|p| is_main_team || !p.is_force_match_selection),
            ctx,
            &mut available,
            &mut available_ids,
        );

        // Development selector: season minutes plan + keeper rotation
        // blocks + stakes slider. The season length prefers the exact
        // table read supplied by the matchday caller; the fallback is
        // estimated from the team's own roster (not the merged reserve
        // pool) so borrowed players' senior appearance counts can't
        // inflate the season the deficits are measured against.
        // A side whose coach holds no plan is planned for the day, on the
        // same chart and shares.
        let drawn = staff.squad_plan.is_empty().then(|| {
            let side = team.team_type.development_weight();
            let mut plan = CoachSquadPlan::new();
            plan.revise(&PlanReading {
                climate: ShareClimate {
                    rotation: CoachProfile::from_staff(staff).rotation_tendency(),
                    development: side,
                    side,
                    ..ShareClimate::ONE_COMPETITION
                },
                ..PlanReading::of(&team.players, tactics.borrow(), ctx.date)
            });
            plan
        });
        let development = DevelopmentSelection {
            team_id: team.id,
            tactics: tactics.borrow(),
            date: ctx.date,
            team_type: team.team_type,
            stakes: DevelopmentStakes::from_context(ctx.pressure(), ctx.is_friendly),
            team_matches: ctx.season_matches_played.unwrap_or_else(|| {
                MatchInvolvement::team_matches_estimate(&team.players.players())
            }),
            coach: CoachProfile::from_staff(staff),
            guest_ids: &guest_ids,
            plan: drawn.as_ref().unwrap_or(&staff.squad_plan),
            bench_size: ctx.bench_size,
        };

        let main_squad = development.select_starting_eleven(&available);

        let main_squad_ids: FxHashSet<u32> = main_squad.iter().map(|mp| mp.id).collect();
        let remaining: Vec<&Player> = available
            .iter()
            .filter(|p| !main_squad_ids.contains(&p.id))
            .copied()
            .collect();

        let mut substitutes = development.select_substitutes(&remaining);

        if substitutes.is_empty() && !remaining.is_empty() {
            debug!(
                "Rotation substitute selection produced empty bench with {} remaining — force-populating",
                remaining.len()
            );
            for player in &remaining {
                if substitutes.len() >= ctx.bench_size {
                    break;
                }
                let pos = SlotFit::best_tactical_position(player, tactics.borrow());
                substitutes.push(MatchPlayer::from_player(
                    team.id,
                    player,
                    pos,
                    false,
                    Some(ctx.date),
                ));
            }
        }

        // Rotation matches (friendlies / dev leagues) don't generate
        // morale-relevant drop events — every player is in line for a
        // run-out and an omission carries no professional sting.
        let overlooked = Self::overlooked_on_roster(team, &available, &main_squad, &substitutes);
        PlayerSelectionResult {
            main_squad,
            substitutes,
            omissions: Vec::new(),
            overlooked,
        }
    }

    // ========== LEGACY PUBLIC API ==========

    pub fn calculate_player_rating_for_position(
        player: &Player,
        staff: &Staff,
        position: PlayerPositionType,
        tactics: &Tactics,
    ) -> f32 {
        let group = position.position_group();
        let engine = ScoringEngine::from_staff(staff);
        let date = Utc::now().date_naive();
        engine.score_player_for_slot(player, position, group, staff, tactics, date, false, &[])
    }

    pub fn select_main_squad(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let engine = ScoringEngine::from_staff(staff);
        let scx = Self::legacy_scoring_context(staff, tactics, &engine);
        scx.select_starting_eleven(team_id, players)
    }

    pub fn select_substitutes_legacy(
        team_id: u32,
        players: &mut Vec<&Player>,
        staff: &Staff,
        tactics: &Tactics,
    ) -> Vec<MatchPlayer> {
        let engine = ScoringEngine::from_staff(staff);
        let scx = Self::legacy_scoring_context(staff, tactics, &engine);
        scx.select_substitutes(team_id, players)
    }

    /// Scoring context for the legacy public selectors: today's date, a
    /// competitive non-friendly fixture with no cup bias, mid importance.
    fn legacy_scoring_context<'a>(
        staff: &'a Staff,
        tactics: &'a Tactics,
        engine: &'a ScoringEngine,
    ) -> competitive::SelectionScoringContext<'a> {
        competitive::SelectionScoringContext {
            staff,
            tactics,
            engine,
            date: Utc::now().date_naive(),
            is_friendly: false,
            match_importance: 0.7,
            stakes: 0.7,
            room: 0.3,
            bench_size: usize::from(LeagueRegulations::BENCH_LOWER),
            coach: None,
            competition: SelectionCompetition::League,
            game_model: None,
            succession_heirs: &[],
            keeper_brief: None,
        }
    }
}

/// Helpers for deriving the soft signals fed into [`StrategyInputs`]
/// from objective squad / fixture state. Kept under one struct so the
/// formulas live in one place — selection and tests read them by
/// name rather than re-deriving inline.
pub(crate) struct StrategyInputsBuilder;

impl StrategyInputsBuilder {
    /// Coach's strength signal vs the opponent (1.0 = even). Derived
    /// from the domestic-cup context's opponent reputation when
    /// available; falls back to 1.0 for league / continental fixtures
    /// because the selection layer doesn't yet carry opponent
    /// reputation outside the cup bracket.
    pub(crate) fn strength_ratio(competition: &SelectionCompetition) -> f32 {
        match *competition {
            SelectionCompetition::DomesticCup {
                own_reputation,
                opponent_reputation,
                ..
            } => {
                let own = own_reputation.max(1) as f32;
                let opp = opponent_reputation.max(1) as f32;
                (own / opp).clamp(0.25, 4.0)
            }
            _ => 1.0,
        }
    }

    /// Squad-depth heuristic in [0.0, 1.0]: 1.0 = deep bench, 0.0 =
    /// thin. Linear ramp over the available pool size — anything at
    /// or below a full matchday squad is thin; seven beyond it is deep.
    /// Selection's force-fill paths can still field an XI from a
    /// thin pool; this signal only shapes how aggressively the
    /// coach rotates.
    pub(crate) fn squad_depth(available_len: usize, bench_size: usize) -> f32 {
        let minimum = (DEFAULT_SQUAD_SIZE + bench_size) as f32;
        let deep = minimum + 7.0;
        let available = available_len as f32;
        ((available - minimum) / (deep - minimum)).clamp(0.0, 1.0)
    }
}

/// Convenience helper bundling the strategy / coach hints derived from
/// a [`SelectionContext`]. Kept as a struct namespace so the conversion
/// rules live in one place — the selection layer reads them by name
/// instead of hand-derived flags scattered across each call site.
pub struct CoachStrategyForSelection;

impl CoachStrategyForSelection {
    /// Compute the coach strategy used by the selection layer's coach
    /// adjustment. Exposed so omissions / tests can rebuild the same
    /// strategy without duplicating the SelectionContext → inputs map.
    pub fn derive(profile: &CoachProfile, ctx: &SelectionContext) -> CoachStrategy {
        let model = ctx.game_model.as_ref();
        StrategyDeriver::derive(&StrategyInputs {
            profile,
            philosophy: ctx.philosophy,
            match_importance: ctx.pressure(),
            is_friendly: ctx.is_friendly,
            is_cup: matches!(
                ctx.competition,
                SelectionCompetition::DomesticCup { .. } | SelectionCompetition::ContinentalCup
            ),
            is_continental: matches!(ctx.competition, SelectionCompetition::ContinentalCup),
            is_derby: model.map(|m| m.is_derby()).unwrap_or(false),
            strength_ratio: model
                .map(|m| m.opponent_profile.strength_ratio)
                .unwrap_or(1.0),
            squad_depth: model.map(|m| m.squad_state.depth).unwrap_or(0.5),
        })
    }
}
