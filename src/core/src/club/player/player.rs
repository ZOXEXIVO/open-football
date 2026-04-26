use crate::club::player::adaptation::PendingSigning;
use crate::club::player::builder::PlayerBuilder;
use crate::club::player::happiness::TeamSeasonState;
use crate::club::player::development::CoachingEffect;
use crate::club::player::injury::processing::MedicalStaffQuality;
use crate::club::player::language::PlayerLanguage;
use crate::club::player::load::PlayerLoad;
use crate::club::player::plan::PlayerPlan;
use crate::club::player::rapport::PlayerRapport;
use crate::club::player::traits::PlayerTrait;
use crate::club::player::utils::PlayerUtils;
use crate::HappinessEventType;
use crate::club::player::mailbox::PlayerContractAsk;
use crate::club::{
    PersonBehaviour, PlayerAttributes, PlayerClubContract, PlayerMailbox,
    PlayerResult, PlayerSkills, PlayerTraining,
};
use crate::context::GlobalContext;
use crate::shared::fullname::FullName;
use crate::utils::DateUtils;
use crate::{
    Person, PersonAttributes, PlayerDecisionHistory, PlayerHappiness,
    PlayerPositionType, PlayerPositions,
    PlayerStatistics, PlayerStatisticsHistory,
    PlayerStatus, PlayerTrainingHistory, PlayerValueCalculator, Relations,
};
use chrono::NaiveDate;
use std::fmt::{Display, Formatter, Result};

/// A sell-on promise owed to a previous seller on the next permanent sale.
/// Stacks: a player can accumulate multiple obligations from different past
/// clubs. Capped at 3 to prevent unbounded growth over long careers.
#[derive(Debug, Clone)]
pub struct SellOnObligation {
    pub beneficiary_club_id: u32,
    pub percentage: f32,
}

#[derive(Debug, Clone)]
pub struct Player {
    //person data
    pub id: u32,
    pub full_name: FullName,
    pub birth_date: NaiveDate,
    pub country_id: u32,
    pub behaviour: PersonBehaviour,
    pub attributes: PersonAttributes,

    //player data
    pub happiness: PlayerHappiness,
    pub statuses: PlayerStatus,
    pub skills: PlayerSkills,
    pub contract: Option<PlayerClubContract>,
    pub contract_loan: Option<PlayerClubContract>,
    pub positions: PlayerPositions,
    pub preferred_foot: PlayerPreferredFoot,
    pub player_attributes: PlayerAttributes,
    pub mailbox: PlayerMailbox,
    pub training: PlayerTraining,
    pub training_history: PlayerTrainingHistory,
    pub relations: Relations,

    pub statistics: PlayerStatistics,
    pub friendly_statistics: PlayerStatistics,
    pub cup_statistics: PlayerStatistics,
    pub statistics_history: PlayerStatisticsHistory,
    pub decision_history: PlayerDecisionHistory,

    /// Languages the player speaks, with proficiency levels.
    pub languages: Vec<PlayerLanguage>,

    /// Set when a player transfers/loans to a new club. Used by season snapshot
    /// to detect recently transferred players and avoid phantom history entries.
    ///
    /// Visibility narrowed to `pub(crate)` — read via `Player::last_transfer_date()`.
    /// Mutation is internal (set by `on_transfer` / `on_loan` / `on_loan_return`).
    pub(crate) last_transfer_date: Option<NaiveDate>,

    /// The club's strategic intent for this signing.
    /// Set when a player is permanently transferred. Protects the player from
    /// being sold before the club has given them a fair evaluation.
    pub plan: Option<PlayerPlan>,

    /// Clubs this player supports or has an affinity for (like FM's "Favoured Clubs").
    /// Affects willingness to join, morale when playing for them, etc.
    pub favorite_clubs: Vec<u32>,

    /// The club that sold this player and the fee paid.
    /// Prevents unrealistic buy-back scenarios where a club sells a player
    /// cheaply and then buys them back at a huge markup one season later.
    pub sold_from: Option<(u32, f64)>, // (club_id, fee_paid)

    /// Active sell-on clauses attached to this player. On the next permanent
    /// sale, each entry routes `percentage * fee` back to `club_id` before
    /// the selling club banks its income. Populated by `complete_transfer`
    /// when the inbound offer carried a `SellOnClause`; drained on sale.
    pub sell_on_obligations: Vec<SellOnObligation>,

    /// Signature moves — trained traits that bias in-match decisions.
    pub traits: Vec<PlayerTrait>,

    /// Rapport with the coaches who have trained this player.
    pub rapport: PlayerRapport,

