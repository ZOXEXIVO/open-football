use crate::club::player::behaviour_config::AdaptationConfig;
use crate::club::player::language::Language;
use crate::club::player::player::{ManagerPromiseKind, Player};
use crate::club::{Person, PlayerPositionType};
use crate::HappinessEventType;
use chrono::NaiveDate;

/// Squad-side context for [`Player::adaptation_score`]. Caller-supplied so
/// the player doesn't need to walk the squad to compute its own number.
/// Empty-default fields contribute neutrally — the score still works
/// without context, just less informed.
#[derive(Debug, Clone, Default)]
pub struct AdaptationSquadContext {
    /// How many other senior squad members speak one of this player's
    /// languages well enough to chat off the pitch (≥40 proficiency or
    /// native). Drives the "language buddy" axis of adaptation.
    pub same_language_teammates: u8,
    /// How many other senior squad members share the player's primary
    /// nationality (country_id). Stack-once: caller may pre-cap at 2.
    pub same_nationality_teammates: u8,
    /// Has a mentor been assigned and is the relationship positive?
    /// `Some(true)` = good mentor, `Some(false)` = bad mentor or open
    /// conflict, `None` = no mentor.
    pub mentor_quality: Option<bool>,
    /// Squad chemistry as the team sees it (0..100). Pulled from
    /// `Relations::get_team_chemistry()`.
    pub squad_chemistry: f32,
    /// Highest staff/manager relation level for this player on the
    /// signed -100..100 axis. 0 if unknown.
    pub manager_relation_level: f32,
    /// Is the player a loan signing? Caps max adaptation at 85 unless they
    /// share the local language or this is a favorite club.
    pub is_loan: bool,
    /// True if signing is to a favorite club — relaxes the loan cap.
    pub is_favorite_club: bool,
}

/// Post-transfer settling window. For the first ~12 weeks at a new club the
/// player's match rating is dampened, and weekly integration events fire.
///
/// Backed by [`AdaptationConfig::settlement_window_days`]. Kept as a `const`
/// so existing callers (test fixtures, doc references) don't break — the
/// config value and this constant must stay in sync. If you need to override
/// it per save, route through the config instead.
pub const SETTLEMENT_WINDOW_DAYS: i64 = 84;

/// Context left on the player by transfer execution. Consumed the next
/// time the player simulates — that's where shock events, role-fit checks
/// and the implicit playing-time promise are emitted. Keeping this as
/// transient state (rather than having execution push events directly)
/// means the player reacts to a new environment as part of his own
/// processing, alongside happiness, language, integration, etc.
#[derive(Debug, Clone)]
pub struct PendingSigning {
    pub previous_salary: Option<u32>,
    pub fee: f64,
    pub is_loan: bool,
    /// Destination club id — needed to check whether the signing is to one
    /// of the player's favorite clubs so the right shock event can fire.
    pub destination_club_id: u32,
}

// All settlement-shock thresholds (ambition gap, dream-move surplus,
// elite-club reputation, salary shock/boost ratios) live in
// `AdaptationConfig`. The functions below pull them via `default()` —
// future per-save overrides can be threaded through without touching the
// call sites.

impl Player {
    /// Days elapsed since the player's most recent transfer/loan, if any.
    pub fn days_since_transfer(&self, now: NaiveDate) -> Option<i64> {
        self.last_transfer_date.map(|d| (now - d).num_days())
    }

    /// Multiplier (0.80..1.00) applied to match rating while settling at a
    /// new club. Linear recovery across the configured settlement window,
    /// trimmed by local-language fluency, adaptability, and step-up status.
    /// Tuning lives in [`AdaptationConfig`].
    pub fn settlement_form_multiplier(
        &self,
        now: NaiveDate,
        country_code: &str,
        club_rep_0_to_1: f32,
    ) -> f32 {
        let cfg = AdaptationConfig::default();
        cfg.settlement_multiplier(
            self.days_since_transfer(now),
            self.speaks_local_language(country_code),
            self.attributes.adaptability,
            self.is_step_up_move(club_rep_0_to_1),
        )
    }

