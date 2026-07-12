use crate::club::player::adaptation::{AdaptationFailureSignals, AdaptationSquadContext};
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::core::player::TransferRequestReason;
use crate::club::player::player::Player;
use crate::club::{PlayerMailbox, PlayerResult, PlayerStatusType};
use crate::context::GlobalContext;
use crate::utils::DateUtils;
use crate::{
    CareerDesireEventContext, CareerDesireEvidence, CareerDesireKind, HappinessEventCause,
    HappinessEventContext, HappinessEventFollowUp, HappinessEventScope, HappinessEventSeverity,
    HappinessEventType, LifeSimulationDesireContext, LifeSimulationDesireKind,
    LifeSimulationSeverity, LifeSimulationTrigger, PlayerSquadStatus,
};
use chrono::NaiveTime;
use chrono::{NaiveDate, NaiveDateTime};

/// Continent ids matching the values documented in
/// `transfers::scouting_region`: 1 = Europe, 3 = South America.
const CONTINENT_EUROPE: u32 = 1;
const CONTINENT_SOUTH_AMERICA: u32 = 3;

/// Minimum number of days a player must *continuously* hold the `Unh`
/// status before generic unhappiness escalates into a formal transfer
/// request — and, through the country listing pass, an actual transfer
/// listing. ≈ 6 months: the manager-talk and loan paths get a full half
/// season to resolve the grievance before the club acts on it. Shared
/// with the country listing pass (`evaluate_player_listing`) so the
/// request and the listing agree on exactly when unhappiness becomes a
/// sell signal.
pub(crate) const UNHAPPY_LISTING_MIN_DAYS: i64 = 182;

/// Inputs the weekly tick collects from `GlobalContext` once and feeds
/// into [`Player::process_transfer_desire`]. Decoupled from
/// `GlobalContext` so the desire logic stays unit-testable and the
/// player code never walks the simulator world.
#[derive(Debug, Clone, Default)]
pub struct TransferDesireContext {
    /// Country id of the player's current club. 0 if unknown.
    pub club_country_id: u32,
    /// Continent id of the player's current club country (matches the
    /// values in `transfers::scouting_region`). 0 if unknown.
    pub club_continent_id: u32,
    /// Continent id of the player's nationality country. 0 if unknown.
    pub player_nationality_continent_id: u32,
    /// Country / club country code for the local-language check.
    /// Empty if unknown.
    pub country_code: String,
    /// League reputation (0..10000). 0 if unknown.
    pub league_reputation: u16,
    /// Club reputation 0..1 normalised. 0.0 if unknown.
    pub club_reputation: f32,
    /// Current league position (1-based). 0 if unknown.
    pub league_position: u8,
    /// Number of teams in the league. 0 if unknown.
    pub league_size: u8,
    /// Season progress 0.0..1.0.
    pub season_progress: f32,
    /// Tier of the main league (1 = top flight). 0 if unknown.
    pub main_league_tier: u8,
    /// Caller-supplied hint that the current club is on a credible
    /// continental qualification path this season — top-third in a top
    /// tier league or an active continental cup run. Read by the
    /// European-ambition detector to avoid firing for clubs already in
    /// the path.
    pub has_continental_path_hint: bool,
    /// Authoritative (not heuristic) evidence that the club cannot offer
    /// continental football this season — a continental ban, or known
    /// non-participation. When set, the European-ambition detector skips
    /// its elite-reputation suppression so a banned elite club's stars can
    /// still voice their frustration (the path hint already reads false).
    pub continental_path_known_absent: bool,
    /// Compatriots / shared-language teammates currently in the squad.
    /// Drives `NoCompatriotSupport` and feeds the chronic-failure
    /// detector.
    pub same_language_or_nationality_teammates: u8,
    /// True if the player's last transfer destination is a favourite
    /// club. Drives the dream-move suppression bar.
    pub destination_is_favourite: bool,
    /// True if the club country == player nationality country.
    pub club_in_home_country: bool,
    /// True when the player's current club country is currently barred
    /// from UEFA competitions (Russia after 2022-02-28). When set, the
    /// European-ambition detector must NOT let its elite-club shortcut
    /// suppress the desire mood — a top Russian club CAN'T offer
    /// continental football regardless of reputation, so its ambitious
    /// stars are legitimately frustrated.
    pub country_uefa_suspended: bool,
}

impl TransferDesireContext {
    /// Build the desire context from the active `GlobalContext` and
    /// the player. Reads only what the global context already
    /// surfaces (club country / continent / league reputation /
    /// position), plus the player's stored country and favourite
    /// clubs. Unknown axes default to 0 / empty so the desire helpers
    /// fail closed (no firing) rather than producing false positives.
    ///
    /// `player_nationality_continent_id` is supplied by the caller in
    /// the same way the free-agent matcher already does — the player
    /// knows only `country_id`, the simulator knows the country →
    /// continent mapping. Without it (zero), all continent gates
    /// fail closed.
    pub fn from_global(player: &Player, gc: &GlobalContext<'_>) -> Self {
        let country_code = gc
            .country
            .as_ref()
            .map(|c| c.code.clone())
            .unwrap_or_default();
        let club_country_id = gc.country.as_ref().map(|c| c.id).unwrap_or(0);
        let club_continent_id = gc.continent.as_ref().map(|c| c.id()).unwrap_or(0);
        let league_reputation = gc.league.as_ref().map(|l| l.reputation).unwrap_or(0);
        let club_reputation = gc.team.as_ref().map(|t| t.reputation).unwrap_or(0.0);
        let (league_position, league_size, total_matches, matches_played, main_league_tier) = gc
            .club
            .as_ref()
            .map(|c| {
                (
                    c.league_position,
                    c.league_size,
                    c.total_league_matches,
                    c.league_matches_played,
                    c.main_league_tier,
                )
            })
            .unwrap_or((0, 0, 0, 0, 0));
        let season_progress = if total_matches > 0 {
            (matches_played as f32 / total_matches as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let club_in_home_country = club_country_id != 0 && club_country_id == player.country_id;

        // UEFA-suspension flag (Russia after 2022-02-28). Treated as a
        // first-class continental ban so the elite-reputation shortcut
        // cannot paper over a real federation suspension.
        let country_uefa_suspended = crate::transfers::TransferRoutePolicy::is_uefa_suspended(
            &country_code,
            gc.simulation.date.date(),
        );

        // Continental-access picture — see
        // [`ContinentalPathHeuristic::is_on_path`] for the realism rules.
        // Live continental-cup state (current participant / qualified for
        // next / ban) isn't surfaced through `GlobalContext` yet, so those
        // default to "unknown"; the reputation-based elite floors carry the
        // realism and keep elite European clubs (Real Madrid, Bayern, …)
        // off the "wants Europe" path. The season-position fallback only
        // kicks in once the table is meaningful. UEFA-suspended countries
        // feed `continental_ban` so a top Russian club is treated as
        // banned regardless of reputation.
        let continental_access = ContinentalAccessContext {
            current_continental_competition: None,
            qualified_for_next_continental_competition: None,
            club_reputation,
            league_reputation,
            main_league_tier,
            league_position,
            league_size,
            season_progress,
            club_continent_id,
            continental_ban: country_uefa_suspended,
        };
        let has_continental_path_hint = ContinentalPathHeuristic::from_access(&continental_access)
            .is_on_path(&continental_access);
        let continental_path_known_absent = continental_access.path_known_absent();

        let destination_is_favourite = gc
            .club
            .as_ref()
            .map(|c| player.favorite_clubs.contains(&c.id))
            .unwrap_or(false);

        // Squad social view computed at team weekly pre-tick (see
        // `SquadSocialViewBuilder`). Defaults to zero when the pre-tick
        // hasn't run yet (newly-loaded save, mid-week first sim, etc.).
        let same_language_or_nationality_teammates = player
            .squad_social_view
            .as_ref()
            .map(|v| v.same_language_or_nationality())
            .unwrap_or(0);

        TransferDesireContext {
            club_country_id,
            club_continent_id,
            player_nationality_continent_id: player.nationality_continent_id,
            country_code,
            league_reputation,
            club_reputation,
            league_position,
            league_size,
            season_progress,
            main_league_tier,
            has_continental_path_hint,
            continental_path_known_absent,
            same_language_or_nationality_teammates,
            destination_is_favourite,
            club_in_home_country,
            country_uefa_suspended,
        }
    }
}

/// Coarse continental-competition buckets a club may participate in or
/// qualify for. The desire logic only cares whether the club holds a
/// *European* berth (UCL/UEL/UECL) — the South-American and `Other`
/// variants round out the model so the same context can describe any
/// club without lying about which tier it sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinentalCompetitionTier {
    ChampionsLeague,
    EuropaLeague,
    ConferenceLeague,
    CopaLibertadores,
    Other,
}

impl ContinentalCompetitionTier {
    /// True for the three European tiers. A club playing (or qualified
    /// for) any of these already offers the European football an
    /// ambitious player is chasing.
    fn is_european(self) -> bool {
        matches!(
            self,
            ContinentalCompetitionTier::ChampionsLeague
                | ContinentalCompetitionTier::EuropaLeague
                | ContinentalCompetitionTier::ConferenceLeague
        )
    }
}

/// Full picture of the current club's access to continental football.
/// Replaces the position-only signal the desire context used to read —
/// see [`ContinentalPathHeuristic::is_on_path`] for the realism rules.
/// Built once per weekly tick alongside [`TransferDesireContext`].
///
/// The simulator doesn't yet surface live continental-cup state into
/// `GlobalContext`, so `current_continental_competition`,
/// `qualified_for_next_continental_competition` and `continental_ban`
/// default to "unknown" when built from the world. The reputation-based
/// elite floors carry the realism until that state is plumbed through,
/// and the live fields take precedence the moment they are.
#[derive(Debug, Clone, Default)]
pub struct ContinentalAccessContext {
    /// Continental competition the club is playing in this season, if any.
    pub current_continental_competition: Option<ContinentalCompetitionTier>,
    /// Continental competition already secured for next season, if known.
    pub qualified_for_next_continental_competition: Option<ContinentalCompetitionTier>,
    /// Club reputation, 0.0..1.0 normalised.
    pub club_reputation: f32,
    /// League reputation, 0..10000.
    pub league_reputation: u16,
    /// Tier of the main league (1 = top flight). 0 if unknown.
    pub main_league_tier: u8,
    /// Current league position (1-based). 0 if unknown.
    pub league_position: u8,
    /// Number of teams in the league. 0 if unknown.
    pub league_size: u8,
    /// Season progress 0.0..1.0.
    pub season_progress: f32,
    /// Continent id of the club's country (1 = Europe, 3 = South America).
    pub club_continent_id: u32,
    /// True when the club is barred from continental competition this
    /// season (FFP / disciplinary). A banned elite club can still
    /// generate ambition frustration, so this does NOT auto-suppress.
    pub continental_ban: bool,
}

impl ContinentalAccessContext {
    /// True when authoritative evidence — not a cheap reputation/position
    /// heuristic — shows the club cannot offer continental football this
    /// season. A continental ban is the clearest case; both berth slots
    /// resolved to non-European also counts. When this holds, the
    /// European-ambition detector must NOT let its elite-reputation
    /// shortcut re-suppress the mood — a banned elite club's stars are
    /// legitimately frustrated. Unknown berth state falls through to the
    /// reputation-based suppression instead.
    pub fn path_known_absent(&self) -> bool {
        if self.continental_ban {
            return true;
        }
        let current_known_non_european = self
            .current_continental_competition
            .is_some_and(|c| !c.is_european());
        let next_known_non_european = self
            .qualified_for_next_continental_competition
            .is_some_and(|c| !c.is_european());
        current_known_non_european && next_known_non_european
    }
}

/// Tunables for the European-ambition desire detector and the elite-club
/// suppression floors. Centralised so the thresholds can't drift between
/// [`ContinentalPathHeuristic::is_on_path`] and
/// [`Player::detect_continental_competition_desire`] — both read the same
/// numbers from here.
#[derive(Debug, Clone, Copy)]
pub struct EuropeanAmbitionConfig {
    /// Minimum `ambition` personality before the mood can fire.
    pub min_ambition: f32,
    /// Inclusive age band — outside it the move-for-Europe story doesn't fit.
    pub min_age: u8,
    pub max_age: u8,
    /// Minimum current ability to plausibly play European football.
    pub min_ca: f32,
    /// Minimum world reputation to plausibly attract a European move.
    pub min_world_rep: f32,
    /// Club reputation at/above which an elite European, top-flight,
    /// top-league club is presumed to offer continental football.
    pub elite_club_rep_suppress: f32,
    /// Club reputation at/above which a super-elite European club is
    /// presumed to offer the Champions League ladder regardless of the
    /// league table (Real Madrid, Bayern, Man City, …).
    pub super_elite_rep_suppress: f32,
    /// League reputation floor for the elite-club suppression.
    pub elite_league_rep_floor: u16,
    /// Days between repeat `WantsEuropeanCompetition` emissions.
    pub cooldown_days: u16,
}

impl Default for EuropeanAmbitionConfig {
    fn default() -> Self {
        EuropeanAmbitionConfig {
            min_ambition: 15.0,
            min_age: 22,
            max_age: 31,
            min_ca: 130.0,
            min_world_rep: 4500.0,
            elite_club_rep_suppress: 0.78,
            super_elite_rep_suppress: 0.88,
            elite_league_rep_floor: 8000,
            cooldown_days: 60,
        }
    }
}

/// Heuristic for "is this club on a credible continental qualification
/// path?" Reputation-aware, season-progress-respecting, and aware of
/// elite European institutions and live continental-cup participation —
/// so an elite European club (or any current participant) is never
/// treated as unable to offer continental football.
///
/// The signal stays conservative — better to underfire (and let the
/// player accept their lot) than to misfire on a low-rep top-flight
/// where merely being in the top half doesn't imply Europe at all.
pub struct ContinentalPathHeuristic {
    pub main_league_tier: u8,
    pub league_reputation: u16,
    pub league_position: u8,
    pub league_size: u8,
    pub season_progress: f32,
}

impl ContinentalPathHeuristic {
    /// Build the season-position heuristic from a [`ContinentalAccessContext`]
    /// so callers don't carry the league-table fields in two places.
    pub fn from_access(access: &ContinentalAccessContext) -> Self {
        ContinentalPathHeuristic {
            main_league_tier: access.main_league_tier,
            league_reputation: access.league_reputation,
            league_position: access.league_position,
            league_size: access.league_size,
            season_progress: access.season_progress,
        }
    }