    /// Promises the manager has made to this player (playing time etc.).
    /// Verified weekly — kept promises reinforce the manager relationship,
    /// broken ones erode it and tank morale.
    pub promises: Vec<ManagerPromise>,

    /// Transient transfer context — set by the transfer pipeline when this
    /// player moves to a new club, consumed by the player's own weekly
    /// processing to emit shock events, check role fit, and record an
    /// implicit playing-time promise. Cleared once consumed.
    pub pending_signing: Option<PendingSigning>,

    /// Rolling competitive workload and form rating. Drives rotation
    /// decisions, injury risk, and form-based morale events.
    pub load: PlayerLoad,

    /// The player's own stated terms after turning down a proposal.
    /// Lets the next club offer converge on a deal the player would sign,
    /// rather than guessing from scratch. Cleared when a deal is accepted.
    pub pending_contract_ask: Option<PlayerContractAsk>,

    /// True if this player was produced by a runtime generator (random squad
    /// fill, youth intake, synthetic national-team filler). False when loaded
    /// from the source database. Useful for filtering, telemetry, and UI hints.
    ///
    /// Visibility narrowed to `pub(crate)` — read it via `Player::is_generated()`.
    /// Mutation is internal (set once by `PlayerBuilder::generated`).
    pub(crate) generated: bool,

    /// Visibility narrowed to `pub(crate)` — read via `Player::is_retired()`.
    /// Mutation is internal (set by end-of-season retirement processing).
    pub(crate) retired: bool,
}

/// What the manager committed to. Deliberately narrow — each new variant
/// must define what "kept" means in `Player::verify_promises`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerPromiseKind {
    /// "You'll play more" — kept if the player logged at least N
    /// appearances between made_on and deadline.
    PlayingTime,
}

#[derive(Debug, Clone)]
pub struct ManagerPromise {
    pub kind: ManagerPromiseKind,
    pub made_on: NaiveDate,
    pub deadline: NaiveDate,
    /// Snapshot of the player's `statistics.played + played_subs` at the
    /// time the promise was made. Used to compute "games since promise".
    pub baseline_apps: u16,
}

impl Player {
    pub fn builder() -> PlayerBuilder {
        PlayerBuilder::new()
    }

    // ========================================================
    // Accessor API
    // --------------------------------------------------------
    // The fields below this struct are still `pub` for backward
    // compatibility — many call sites and Askama templates read them
    // directly, and narrowing visibility is a separate sweep that touches
    // every web template.
    //
    // **New code should prefer these accessors.** They give us a single
    // place to:
    //   • intercept reads (e.g. for caching, telemetry, lazy compute);
    //   • change underlying storage (skills as a registry, happiness as
    //     an event-sourced log) without breaking callers;
    //   • narrow visibility incrementally — once every consumer goes
    //     through accessors, the underlying field can flip to `pub(crate)`
    //     in one change.
    //
    // Naming: the immutable accessor matches the field name; the mutable
    // accessor uses a `_mut` suffix.
    // ========================================================

    pub fn skills(&self) -> &PlayerSkills { &self.skills }
    pub fn skills_mut(&mut self) -> &mut PlayerSkills { &mut self.skills }

    pub fn attributes(&self) -> &PersonAttributes { &self.attributes }
    pub fn attributes_mut(&mut self) -> &mut PersonAttributes { &mut self.attributes }

    pub fn player_attributes(&self) -> &PlayerAttributes { &self.player_attributes }
    pub fn player_attributes_mut(&mut self) -> &mut PlayerAttributes { &mut self.player_attributes }

    pub fn happiness(&self) -> &PlayerHappiness { &self.happiness }
    pub fn happiness_mut(&mut self) -> &mut PlayerHappiness { &mut self.happiness }

    pub fn statuses(&self) -> &PlayerStatus { &self.statuses }
    pub fn statuses_mut(&mut self) -> &mut PlayerStatus { &mut self.statuses }

    pub fn statistics(&self) -> &PlayerStatistics { &self.statistics }
    pub fn statistics_mut(&mut self) -> &mut PlayerStatistics { &mut self.statistics }

    pub fn cup_statistics(&self) -> &PlayerStatistics { &self.cup_statistics }
    pub fn friendly_statistics(&self) -> &PlayerStatistics { &self.friendly_statistics }
    pub fn statistics_history(&self) -> &PlayerStatisticsHistory { &self.statistics_history }

    pub fn contract(&self) -> Option<&PlayerClubContract> { self.contract.as_ref() }
    pub fn contract_loan(&self) -> Option<&PlayerClubContract> { self.contract_loan.as_ref() }