    /// Settlement multiplier adjusted by the adaptation_score. Slots into
    /// rating in match_events the same way the legacy version does, but
    /// reads the richer adaptation signal so a well-supported foreign
    /// signing recovers form much faster than an isolated one.
    ///
    /// Bands map to multipliers as:
    ///   * adaptation ≥ 80 → 0.98..1.00 (essentially no penalty)
    ///   * 60-79 → 0.94..0.98
    ///   * 40-59 → 0.88..0.94
    ///   * 20-39 → 0.82..0.88
    ///   * <20 → 0.78..0.82 (worst case, never below 0.78)
    /// Highly-adapted dream moves can earn a tiny positive lift up to 1.02.
    pub fn settlement_form_multiplier_from_adaptation(
        &self,
        adaptation_score: f32,
        is_dream_move: bool,
    ) -> f32 {
        let s = adaptation_score.clamp(0.0, 100.0);
        let base = if s >= 80.0 {
            // 0.98..1.00 across [80, 100]
            0.98 + ((s - 80.0) / 20.0) * 0.02
        } else if s >= 60.0 {
            0.94 + ((s - 60.0) / 20.0) * 0.04
        } else if s >= 40.0 {
            0.88 + ((s - 40.0) / 20.0) * 0.06
        } else if s >= 20.0 {
            0.82 + ((s - 20.0) / 20.0) * 0.06
        } else {
            0.78 + (s / 20.0) * 0.04
        };
        if is_dream_move && s >= 80.0 {
            (base + 0.02).clamp(0.78, 1.02)
        } else {
            base.clamp(0.78, 1.02)
        }
    }

    /// Derived 0..100 adaptation score. Read by:
    ///   * settlement form multiplier (via
    ///     [`Player::settlement_form_multiplier_from_adaptation`]);
    ///   * `ScoringEngine::newcomer_penalty` for selection;
    ///   * isolation / bonding event gates;
    ///   * training receptiveness modifier in coach-vs-player coaching.
    ///
    /// Inputs follow the spec exactly so behaviour is reproducible:
    ///   - time at club (linear up to 84 days)
    ///   - local-language proficiency (fluent / basic / none)
    ///   - adaptability and professionalism personality attributes
    ///   - role fit against the formation
    ///   - manager relationship
    ///   - mentor presence + quality
    ///   - same-language and same-nationality teammates
    ///   - squad chemistry deviation from neutral
    ///   - recent appearances + starts post-transfer
    ///   - loan cap, dream-move lift, salary/ambition shock
    /// Final score clamped to 0..100.
    pub fn adaptation_score(
        &self,
        now: NaiveDate,
        country_code: &str,
        club_rep_0_to_1: f32,
        formation: Option<&[PlayerPositionType; 11]>,
        squad: &AdaptationSquadContext,
    ) -> f32 {
        let mut score: f32 = 35.0;

        // Time at club — caps at 84 days (12 weeks).
        if let Some(days) = self.days_since_transfer(now) {
            let factor = ((days as f32) / 84.0).clamp(0.0, 1.0);
            score += factor * 25.0;
        } else {
            // Player has been at the club a long time — full settle bonus.
            score += 25.0;
        }

        // Local language tier.
        if !country_code.is_empty() {
            let langs = Language::from_country_code(country_code);
            if !langs.is_empty() {
                let mut highest_proficiency: u8 = 0;
                let mut native_or_fluent = false;
                for target in &langs {
                    for pl in &self.languages {
                        if pl.language != *target {
                            continue;
                        }
                        if pl.is_native || pl.proficiency >= 70 {
                            native_or_fluent = true;
                        }
                        if pl.proficiency > highest_proficiency {
                            highest_proficiency = pl.proficiency;
                        }
                    }
                }
                if native_or_fluent {
                    score += 15.0;
                } else if (40..=69).contains(&highest_proficiency) {
                    score += 7.0;
                } else if highest_proficiency == 0 {
                    score -= 10.0;
                }
            }
        }

        // Adaptability + professionalism contributions.
        let adapt_contribution =
            ((self.attributes.adaptability - 10.0) * 1.2).clamp(-10.0, 12.0);
        score += adapt_contribution;
        let prof_contribution =
            ((self.attributes.professionalism - 10.0) * 0.7).clamp(-6.0, 7.0);
        score += prof_contribution;

        // Role fit against the formation.
        if let Some(f) = formation {
            let primary = self.position();
            if f.iter().any(|p| *p == primary) {
                score += 10.0;
            } else if f.iter().any(|p| p.position_group() == primary.position_group()) {
                score += 4.0;
            } else {
                score -= 12.0;
            }
        }

        // Manager relationship — normalise -100..100 → -8..+8.
        let manager_norm = (squad.manager_relation_level / 100.0 * 8.0).clamp(-8.0, 8.0);
        score += manager_norm;

        // Mentor support.
        match squad.mentor_quality {
            Some(true) => score += 8.0,
            Some(false) => score -= 8.0,
            None => {}
        }

        // Language buddies in the squad.
        match squad.same_language_teammates {
            0 => {}
            1 => score += 4.0,
            _ => score += 7.0,
        }

        // Same-nationality presence — stack once only.
        if squad.same_nationality_teammates >= 1 {
            score += 3.0;
        }

        // Squad chemistry deviation.
        let chem_contribution = ((squad.squad_chemistry - 50.0) * 0.15).clamp(-7.5, 7.5);
        score += chem_contribution;

        // Recent appearances bonus — first 5 appearances and starts after a
        // transfer count for a small lift each. Without per-window tracking,
        // approximate with total post-transfer matches.
        let appearances_after_transfer =
            (self.statistics.played + self.statistics.played_subs) as i32;
        let starts_after_transfer = self.statistics.played as i32;
        let app_bonus = appearances_after_transfer.min(5) as f32 * 2.0;
        let start_bonus = starts_after_transfer.min(5) as f32 * 2.0;
        score += app_bonus + start_bonus;

        // Step-up dream move.
        if self.is_step_up_move(club_rep_0_to_1) {
            score += 5.0;
        }

        // Salary / ambition shock penalty — a recent shock event signals
        // the player still hasn't reconciled with the move's terms.
        let recent_shock = self.happiness.recent_events.iter().any(|e| {
            (e.event_type == HappinessEventType::SalaryShock
                || e.event_type == HappinessEventType::AmbitionShock)
                && e.days_ago <= 60
        });
        if recent_shock {
            score -= 8.0;
        }

        // Loan cap — capped at 85 unless same language or favorite club.
        if squad.is_loan {
            let speaks_local = self.speaks_local_language(country_code);
            if !(speaks_local || squad.is_favorite_club) {
                score = score.min(85.0);
            }
        }

        score.clamp(0.0, 100.0)
    }

