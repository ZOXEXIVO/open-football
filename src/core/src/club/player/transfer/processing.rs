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
    LifeSimulationSeverity, LifeSimulationTrigger,
};
use chrono::NaiveTime;
use chrono::{NaiveDate, NaiveDateTime};

/// Continent ids matching the values documented in
/// `transfers::scouting_region`: 1 = Europe, 3 = South America.
const CONTINENT_EUROPE: u32 = 1;
const CONTINENT_SOUTH_AMERICA: u32 = 3;

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
    /// Compatriots / shared-language teammates currently in the squad.
    /// Drives `NoCompatriotSupport` and feeds the chronic-failure
    /// detector.
    pub same_language_or_nationality_teammates: u8,
    /// True if the player's last transfer destination is a favourite
    /// club. Drives the dream-move suppression bar.
    pub destination_is_favourite: bool,
    /// True if the club country == player nationality country.
    pub club_in_home_country: bool,
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

        // Continental-path heuristic — see `ContinentalPathHeuristic` for
        // the realism rules. Position-based fallback only kicks in once
        // the season has progressed enough for the table to be meaningful.
        let has_continental_path_hint = ContinentalPathHeuristic {
            main_league_tier,
            league_reputation,
            league_position,
            league_size,
            season_progress,
        }
        .is_on_path();

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
            same_language_or_nationality_teammates,
            destination_is_favourite,
            club_in_home_country,
        }
    }
}

/// Heuristic for "is this club on a credible continental qualification
/// path?" Replaces the naive top-35% league-position rule with a
/// reputation-aware band that respects season progress.
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
    /// True if the current club is plausibly on a continental
    /// qualification path this season.
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
    pub fn is_on_path(&self) -> bool {
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

        let has_unh_long = self
            .statuses
            .statuses
            .iter()
            .any(|s| s.status == PlayerStatusType::Unh && (now - s.start_date).num_days() > 30);
        if has_unh_long {
            active_reasons.push(TransferRequestReason::LongUnhappiness);
        }

        if !recently_transferred && self.happiness.factors.ambition_fit <= -7.0 {
            let has_unh_short =
                self.statuses.statuses.iter().any(|s| {
                    s.status == PlayerStatusType::Unh && (now - s.start_date).num_days() > 14
                });
            if has_unh_short {
                active_reasons.push(TransferRequestReason::AmbitionMismatch);
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
            if !self.statuses.get().contains(&PlayerStatusType::Req) {
                self.statuses.add(now, PlayerStatusType::Req);
            }
            result.wants_to_leave = true;
            result.request_transfer(self.id);
        } else if self.statuses.get().contains(&PlayerStatusType::Req) {
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
        // Already on a credible continental path → don't fire.
        if ctx.has_continental_path_hint {
            return false;
        }
        // Cooldown.
        if self
            .happiness
            .has_recent_event(&HappinessEventType::WantsEuropeanCompetition, 60)
        {
            return false;
        }

        // Universal logic: score the gap. The same arithmetic covers
        // a Premier League fringe player at a relegation side and a
        // Champions League regular at a club mid-table — the higher
        // the realistic European ceiling, the more the gap matters.
        let ambition = self.attributes.ambition;
        if ambition < 15.0 {
            return false;
        }
        let age = DateUtils::age(self.birth_date, now);
        if !(22..=31).contains(&age) {
            return false;
        }
        let ca = self.player_attributes.current_ability as f32;
        let world_rep = self.player_attributes.world_reputation as f32;
        // Realistic European market: only players who could plausibly
        // play at that level. Floor at CA 130 / world_rep 4500 — a
        // Tier-3 squad player won't get a Champions League move.
        if ca < 130.0 || world_rep < 4500.0 {
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
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
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
        assert!(p.statuses.get().contains(&PlayerStatusType::Req));
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
            p.statuses.get().contains(&PlayerStatusType::Req),
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
    fn req_clears_when_all_reasons_resolved() {
        let today = d(2026, 5, 1);
        let mut p = build(28, 14.0, 12.0, 10.0, 12.0, 30, 130, 5000, 200, today);
        p.statuses.add(
            today
                .checked_sub_signed(chrono::Duration::days(40))
                .unwrap(),
            PlayerStatusType::Unh,
        );
        let mut ctx = TransferDesireContext::default();
        ctx.country_code = "es".to_string();
        ctx.player_nationality_continent_id = 1;
        let mut result = PlayerResult::new(p.id);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(p.statuses.get().contains(&PlayerStatusType::Req));

        // Unh status removed — no other reason active.
        p.statuses.remove(PlayerStatusType::Unh);
        p.process_transfer_desire(&mut result, today, &ctx);
        assert!(!p.statuses.get().contains(&PlayerStatusType::Req));
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

    #[test]
    fn continental_path_heuristic_pre_quarter_season_uses_rep() {
        let h = ContinentalPathHeuristic {
            main_league_tier: 1,
            league_reputation: 8000,
            league_position: 12,
            league_size: 20,
            season_progress: 0.10,
        };
        assert!(
            h.is_on_path(),
            "high-rep tier-1 club is presumed on path before 25% season"
        );
    }

    #[test]
    fn continental_path_heuristic_position_load_bearing_post_quarter() {
        let h = ContinentalPathHeuristic {
            main_league_tier: 1,
            league_reputation: 9000,
            league_position: 12,
            league_size: 20,
            season_progress: 0.50,
        };
        assert!(
            !h.is_on_path(),
            "12th-place team is no longer on the path past quarter season"
        );
    }

    #[test]
    fn continental_path_heuristic_low_rep_league_never_on_path() {
        let h = ContinentalPathHeuristic {
            main_league_tier: 1,
            league_reputation: 3500,
            league_position: 1,
            league_size: 20,
            season_progress: 0.80,
        };
        assert!(!h.is_on_path());
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
}