    /// True if the current club realistically offers continental football.
    ///
    /// Rules (in order — the first to match wins):
    ///   1. A continental ban does NOT prove the club is on the path. A
    ///      banned elite side still can't offer Europe this season, so
    ///      its players' ambition frustration is legitimate — fail closed
    ///      here before any elite shortcut can paper over the ban.
    ///   2. Currently playing European football (UCL/UEL/UECL) → on the
    ///      path, full stop.
    ///   3. Already qualified for next season's European football → same.
    ///   4. Super-elite European institution (reputation ≥
    ///      `super_elite_rep_suppress`): in the Champions League ladder as
    ///      a baseline. Reputation alone clears it, so missing league-table
    ///      data can't create a false "no Europe". This is the Real Madrid
    ///      / Bayern / Man City floor.
    ///   5. Elite European club in a top-flight, top-reputation league
    ///      (tier 1, league rep ≥ `elite_league_rep_floor`, club rep ≥
    ///      `elite_club_rep_suppress`): a perennial participant even when
    ///      this season's table isn't surfaced.
    ///   6. Otherwise fall back to the season-position heuristic. Missing
    ///      table data fails closed *here only* — the elite shortcuts above
    ///      already protect top clubs from a false negative.
    pub fn is_on_path(&self, access: &ContinentalAccessContext) -> bool {
        let cfg = EuropeanAmbitionConfig::default();

        if access.continental_ban {
            return false;
        }

        if access
            .current_continental_competition
            .is_some_and(ContinentalCompetitionTier::is_european)
        {
            return true;
        }

        if access
            .qualified_for_next_continental_competition
            .is_some_and(ContinentalCompetitionTier::is_european)
        {
            return true;
        }

        let is_european_club = access.club_continent_id == CONTINENT_EUROPE;

        if is_european_club && access.club_reputation >= cfg.super_elite_rep_suppress {
            return true;
        }

        if is_european_club
            && access.main_league_tier == 1
            && access.league_reputation >= cfg.elite_league_rep_floor
            && access.club_reputation >= cfg.elite_club_rep_suppress
        {
            return true;
        }

        self.position_on_path()
    }

    /// Season-position fallback. Tracks whether the club is sitting in a
    /// continental-berth slot on current league standing.
    ///
    /// Rules:
    ///   1. Below tier-1 → not on a path.
    ///   2. League reputation must be ≥ 5500 (mid-tier European or
    ///      strong South American). Below that, even the title-winner
    ///      lands in the qualifying rounds at best, so the player's
    ///      ambition mood should still fire.
    ///   3. The position threshold scales with league reputation:
    ///        rep ≥ 8500 → top 5 of league size (UCL/UEL spots),
    ///        rep ≥ 7000 → top 4,
    ///        rep ≥ 5500 → top 2 (only champion / runner-up).
    ///   4. Before 25% of the season is played, the table is too noisy
    ///      to be load-bearing. Fall back to a reputation-only signal:
    ///      a top-tier club in a high-rep league is presumed to be on
    ///      the path until the season tells us otherwise.
    fn position_on_path(&self) -> bool {
        if self.main_league_tier > 1 {
            return false;
        }
        if self.league_reputation < 5500 {
            return false;
        }
        if self.season_progress < 0.25 {
            // Pre-quarter-season: trust the league-rep band only.
            // High-rep leagues have many continental berths, so a
            // top-tier club there is on the path by default until the
            // table proves otherwise.
            return self.league_reputation >= 7000;
        }
        if self.league_position == 0 || self.league_size == 0 {
            return false;
        }
        let cap = match self.league_reputation {
            r if r >= 8500 => 5,
            r if r >= 7000 => 4,
            _ => 2,
        };
        self.league_position as u32 <= cap
    }
}

impl Player {
    pub(crate) fn process_contract(&mut self, result: &mut PlayerResult, now: NaiveDateTime) {
        // Snapshot threshold inputs before borrowing contract mutably.
        // Career apps for the threshold clause means *club-career* — the
        // sum of every season this player has spent at the current club,
        // not just this season's apps. The history helper sums prior
        // frozen seasons + current-season live stats keyed by the active
        // current-spell team slug.
        let career_apps = self
            .statistics_history
            .current_club_career_apps(self.statistics.played, self.statistics.played_subs);
        let caps = self.player_attributes.international_apps;

        if let Some(ref mut contract) = self.contract {
            const ONE_YEAR_DAYS: i64 = 365;

            if contract.days_to_expiration(now) < ONE_YEAR_DAYS {
                // For loaned players this signals the parent club to renew remotely
                result.contract.want_extend_contract = true;
            }

            // Yearly wage-rise clause: applies on the contract anniversary.
            // The helper guards idempotency via a per-year memo so a
            // same-day double tick won't double-apply.
            let _ = contract.try_apply_yearly_wage_rise(now.date());

            // Threshold clauses fire as soon as the daily count crosses
            // the negotiated threshold. Each helper consumes its clause
            // on success so the next tick is a cheap no-op.
            let _ = contract.try_apply_wage_after_career_apps(career_apps);
            let _ = contract.try_apply_wage_after_caps(caps);

            // Final-season auto-extension: triggers once the contract is
            // inside its last 365 days and the player has crossed the
            // appearance threshold this season. Helper handles both
            // gates internally.
            let season_apps = self.statistics.played + self.statistics.played_subs;
            let _ = contract.try_apply_appearance_extension(season_apps, now.date());
        } else if !self.is_on_loan() {
            result.contract.no_contract = true;
        }
    }

    pub(crate) fn process_mailbox(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        PlayerMailbox::process(self, result, now);
    }