    /// A step-up move is one where the club's reputation visibly exceeds
    /// what the player's ambition was already expecting.
    pub fn is_step_up_move(&self, club_rep_0_to_1: f32) -> bool {
        AdaptationConfig::default().is_step_up_move(self.attributes.ambition, club_rep_0_to_1)
    }

    /// True if the player speaks the country's primary language well enough
    /// (native or ≥70 proficiency) that culture shock is muted.
    pub fn speaks_local_language(&self, country_code: &str) -> bool {
        if country_code.is_empty() {
            return true;
        }
        let langs = Language::from_country_code(country_code);
        if langs.is_empty() {
            return true;
        }
        langs.iter().any(|l| {
            self.languages.iter().any(|pl| {
                pl.language == *l && (pl.is_native || pl.proficiency >= 70)
            })
        })
    }

    /// Consume a pending signing: emit the one-shot shock events, check role
    /// fit against the current formation, and record the implicit playing-
    /// time promise. Safe to call every tick — it's a no-op if nothing is
    /// pending.
    pub fn process_transfer_shock(
        &mut self,
        now: NaiveDate,
        club_rep_0_to_1: f32,
        country_code: &str,
        formation: Option<&[PlayerPositionType; 11]>,
    ) {
        let Some(pending) = self.pending_signing.take() else { return };
        let cfg = AdaptationConfig::default();

        // Ambition / dream / elite-club reactions fire for loans too —
        // being loaned to Real Madrid is still the move of your life, even
        // if you're going back in a year. Loans pay at the borrowing club's
        // loan wage (distinct from a full contract) so salary shock/boost
        // is skipped for them; that lever is tuned for permanent moves.
        let loan_damp = if pending.is_loan { cfg.loan_damp_factor } else { 1.0 };
        let is_favorite_destination = self.favorite_clubs.contains(&pending.destination_club_id);
        // Ambition shock is muted when joining a favorite club — the player
        // knew what they were signing for and the sentimental pull covers the
        // ambition gap. Reputation-based "I should be at a bigger club"
        // doesn't apply to your boyhood side.
        if !is_favorite_destination {
            self.emit_ambition_shock(club_rep_0_to_1, loan_damp);
        }
        if is_favorite_destination {
            // Signing for a childhood/legend club trumps the reputation-gap
            // logic — fire DreamMove at full weight regardless of where the
            // club sits on the prestige ladder. A player returning to boyhood
            // club feels this even if it's a rep-drop move. Veterans get a
            // softer boyhood-return event rather than a "dream move of his
            // career" framing.
            let mag = if self.age(now) >= 32 { 8.0 } else { 15.0 };
            self.happiness
                .add_event(HappinessEventType::DreamMove, mag * loan_damp);
        } else {
            self.emit_dream_move(club_rep_0_to_1, loan_damp, now);
        }
        self.emit_joining_elite(club_rep_0_to_1, loan_damp);

        if !pending.is_loan {
            self.emit_salary_shock(pending.previous_salary);
            self.emit_salary_boost(pending.previous_salary);
        }

        // Shirt number prestige — getting a single-digit or iconic number
        // at the new club is a real pride moment, especially for younger
        // players. Fires once per signing.
        if let Some(shirt) = self.contract.as_ref().and_then(|c| c.shirt_number) {
            let magnitude = match shirt {
                7 | 9 | 10 => 4.0,
                1..=11 => 2.0,
                _ => 0.0,
            };
            if magnitude > 0.0 {
                self.happiness
                    .add_event(HappinessEventType::ShirtNumberPromotion, magnitude);
            }
        }

        if !self.speaks_local_language(country_code) {
            let mag = if pending.is_loan { -3.0 } else { -5.0 };
            self.happiness.add_event(HappinessEventType::FeelingIsolated, mag);
        }

        if let Some(f) = formation {
            self.emit_role_mismatch_if_unfit(f);
        }

        // Big-money signings (or loans — the borrowing club took him to play)
        // arrive with an implicit playing-time promise.
        let promise_horizon = cfg.promise_horizon_days(pending.is_loan, pending.fee);
        if promise_horizon > 0 {
            self.record_promise(ManagerPromiseKind::PlayingTime, now, promise_horizon);
        }
    }