    pub fn plan(&self) -> Option<&PlayerPlan> { self.plan.as_ref() }
    pub fn promises(&self) -> &[ManagerPromise] { &self.promises }
    pub fn traits(&self) -> &[PlayerTrait] { &self.traits }

    pub fn relations(&self) -> &Relations { &self.relations }
    pub fn rapport(&self) -> &PlayerRapport { &self.rapport }
    pub fn load(&self) -> &PlayerLoad { &self.load }

    pub fn favorite_clubs(&self) -> &[u32] { &self.favorite_clubs }
    pub fn languages(&self) -> &[PlayerLanguage] { &self.languages }
    pub fn last_transfer_date(&self) -> Option<NaiveDate> { self.last_transfer_date }
    pub fn is_retired(&self) -> bool { self.retired }
    pub fn is_generated(&self) -> bool { self.generated }

    /// Canonical URL segment for this player: `{id}-{ascii-folded-name}`.
    /// Falls back to just the id when the name folds to nothing (e.g. all
    /// punctuation), so every player is guaranteed a resolvable URL.
    pub fn slug(&self) -> String {
        let name_slug = self.full_name.slug();
        if name_slug.is_empty() {
            self.id.to_string()
        } else {
            format!("{}-{}", self.id, name_slug)
        }
    }

    /// Is this player protected from being targeted by other clubs?
    ///
    /// A player signed during the currently open transfer window cannot be
    /// sold in that same window. Between windows, the pipeline gate already
    /// blocks transfers. The `PlayerPlan` then governs the next window.
    ///
    /// `current_window` is `Some((start, end))` when a transfer window is
    /// open, `None` otherwise.
    pub fn is_transfer_protected(
        &self,
        date: NaiveDate,
        current_window: Option<(NaiveDate, NaiveDate)>,
    ) -> bool {
        // Same-window protection: signed during this open window → protected
        if let (Some(transfer_date), Some((window_start, window_end))) =
            (self.last_transfer_date, current_window)
        {
            if transfer_date >= window_start && transfer_date <= window_end {
                return true;
            }
        }

        // Club has a signing plan — don't poach until the plan concludes
        if let Some(ref plan) = self.plan {
            let total_apps = self.statistics.played + self.statistics.played_subs;
            if !plan.is_evaluated(date, total_apps) && !plan.is_expired(date) {
                return true;
            }
        }

        false
    }

    /// Record a new manager promise. Deduped — only the freshest promise
    /// of any given kind is tracked (a new promise supersedes an unresolved
    /// earlier one of the same kind).
    pub fn record_promise(&mut self, kind: ManagerPromiseKind, made_on: NaiveDate, horizon_days: i64) {
        let deadline = made_on + chrono::Duration::days(horizon_days);
        let baseline_apps = self.statistics.played + self.statistics.played_subs;
        self.promises.retain(|p| p.kind != kind);
        self.promises.push(ManagerPromise { kind, made_on, deadline, baseline_apps });
    }