    /// Transfer desire based on multiple factors, not just behaviour.
    /// Reads a [`TransferDesireContext`] built once per weekly tick by
    /// `Player::simulate` from `GlobalContext` so the player code never
    /// walks the simulator world.
    pub(crate) fn process_transfer_desire(
        &mut self,
        result: &mut PlayerResult,
        now: NaiveDate,
        ctx: &TransferDesireContext,
    ) {
        // Loaned players belong to their parent club — they cannot request
        // transfers or be unhappy with salary at the loan club
        if self.is_on_loan() {
            return;
        }

        // Under-16 players cannot request transfers — only free release
        let age = DateUtils::age(self.birth_date, now);
        if age < 16 {
            return;
        }

        // Honeymoon: newly-transferred players don't fire off a request in
        // the first 21 days regardless of shock events — they need a fair
        // look first (unless behaviour is already broken).
        let recently_transferred = self
            .days_since_transfer(now)
            .map(|d| d >= 0 && d < 21)
            .unwrap_or(false);

        // Career-desire moods: emit (or refresh) the WantsReturnHome /
        // WantsEuropeanCompetition / WantsCopaLibertadores ambient mood
        // events before deciding whether to escalate to Req. The
        // helpers themselves are cooldowned so they don't spam.
        if !recently_transferred {
            // Adaptation score uses the squad social view from the
            // weekly pre-tick — same source the desire context already
            // reads. No formation here (we don't carry it through the
            // weekly tick); the formation arm of `adaptation_score`
            // contributes ±10 at most so the missing input only damps
            // the signal.
            let squad_ctx = AdaptationSquadContext {
                same_language_teammates: self
                    .squad_social_view
                    .as_ref()
                    .map(|v| v.same_language_teammates)
                    .unwrap_or(0),
                same_nationality_teammates: self
                    .squad_social_view
                    .as_ref()
                    .map(|v| v.same_nationality_teammates)
                    .unwrap_or(0),
                mentor_quality: None,
                squad_chemistry: 50.0,
                manager_relation_level: 0.0,
                is_loan: self.is_on_loan(),
                is_favorite_club: ctx.destination_is_favourite,
            };
            let adaptation_score = self.adaptation_score(
                now,
                &ctx.country_code,
                ctx.club_reputation,
                None,
                &squad_ctx,
            );
            let signals = AdaptationFailureSignals {
                player_nationality_continent_id: ctx.player_nationality_continent_id,
                club_continent_id: ctx.club_continent_id,
                club_in_home_country: ctx.club_in_home_country,
                destination_is_favourite: ctx.destination_is_favourite,
                same_language_or_nationality_teammates: ctx.same_language_or_nationality_teammates,
                adaptation_score,
                club_fit: self.happiness.factors.club_fit,
            };
            self.process_chronic_adaptation_failure(now, &ctx.country_code, &signals);
            self.detect_career_desire_priority(now, ctx);
            // Post-relegation exodus — independent of the continental
            // priority resolution; keyed off the `Relegated` team event
            // already sitting on the player, so no league plumbing.
            self.detect_relegation_escape_desire(now, ctx);
            // Item 8: broader life-simulation moods. Each detector is
            // cooldowned and gated — they fire ambient mood events
            // separately from the transfer-request escalation path.
            self.detect_life_simulation_desires(now, ctx, adaptation_score);
        }

        // Re-evaluate every reason every tick. A reason in the set
        // *now* is unresolved; a reason that's silent has gone away.
        // Req only clears when no reasons remain.
        let mut active_reasons: Vec<TransferRequestReason> = Vec::new();

        if self.behaviour.is_poor() {
            active_reasons.push(TransferRequestReason::PoorBehaviour);
        }

        // Generic unhappiness only escalates to a formal request once the
        // mood has held for ~6 months — long enough for the manager-talk /
        // loan paths to have tried and failed to resolve it. A fresher
        // grievance keeps the player unsettled but not yet asking out.
        let has_unh_long = self
            .statuses
            .held_for_days(PlayerStatusType::Unh, now)
            .is_some_and(|d| d >= UNHAPPY_LISTING_MIN_DAYS);
        if has_unh_long {
            active_reasons.push(TransferRequestReason::LongUnhappiness);
        }

        // A genuine ambition mismatch (wrong-size club / relegation slide)
        // is a distinct, faster grievance — a fortnight of unhappiness on
        // top of a clearly-too-small club is enough to want a move.
        if !recently_transferred && self.happiness.factors.ambition_fit <= -7.0 {
            let has_unh_short = self
                .statuses
                .held_for_days(PlayerStatusType::Unh, now)
                .is_some_and(|d| d > 14);
            if has_unh_short {
                active_reasons.push(TransferRequestReason::AmbitionMismatch);
            }
        }

        // Outgrown club: an ambitious player at a club well beneath his
        // stature wants to step UP even while otherwise settled — healthy
        // career ambition, not a grievance. Unlike `AmbitionMismatch` it
        // needs no spell of `Unh`; the structural prestige deficit alone
        // drives it. Gated hard so only players who have genuinely outgrown
        // their club agitate — a deep ambition-fit deficit, real personal
        // ambition, still young enough to climb, and settled at the club
        // for a full year (a recent signing gets his look first). This is
        // the missing "good player at a small club finally moves up"
        // signal; without it the market only ever recirculated unhappy or
        // surplus players, so established players never moved.
        if !recently_transferred
            && self.happiness.factors.ambition_fit <= -10.0
            && self.attributes.ambition >= 15.0
            && age <= 28
            && self
                .days_since_transfer(now)
                .map(|d| d >= 365)
                .unwrap_or(true)
        {
            active_reasons.push(TransferRequestReason::OutgrownClub);
        }

        // New challenge: long service at ONE club breeds a desire for a fresh
        // test, independent of whether he has outgrown it. This is the "many
        // years at one club" case — a settled star at a big club who has won
        // it all and wants a new league — which the prestige-based
        // OutgrownClub signal (a too-small club + youth) misses. The tenure
        // restlessness has already dented his morale via `ambition_fit`; this
        // turns a long-festering itch into an actual request. Gated tightly —
        // genuine ambition, below-average loyalty, a prime mobile age and a
        // long stay — so a true one-club legend or a content servant stays
        // put rather than every veteran walking out.
        if !recently_transferred {
            let years_at_club = self
                .days_since_transfer(now)
                .map(|d| d as f32 / 365.0)
                .unwrap_or_else(|| (age as f32 - 17.0).max(0.0));
            let restless_for_a_new_challenge = years_at_club >= 8.0
                && self.attributes.ambition >= 16.0
                && self.attributes.loyalty < 12.0
                && (25..=30).contains(&age);
            if restless_for_a_new_challenge {
                active_reasons.push(TransferRequestReason::NewChallenge);
            }
        }

        if let Some(first_request) = self.happiness.last_salary_negotiation {
            let days = (now - first_request).num_days();
            if days > 540 && days <= 730 && self.happiness.factors.salary_satisfaction <= -5.0 {
                active_reasons.push(TransferRequestReason::SalaryUnresolved);
            }
        }

        if !recently_transferred && self.return_home_request_pressure(now, ctx) {
            active_reasons.push(TransferRequestReason::ReturnHome);
        }

        if !recently_transferred && self.european_request_pressure(now) {
            active_reasons.push(TransferRequestReason::EuropeanAmbition);
        }
        if !recently_transferred && self.libertadores_request_pressure(now) {
            active_reasons.push(TransferRequestReason::CopaLibertadoresAmbition);
        }
        if !recently_transferred && self.relegation_escape_pressure(now) {
            active_reasons.push(TransferRequestReason::RelegationEscape);
        }

        // Honeymoon overrides everything except poor behaviour. A
        // newly-signed player won't formally request a transfer in the
        // first 21 days unless their character has actually broken.
        if recently_transferred {
            active_reasons.retain(|r| matches!(r, TransferRequestReason::PoorBehaviour));
        }

        let wants_transfer = !active_reasons.is_empty();

        // Reasons drive the persisted state — used by Item 4's
        // "don't clear Req while any unresolved reason remains".
        self.transfer_request_reasons = active_reasons;

        if wants_transfer {
            if !self.statuses.has(PlayerStatusType::Req) {
                self.statuses.add(now, PlayerStatusType::Req);
            }
            result.wants_to_leave = true;
            result.request_transfer(self.id);
        } else if self.statuses.has(PlayerStatusType::Req) {
            // No active reason left — Req can finally clear.
            self.statuses.remove(PlayerStatusType::Req);
        }
    }

    /// Pick at most one career-desire mood per tick. Resolves the
    /// European-vs-Libertadores overlap so a South American star at a
    /// weak SA club doesn't end up with both moods firing on top of each
    /// other; see [`Self::primary_career_desire`] for the priority
    /// rules.
    fn detect_career_desire_priority(&mut self, now: NaiveDate, ctx: &TransferDesireContext) {
        match self.primary_career_desire(now, ctx) {
            Some(CareerDesireKind::CopaLibertadoresAmbition) => {
                self.detect_copa_libertadores_desire(now, ctx);
            }
            Some(CareerDesireKind::EuropeanCompetitionAmbition) => {
                self.detect_continental_competition_desire(now, ctx);
            }
            _ => {}
        }
    }

    /// Resolve which continental ambition fits this player best, given
    /// nationality, age, ability, and current club continent. Returns
    /// `None` when neither flavour applies.
    ///
    /// Priority rules (Item 5):
    ///   - South American outside South America: prefer Libertadores
    ///     unless they're an elite young prospect (CA ≥ 145, age ≤ 26)
    ///     in or near Europe — those still chase the Champions League
    ///     ladder.
    ///   - South American at a weak SA club: prefer Libertadores.
    ///   - South American at a strong SA club: neither (they're already
    ///     in the relevant path).
    ///   - Older South Americans (age ≥ 30) returning to / in South
    ///     America: prefer Libertadores even if Europe was the historic
    ///     fit — late-career sentimental gravity.
    ///   - Everyone else: European-competition ambition is the only
    ///     option that can plausibly fire.
    fn primary_career_desire(
        &self,
        now: NaiveDate,
        ctx: &TransferDesireContext,
    ) -> Option<CareerDesireKind> {
        let age = DateUtils::age(self.birth_date, now);
        let ca = self.player_attributes.current_ability;
        let is_sa_heritage = ctx.player_nationality_continent_id == CONTINENT_SOUTH_AMERICA;
        let at_sa_club = ctx.club_continent_id == CONTINENT_SOUTH_AMERICA;
        let at_eu_club = ctx.club_continent_id == CONTINENT_EUROPE;

        if is_sa_heritage {
            let elite_young_in_europe = at_eu_club && ca >= 145 && age <= 26;
            if elite_young_in_europe {
                return Some(CareerDesireKind::EuropeanCompetitionAmbition);
            }
            if age >= 30 && (at_sa_club || ctx.club_continent_id == 0) {
                return Some(CareerDesireKind::CopaLibertadoresAmbition);
            }
            // Default South American flavour.
            return Some(CareerDesireKind::CopaLibertadoresAmbition);
        }
        Some(CareerDesireKind::EuropeanCompetitionAmbition)
    }