    fn emit_ambition_shock(&mut self, club_rep_0_to_1: f32, damp: f32) {
        let cfg = AdaptationConfig::default();
        let ambition = self.attributes.ambition;
        if ambition <= cfg.ambition_shock_min_ambition {
            return;
        }
        let expected_rep =
            (ambition - cfg.ambition_shock_floor) * cfg.ambition_to_expected_rep_factor;
        let club_rep = club_rep_0_to_1 * 10000.0;
        let gap = expected_rep - club_rep;
        if gap <= cfg.ambition_shock_threshold {
            return;
        }
        let severity = (gap / 8000.0).clamp(0.5, 2.0);
        self.happiness
            .add_event(HappinessEventType::AmbitionShock, -8.0 * severity * damp);
    }

    fn emit_salary_shock(&mut self, previous_salary: Option<u32>) {
        let cfg = AdaptationConfig::default();
        let Some(prev) = previous_salary else { return };
        let Some(new) = self.contract.as_ref().map(|c| c.salary) else { return };
        if prev == 0 {
            return;
        }
        let ratio = new as f32 / prev as f32;
        if ratio >= cfg.salary_shock_ratio {
            return;
        }
        let severity =
            ((cfg.salary_shock_ratio - ratio) / cfg.salary_shock_ratio).clamp(0.0, 1.0);
        self.happiness
            .add_event(HappinessEventType::SalaryShock, -6.0 - 6.0 * severity);
    }