    /// Evaluate every promise whose deadline has passed. Kept → small
    /// positive event and trust bump; broken → large negative event,
    /// salary/playing-time frustration already covers the rest.
    pub fn verify_promises(&mut self, now: NaiveDate) {
        if self.promises.is_empty() {
            return;
        }
        let current_apps = self.statistics.played + self.statistics.played_subs;
        let mut kept_count = 0;
        let mut broken_count = 0;

        self.promises.retain(|p| {
            if now < p.deadline {
                return true; // still pending
            }
            let delta_apps = current_apps.saturating_sub(p.baseline_apps);
            let days = (p.deadline - p.made_on).num_days().max(1) as u16;
            let kept = match p.kind {
                ManagerPromiseKind::PlayingTime => {
                    // Kept if the player got at least one appearance every
                    // ~10 days of the promise window. 30-day window → 3 apps.
                    let required = (days / 10).max(1);
                    delta_apps >= required
                }
            };
            if kept { kept_count += 1; } else { broken_count += 1; }
            false // remove resolved
        });

        if kept_count > 0 {
            self.happiness.add_event(HappinessEventType::PromiseKept, 4.0 * kept_count as f32);
            // Directly reinforce the manager-relationship factor too.
            self.happiness.factors.manager_relationship =
                (self.happiness.factors.manager_relationship + 2.0 * kept_count as f32).clamp(-15.0, 15.0);
        }
        if broken_count > 0 {
            self.happiness.add_event(HappinessEventType::PromiseBroken, -6.0 * broken_count as f32);
            self.happiness.factors.manager_relationship =
                (self.happiness.factors.manager_relationship - 4.0 * broken_count as f32).clamp(-15.0, 15.0);
            // Broken playing-time promise often becomes a transfer request eventually.
            // Feed unhappy status via existing factor path — status is still decided by process_happiness.
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> PlayerResult {
        let now = ctx.simulation.date;

        let mut result = PlayerResult::new(self.id);

        // Age the rolling workload windows before anything reads them today.
        // Cheap and idempotent — safe to call before every other step.
        self.load.daily_decay(now.date());

        // Birthday
        if DateUtils::is_birthday(self.birth_date, now.date()) {
            self.behaviour.try_increase();
        }

        // First tick after a transfer: react to the new club — ambition /
        // salary shocks, role fit, implicit playing-time promise. No-op if
        // nothing is pending.
        if self.pending_signing.is_some() {
            let country_code = ctx.country.as_ref().map(|c| c.code.as_str()).unwrap_or("");
            let club_rep = ctx.team.as_ref().map(|t| t.reputation).unwrap_or(0.0);
            let formation = ctx.team.as_ref().and_then(|t| t.formation);
            self.process_transfer_shock(
                now.date(),
                club_rep,
                country_code,
                formation.as_ref(),
            );
        }

        // Injury recovery (daily) — driven by the parent club's medical
        // staff quality (physiotherapy + sports science).
        let medical = MedicalStaffQuality {
            physio: ctx.club_medical_quality(),
            sports_science: ctx.club_sports_science_quality(),
        };
        self.process_injury(&mut result, now.date(), &medical);

        // Natural condition recovery for non-injured players
        self.process_condition_recovery(now.date());

        // Match readiness decay for players not playing (rebuilds during pre-season)
        self.process_match_readiness_decay(now.date());

        // Player happiness & morale evaluation (weekly)
        let team_reputation = ctx.team.as_ref().map(|t| t.reputation).unwrap_or(0.0);
        if ctx.simulation.is_week_beginning() {
            // Verify promises before happiness so kept/broken events feed
            // into the same weekly morale recalculation.
            self.verify_promises(now.date());
            let season_state = TeamSeasonState {
                league_position: ctx.club.as_ref().map(|c| c.league_position).unwrap_or(0),
                league_size: ctx.club.as_ref().map(|c| c.league_size).unwrap_or(0),
                season_progress: ctx.club.as_ref()
                    .map(|c| if c.total_league_matches > 0 {
                        c.league_matches_played as f32 / c.total_league_matches as f32
                    } else { 0.0 })
                    .unwrap_or(0.0)
                    .clamp(0.0, 1.0),
                league_reputation: ctx.league.as_ref().map(|l| l.reputation).unwrap_or(0),
            };
            self.process_happiness(&mut result, now.date(), team_reputation, season_state);
            // Natural skill development (weekly). Build the coaching effect
            // once per player from the club's best coach scores.
            let league_reputation = ctx.league.as_ref().map(|l| l.reputation).unwrap_or(0);
            let coach_effect = ctx
                .club
                .as_ref()
                .map(|c| CoachingEffect::from_scores(
                    c.coach_best_technical,
                    c.coach_best_mental,
                    c.coach_best_fitness,
                    c.coach_best_goalkeeping,
                    c.youth_coaching_quality,
                ))
                .unwrap_or_else(CoachingEffect::neutral);
            self.process_development(now.date(), league_reputation, &coach_effect, team_reputation);
            // Language learning when playing abroad
            let country_code = ctx.country.as_ref().map(|c| c.code.as_str()).unwrap_or("");
            self.process_language_learning(now.date(), country_code);
            // Post-transfer integration: bonding / isolation events for the
            // first ~24 weeks at a new club.
            self.process_integration(now.date(), country_code);
        }

        // Contract processing
        self.process_contract(&mut result, now);
        self.process_mailbox(&mut result, now.date());

        // Transfer desire based on multiple factors
        self.process_transfer_desire(&mut result, now.date());

        result
    }

    pub fn shirt_number(&self) -> u8 {
        if let Some(contract) = &self.contract {
            return contract.shirt_number.unwrap_or(0);
        }

        0
    }

    pub fn value(&self, date: NaiveDate, league_reputation: u16, club_reputation: u16) -> f64 {
        PlayerValueCalculator::calculate(self, date, 1.0, league_reputation, club_reputation)
    }

    pub fn value_with_price_level(&self, date: NaiveDate, price_level: f32, league_reputation: u16, club_reputation: u16) -> f64 {
        PlayerValueCalculator::calculate(self, date, price_level, league_reputation, club_reputation)
    }

    #[inline]
    pub fn positions(&self) -> Vec<PlayerPositionType> {
        self.positions.positions()
    }

    #[inline]
    pub fn position(&self) -> PlayerPositionType {
        *self
            .positions
            .positions()
            .first()
            .expect("no position found")
    }

    pub fn preferred_foot_str(&self) -> &'static str {
        match self.preferred_foot {
            PlayerPreferredFoot::Left => "Left",
            PlayerPreferredFoot::Right => "Right",
            PlayerPreferredFoot::Both => "Both",
        }
    }

    pub fn is_on_loan(&self) -> bool {
        self.contract_loan.is_some()
    }

    pub fn is_ready_for_match(&self) -> bool {
        !self.player_attributes.is_injured
            && !self.player_attributes.is_banned
            && !self.player_attributes.is_in_recovery()
            && self.player_attributes.condition_percentage() > 30
    }

    pub fn growth_potential(&self, now: NaiveDate) -> u8 {
        PlayerUtils::growth_potential(self, now)
    }

    /// Weekly language learning: if the player is in a country whose language
    /// they don't speak natively, they gradually learn it.
    fn process_language_learning(&mut self, now: NaiveDate, country_code: &str) {
        use crate::club::player::language::{weekly_language_progress, Language};

        if country_code.is_empty() {
            return;
        }

        let country_languages = Language::from_country_code(country_code);
        if country_languages.is_empty() {
            return;
        }

        let age = DateUtils::age(self.birth_date, now);

        for target_lang in &country_languages {
            // Check if player already speaks this language natively
            let already_native = self.languages.iter().any(|l| l.language == *target_lang && l.is_native);
            if already_native {
                continue;
            }

            // Check if already fully fluent (proficiency >= 100)
            let already_fluent = self.languages.iter().any(|l| l.language == *target_lang && l.proficiency >= 100);
            if already_fluent {
                continue;
            }

            let current_prof = self.languages.iter()
                .find(|l| l.language == *target_lang)
                .map(|l| l.proficiency)
                .unwrap_or(0);

            let gain = weekly_language_progress(
                self.attributes.adaptability,
                self.attributes.professionalism,
                age,
                self.player_attributes.current_ability,
                current_prof,
            );

            if gain == 0 {
                continue;
            }

            // Threshold-crossing detection so morale only lifts on meaningful
            // milestones (basic, conversational, functional, fluent). Weekly
            // +1% increments would otherwise spam the event log.
            //
            // Magnitude scales by milestone — first words feel small, the
            // jump to functional ("can do an interview unaided") is bigger,
            // and full fluency is a quiet pride moment. Catalog default is
            // the conversational rung; the multiplier maps each threshold
            // around it.
            const THRESHOLDS: &[u8] = &[40, 55, 70, 90];
            let prev_prof = current_prof;
            let new_prof = (current_prof + gain).min(100);
            let crossed_threshold = THRESHOLDS
                .iter()
                .find(|&&t| prev_prof < t && new_prof >= t)
                .copied();

            if let Some(lang_entry) = self.languages.iter_mut().find(|l| l.language == *target_lang) {
                lang_entry.proficiency = new_prof;
            } else {
                self.languages.push(PlayerLanguage::learning(*target_lang, gain));
            }

            if let Some(t) = crossed_threshold {
                // 40 basic → 0.7×, 55 conversational → 1.0× (catalog),
                // 70 functional → 1.4×, 90 fluent → 0.9× (quiet pride).
                let factor = match t {
                    40 => 0.7,
                    55 => 1.0,
                    70 => 1.4,
                    90 => 0.9,
                    _ => 1.0,
                };
                let cfg = crate::club::player::behaviour_config::HappinessConfig::default();
                let mag = cfg.catalog.language_progress * factor;
                // Cooldown 30d so two languages crossing thresholds in the
                // same fortnight don't both fire (rare, but tidy).
                self.happiness.add_event_with_cooldown(
                    crate::HappinessEventType::LanguageProgress,
                    mag,
                    30,
                );
            }
        }
    }
}

impl Person for Player {
    fn id(&self) -> u32 {
        self.id
    }

    fn fullname(&self) -> &FullName {
        &self.full_name
    }

    fn birthday(&self) -> NaiveDate {
        self.birth_date
    }

    fn behaviour(&self) -> &PersonBehaviour {
        &self.behaviour
    }

    fn attributes(&self) -> &PersonAttributes {
        &self.attributes
    }

    fn relations(&self) -> &Relations {
        &self.relations
    }
}

#[derive(Debug, Clone)]
pub enum PlayerPreferredFoot {
    Left,
    Right,
    Both,
}

//DISPLAY
impl Display for Player {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}, {}", self.full_name, self.birth_date)
    }
}

impl PartialEq for Player {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn player_is_correct() {
        assert_eq!(10, 10);
    }
}