    /// Emit the `WantsEuropeanCompetition` mood when an ambitious
    /// player at a non-continental-path club fits the realistic
    /// criteria. Cooldown 60 days. Returns `true` if the mood landed.
    fn detect_continental_competition_desire(
        &mut self,
        now: NaiveDate,
        ctx: &TransferDesireContext,
    ) -> bool {
        let cfg = EuropeanAmbitionConfig::default();

        // Already on a credible continental path — a current/qualified
        // participant, or an elite European institution (the hint folds
        // in the ban / current-comp / elite-floor rules). Don't fire.
        if ctx.has_continental_path_hint {
            return false;
        }

        // Strengthened suppression: even where the cheap path hint missed
        // it, a club whose own reputation is at/above the elite floor can
        // offer continental football — a top-reputation side wanting
        // Europe is unrealistic. Real Madrid is caught by the hint's
        // super-elite floor; this guards the band sitting just under the
        // league-rep elite floor. The hint already encodes "not a current
        // continental participant", so the two together satisfy the
        // realism bar before any emission.
        //
        // Exception: when there's authoritative no-path evidence (a
        // continental ban, or known non-participation), the false hint is
        // trustworthy and this reputation heuristic must not override it —
        // a banned elite club's star is legitimately frustrated.
        if !ctx.continental_path_known_absent && ctx.club_reputation >= cfg.elite_club_rep_suppress
        {
            return false;
        }

        // Cooldown.
        if self.happiness.has_recent_event(
            &HappinessEventType::WantsEuropeanCompetition,
            cfg.cooldown_days,
        ) {
            return false;
        }

        // Universal logic: score the gap. The same arithmetic covers
        // a Premier League fringe player at a relegation side and a
        // Champions League regular at a club mid-table — the higher
        // the realistic European ceiling, the more the gap matters.
        let ambition = self.attributes.ambition;
        if ambition < cfg.min_ambition {
            return false;
        }
        let age = DateUtils::age(self.birth_date, now);
        if !(cfg.min_age..=cfg.max_age).contains(&age) {
            return false;
        }
        let ca = self.player_attributes.current_ability as f32;
        let world_rep = self.player_attributes.world_reputation as f32;
        // Realistic European market: only players who could plausibly
        // play at that level. Floor at CA 130 / world_rep 4500 — a
        // Tier-3 squad player won't get a Champions League move.
        if ca < cfg.min_ca || world_rep < cfg.min_world_rep {
            return false;
        }
        // Player is at a credible top-tier league but the club isn't
        // in the path. Top-tier leagues weight more — a player in a
        // tier-3 league wanting Europe is unrealistic, so we gate
        // by main_league_tier.
        if ctx.main_league_tier > 2 {
            return false;
        }

        // Player must already feel the gap — either currently in
        // Europe or hungry for it. A loyal settled player in a club
        // outside Europe should not generate this mood unless the
        // numerical fit (ambition + CA) is overwhelming.
        let loyalty = self.attributes.loyalty.clamp(0.0, 20.0);
        if loyalty >= 16.0 && ambition < 17.0 {
            return false;
        }

        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.wants_european_competition;

        let mut desire_ctx =
            CareerDesireEventContext::new(CareerDesireKind::EuropeanCompetitionAmbition);
        desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::HighAmbition);
        desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::CurrentClubNotContinental);

        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationAdmiration,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_career_desire_context(desire_ctx)
        .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);

        self.happiness.add_event_with_context(
            HappinessEventType::WantsEuropeanCompetition,
            mag,
            None,
            happiness_ctx,
        );
        true
    }

    /// Emit the `WantsCopaLibertadores` mood for a South-American
    /// heritage player whose current club is outside South America or
    /// at a clearly sub-Libertadores level. Cooldown 60 days.
    fn detect_copa_libertadores_desire(
        &mut self,
        now: NaiveDate,
        ctx: &TransferDesireContext,
    ) -> bool {
        // Cooldown.
        if self
            .happiness
            .has_recent_event(&HappinessEventType::WantsCopaLibertadores, 60)
        {
            return false;
        }

        // Heritage gate: nationality continent must be South America.
        if ctx.player_nationality_continent_id != CONTINENT_SOUTH_AMERICA {
            return false;
        }

        let ambition = self.attributes.ambition;
        if ambition < 13.0 {
            return false;
        }
        let age = DateUtils::age(self.birth_date, now);
        if !(20..=32).contains(&age) {
            return false;
        }

        // If currently outside South America: always plausible. If at a
        // South American club, only fires for clearly sub-Libertadores
        // sides (low league reputation).
        let outside_south_america =
            ctx.club_continent_id != 0 && ctx.club_continent_id != CONTINENT_SOUTH_AMERICA;
        let weak_sa_club =
            ctx.club_continent_id == CONTINENT_SOUTH_AMERICA && ctx.league_reputation < 6500;
        if !outside_south_america && !weak_sa_club {
            return false;
        }

        // Realistic-market gate: low-ability players don't generate
        // this mood — they wouldn't be on a Libertadores side anyway.
        if self.player_attributes.current_ability < 110 {
            return false;
        }

        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.wants_copa_libertadores;
        let mut desire_ctx =
            CareerDesireEventContext::new(CareerDesireKind::CopaLibertadoresAmbition);
        if outside_south_america {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::DifferentContinent);
        }
        desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::HighAmbition);
        desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::CurrentClubNotContinental);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationAdmiration,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_career_desire_context(desire_ctx)
        .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);

        self.happiness.add_event_with_context(
            HappinessEventType::WantsCopaLibertadores,
            mag,
            None,
            happiness_ctx,
        );
        true
    }

    /// True when persistent return-home pressure justifies escalating
    /// to a transfer request. Reads recent `WantsReturnHome` events
    /// plus the player's adaptation signals.
    /// Emit the `WantsToLeaveAfterRelegation` mood — the post-relegation
    /// exodus. Fires for a recognised first-teamer good enough for the
    /// division the club just lost, in the months right after the drop.
    /// Keyed off the `Relegated` team-season event already sitting on
    /// the player, so the desire pass needs no league plumbing. The
    /// request-pressure twin escalates once the mood has lingered.
    fn detect_relegation_escape_desire(
        &mut self,
        now: NaiveDate,
        _ctx: &TransferDesireContext,
    ) -> bool {
        // The relegation must be fresh — the exodus happens in the
        // window(s) right after the drop, not a year later.
        if !self
            .happiness
            .has_recent_event(&HappinessEventType::Relegated, 150)
        {
            return false;
        }
        // Cooldown.
        if self
            .happiness
            .has_recent_event(&HappinessEventType::WantsToLeaveAfterRelegation, 60)
        {
            return false;
        }
        // Joined at (or after) the drop → he signed up for this level
        // knowingly; no exodus story.
        if self
            .days_since_transfer(now)
            .map(|d| d < 120)
            .unwrap_or(false)
        {
            return false;
        }
        // Only recognised first-teamers lead the exodus — squad players
        // are at their level either way, and the fringe is the listing
        // machinery's business.
        let is_first_teamer = self
            .contract
            .as_ref()
            .map(|c| {
                matches!(
                    c.squad_status,
                    PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
                )
            })
            .unwrap_or(false);
        if !is_first_teamer {
            return false;
        }
        let age = DateUtils::age(self.birth_date, now);
        if !(21..=31).contains(&age) {
            return false;
        }
        let ambition = self.attributes.ambition;
        if ambition < 11.0 {
            return false;
        }
        // Good enough that clubs at the old level would actually want him.
        let ca = self.player_attributes.current_ability;
        let world_rep = self.player_attributes.world_reputation;
        if ca < 105 && world_rep < 3000 {
            return false;
        }
        // A loyal servant stays and fights for promotion.
        let loyalty = self.attributes.loyalty.clamp(0.0, 20.0);
        if loyalty >= 16.0 && ambition < 16.0 {
            return false;
        }

        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.wants_to_leave_after_relegation;

        let mut desire_ctx =
            CareerDesireEventContext::new(CareerDesireKind::PostRelegationAmbition)
                .with_player_ability(ca)
                .with_evidence(CareerDesireEvidence::RelegatedWithClub);
        if ambition >= 14.0 {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::HighAmbition);
        }
        if (24..=31).contains(&age) {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::PrimeCareerWindow);
        }
        if world_rep >= 4500 {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::PlayerAboveClubLevel);
        }

        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationAdmiration,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_career_desire_context(desire_ctx)
        .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);

        self.happiness.add_event_with_context(
            HappinessEventType::WantsToLeaveAfterRelegation,
            mag,
            None,
            happiness_ctx,
        );
        true
    }

    /// True when the post-relegation exodus mood has lingered long
    /// enough to escalate to a formal transfer request. No `Unh`
    /// requirement — a perfectly settled professional still leaves to
    /// stay at the level (mirrors `OutgrownClub`, not the continental
    /// pressures, which gate on unhappiness).
    fn relegation_escape_pressure(&self, _now: NaiveDate) -> bool {
        let mood_count = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                e.event_type == HappinessEventType::WantsToLeaveAfterRelegation && e.days_ago <= 120
            })
            .count();
        mood_count >= 2
    }

    fn return_home_request_pressure(&self, now: NaiveDate, _ctx: &TransferDesireContext) -> bool {
        // Need a fair window (60-120 days post-transfer baseline).
        let days = match self.days_since_transfer(now) {
            Some(d) if d >= 60 => d,
            _ => return false,
        };

        // Must have at least two return-home moods in the last 90 days
        // (i.e. the chronic detector has fired at least twice across
        // its 60-day cooldown window) OR one mood combined with low
        // morale + poor club fit.
        let return_home_count = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::WantsReturnHome && e.days_ago <= 120)
            .count();
        if return_home_count == 0 {
            return false;
        }

        let weak_morale = self.happiness.morale < 35.0;
        let low_club_fit = self.happiness.factors.club_fit <= -3.0;
        if !(weak_morale || low_club_fit) {
            return false;
        }

        // High loyalty / professionalism delays — but doesn't fully
        // suppress — the request once the mood persists past 120 days.
        let loyalty = self.attributes.loyalty.clamp(0.0, 20.0);
        let prof = self.attributes.professionalism.clamp(0.0, 20.0);
        if (loyalty >= 16.0 || prof >= 16.0) && days < 150 && return_home_count < 2 {
            return false;
        }
        true
    }

    /// True when the European-competition mood has lingered long
    /// enough to escalate to a transfer request. Distinct from the
    /// Libertadores variant so `transfer_request_reasons` can record
    /// the right reason and the renderer can surface it.
    fn european_request_pressure(&self, _now: NaiveDate) -> bool {
        let has_unh = self
            .statuses
            .statuses
            .iter()
            .any(|s| s.status == PlayerStatusType::Unh);
        if !has_unh {
            return false;
        }
        let mood_count = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                e.event_type == HappinessEventType::WantsEuropeanCompetition && e.days_ago <= 120
            })
            .count();
        mood_count >= 2
    }

    /// Run the broader life-simulation desire detectors. Each fires
    /// at most once per cooldown window and emits a
    /// `LifeSimulationDesire` event tagged with the matching kind.
    /// Caller has already enforced honeymoon and loan guards.
    fn detect_life_simulation_desires(
        &mut self,
        now: NaiveDate,
        ctx: &TransferDesireContext,
        adaptation_score: f32,
    ) {
        let detector = LifeSimulationDesireDetector;
        detector.detect_language_tutor(self, now, ctx);
        detector.detect_mentor_support(self, now, ctx, adaptation_score);
        detector.detect_lower_pressure_club(self, now);
        detector.detect_tactical_role_request(self, now);
        detector.detect_cultural_familiarity(self, now, ctx);
        detector.detect_veteran_homecoming(self, now, ctx);
    }

    /// True when the Libertadores ambition mood has lingered long
    /// enough to escalate to a transfer request.
    fn libertadores_request_pressure(&self, _now: NaiveDate) -> bool {
        let has_unh = self
            .statuses
            .statuses
            .iter()
            .any(|s| s.status == PlayerStatusType::Unh);
        if !has_unh {
            return false;
        }
        let mood_count = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                e.event_type == HappinessEventType::WantsCopaLibertadores && e.days_ago <= 120
            })
            .count();
        mood_count >= 2
    }
}

/// Detector cluster for the broader life-simulation moods. Each
/// detector emits a single `LifeSimulationDesire` event with a closed
/// kind / severity / trigger context so the renderer can pick copy
/// without parsing free-text. Detectors are intentionally conservative:
/// the ambient mood layer is meant to colour the player's narrative,
/// not to spam the event log.
pub struct LifeSimulationDesireDetector;