    fn emit_dream_move(&mut self, club_rep_0_to_1: f32, damp: f32, now: NaiveDate) {
        let cfg = AdaptationConfig::default();
        let ambition = self.attributes.ambition;
        let expected_rep =
            (ambition - cfg.ambition_dream_floor).max(0.0) * cfg.ambition_to_expected_rep_factor;
        let club_rep = club_rep_0_to_1 * 10000.0;
        let surplus = club_rep - expected_rep;
        if surplus < cfg.dream_move_threshold {
            return;
        }

        // Player-reputation gate. A "dream move" requires the new club to
        // be meaningfully bigger than where the player has been. Pinsoglio
        // (Juventus reserve, world_rep ~4500) joining Cittadella (rep ~3000)
        // is a step DOWN, never a dream — even if his ambition is modest.
        // Require the club to sit at least 1000 rep above the player's
        // own world rep before the framing fits.
        let player_world_rep = self.player_attributes.world_reputation as f32;
        if club_rep <= player_world_rep + 1000.0 {
            return;
        }

        // Age gate. "Dream move of his career" doesn't fit a 32+ veteran —
        // late-career moves are pragmatic, not dream-fulfilment. For 32+
        // require an extra 1500 rep margin; over 35, suppress unless the
        // destination is an outright elite club.
        let age = self.age(now);
        if age >= 32 && club_rep < player_world_rep + 2500.0 {
            return;
        }
        if age >= 35 && club_rep < cfg.elite_club_reputation {
            return;
        }

        // Magnitude scales with how far above expectations the move is;
        // ambitious players (high `ambition`) also feel it more strongly.
        let severity = (surplus / 6000.0).clamp(0.5, 2.0);
        let ambition_weight = (ambition / 20.0).clamp(0.4, 1.2);
        let age_dampen = if age >= 32 { 0.6 } else { 1.0 };
        self.happiness.add_event(
            HappinessEventType::DreamMove,
            10.0 * severity * ambition_weight * damp * age_dampen,
        );
    }

    fn emit_joining_elite(&mut self, club_rep_0_to_1: f32, damp: f32) {
        let cfg = AdaptationConfig::default();
        let club_rep = club_rep_0_to_1 * 10000.0;
        if club_rep < cfg.elite_club_reputation {
            return;
        }
        let player_rep = self.player_attributes.world_reputation as f32;
        // Only fire if the club is meaningfully above the player's own
        // standing — a Ballon d'Or winner moving clubs doesn't feel this.
        if club_rep - player_rep < cfg.elite_club_min_player_gap {
            return;
        }
        self.happiness
            .add_event(HappinessEventType::JoiningElite, 6.0 * damp);
    }

    fn emit_salary_boost(&mut self, previous_salary: Option<u32>) {
        let cfg = AdaptationConfig::default();
        let Some(prev) = previous_salary else { return };
        let Some(new) = self.contract.as_ref().map(|c| c.salary) else { return };
        if prev == 0 {
            return;
        }
        let ratio = new as f32 / prev as f32;
        if ratio < cfg.salary_boost_ratio {
            return;
        }
        let severity = ((ratio - cfg.salary_boost_ratio) / 2.0).clamp(0.0, 1.5);
        self.happiness
            .add_event(HappinessEventType::SalaryBoost, 4.0 + 4.0 * severity);
    }

    fn emit_role_mismatch_if_unfit(&mut self, formation: &[PlayerPositionType; 11]) {
        let primary = self.position();
        if formation.iter().any(|p| *p == primary) {
            return;
        }
        let group_match = formation
            .iter()
            .any(|p| p.position_group() == primary.position_group());
        let mag = if group_match { -4.0 } else { -8.0 };
        self.happiness.add_event(HappinessEventType::RoleMismatch, mag);
    }

    /// Development multiplier applied when a player has just stepped up to
    /// a better club. Training alongside higher-calibre teammates and
    /// absorbing a new tactical culture accelerates growth — but only
    /// while there's still catching up to do. The effect fades over the
    /// settlement window and is proportional to the rep gap.
    pub fn step_up_development_multiplier(
        &self,
        now: NaiveDate,
        club_rep_0_to_1: f32,
    ) -> f32 {
        AdaptationConfig::default().step_up_dev_multiplier(
            self.days_since_transfer(now),
            club_rep_0_to_1,
            self.player_attributes.world_reputation as f32,
        )
    }

    /// Weekly integration tick. During the settlement window the player
    /// either bonds with the squad or feels isolated, depending on language
    /// fluency, personality, and age. Runs for ~24 weeks after a transfer so
    /// there's a tail of recovery even once the form penalty has faded.
    pub fn process_integration(&mut self, now: NaiveDate, country_code: &str) {
        // Default: caller doesn't supply squad context — same behaviour as
        // before, but shared-language buddies and mentor support default to
        // zero / none so we err toward firing isolation events when the
        // information isn't available.
        self.process_integration_with_squad(now, country_code, &AdaptationSquadContext::default());
    }

    /// Integration tick variant that reads the [`AdaptationSquadContext`] —
    /// shared-language teammates reduce isolation chance, mentor support
    /// accelerates language progress, and a no-shared-language low-adaptability
    /// player sees a higher early isolation rate.
    pub fn process_integration_with_squad(
        &mut self,
        now: NaiveDate,
        country_code: &str,
        squad: &AdaptationSquadContext,
    ) {
        let cfg = AdaptationConfig::default();
        let Some(days) = self.days_since_transfer(now) else {
            self.process_chronic_language_isolation(now, country_code);
            return;
        };
        if !(0..=cfg.integration_window_days).contains(&days) {
            self.process_chronic_language_isolation(now, country_code);
            return;
        }

        let weeks = days / 7;
        let speaks_local = self.speaks_local_language(country_code);
        let adapt = self.attributes.adaptability.clamp(0.0, 20.0);
        let prof = self.attributes.professionalism.clamp(0.0, 20.0);
        let pull_toward_bonding = (adapt + prof) / 40.0;

        // Shared-language buddies in the squad shave the isolation chance.
        // 1 buddy → −40% chance; 2+ → −70%.
        let isolation_dampener: f32 = match squad.same_language_teammates {
            0 => 1.0,
            1 => 0.6,
            _ => 0.3,
        };

        // Local-language fluency tier reduces settlement penalty.
        // (Settlement multiplier branches handle this; we mirror the same
        // tiering when deciding whether to fire isolation.)
        let in_early_window = weeks < cfg.early_isolation_max_weeks;
        let strict_isolation_gate = !speaks_local
            && adapt < cfg.early_isolation_max_adaptability;
        let no_shared_language_low_adapt = squad.same_language_teammates == 0
            && !speaks_local
            && adapt < 8.0;

        // Higher chance for no-shared-language low-adaptability foreign
        // signings (35% per week for first 4 weeks, before dampener).
        let isolation_base_chance = if no_shared_language_low_adapt && in_early_window {
            0.35
        } else if strict_isolation_gate && in_early_window {
            // Original behaviour — fires reliably.
            1.0
        } else {
            0.0
        };
        let isolation_chance =
            (isolation_base_chance * isolation_dampener).clamp(0.0, 1.0);

        if isolation_chance > 0.0 {
            // Use deterministic per-day roll so testing stays stable.
            let roll = isolation_roll(self.id, now);
            if roll < isolation_chance {
                self.happiness
                    .add_event(HappinessEventType::FeelingIsolated, -2.0);
                return;
            }
        }

        // Settled-into-squad lift. Long cooldown so the per-tick
        // bonding/language predicate doesn't refire the event week after
        // week — happiness clears on transfer, so a fresh club gets a
        // fresh emission.
        if weeks >= cfg.settled_min_weeks
            && (pull_toward_bonding > cfg.settled_pull_threshold || speaks_local)
        {
            self.happiness.add_event_with_cooldown(
                HappinessEventType::SettledIntoSquad,
                1.0,
                365,
            );
        }
    }

}