impl LifeSimulationDesireDetector {
    fn emit(
        &self,
        player: &mut Player,
        kind: LifeSimulationDesireKind,
        severity: LifeSimulationSeverity,
        trigger: Option<LifeSimulationTrigger>,
        evidence: &[CareerDesireEvidence],
        cooldown_days: u16,
        cause: HappinessEventCause,
        scope: HappinessEventScope,
    ) -> bool {
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.life_simulation_desire;
        let severity_multiplier = match severity {
            LifeSimulationSeverity::Mild => 0.5,
            LifeSimulationSeverity::Moderate => 1.0,
            LifeSimulationSeverity::Strong => 1.5,
            LifeSimulationSeverity::Acute => 2.0,
        };
        let mag = base * severity_multiplier;
        let mut life_ctx = LifeSimulationDesireContext::new(kind, severity);
        if let Some(t) = trigger {
            life_ctx = life_ctx.with_trigger(t);
        }
        for ev in evidence {
            life_ctx = life_ctx.with_evidence(*ev);
        }
        let happiness_ctx =
            HappinessEventContext::new(cause, HappinessEventSeverity::from_magnitude(mag), scope)
                .with_life_simulation_desire_context(life_ctx);
        player.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::LifeSimulationDesire,
            mag,
            None,
            happiness_ctx,
            cooldown_days,
        )
    }

    /// Foreign player who's been at the club long enough to feel the
    /// language barrier but doesn't speak the local tongue. Asks for
    /// a tutor / cultural integration support before homesickness
    /// escalates.
    fn detect_language_tutor(
        &self,
        player: &mut Player,
        now: NaiveDate,
        ctx: &TransferDesireContext,
    ) {
        let days = match player.days_since_transfer(now) {
            Some(d) if (45..=300).contains(&d) => d,
            _ => return,
        };
        if player.speaks_local_language(&ctx.country_code) {
            return;
        }
        let adapt = player.attributes.adaptability.clamp(0.0, 20.0);
        let prof = player.attributes.professionalism.clamp(0.0, 20.0);
        // Mid-band adaptability + professionalism: actively wants help,
        // not so checked out they're already pushing for return-home.
        if adapt >= 14.0 || (adapt + prof) / 2.0 < 8.0 {
            return;
        }
        let severity = if days > 150 {
            LifeSimulationSeverity::Strong
        } else {
            LifeSimulationSeverity::Moderate
        };
        self.emit(
            player,
            LifeSimulationDesireKind::WantsLanguageTutor,
            severity,
            Some(LifeSimulationTrigger::LanguageBarrier),
            &[CareerDesireEvidence::NoLocalLanguage],
            120,
            HappinessEventCause::AdaptationIsolation,
            HappinessEventScope::Personal,
        );
    }

    /// Player asks for mentor / compatriot support — adaptation is
    /// shaky, no senior peer has stepped up. Falls between healthy
    /// settling and homesickness.
    fn detect_mentor_support(
        &self,
        player: &mut Player,
        now: NaiveDate,
        ctx: &TransferDesireContext,
        adaptation_score: f32,
    ) {
        let days = match player.days_since_transfer(now) {
            Some(d) if (60..=240).contains(&d) => d,
            _ => return,
        };
        let _ = days;
        // Already asking for return-home — no need to layer on a
        // mentor request.
        if player
            .happiness
            .has_recent_event(&HappinessEventType::WantsReturnHome, 60)
        {
            return;
        }
        if adaptation_score == 0.0 || adaptation_score >= 55.0 {
            return;
        }
        if ctx.same_language_or_nationality_teammates >= 2 {
            return;
        }
        self.emit(
            player,
            LifeSimulationDesireKind::WantsMentorSupport,
            LifeSimulationSeverity::Moderate,
            Some(LifeSimulationTrigger::LackOfMentor),
            &[
                CareerDesireEvidence::NoCompatriotSupport,
                CareerDesireEvidence::PoorAdaptationScore,
            ],
            150,
            HappinessEventCause::AdaptationIsolation,
            HappinessEventScope::DressingRoom,
        );
    }

    /// High-CA player at a top-rep club under a media / fan-criticism
    /// cycle would prefer a lower-pressure environment.
    fn detect_lower_pressure_club(&self, player: &mut Player, _now: NaiveDate) {
        let ca = player.player_attributes.current_ability;
        if ca < 130 {
            return;
        }
        let recent_media_pressure = player.happiness.recent_events.iter().any(|e| {
            matches!(
                e.event_type,
                HappinessEventType::MediaCriticism | HappinessEventType::FanCriticism
            ) && e.days_ago <= 60
        });
        if !recent_media_pressure {
            return;
        }
        let pressure_attr = player.attributes.pressure.clamp(0.0, 20.0);
        if pressure_attr >= 14.0 {
            return;
        }
        self.emit(
            player,
            LifeSimulationDesireKind::WantsLowerPressureClub,
            LifeSimulationSeverity::Moderate,
            Some(LifeSimulationTrigger::MediaAbuseOrFanCriticism),
            &[],
            180,
            HappinessEventCause::MediaPressure,
            HappinessEventScope::Media,
        );
    }

    /// Player asks for a tactical role / position change after repeated
    /// role-mismatch events.
    fn detect_tactical_role_request(&self, player: &mut Player, _now: NaiveDate) {
        let mismatch_count = player
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                matches!(e.event_type, HappinessEventType::RoleMismatch) && e.days_ago <= 90
            })
            .count();
        if mismatch_count < 2 {
            return;
        }
        self.emit(
            player,
            LifeSimulationDesireKind::WantsPreferredTacticalRole,
            LifeSimulationSeverity::Moderate,
            Some(LifeSimulationTrigger::TacticalMisuse),
            &[],
            120,
            HappinessEventCause::TacticalDisagreement,
            HappinessEventScope::MatchDay,
        );
    }

    /// Foreign player whose nationality continent matches a
    /// linguistically/culturally close cluster (e.g. Argentinian in
    /// Spain) gets a positive cultural-fit recognition. Lower
    /// magnitude than the home-return path because they're not
    /// actually home — just close enough that the transfer feels
    /// natural.
    fn detect_cultural_familiarity(
        &self,
        player: &mut Player,
        now: NaiveDate,
        ctx: &TransferDesireContext,
    ) {
        let days = match player.days_since_transfer(now) {
            Some(d) if (30..=180).contains(&d) => d,
            _ => return,
        };
        let _ = days;
        if ctx.club_in_home_country {
            return;
        }
        if !player.speaks_local_language(&ctx.country_code) {
            return;
        }
        // Limit to South-American → Spain / Portugal-style affinity:
        // same continent (Europe→Europe, SA→SA), or shared language at
        // native level. Use language presence as the proxy here.
        let has_native_language_match = player.languages.iter().any(|l| l.is_native);
        if !has_native_language_match {
            return;
        }
        self.emit(
            player,
            LifeSimulationDesireKind::PrefersCulturalFamiliarity,
            LifeSimulationSeverity::Mild,
            Some(LifeSimulationTrigger::CulturalFitRecognition),
            &[CareerDesireEvidence::HomeOrFavouriteLink],
            240,
            HappinessEventCause::NationalityIntegration,
            HappinessEventScope::Personal,
        );
    }

    /// Veteran (32+) at a foreign club with a recent return-home mood
    /// and approaching contract end signals an explicit desire for a
    /// final homecoming season.
    fn detect_veteran_homecoming(
        &self,
        player: &mut Player,
        now: NaiveDate,
        ctx: &TransferDesireContext,
    ) {
        let age = DateUtils::age(player.birth_date, now);
        if age < 32 {
            return;
        }
        if ctx.club_in_home_country {
            return;
        }
        let return_home_mood = player
            .happiness
            .has_recent_event(&HappinessEventType::WantsReturnHome, 180);
        if !return_home_mood {
            return;
        }
        let contract_close = player
            .contract
            .as_ref()
            .map(|c| {
                let now_dt = NaiveDateTime::new(now, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
                c.days_to_expiration(now_dt) < 540
            })
            .unwrap_or(true);
        if !contract_close {
            return;
        }
        self.emit(
            player,
            LifeSimulationDesireKind::VeteranHomecomingSeason,
            LifeSimulationSeverity::Strong,
            Some(LifeSimulationTrigger::LateCareerWindow),
            &[CareerDesireEvidence::HomeOrFavouriteLink],
            240,
            HappinessEventCause::AdaptationIsolation,
            HappinessEventScope::Personal,
        );
    }
}

#[cfg(test)]
mod career_desire_tests {
    use super::*;
    use crate::club::player::adaptation::AdaptationFailureSignals;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn person(
        ambition: f32,
        adaptability: f32,
        loyalty: f32,
        professionalism: f32,
    ) -> PersonAttributes {
        PersonAttributes {
            adaptability,
            ambition,
            controversy: 10.0,
            loyalty,
            pressure: 10.0,
            professionalism,
            sportsmanship: 10.0,
            temperament: 10.0,
            consistency: 10.0,
            important_matches: 10.0,
            dirtiness: 10.0,
        }
    }