/// Deterministic per-day per-player isolation roll, in `[0, 1)`. Same date +
/// id produces the same number — keeps weekly tests stable.
fn isolation_roll(player_id: u32, date: NaiveDate) -> f32 {
    use chrono::Datelike;
    let h = (player_id as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(date.num_days_from_ce() as u64);
    let frac = ((h >> 13) as u32 as f32) / (u32::MAX as f32);
    frac.clamp(0.0, 0.999)
}

impl Player {
    /// Post-settlement ongoing language check. A player who's been at a
    /// foreign club for years but never picked up the language keeps
    /// accruing small isolation hits — the dressing-room outsider model.
    /// Runs monthly (day-of-month 1) instead of weekly to avoid stacking.
    fn process_chronic_language_isolation(&mut self, now: NaiveDate, country_code: &str) {
        use chrono::Datelike;
        if now.day() != 1 {
            return;
        }
        if self.speaks_local_language(country_code) {
            return;
        }
        // Passive acceptance: high adaptability/professionalism masks it.
        let cfg = AdaptationConfig::default();
        let adapt = self.attributes.adaptability.clamp(0.0, 20.0);
        let prof = self.attributes.professionalism.clamp(0.0, 20.0);
        if (adapt + prof) / 40.0 > cfg.chronic_isolation_suppress_threshold {
            return;
        }
        self.happiness
            .add_event(HappinessEventType::FeelingIsolated, -1.5);
    }
}

#[cfg(test)]
mod dream_move_gating_tests {
    use super::*;
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

    fn person(ambition: f32) -> PersonAttributes {
        PersonAttributes {
            adaptability: 10.0,
            ambition,
            controversy: 10.0,
            loyalty: 10.0,
            pressure: 10.0,
            professionalism: 10.0,
            sportsmanship: 10.0,
            temperament: 10.0,
            consistency: 10.0,
            important_matches: 10.0,
            dirtiness: 10.0,
        }
    }

    fn player(age: u8, ambition: f32, world_rep: i16) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.world_reputation = world_rep;
        attrs.current_reputation = world_rep;
        attrs.current_ability = 100;
        attrs.potential_ability = 100;
        let today = d(2026, 4, 26);
        let birth = today
            .checked_sub_signed(chrono::Duration::days(age as i64 * 365))
            .unwrap();
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("X".into(), "Y".into()))
            .birth_date(birth)
            .country_id(1)
            .attributes(person(ambition))
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    fn dream_count(p: &Player) -> usize {
        p.happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::DreamMove)
            .count()
    }

    #[test]
    fn pinsoglio_to_cittadella_does_not_fire_dream_move() {
        // 36yo high-rep keeper from Juventus (world_rep ~4500) joining
        // Cittadella (rep ~3000 → 0.30 normalised). Should NOT fire.
        let mut p = player(36, 10.0, 4500);
        let now = d(2026, 4, 26);
        p.emit_dream_move(0.30, 1.0, now);
        assert_eq!(dream_count(&p), 0);
    }

    #[test]
    fn step_down_at_any_age_does_not_fire_dream_move() {
        // 25yo player with world_rep 6000 joining a club at rep 4000.
        let mut p = player(25, 12.0, 6000);
        let now = d(2026, 4, 26);
        p.emit_dream_move(0.40, 1.0, now);
        assert_eq!(dream_count(&p), 0);
    }

    #[test]
    fn young_prospect_to_top_club_fires_dream_move() {
        // 22yo with modest world_rep 2000 joining a top club (rep 8500).
        // Ambition 10 keeps expected_rep at ~4000 — surplus is well above
        // the dream_move_threshold and the rep gate (club > player + 1000)
        // is comfortably met.
        let mut p = player(22, 10.0, 2000);
        let now = d(2026, 4, 26);
        p.emit_dream_move(0.85, 1.0, now);
        assert_eq!(dream_count(&p), 1);
    }

    #[test]
    fn veteran_needs_extra_margin_for_dream_move() {
        // 33yo with world_rep 5000. Club at 6000 — only 1000 above.
        // 32+ requires 2500+ gap, so this should NOT fire.
        let mut p = player(33, 12.0, 5000);
        let now = d(2026, 4, 26);
        p.emit_dream_move(0.60, 1.0, now);
        assert_eq!(dream_count(&p), 0);

        // Same player to a clearly elite club: world_rep 5000, club 8000.
        let mut p2 = player(33, 12.0, 5000);
        p2.emit_dream_move(0.80, 1.0, now);
        assert_eq!(dream_count(&p2), 1);
    }

    #[test]
    fn over_35_requires_elite_destination() {
        // 36yo, world_rep 3000. Club at 6000 — 3000 above the player but
        // not elite (< 7500). 35+ gate should suppress.
        let mut p = player(36, 12.0, 3000);
        let now = d(2026, 4, 26);
        p.emit_dream_move(0.60, 1.0, now);
        assert_eq!(dream_count(&p), 0);

        // Same player to genuinely elite club (rep 8500). Fires.
        let mut p2 = player(36, 12.0, 3000);
        p2.emit_dream_move(0.85, 1.0, now);
        assert_eq!(dream_count(&p2), 1);
    }
}