    fn build(
        age: u8,
        ambition: f32,
        adaptability: f32,
        loyalty: f32,
        professionalism: f32,
        country_id: u32,
        ca: u8,
        world_rep: i16,
        days_at_club: i64,
        today: NaiveDate,
    ) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.world_reputation = world_rep;
        attrs.current_reputation = world_rep;
        attrs.current_ability = ca;
        attrs.potential_ability = ca;
        let birth = today
            .checked_sub_signed(chrono::Duration::days(age as i64 * 365))
            .unwrap();
        let mut p = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".into(), "Player".into()))
            .birth_date(birth)
            .country_id(country_id)
            .attributes(person(ambition, adaptability, loyalty, professionalism))
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        // Set the transfer date so adaptation timing can be measured.
        if days_at_club > 0 {
            p.last_transfer_date = today.checked_sub_signed(chrono::Duration::days(days_at_club));
        }
        p
    }

    fn count_event(p: &Player, kind: HappinessEventType) -> usize {
        p.happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == kind)
            .count()
    }

    fn with_status(mut p: Player, status: PlayerSquadStatus) -> Player {
        let mut c = PlayerClubContract::new(500_000, NaiveDate::from_ymd_opt(2029, 6, 30).unwrap());
        c.squad_status = status;
        p.contract = Some(c);
        p
    }

    // ── Post-relegation exodus ──────────────────────────────────

    #[test]
    fn relegated_first_teamer_wants_out() {
        let today = d(2026, 6, 1);
        let mut p = with_status(
            build(26, 14.0, 12.0, 10.0, 12.0, 1, 125, 4000, 800, today),
            PlayerSquadStatus::KeyPlayer,
        );
        p.happiness.add_event(HappinessEventType::Relegated, -5.0);
        p.detect_relegation_escape_desire(today, &TransferDesireContext::default());
        assert_eq!(
            count_event(&p, HappinessEventType::WantsToLeaveAfterRelegation),
            1,
            "a quality first-teamer at a just-relegated club should want to stay at the level"
        );
    }

    #[test]
    fn no_relegation_means_no_exodus_mood() {
        let today = d(2026, 6, 1);
        let mut p = with_status(
            build(26, 14.0, 12.0, 10.0, 12.0, 1, 125, 4000, 800, today),
            PlayerSquadStatus::KeyPlayer,
        );
        p.detect_relegation_escape_desire(today, &TransferDesireContext::default());
        assert_eq!(
            count_event(&p, HappinessEventType::WantsToLeaveAfterRelegation),
            0
        );
    }

    #[test]
    fn loyal_servant_stays_after_relegation() {
        let today = d(2026, 6, 1);
        let mut p = with_status(
            build(26, 12.0, 12.0, 17.0, 12.0, 1, 125, 4000, 800, today),
            PlayerSquadStatus::KeyPlayer,
        );
        p.happiness.add_event(HappinessEventType::Relegated, -5.0);
        p.detect_relegation_escape_desire(today, &TransferDesireContext::default());
        assert_eq!(
            count_event(&p, HappinessEventType::WantsToLeaveAfterRelegation),
            0,
            "a loyal servant stays and fights for promotion"
        );
    }

    #[test]
    fn squad_player_is_not_part_of_the_exodus() {
        let today = d(2026, 6, 1);
        let mut p = with_status(
            build(26, 14.0, 12.0, 10.0, 12.0, 1, 125, 4000, 800, today),
            PlayerSquadStatus::MainBackupPlayer,
        );
        p.happiness.add_event(HappinessEventType::Relegated, -5.0);
        p.detect_relegation_escape_desire(today, &TransferDesireContext::default());
        assert_eq!(
            count_event(&p, HappinessEventType::WantsToLeaveAfterRelegation),
            0,
            "squad players are at their level either way"
        );
    }

    #[test]
    fn relegation_pressure_needs_a_lingering_mood() {
        let today = d(2026, 6, 1);
        let mut p = with_status(
            build(26, 14.0, 12.0, 10.0, 12.0, 1, 125, 4000, 800, today),
            PlayerSquadStatus::KeyPlayer,
        );
        p.happiness
            .add_event(HappinessEventType::WantsToLeaveAfterRelegation, -4.0);
        assert!(
            !p.relegation_escape_pressure(today),
            "one mood note is not yet a formal request"
        );
        p.happiness
            .add_event(HappinessEventType::WantsToLeaveAfterRelegation, -4.0);
        assert!(
            p.relegation_escape_pressure(today),
            "a lingering exodus mood escalates to a transfer request"
        );
    }

    #[test]
    fn isolated_south_american_in_asia_emits_return_home_mood() {
        // Low-adaptability South American striker, 90 days into a move
        // to a club on a different continent (Asia, id=4), no language,
        // no compatriots, weak club_fit, repeated isolation events. The
        // chronic-failure detector should fire WantsReturnHome.
        let today = d(2026, 5, 1);
        let mut p = build(
            27, /* ambition */ 12.0, /* adaptability */ 6.0, /* loyalty */ 10.0,
            /* professionalism */ 10.0, /* country_id (SA) */ 30, 130, 4500, 90, today,
        );
        // Push a couple of FeelingIsolated events so the detector sees
        // chronic isolation.
        p.happiness
            .add_event(HappinessEventType::FeelingIsolated, -2.0);
        p.happiness
            .add_event(HappinessEventType::FeelingIsolated, -2.0);
        p.happiness.factors.club_fit = -5.0;
        p.happiness.morale = 30.0;

        let signals = AdaptationFailureSignals {
            player_nationality_continent_id: 3, // South America
            club_continent_id: 4,               // Asia
            club_in_home_country: false,
            destination_is_favourite: false,
            same_language_or_nationality_teammates: 0,
            adaptation_score: 25.0, // poor
            club_fit: -5.0,
        };
        let fired = p.process_chronic_adaptation_failure(today, "jp", &signals);
        assert!(fired, "expected WantsReturnHome to fire");
        assert_eq!(count_event(&p, HappinessEventType::WantsReturnHome), 1);
    }

    #[test]
    fn well_supported_compatriot_player_does_not_emit_return_home() {
        // Same shape but high adaptability, speaks the language, has a
        // compatriot, neutral club_fit. Detector should hold its fire.
        let today = d(2026, 5, 1);
        let mut p = build(27, 12.0, 16.0, 10.0, 14.0, 30, 130, 4500, 90, today);
        // Add the player's native language at a level that counts as
        // local-language fluent for the test country code.
        p.languages
            .push(crate::club::player::language::PlayerLanguage {
                language: crate::club::player::language::Language::English,
                proficiency: 100,
                is_native: true,
            });
        p.happiness.morale = 60.0;
        p.happiness.factors.club_fit = 1.0;

        let signals = AdaptationFailureSignals {
            player_nationality_continent_id: 3,
            club_continent_id: 1, // Europe (different)
            club_in_home_country: false,
            destination_is_favourite: false,
            same_language_or_nationality_teammates: 2,
            adaptation_score: 75.0,
            club_fit: 1.0,
        };
        let fired = p.process_chronic_adaptation_failure(today, "gb", &signals);
        assert!(!fired, "should not fire for well-supported player");
        assert_eq!(count_event(&p, HappinessEventType::WantsReturnHome), 0);
    }

    #[test]
    fn return_home_does_not_fire_in_honeymoon_window() {
        // Same isolation profile, only 30 days at club — under the
        // 60-day honeymoon. Must not fire.
        let today = d(2026, 5, 1);
        let mut p = build(27, 12.0, 6.0, 10.0, 10.0, 30, 130, 4500, 30, today);
        p.happiness
            .add_event(HappinessEventType::FeelingIsolated, -2.0);
        p.happiness
            .add_event(HappinessEventType::FeelingIsolated, -2.0);
        p.happiness.morale = 30.0;
        p.happiness.factors.club_fit = -5.0;
        let signals = AdaptationFailureSignals {
            player_nationality_continent_id: 3,
            club_continent_id: 4,
            club_in_home_country: false,
            destination_is_favourite: false,
            same_language_or_nationality_teammates: 0,
            adaptation_score: 25.0,
            club_fit: -5.0,
        };
        assert!(!p.process_chronic_adaptation_failure(today, "jp", &signals));
        assert_eq!(count_event(&p, HappinessEventType::WantsReturnHome), 0);
    }

    #[test]
    fn return_home_cooldown_prevents_repeat_within_60_days() {
        let today = d(2026, 5, 1);
        let mut p = build(27, 12.0, 6.0, 10.0, 10.0, 30, 130, 4500, 90, today);
        p.happiness
            .add_event(HappinessEventType::FeelingIsolated, -2.0);
        p.happiness
            .add_event(HappinessEventType::FeelingIsolated, -2.0);
        p.happiness.morale = 30.0;
        p.happiness.factors.club_fit = -5.0;
        let signals = AdaptationFailureSignals {
            player_nationality_continent_id: 3,
            club_continent_id: 4,
            club_in_home_country: false,
            destination_is_favourite: false,
            same_language_or_nationality_teammates: 0,
            adaptation_score: 25.0,
            club_fit: -5.0,
        };
        assert!(p.process_chronic_adaptation_failure(today, "jp", &signals));
        // Re-fire within cooldown is suppressed.
        assert!(!p.process_chronic_adaptation_failure(today, "jp", &signals));
        assert_eq!(count_event(&p, HappinessEventType::WantsReturnHome), 1);
    }

    #[test]
    fn ambitious_top_tier_player_emits_european_competition_desire() {
        // 26yo CA 150 player, ambition 17, at a top-tier club not
        // currently in the continental path. Should emit
        // WantsEuropeanCompetition.
        let today = d(2026, 5, 1);
        let mut p = build(26, 17.0, 12.0, 10.0, 12.0, 1, 150, 5500, 60, today);
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "gb".to_string();
        ctx.club_continent_id = 1;
        ctx.player_nationality_continent_id = 1;
        ctx.league_reputation = 9000;
        ctx.club_reputation = 0.55;
        ctx.main_league_tier = 1;
        ctx.has_continental_path_hint = false;
        let fired = p.detect_continental_competition_desire(today, &ctx);
        assert!(fired, "ambitious top-tier player should fire");
        assert_eq!(
            count_event(&p, HappinessEventType::WantsEuropeanCompetition),
            1
        );
    }

    #[test]
    fn low_ability_player_does_not_get_european_desire() {
        let today = d(2026, 5, 1);
        // CA only 90 — wouldn't realistically land at a UCL club.
        let mut p = build(26, 17.0, 12.0, 10.0, 12.0, 1, 90, 2500, 60, today);
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "gb".to_string();
        ctx.club_continent_id = 1;
        ctx.player_nationality_continent_id = 1;
        ctx.league_reputation = 9000;
        ctx.club_reputation = 0.55;
        ctx.main_league_tier = 1;
        let fired = p.detect_continental_competition_desire(today, &ctx);
        assert!(!fired, "low CA player should not fire European desire");
    }

    #[test]
    fn club_already_on_continental_path_suppresses_european_desire() {
        let today = d(2026, 5, 1);
        let mut p = build(26, 17.0, 12.0, 10.0, 12.0, 1, 150, 5500, 60, today);
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "gb".to_string();
        ctx.club_continent_id = 1;
        ctx.player_nationality_continent_id = 1;
        ctx.league_reputation = 9000;
        ctx.club_reputation = 0.7;
        ctx.main_league_tier = 1;
        ctx.has_continental_path_hint = true;
        let fired = p.detect_continental_competition_desire(today, &ctx);
        assert!(!fired, "should not fire when already in continental path");
    }

    #[test]
    fn south_american_outside_libertadores_path_emits_desire() {
        // SA-heritage player at an Asian club with high ambition + CA.
        let today = d(2026, 5, 1);
        let mut p = build(24, 14.0, 12.0, 10.0, 12.0, 30, 130, 5000, 60, today);
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "jp".to_string();
        ctx.club_continent_id = 4;
        ctx.player_nationality_continent_id = 3;
        ctx.league_reputation = 4000;
        ctx.main_league_tier = 1;
        let fired = p.detect_copa_libertadores_desire(today, &ctx);
        assert!(
            fired,
            "SA player outside SA should fire Libertadores desire"
        );
        assert_eq!(
            count_event(&p, HappinessEventType::WantsCopaLibertadores),
            1
        );
    }

    #[test]
    fn weak_sa_club_emits_libertadores_desire_for_high_ambition_player() {
        let today = d(2026, 5, 1);
        let mut p = build(24, 16.0, 12.0, 10.0, 12.0, 30, 140, 5500, 60, today);
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "ar".to_string();
        ctx.club_continent_id = 3; // South America
        ctx.player_nationality_continent_id = 3;
        ctx.league_reputation = 4500; // sub-Libertadores tier
        ctx.main_league_tier = 1;
        let fired = p.detect_copa_libertadores_desire(today, &ctx);
        assert!(
            fired,
            "high-ambition player at sub-Libertadores SA club should fire"
        );
    }

    #[test]
    fn non_south_american_player_never_fires_libertadores_desire() {
        let today = d(2026, 5, 1);
        let mut p = build(24, 16.0, 12.0, 10.0, 12.0, 1, 140, 5500, 60, today);
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "es".to_string();
        ctx.club_continent_id = 1;
        ctx.player_nationality_continent_id = 1;
        ctx.league_reputation = 9000;
        ctx.main_league_tier = 1;
        let fired = p.detect_copa_libertadores_desire(today, &ctx);
        assert!(!fired);
    }

    #[test]
    fn low_ambition_player_does_not_spam_career_desire_moods() {
        // Ambition 9, CA 150 — should not fire either desire.
        let today = d(2026, 5, 1);
        let mut p = build(26, 9.0, 12.0, 10.0, 12.0, 30, 150, 5500, 60, today);
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "jp".to_string();
        ctx.club_continent_id = 4;
        ctx.player_nationality_continent_id = 3;
        ctx.league_reputation = 5000;
        ctx.main_league_tier = 1;
        assert!(!p.detect_continental_competition_desire(today, &ctx));
        assert!(!p.detect_copa_libertadores_desire(today, &ctx));
    }

    #[test]
    fn process_transfer_desire_honeymoon_blocks_career_desire_emission() {
        let today = d(2026, 5, 1);
        let mut p = build(27, 12.0, 6.0, 10.0, 10.0, 30, 130, 4500, 10, today);
        // Even with the worst signals, recently-transferred player
        // doesn't get desire moods staged.
        p.happiness.morale = 30.0;
        p.happiness.factors.club_fit = -5.0;
        let mut result = PlayerResult::new(p.id);
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "jp".to_string();
        ctx.club_continent_id = 4;
        ctx.player_nationality_continent_id = 3;
        ctx.league_reputation = 4000;
        ctx.main_league_tier = 1;
        p.process_transfer_desire(&mut result, today, &ctx);
        assert_eq!(count_event(&p, HappinessEventType::WantsReturnHome), 0);
        assert_eq!(
            count_event(&p, HappinessEventType::WantsCopaLibertadores),
            0
        );
    }

    // ── Item 4: Req-reason tracking ─────────────────────────────

    #[test]
    fn req_persists_while_one_reason_unresolved() {
        // Player has both salary-unhappiness AND ambition-mismatch
        // active. Resolving salary alone must not clear Req while
        // ambition is still red.
        let today = d(2026, 5, 1);
        let mut p = build(28, 14.0, 12.0, 10.0, 12.0, 30, 130, 5000, 200, today);
        p.happiness.factors.ambition_fit = -8.0;
        p.statuses.add(
            today
                .checked_sub_signed(chrono::Duration::days(20))
                .unwrap(),
            PlayerStatusType::Unh,
        );
        p.happiness.last_salary_negotiation = Some(
            today
                .checked_sub_signed(chrono::Duration::days(600))
                .unwrap(),
        );
        p.happiness.factors.salary_satisfaction = -6.0;
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "es".to_string();
        ctx.player_nationality_continent_id = 1;
        let mut result = PlayerResult::new(p.id);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(p.statuses.has(PlayerStatusType::Req));
        assert!(
            p.transfer_request_reasons
                .contains(&TransferRequestReason::SalaryUnresolved)
        );
        assert!(
            p.transfer_request_reasons
                .contains(&TransferRequestReason::AmbitionMismatch)
        );

        // Salary issue resolves — but ambition mismatch remains.
        p.happiness.factors.salary_satisfaction = 0.0;
        p.happiness.last_salary_negotiation = None;
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(
            p.statuses.has(PlayerStatusType::Req),
            "Req must persist while ambition still red"
        );
        assert!(
            !p.transfer_request_reasons
                .contains(&TransferRequestReason::SalaryUnresolved)
        );
        assert!(
            p.transfer_request_reasons
                .contains(&TransferRequestReason::AmbitionMismatch)
        );
    }

    #[test]
    fn outgrown_club_ambitious_player_requests_step_up_without_unhappiness() {
        // Ambitious (16), young (24), long-settled (400d) striker whose
        // club is far beneath his stature (ambition_fit -10) — but he is
        // NOT unhappy. He should still hand in a request to step up, via
        // the new OutgrownClub reason, with no Unh ever on his record.
        let today = d(2026, 5, 1);
        let mut p = build(24, 16.0, 12.0, 10.0, 12.0, 1, 150, 6000, 400, today);
        p.happiness.factors.ambition_fit = -10.0;
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "es".to_string();
        ctx.player_nationality_continent_id = 1;
        let mut result = PlayerResult::new(p.id);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(
            p.transfer_request_reasons
                .contains(&TransferRequestReason::OutgrownClub),
            "an ambitious player who outgrew his club should request a step up"
        );
        assert!(p.statuses.has(PlayerStatusType::Req));
        assert!(
            !p.statuses.has(PlayerStatusType::Unh),
            "OutgrownClub must not require the player to be unhappy first"
        );
        assert!(
            !p.transfer_request_reasons
                .contains(&TransferRequestReason::AmbitionMismatch),
            "AmbitionMismatch needs Unh; only OutgrownClub should fire here"
        );
    }

    #[test]
    fn content_player_at_fitting_club_does_not_request_step_up() {
        // Same ambitious young player, but his club fits his stature
        // (ambition_fit only mildly negative) — no desire to move on.
        let today = d(2026, 5, 1);
        let mut p = build(24, 16.0, 12.0, 10.0, 12.0, 1, 150, 6000, 400, today);
        p.happiness.factors.ambition_fit = -3.0;
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "es".to_string();
        ctx.player_nationality_continent_id = 1;
        let mut result = PlayerResult::new(p.id);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(
            !p.transfer_request_reasons
                .contains(&TransferRequestReason::OutgrownClub)
        );
        assert!(!p.statuses.has(PlayerStatusType::Req));
    }

    #[test]
    fn low_ambition_servant_does_not_outgrow_club() {
        // A deep prestige deficit, but the player simply isn't ambitious
        // (12): a loyal lower-league servant doesn't agitate to leave.
        // Discovering HIM is the buyer side's job, not a want-away.
        let today = d(2026, 5, 1);
        let mut p = build(24, 12.0, 12.0, 14.0, 12.0, 1, 150, 6000, 400, today);
        p.happiness.factors.ambition_fit = -12.0;
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "es".to_string();
        ctx.player_nationality_continent_id = 1;
        let mut result = PlayerResult::new(p.id);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(
            !p.transfer_request_reasons
                .contains(&TransferRequestReason::OutgrownClub),
            "a low-ambition player doesn't agitate even at a too-small club"
        );
    }

    #[test]
    fn long_serving_ambitious_player_wants_a_new_challenge() {
        let today = d(2026, 5, 1);
        // 28-y-o, very ambitious (17), below-average loyalty (8), nine years
        // at the same club. Even with a healthy ambition_fit (the club still
        // fits him — he hasn't outgrown it) he wants a fresh test elsewhere.
        let mut p = build(28, 17.0, 12.0, 8.0, 12.0, 1, 150, 6000, 9 * 365, today);
        p.happiness.factors.ambition_fit = 2.0; // club still fits — not outgrown
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "es".to_string();
        ctx.player_nationality_continent_id = 1;
        let mut result = PlayerResult::new(p.id);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(
            p.transfer_request_reasons
                .contains(&TransferRequestReason::NewChallenge),
            "a long-serving ambitious player should want a new challenge"
        );
        assert!(p.statuses.has(PlayerStatusType::Req));
        assert!(
            !p.transfer_request_reasons
                .contains(&TransferRequestReason::OutgrownClub),
            "he hasn't outgrown the club — the itch is tenure, not stature"
        );
    }

    #[test]
    fn loyal_one_club_man_stays_despite_long_service() {
        let today = d(2026, 5, 1);
        // Same long service and ambition, but a fiercely loyal club legend
        // (loyalty 18) — he stays put.
        let mut p = build(28, 17.0, 12.0, 18.0, 12.0, 1, 150, 6000, 9 * 365, today);
        p.happiness.factors.ambition_fit = 2.0;
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "es".to_string();
        ctx.player_nationality_continent_id = 1;
        let mut result = PlayerResult::new(p.id);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(
            !p.transfer_request_reasons
                .contains(&TransferRequestReason::NewChallenge),
            "a fiercely loyal club legend stays despite long service"
        );
    }

    #[test]
    fn recently_arrived_player_has_no_new_challenge_itch() {
        let today = d(2026, 5, 1);
        // Same ambitious profile but only two years at the club — far too
        // freshly settled to be restless yet.
        let mut p = build(28, 17.0, 12.0, 8.0, 12.0, 1, 150, 6000, 2 * 365, today);
        p.happiness.factors.ambition_fit = 2.0;
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "es".to_string();
        ctx.player_nationality_continent_id = 1;
        let mut result = PlayerResult::new(p.id);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(
            !p.transfer_request_reasons
                .contains(&TransferRequestReason::NewChallenge),
            "a player only two years in isn't itching for a move"
        );
    }

    #[test]
    fn req_clears_when_all_reasons_resolved() {
        let today = d(2026, 5, 1);
        let mut p = build(28, 14.0, 12.0, 10.0, 12.0, 30, 130, 5000, 200, today);
        // Unhappy well past the six-month listing gate so LongUnhappiness is
        // the single active reason driving the request.
        p.statuses.add(
            today
                .checked_sub_signed(chrono::Duration::days(200))
                .unwrap(),
            PlayerStatusType::Unh,
        );
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "es".to_string();
        ctx.player_nationality_continent_id = 1;
        let mut result = PlayerResult::new(p.id);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(p.statuses.has(PlayerStatusType::Req));

        // Unh status removed — no other reason active.
        p.statuses.remove(PlayerStatusType::Unh);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(!p.statuses.has(PlayerStatusType::Req));
        assert!(p.transfer_request_reasons.is_empty());
    }

    // ── Item 5: European vs Libertadores priority ───────────────

    #[test]
    fn priority_picks_libertadores_for_sa_at_asian_club() {
        let today = d(2026, 5, 1);
        let p = build(24, 14.0, 12.0, 10.0, 12.0, 30, 130, 5000, 60, today);
        let mut ctx = TransferDesireContext::default();
        ctx.club_continent_id = 4; // Asia
        ctx.player_nationality_continent_id = 3; // South America
        let kind = p.primary_career_desire(today, &ctx);
        assert_eq!(kind, Some(CareerDesireKind::CopaLibertadoresAmbition));
    }

    #[test]
    fn priority_picks_european_for_elite_young_sa_in_europe() {
        let today = d(2026, 5, 1);
        let p = build(23, 16.0, 12.0, 10.0, 12.0, 30, 150, 7000, 60, today);
        let mut ctx = TransferDesireContext::default();
        ctx.club_continent_id = 1; // Europe
        ctx.player_nationality_continent_id = 3; // South America
        let kind = p.primary_career_desire(today, &ctx);
        assert_eq!(kind, Some(CareerDesireKind::EuropeanCompetitionAmbition));
    }

    // ── Item 6: Continental path heuristic ──────────────────────

    /// Access context that isolates the season-position fallback:
    /// `club_continent_id == 0` keeps the elite-European floors inert and
    /// the live continental-cup fields default to unknown.
    fn position_access(
        main_league_tier: u8,
        league_reputation: u16,
        league_position: u8,
        league_size: u8,
        season_progress: f32,
    ) -> ContinentalAccessContext {
        ContinentalAccessContext {
            main_league_tier,
            league_reputation,
            league_position,
            league_size,
            season_progress,
            club_continent_id: 0,
            ..Default::default()
        }
    }

    #[test]
    fn continental_path_heuristic_pre_quarter_season_uses_rep() {
        let access = position_access(1, 8000, 12, 20, 0.10);
        assert!(
            ContinentalPathHeuristic::from_access(&access).is_on_path(&access),
            "high-rep tier-1 club is presumed on path before 25% season"
        );
    }

    #[test]
    fn continental_path_heuristic_position_load_bearing_post_quarter() {
        let access = position_access(1, 9000, 12, 20, 0.50);
        assert!(
            !ContinentalPathHeuristic::from_access(&access).is_on_path(&access),
            "12th-place team is no longer on the path past quarter season"
        );
    }

    #[test]
    fn continental_path_heuristic_low_rep_league_never_on_path() {
        let access = position_access(1, 3500, 1, 20, 0.80);
        assert!(!ContinentalPathHeuristic::from_access(&access).is_on_path(&access));
    }

    // ── Elite-club continental-access floors (Mbappe-at-Real-Madrid) ──

    /// Mirror of `from_global`'s wiring for tests: compute the path hint
    /// from a [`ContinentalAccessContext`] and project it onto a
    /// [`TransferDesireContext`] the desire detector reads.
    fn desire_ctx_from_access(
        access: &ContinentalAccessContext,
        player_continent: u32,
    ) -> TransferDesireContext {
        let hint = ContinentalPathHeuristic::from_access(access).is_on_path(access);
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "gb".to_string();
        ctx.club_continent_id = access.club_continent_id;
        ctx.player_nationality_continent_id = player_continent;
        ctx.league_reputation = access.league_reputation;
        ctx.club_reputation = access.club_reputation;
        ctx.league_position = access.league_position;
        ctx.league_size = access.league_size;
        ctx.season_progress = access.season_progress;
        ctx.main_league_tier = access.main_league_tier;
        ctx.has_continental_path_hint = hint;
        ctx.continental_path_known_absent = access.path_known_absent();
        ctx
    }

    #[test]
    fn mbappe_at_real_madrid_never_wants_european_football() {
        // Super-elite European club. Reputation alone clears the path —
        // even with no league-table data (position 0) or a poor table
        // position (6th), the mood must stay silent.
        let today = d(2026, 5, 1);
        for position in [0u8, 6u8] {
            let mut p = build(27, 18.0, 12.0, 10.0, 12.0, 1, 190, 9500, 200, today);
            let access = ContinentalAccessContext {
                club_reputation: 0.95,
                league_reputation: 9000,
                main_league_tier: 1,
                league_position: position,
                league_size: 20,
                season_progress: 0.5,
                club_continent_id: 1, // Europe
                ..Default::default()
            };
            let ctx = desire_ctx_from_access(&access, 1);
            assert!(
                ctx.has_continental_path_hint,
                "super-elite European club must register as on-path (pos {position})"
            );
            let fired = p.detect_continental_competition_desire(today, &ctx);
            assert!(
                !fired,
                "Mbappe at Real Madrid must not want European football (pos {position})"
            );
            assert_eq!(
                count_event(&p, HappinessEventType::WantsEuropeanCompetition),
                0
            );
        }
    }

    #[test]
    fn elite_club_currently_in_champions_league_suppresses_desire() {
        let today = d(2026, 5, 1);
        let mut p = build(27, 18.0, 12.0, 10.0, 12.0, 1, 175, 8500, 200, today);
        let access = ContinentalAccessContext {
            current_continental_competition: Some(ContinentalCompetitionTier::ChampionsLeague),
            // Modest reputation so only the current-participant rule can
            // suppress — proves rule 2 is doing the work.
            club_reputation: 0.6,
            league_reputation: 9000,
            main_league_tier: 1,
            league_position: 8,
            league_size: 20,
            season_progress: 0.5,
            club_continent_id: 1,
            ..Default::default()
        };
        let ctx = desire_ctx_from_access(&access, 1);
        assert!(ctx.has_continental_path_hint);
        assert!(!p.detect_continental_competition_desire(today, &ctx));
    }

    #[test]
    fn top_player_at_mid_table_premier_league_club_can_emit() {
        let today = d(2026, 5, 1);
        let mut p = build(27, 17.0, 12.0, 10.0, 12.0, 1, 160, 7000, 200, today);
        let access = ContinentalAccessContext {
            club_reputation: 0.55,
            league_reputation: 9000,
            main_league_tier: 1,
            league_position: 12,
            league_size: 20,
            season_progress: 0.5,
            club_continent_id: 1,
            ..Default::default()
        };
        let ctx = desire_ctx_from_access(&access, 1);
        assert!(
            !ctx.has_continental_path_hint,
            "12th of a high-rep league mid-season is not on the path"
        );
        assert!(
            p.detect_continental_competition_desire(today, &ctx),
            "ambitious top player at a mid-table club should be able to want Europe"
        );
    }

    #[test]
    fn elite_club_with_continental_ban_may_emit() {
        // A banned elite club genuinely can't offer Europe this season,
        // so its star's ambition frustration is legitimate. The ban must
        // override the elite-reputation shortcut in the path hint, and the
        // detector's own elite-rep suppression must not re-block it.
        let today = d(2026, 5, 1);
        let mut p = build(27, 18.0, 12.0, 10.0, 12.0, 1, 185, 9000, 200, today);
        let access = ContinentalAccessContext {
            club_reputation: 0.90,
            league_reputation: 9000,
            main_league_tier: 1,
            league_position: 9, // outside qualification
            league_size: 20,
            season_progress: 0.5,
            club_continent_id: 1,
            continental_ban: true,
            ..Default::default()
        };
        let ctx = desire_ctx_from_access(&access, 1);
        assert!(
            !ctx.has_continental_path_hint,
            "a continental ban must keep the club off the path"
        );
        assert!(
            p.detect_continental_competition_desire(today, &ctx),
            "a banned elite club's star may still want European football"
        );
    }

    #[test]
    fn unknown_league_table_does_not_emit_for_elite_club() {
        // No league-table data (position 0). An elite club must not be
        // judged "no Europe" on missing data — the reputation floor holds.
        let today = d(2026, 5, 1);
        let mut p = build(27, 18.0, 12.0, 10.0, 12.0, 1, 185, 9000, 200, today);
        let access = ContinentalAccessContext {
            club_reputation: 0.90,
            league_reputation: 9000,
            main_league_tier: 1,
            league_position: 0,
            league_size: 0,
            season_progress: 0.5,
            club_continent_id: 1,
            ..Default::default()
        };
        let ctx = desire_ctx_from_access(&access, 1);
        assert!(
            ctx.has_continental_path_hint,
            "missing table data must not strip an elite club off the path"
        );
        assert!(!p.detect_continental_competition_desire(today, &ctx));
    }

    #[test]
    fn low_reputation_top_flight_club_may_emit() {
        // Top division but a small club not in any continental cup. An
        // ambitious high-level player there can legitimately want Europe.
        let today = d(2026, 5, 1);
        let mut p = build(27, 17.0, 12.0, 10.0, 12.0, 1, 150, 6000, 200, today);
        let access = ContinentalAccessContext {
            club_reputation: 0.35,
            league_reputation: 6500,
            main_league_tier: 1,
            league_position: 9,
            league_size: 20,
            season_progress: 0.5,
            club_continent_id: 1,
            ..Default::default()
        };
        let ctx = desire_ctx_from_access(&access, 1);
        assert!(!ctx.has_continental_path_hint);
        assert!(
            p.detect_continental_competition_desire(today, &ctx),
            "ambitious player at a low-rep top-flight club may want Europe"
        );
    }

    // ── Item 7: Self-amplifying isolation loop fixed ────────────

    #[test]
    fn return_home_companion_does_not_feed_back_into_detector() {
        // Build a fresh player with HEALTHY attributes (adapt 14, prof
        // 14, loyalty 14, in-home-country false but everything else
        // recovered) and inject a single companion FeelingIsolated
        // event with the StillStrugglingToSettle marker. The detector
        // should NOT fire on this configuration — the companion alone
        // is intentionally not enough to tip the score.
        use crate::PersonalAdaptationEventContext;
        use crate::PersonalAdaptationKind;
        let today = d(2026, 5, 1);
        let mut p = build(27, 12.0, 14.0, 14.0, 14.0, 30, 130, 4500, 90, today);
        // Mark player as native English speaker so speaks_local("gb") is true
        // (removes the !speaks_local penalty).
        p.languages
            .push(crate::club::player::language::PlayerLanguage {
                language: crate::club::player::language::Language::English,
                proficiency: 100,
                is_native: true,
            });
        p.happiness.morale = 60.0;
        p.happiness.factors.club_fit = 0.0;
        // Inject ONE companion FeelingIsolated with the
        // StillStrugglingToSettle marker — exactly what the helper
        // emits on the back of a WantsReturnHome.
        let pactx = PersonalAdaptationEventContext::new(
            PersonalAdaptationKind::StillStrugglingToSettle,
            90,
        );
        let ctx = HappinessEventContext::new(
            HappinessEventCause::AdaptationIsolation,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::DressingRoom,
        )
        .with_personal_adaptation_context(pactx);
        p.happiness
            .add_event_with_context(HappinessEventType::FeelingIsolated, -1.0, None, ctx);
        let signals = AdaptationFailureSignals {
            player_nationality_continent_id: 3,
            club_continent_id: 4,
            club_in_home_country: false,
            destination_is_favourite: false,
            same_language_or_nationality_teammates: 1,
            adaptation_score: 60.0,
            club_fit: 0.0,
        };
        // With the companion as the only isolation signal, score is:
        //   different_continent +2.0 (SA vs Asia)
        //   adapt 14 → not low, no point
        // = 2.0; below the 5.0 threshold, so detector does NOT fire.
        let fired = p.process_chronic_adaptation_failure(today, "gb", &signals);
        assert!(
            !fired,
            "companion StillStrugglingToSettle marker must not feed the detector"
        );
    }

    // ── Item 8: Life-simulation desires ─────────────────────────

    #[test]
    fn language_tutor_request_for_struggling_foreign_player() {
        use crate::club::player::core::player::SquadSocialView;
        let today = d(2026, 5, 1);
        let mut p = build(25, 12.0, 10.0, 10.0, 11.0, 30, 130, 5000, 90, today);
        // No local language proficiency. Adapt mid-band — pushes
        // detector into the "wants tutor" range.
        p.squad_social_view = Some(SquadSocialView::default());
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "jp".to_string();
        ctx.player_nationality_continent_id = 3;
        ctx.club_continent_id = 4;
        let mut result = PlayerResult::new(p.id);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(
            count_event(&p, HappinessEventType::LifeSimulationDesire) >= 1,
            "expected at least one LifeSimulationDesire event"
        );
    }

    // ── Item 9: Renderer / i18n key consistency ─────────────────

    #[test]
    fn life_simulation_kind_keys_round_trip() {
        // Every kind must yield a distinct, non-empty i18n key. Tests
        // catch typos / merge conflicts in the kind→key match.
        let kinds = [
            LifeSimulationDesireKind::FamilyUnsettledAbroad,
            LifeSimulationDesireKind::PartnerSchoolingConcern,
            LifeSimulationDesireKind::BereavementLeave,
            LifeSimulationDesireKind::FamilyBirthLeave,
            LifeSimulationDesireKind::DivorceImpact,
            LifeSimulationDesireKind::WantsLanguageTutor,
            LifeSimulationDesireKind::WantsMentorSupport,
            LifeSimulationDesireKind::WantsPreferredTacticalRole,
            LifeSimulationDesireKind::WantsLowerPressureClub,
            LifeSimulationDesireKind::WantsReleaseClause,
            LifeSimulationDesireKind::WantsPromiseToSell,
            LifeSimulationDesireKind::WantsLoanNotPermanent,
            LifeSimulationDesireKind::WantsNationalTeamVisibility,
            LifeSimulationDesireKind::WantsLeagueWithNtBias,
            LifeSimulationDesireKind::PrefersCulturalFamiliarity,
            LifeSimulationDesireKind::VeteranHomecomingSeason,
            LifeSimulationDesireKind::ClubLegendRefusesLeave,
            LifeSimulationDesireKind::RefusesRivalMoveDespiteUpgrade,
        ];
        let mut seen = std::collections::HashSet::new();
        for k in kinds {
            let key = k.as_i18n_key();
            assert!(!key.is_empty());
            assert!(seen.insert(key), "duplicate i18n key {}", key);
        }
    }

    #[test]
    fn opportunity_classifier_emits_evidence_for_european_path() {
        use crate::club::player::events::transfer_social::{
            TransferContinentalPath, TransferInterestSignal,
        };
        let today = d(2026, 5, 1);
        let mut p = build(27, 16.0, 12.0, 10.0, 12.0, 1, 145, 7500, 200, today);
        // Stage WantsEuropeanCompetition so the classifier prefers the
        // continental-path narrative even at modest rep gaps.
        p.happiness
            .add_event(HappinessEventType::WantsEuropeanCompetition, -3.0);
        let sig = TransferInterestSignal {
            interested_club_id: 100,
            interested_league_id: Some(1),
            buyer_rep: 0.85,
            seller_rep: 0.75,
            buyer_league_rep: 9000,
            seller_league_rep: 5000,
            stage: crate::TransferInterestStage::ConcreteInterest,
            source: crate::TransferInterestSource::ConfirmedApproach,
            repeated_attention: false,
            is_rival: false,
            is_home_country: false,
            is_seller_in_home_country: false,
            is_former_club: false,
            buyer_country_id: 10,
            buyer_continent_id: 1,
            buyer_has_continental_path: true,
            buyer_competition_path: Some(TransferContinentalPath::EliteEurope),
        };
        let landed = p.on_transfer_interest_signal(&sig);
        assert!(landed);
        // The classifier-routed event should carry
        // EuropeanCompetitionOpportunity-flavoured evidence on the
        // transfer-interest context payload.
        let mut found = false;
        for ev in &p.happiness.recent_events {
            if let Some(ctx) = ev.context.as_ref() {
                if let Some(tic) = ctx.transfer_interest_context.as_ref() {
                    if tic.evidence.iter().any(|e| {
                        matches!(
                            e,
                            crate::TransferInterestEvidence::EuropeanCompetitionOpportunity
                        )
                    }) {
                        found = true;
                        break;
                    }
                }
            }
        }
        assert!(
            found,
            "expected EuropeanCompetitionOpportunity evidence on the staged event"
        );
    }

    // ── UEFA suspension (Russia) ────────────────────────────────

    #[test]
    fn russian_elite_club_does_not_suppress_european_desire() {
        // Spartak Moscow at elite reputation: pre-suspension the
        // elite-club shortcut would mark it "on path" and suppress the
        // ambition mood. With the country flagged as UEFA-suspended,
        // the shortcut must NOT fire — an ambitious top-tier player
        // there is legitimately frustrated.
        let today = d(2026, 5, 1);
        let mut p = build(26, 17.0, 12.0, 10.0, 12.0, 1, 150, 5500, 60, today);
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "ru".to_string();
        ctx.club_continent_id = 1;
        ctx.player_nationality_continent_id = 1;
        ctx.league_reputation = 7000;
        // Elite reputation that WOULD trip the suppression off-the-bat.
        ctx.club_reputation = 0.85;
        ctx.main_league_tier = 1;
        // The from_global wiring would set this from the policy;
        // emulate that here.
        ctx.country_uefa_suspended = true;
        ctx.continental_path_known_absent = true;
        ctx.has_continental_path_hint = false;
        let fired = p.detect_continental_competition_desire(today, &ctx);
        assert!(
            fired,
            "ambitious player at UEFA-suspended Russian elite club must voice desire"
        );
        assert_eq!(
            count_event(&p, HappinessEventType::WantsEuropeanCompetition),
            1
        );
    }

    #[test]
    fn uefa_suspension_flows_through_continental_access() {
        // Cross-check: ContinentalAccessContext treats the suspension
        // as a continental ban, so the path heuristic refuses to mark
        // the club as on-path regardless of reputation. Mirrors what
        // `from_global` builds on every weekly tick.
        let access = ContinentalAccessContext {
            club_reputation: 0.90,
            league_reputation: 7000,
            main_league_tier: 1,
            league_position: 1,
            league_size: 16,
            season_progress: 0.5,
            club_continent_id: 1,
            continental_ban: true, // UEFA suspension
            ..Default::default()
        };
        let on_path = ContinentalPathHeuristic::from_access(&access).is_on_path(&access);
        assert!(!on_path, "suspended club must NOT register as on-path");
        assert!(access.path_known_absent());
    }
}
