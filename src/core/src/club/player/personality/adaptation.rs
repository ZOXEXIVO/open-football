use crate::HappinessEventType;
use crate::club::player::behaviour_config::AdaptationConfig;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::language::Language;
use crate::club::player::player::{ManagerPromiseKind, Player};
use crate::club::{Person, PlayerPositionType};
use crate::{
    CareerDesireEventContext, CareerDesireEvidence, CareerDesireKind, ContractEventContext,
    ContractEventEvidence, ContractEventKind, HappinessEventCause, HappinessEventContext,
    HappinessEventEvidence, HappinessEventFollowUp, HappinessEventScope, HappinessEventSeverity,
    PersonalAdaptationEventContext, PersonalAdaptationKind, RoleStatusEventContext, RoleStatusKind,
};
use chrono::NaiveDate;
use std::cmp::Reverse;

/// Multi-axis reputation gap between the player and his current
/// surroundings. Used by morale-factor calculators (pressure_load,
/// club_fit, coach_credibility), the adaptation score, and squad-level
/// processing to share one source of truth for "how much of an outlier
/// is this player at this club / in this league?"
///
/// All gaps are signed: positive = player above his surroundings
/// (Messi at a small club); negative = player below (a Tier-4 fringe
/// player at Real Madrid). Inputs are normalised to common units.
#[derive(Debug, Clone, Copy)]
pub struct ReputationGap {
    /// player.world_reputation − club_reputation. Both 0..10000.
    pub player_vs_club: i32,
    /// player.world_reputation − league_reputation. Both 0..10000.
    pub player_vs_league: i32,
    /// player.current_ability − expected ability for the club tier
    /// (10× league rep / 1000, capped). Surfaces over- or under-fits.
    pub ability_vs_tier: i32,
}

impl ReputationGap {
    /// Compute against the rounded inputs the caller already has.
    /// `team_reputation_0_to_1` matches the convention used elsewhere
    /// (ClubContext, ScoringEngine) — multiplied by 10000 to share the
    /// player_world_reputation 0..10000 axis.
    pub fn compute(player: &Player, team_reputation_0_to_1: f32, league_reputation: u16) -> Self {
        let p_rep = player.player_attributes.world_reputation as i32;
        let c_rep = (team_reputation_0_to_1.clamp(0.0, 1.0) * 10000.0) as i32;
        let l_rep = league_reputation as i32;
        let expected_tier = ((league_reputation as i32) / 100).clamp(20, 200);
        ReputationGap {
            player_vs_club: p_rep - c_rep,
            player_vs_league: p_rep - l_rep,
            ability_vs_tier: player.player_attributes.current_ability as i32 - expected_tier,
        }
    }

    /// True if the player is meaningfully above where they're playing —
    /// triggers pressure_load spikes, role_clarity sensitivity, and the
    /// squad-side awe/jealousy dynamics already in
    /// `process_reputation_dynamics`.
    pub fn is_outlier_above(&self) -> bool {
        self.player_vs_club >= 3000 || self.player_vs_league >= 3000
    }

    /// True if the player is meaningfully below where they're playing —
    /// triggers role-mismatch tolerance and isolation susceptibility.
    pub fn is_outlier_below(&self) -> bool {
        self.player_vs_club <= -3000 || self.player_vs_league <= -3000
    }
}

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
    /// True when the player carried a recent `WantsReturnHome` mood at
    /// the moment the transfer was finalised. Drives
    /// `HomeReturnOpportunity`-on-completion satisfaction events when
    /// the destination is the player's home country / former / favourite
    /// club.
    pub had_return_home_desire: bool,
    /// True when the player carried a recent `WantsEuropeanCompetition`
    /// mood at the moment of the transfer.
    pub had_european_desire: bool,
    /// True when the player carried a recent `WantsCopaLibertadores`
    /// mood at the moment of the transfer.
    pub had_libertadores_desire: bool,
    /// Source club's world reputation (0..10000). Captured at staging time
    /// — the source club is gone by the time `process_transfer_shock`
    /// runs. Used by the [`TransferEnvironmentProfile`] gates that compare
    /// step-up / step-down magnitudes. 0 when unknown (e.g. free-agent
    /// signings have no source club).
    pub source_club_reputation: u16,
    /// Source league reputation. 0 when unknown.
    pub source_league_reputation: u16,
    /// Destination position-group depth rank — 1 = clear first choice,
    /// 2 = second option, etc. `None` when the caller didn't compute it.
    /// Drives the `RolePathBlockedAtEliteClub` gate.
    pub dest_position_depth_rank: Option<u8>,
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
        let adapt_contribution = ((self.attributes.adaptability - 10.0) * 1.2).clamp(-10.0, 12.0);
        score += adapt_contribution;
        let prof_contribution = ((self.attributes.professionalism - 10.0) * 0.7).clamp(-6.0, 7.0);
        score += prof_contribution;

        // Role fit against the formation.
        if let Some(f) = formation {
            let primary = self.position();
            if f.iter().any(|p| *p == primary) {
                score += 10.0;
            } else if f
                .iter()
                .any(|p| p.position_group() == primary.position_group())
            {
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
            self.languages
                .iter()
                .any(|pl| pl.language == *l && (pl.is_native || pl.proficiency >= 70))
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
        league_reputation: u16,
        country_code: &str,
        formation: Option<&[PlayerPositionType; 11]>,
    ) {
        let Some(pending) = self.pending_signing.take() else {
            return;
        };
        let cfg = AdaptationConfig::default();

        // Ambition / elite-club reactions fire for loans too — being loaned
        // to Real Madrid is still a real prestige moment, even if you're
        // going back in a year. The *DreamMove* framing, by contrast, is
        // reserved for **permanent** career-defining upward moves; loans
        // get the dedicated `DreamLoanOpportunity` event instead so the
        // "sealed the dream move of his career" copy never lands on a
        // year-long borrow. Loans pay at the borrowing club's loan wage
        // (distinct from a full contract) so salary shock/boost is skipped
        // for them.
        let loan_damp = if pending.is_loan {
            cfg.loan_damp_factor
        } else {
            1.0
        };
        let is_favorite_destination = self.favorite_clubs.contains(&pending.destination_club_id);
        // Ambition shock is muted when joining a favorite club — the player
        // knew what they were signing for and the sentimental pull covers the
        // ambition gap. Reputation-based "I should be at a bigger club"
        // doesn't apply to your boyhood side.
        if !is_favorite_destination {
            self.emit_ambition_shock(club_rep_0_to_1, loan_damp);
        }
        // Source-aware classification. Loan-to-elite goes through the
        // dedicated `DreamLoanOpportunity` framing; permanent-to-elite
        // through `DreamMove`; favourite-club permanent moves prefer the
        // sentimental `HomeReturnOpportunity` unless they ALSO clear the
        // real dream-move gates. None of these branches double-emit.
        let player_world_rep = self.player_attributes.world_reputation as f32;
        let club_rep_abs = club_rep_0_to_1.clamp(0.0, 1.0) * 10000.0;
        let source_aware_step_up = Self::is_source_aware_step_up(
            club_rep_abs as u16,
            league_reputation,
            pending.source_club_reputation,
            pending.source_league_reputation,
        );
        if pending.is_loan {
            self.emit_dream_loan_opportunity(
                club_rep_0_to_1,
                player_world_rep,
                source_aware_step_up,
                loan_damp,
                now,
            );
        } else if is_favorite_destination {
            // Permanent favourite-club move: prefer the sentimental
            // homecoming framing. Only upgrade to DreamMove when the
            // numbers also justify it (real reputation step-up over the
            // source club / league).
            let real_dream =
                source_aware_step_up && self.passes_dream_move_gates(club_rep_0_to_1, now);
            if real_dream {
                self.emit_dream_move_with_source(
                    club_rep_0_to_1,
                    source_aware_step_up,
                    loan_damp,
                    now,
                );
            } else {
                self.emit_favourite_club_homecoming(now);
            }
        } else {
            self.emit_dream_move_with_source(
                club_rep_0_to_1,
                source_aware_step_up,
                loan_damp,
                now,
            );
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
            let ctx = HappinessEventContext::new(
                HappinessEventCause::AdaptationIsolation,
                HappinessEventSeverity::from_magnitude(mag),
                HappinessEventScope::DressingRoom,
            )
            .with_evidence(HappinessEventEvidence::LanguageBarrier)
            .with_evidence(HappinessEventEvidence::NewSigningStillSettling)
            .with_follow_up(HappinessEventFollowUp::SettlingInProgress);
            self.happiness.add_event_with_context(
                HappinessEventType::FeelingIsolated,
                mag,
                None,
                ctx,
            );
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

        // Career-desire satisfaction events. These run at the *new*
        // club's first tick (after happiness was reset by the move) so
        // they latch on top of the fresh state. Cooldowns prevent any
        // duplicate emission if process_transfer_shock fires twice for
        // the same signing (defensive guard — pending_signing is
        // single-shot today). The continental helper consults the
        // UEFA-suspension policy — a top Russian club after 2022-02-28
        // can't satisfy a "wants Europe" desire because the club itself
        // can't enter Europe regardless of reputation.
        self.emit_continental_satisfaction_on_signing(&pending, club_rep_0_to_1, country_code, now);
        self.emit_home_return_satisfaction_on_signing(&pending, country_code);

        // Transfer-environment realism: weak↔elite and star↔weak
        // narratives layered on top of the existing shock events.
        // Builds the `TransferEnvironmentProfile` from the staged
        // `PendingSigning` + current ctx and fires the matching
        // first-tick events.
        let profile = TransferEnvironmentProfile::build(
            self,
            now,
            &pending,
            club_rep_0_to_1,
            league_reputation,
        );
        self.apply_first_tick_environment_events(now, &profile);
    }

    /// Stage `ContinentalAmbitionSatisfied` when a player who carried a
    /// recent European or Libertadores desire mood signs for a club at
    /// a credible continental tier. Reads `pending_signing` flags
    /// captured before happiness was reset.
    fn emit_continental_satisfaction_on_signing(
        &mut self,
        pending: &PendingSigning,
        club_rep_0_to_1: f32,
        country_code: &str,
        now: NaiveDate,
    ) {
        if !(pending.had_european_desire || pending.had_libertadores_desire) {
            return;
        }
        // Conservative tier guard: club rep ≥ 0.55 (mid-table top-flight)
        // is the floor for a credible "you got Europe" / "you got
        // Libertadores" moment. Below that the move's a step-up at most.
        if club_rep_0_to_1 < 0.55 {
            return;
        }
        // Federation suspension: a club whose country is currently
        // barred from UEFA can't satisfy a `WantsEuropeanCompetition`
        // desire, no matter its reputation. The Libertadores branch is
        // South-American so suspension doesn't apply, but the European
        // one is gated specifically by the club's country.
        if pending.had_european_desire
            && crate::transfers::TransferRoutePolicy::is_uefa_suspended(country_code, now)
        {
            return;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.continental_ambition_satisfied;
        let mut desire_ctx = if pending.had_libertadores_desire {
            CareerDesireEventContext::new(CareerDesireKind::CopaLibertadoresAmbition)
        } else {
            CareerDesireEventContext::new(CareerDesireKind::EuropeanCompetitionAmbition)
        };
        desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::HighAmbition);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationAdmiration,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_career_desire_context(desire_ctx)
        .with_follow_up(HappinessEventFollowUp::LikelyToSettle);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::ContinentalAmbitionSatisfied,
            mag,
            None,
            happiness_ctx,
            120,
        );
    }

    /// Called when the player's club secures continental qualification
    /// (Europe, Libertadores). Stages `ContinentalAmbitionSatisfied`
    /// when the player has a recent matching desire mood — the team's
    /// season delivered exactly what the player was asking for.
    /// Cooldown 365d: at most one satisfaction event per season.
    pub fn on_continental_qualification_satisfaction(&mut self) {
        let european = self
            .happiness
            .has_recent_event(&HappinessEventType::WantsEuropeanCompetition, 240);
        let libertadores = self
            .happiness
            .has_recent_event(&HappinessEventType::WantsCopaLibertadores, 240);
        if !(european || libertadores) {
            return;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.continental_ambition_satisfied;
        let mut desire_ctx = if libertadores {
            CareerDesireEventContext::new(CareerDesireKind::CopaLibertadoresAmbition)
        } else {
            CareerDesireEventContext::new(CareerDesireKind::EuropeanCompetitionAmbition)
        };
        desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::HighAmbition);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationAdmiration,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_career_desire_context(desire_ctx)
        .with_follow_up(HappinessEventFollowUp::LikelyToSettle);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::ContinentalAmbitionSatisfied,
            mag,
            None,
            happiness_ctx,
            365,
        );
    }

    /// Stage `HomeReturnOpportunity` when a player who carried a recent
    /// `WantsReturnHome` mood signs for a club in their home country
    /// (or a favourite club). The earlier "approach landed" emission
    /// in `transfer_social` covers the *interest* moment; this one
    /// covers the *signature* moment.
    fn emit_home_return_satisfaction_on_signing(
        &mut self,
        pending: &PendingSigning,
        country_code: &str,
    ) {
        if !pending.had_return_home_desire {
            return;
        }
        // Either the destination is a favourite club (covers heritage
        // / boyhood-club returns), or the local language matches the
        // player's native — a reasonable proxy for "home-country move"
        // when the country_id mapping isn't surfaced here.
        let is_favourite = self.favorite_clubs.contains(&pending.destination_club_id);
        let speaks_native =
            self.speaks_local_language(country_code) && self.languages.iter().any(|l| l.is_native);
        if !(is_favourite || speaks_native) {
            return;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.home_return_opportunity;
        let mut desire_ctx =
            CareerDesireEventContext::new(CareerDesireKind::ReturnHomeAfterPoorAdaptation);
        desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::HomeOrFavouriteLink);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::AdaptationIsolation,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_career_desire_context(desire_ctx)
        .with_follow_up(HappinessEventFollowUp::LikelyToSettle);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::HomeReturnOpportunity,
            mag,
            None,
            happiness_ctx,
            120,
        );
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
        let Some(new) = self.contract.as_ref().map(|c| c.salary) else {
            return;
        };
        if prev == 0 {
            return;
        }
        let ratio = new as f32 / prev as f32;
        if ratio >= cfg.salary_shock_ratio {
            return;
        }
        let severity = ((cfg.salary_shock_ratio - ratio) / cfg.salary_shock_ratio).clamp(0.0, 1.0);
        let mag = -6.0 - 6.0 * severity;
        let cctx = ContractEventContext::new(ContractEventKind::SalaryShock)
            .with_wage_vs_previous(ratio)
            .with_evidence(ContractEventEvidence::SquadStatusDowngrade);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Boardroom,
        )
        .with_contract_context(cctx);
        self.happiness.add_event_with_context(
            HappinessEventType::SalaryShock,
            mag,
            None,
            happiness_ctx,
        );
    }

    /// Source-aware step-up predicate: a permanent "dream" move requires
    /// the destination to be **meaningfully** bigger than where the
    /// player came from. Two-axis check — either the club rep jumps by
    /// at least `dream_move_source_club_gap` (default 2000) or the
    /// league rep jumps by `dream_move_source_league_gap` (default 1500).
    /// Either gap is enough; both is gilding.
    ///
    /// A move with no source data (free agent, manual stage with zero
    /// source rep) deliberately fails the gate — DreamMove should not
    /// fire on a free-agent landing without independent evidence.
    fn is_source_aware_step_up(
        dest_club_rep: u16,
        dest_league_rep: u16,
        source_club_rep: u16,
        source_league_rep: u16,
    ) -> bool {
        let cfg = AdaptationConfig::default();
        if source_club_rep == 0 && source_league_rep == 0 {
            return false;
        }
        let club_gap = (dest_club_rep as i32) - (source_club_rep as i32);
        let league_gap = (dest_league_rep as i32) - (source_league_rep as i32);
        club_gap >= cfg.dream_move_source_club_gap as i32
            || league_gap >= cfg.dream_move_source_league_gap as i32
    }

    /// All non-source dream-move gates the legacy `emit_dream_move`
    /// enforced: ambition surplus, player-world-rep margin, and the
    /// age-band cutoffs. Pulled out so the favourite-club branch can
    /// check whether a sentimental move ALSO qualifies as a real dream
    /// move without firing the event itself.
    fn passes_dream_move_gates(&self, club_rep_0_to_1: f32, now: NaiveDate) -> bool {
        let cfg = AdaptationConfig::default();
        let ambition = self.attributes.ambition;
        let expected_rep =
            (ambition - cfg.ambition_dream_floor).max(0.0) * cfg.ambition_to_expected_rep_factor;
        let club_rep = club_rep_0_to_1 * 10000.0;
        let surplus = club_rep - expected_rep;
        if surplus < cfg.dream_move_threshold {
            return false;
        }
        let player_world_rep = self.player_attributes.world_reputation as f32;
        if club_rep <= player_world_rep + 1000.0 {
            return false;
        }
        let age = self.age(now);
        if age >= 32 && club_rep < player_world_rep + 2500.0 {
            return false;
        }
        if age >= 35 && club_rep < cfg.elite_club_reputation {
            return false;
        }
        true
    }

    /// Permanent **DreamMove** with full source-awareness. Emits only
    /// when both the legacy ambition / world-rep / age gates pass AND
    /// the source club / league reputation gap is meaningful. Loans
    /// must go through `emit_dream_loan_opportunity` instead.
    fn emit_dream_move_with_source(
        &mut self,
        club_rep_0_to_1: f32,
        source_aware_step_up: bool,
        damp: f32,
        now: NaiveDate,
    ) {
        if !source_aware_step_up {
            return;
        }
        if !self.passes_dream_move_gates(club_rep_0_to_1, now) {
            return;
        }
        let cfg = AdaptationConfig::default();
        let ambition = self.attributes.ambition;
        let expected_rep =
            (ambition - cfg.ambition_dream_floor).max(0.0) * cfg.ambition_to_expected_rep_factor;
        let club_rep = club_rep_0_to_1 * 10000.0;
        let surplus = club_rep - expected_rep;
        let age = self.age(now);
        let severity = (surplus / 6000.0).clamp(0.5, 2.0);
        let ambition_weight = (ambition / 20.0).clamp(0.4, 1.2);
        let age_dampen = if age >= 32 { 0.6 } else { 1.0 };
        self.happiness.add_event(
            HappinessEventType::DreamMove,
            10.0 * severity * ambition_weight * damp * age_dampen,
        );
    }

    /// Loan equivalent of `emit_dream_move_with_source`. Fires only for
    /// genuinely top-tier loan destinations — the borrowing club sits
    /// at/above the elite-reputation floor AND meaningfully above the
    /// parent club. Magnitude pulls from `dream_loan_opportunity` so
    /// the lift stays smaller than a permanent dream move (it's still
    /// a temporary borrow). Suppressed at 35+: late-career loans don't
    /// fit the "opportunity" narrative.
    ///
    /// The emitted event carries a full `HappinessEventContext` so the
    /// event page can explain why it fired: the destination is elite
    /// (`JoinedEliteClub`), the parent ↔ destination reputation gap is
    /// meaningful (`ReputationGap`), the player's own ambition tipped
    /// the framing toward "opportunity" rather than "development loan"
    /// (`HighAmbition` when applicable), and the temporary nature is
    /// surfaced via the `SettlingInProgress` follow-up.
    fn emit_dream_loan_opportunity(
        &mut self,
        club_rep_0_to_1: f32,
        player_world_rep: f32,
        source_aware_step_up: bool,
        damp: f32,
        now: NaiveDate,
    ) {
        let cfg = AdaptationConfig::default();
        let club_rep = club_rep_0_to_1 * 10000.0;
        // Borrowing club must be elite — otherwise it's an ordinary
        // development loan, not a "loan of a lifetime".
        if club_rep < cfg.elite_club_reputation {
            return;
        }
        // Must also be a step up relative to where the player came from
        // — a star at Juventus loaned to Real Madrid is one thing; a
        // squad swap inside the same tier isn't.
        if !source_aware_step_up {
            return;
        }
        // Player-world-rep margin keeps the framing reserved for players
        // for whom the move is genuinely a level up.
        if club_rep <= player_world_rep + 1000.0 {
            return;
        }
        // Age gate. Veterans go out on loan for minutes, not prestige.
        let age = self.age(now);
        if age >= 35 {
            return;
        }
        let cat = HappinessConfig::default().catalog;
        let base = cat.dream_loan_opportunity;
        let ambition = self.attributes.ambition;
        let ambition_weight = (ambition / 20.0).clamp(0.5, 1.2);
        let age_dampen = if age >= 32 { 0.7 } else { 1.0 };
        let mag = base * ambition_weight * damp * age_dampen;

        let mut ev_ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationAdmiration,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_evidence(HappinessEventEvidence::JoinedEliteClub)
        .with_evidence(HappinessEventEvidence::ReputationGap)
        .with_follow_up(HappinessEventFollowUp::SettlingInProgress);
        if ambition >= 14.0 {
            ev_ctx = ev_ctx.with_evidence(HappinessEventEvidence::HighAmbition);
        }
        self.happiness.add_event_with_context(
            HappinessEventType::DreamLoanOpportunity,
            mag,
            None,
            ev_ctx,
        );
    }

    /// Sentimental homecoming event for a permanent move to a favourite
    /// club that does NOT clear the source-aware dream-move gates. Tagged
    /// with the dedicated `FavoriteClubHomecoming` desire kind — the
    /// player isn't escaping a failed adaptation, they're answering a
    /// heritage pull, so the framing must not borrow the
    /// `ReturnHomeAfterPoorAdaptation` flavour. The event itself stays
    /// `HomeReturnOpportunity` so the existing follow-up / "likely to
    /// settle" wiring keeps working; only the context kind changes so
    /// the renderer can pick favourite-specific copy.
    fn emit_favourite_club_homecoming(&mut self, now: NaiveDate) {
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.home_return_opportunity;
        // Veterans get the same softer signal — boyhood return is
        // already a settling moment, not a fresh ambition lift.
        let mag = if self.age(now) >= 32 { base * 0.7 } else { base };
        let desire_ctx = CareerDesireEventContext::new(CareerDesireKind::FavoriteClubHomecoming)
            .with_evidence(CareerDesireEvidence::HomeOrFavouriteLink);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::SupporterIdentification,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_career_desire_context(desire_ctx)
        .with_follow_up(HappinessEventFollowUp::LikelyToSettle);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::HomeReturnOpportunity,
            mag,
            None,
            happiness_ctx,
            120,
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
        let Some(new) = self.contract.as_ref().map(|c| c.salary) else {
            return;
        };
        if prev == 0 {
            return;
        }
        let ratio = new as f32 / prev as f32;
        if ratio < cfg.salary_boost_ratio {
            return;
        }
        let severity = ((ratio - cfg.salary_boost_ratio) / 2.0).clamp(0.0, 1.5);
        let mag = 4.0 + 4.0 * severity;
        let cctx = ContractEventContext::new(ContractEventKind::SalaryBoost)
            .with_wage_vs_previous(ratio)
            .with_evidence(ContractEventEvidence::OverpaidVsExpectation);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Boardroom,
        )
        .with_contract_context(cctx);
        self.happiness.add_event_with_context(
            HappinessEventType::SalaryBoost,
            mag,
            None,
            happiness_ctx,
        );
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
        let rctx = RoleStatusEventContext::new(RoleStatusKind::NoNaturalRoleInFormation);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::TacticalDisagreement,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::MatchDay,
        )
        .with_role_status_context(rctx);
        self.happiness.add_event_with_context(
            HappinessEventType::RoleMismatch,
            mag,
            None,
            happiness_ctx,
        );
    }

    /// Development multiplier applied when a player has just stepped up to
    /// a better club. Training alongside higher-calibre teammates and
    /// absorbing a new tactical culture accelerates growth — but only
    /// while there's still catching up to do. The effect fades over the
    /// settlement window and is proportional to the rep gap.
    pub fn step_up_development_multiplier(&self, now: NaiveDate, club_rep_0_to_1: f32) -> f32 {
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
        let strict_isolation_gate = !speaks_local && adapt < cfg.early_isolation_max_adaptability;
        let no_shared_language_low_adapt =
            squad.same_language_teammates == 0 && !speaks_local && adapt < 8.0;

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
        let isolation_chance = (isolation_base_chance * isolation_dampener).clamp(0.0, 1.0);

        if isolation_chance > 0.0 {
            // Use deterministic per-day roll so testing stays stable.
            let roll = isolation_roll(self.id, now);
            if roll < isolation_chance {
                let mag = -2.0;
                let mut ctx = HappinessEventContext::new(
                    HappinessEventCause::AdaptationIsolation,
                    HappinessEventSeverity::from_magnitude(mag),
                    HappinessEventScope::DressingRoom,
                )
                .with_evidence(HappinessEventEvidence::NoInnerCircleYet)
                .with_evidence(HappinessEventEvidence::NewSigningStillSettling)
                .with_follow_up(HappinessEventFollowUp::SettlingInProgress);
                if !speaks_local {
                    ctx = ctx.with_evidence(HappinessEventEvidence::LanguageBarrier);
                }
                self.happiness.add_event_with_context(
                    HappinessEventType::FeelingIsolated,
                    mag,
                    None,
                    ctx,
                );
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
            let mag = 1.0;
            let ctx = HappinessEventContext::new(
                HappinessEventCause::TrainingPartnership,
                HappinessEventSeverity::from_magnitude(mag),
                HappinessEventScope::DressingRoom,
            )
            .with_evidence(HappinessEventEvidence::StrongExistingBond)
            .with_follow_up(HappinessEventFollowUp::SettlingInProgress);
            self.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::SettledIntoSquad,
                mag,
                None,
                ctx,
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

/// Inputs for chronic-adaptation-failure detection. Keeps the player
/// helper free of world walks: the caller (Player::simulate via the
/// transfer-desire context) collects continent / language / squad
/// signals from `GlobalContext` once and feeds them through.
#[derive(Debug, Clone, Copy)]
pub struct AdaptationFailureSignals {
    /// Continent of the player's nationality country.
    pub player_nationality_continent_id: u32,
    /// Continent of the player's current club country. 0 if unknown.
    pub club_continent_id: u32,
    /// True if club country == player nationality country.
    pub club_in_home_country: bool,
    /// True if the destination is one of the player's favourite clubs
    /// (lifts the suppression bar — favourite-club moves don't fire
    /// return-home unless adaptation is genuinely terrible).
    pub destination_is_favourite: bool,
    /// Compatriots / shared-language teammates currently in the squad.
    /// Drives the `NoCompatriotSupport` evidence.
    pub same_language_or_nationality_teammates: u8,
    /// Pre-computed adaptation score (0..100) at emit time. Pass 0 if
    /// the caller didn't compute it; the helper falls back to recent
    /// isolation counts.
    pub adaptation_score: f32,
    /// Current `club_fit` morale axis. Drives `LowClubFit` evidence and
    /// the suppression bar.
    pub club_fit: f32,
}

impl Player {
    /// Detect chronic post-settlement adaptation failure and emit a
    /// `WantsReturnHome` mood (with `CareerDesireEventContext`). Fires
    /// only after a fair window (60–120 days post-transfer) and is
    /// suppressed for favourite-club moves / clear dream moves unless
    /// adaptation is genuinely poor. Cooldown 60 days so the mood
    /// doesn't spam the event log.
    ///
    /// Caller — typically `process_transfer_desire` from the weekly
    /// tick — decides whether the cumulative mood justifies escalating
    /// to `Req`. This helper just stages the *fact* on happiness; the
    /// transfer-desire path reads the recent events.
    pub fn process_chronic_adaptation_failure(
        &mut self,
        now: NaiveDate,
        country_code: &str,
        signals: &AdaptationFailureSignals,
    ) -> bool {
        // Honeymoon guard — settlement window plus a tail. The weakest
        // bound (60 days) is the request requirement; the upper bound
        // (no upper bound) lets the mood persist as long as the
        // mismatch does, gated by cooldown.
        let days_at_club = match self.days_since_transfer(now) {
            Some(d) if d >= 60 => d,
            _ => return false,
        };

        // Cooldown — 60 days between WantsReturnHome events on the same
        // player. Short enough that weekly ticks can re-fire as the
        // mood lingers, long enough to avoid spam.
        if self
            .happiness
            .has_recent_event(&HappinessEventType::WantsReturnHome, 60)
        {
            return false;
        }

        // Players in their home country never want to "return home".
        if signals.club_in_home_country {
            return false;
        }

        let speaks_local = self.speaks_local_language(country_code);
        let adapt = self.attributes.adaptability.clamp(0.0, 20.0);
        let prof = self.attributes.professionalism.clamp(0.0, 20.0);
        // Both ids must be known — if the caller passed 0 for either,
        // we can't claim the move crossed a continent boundary, so the
        // signal is dropped to avoid false positives on foreign-country
        // players whose nationality continent the caller couldn't
        // resolve.
        let different_continent = signals.club_continent_id != 0
            && signals.player_nationality_continent_id != 0
            && signals.club_continent_id != signals.player_nationality_continent_id;

        // Repeated isolation events in the last 90 days — a chronic
        // outsider, not a one-off bad fortnight. Filter out the
        // companion `StillStrugglingToSettle` markers this helper itself
        // emits below, otherwise the detector would feed itself: every
        // fired WantsReturnHome would stage an isolation event that
        // counts toward the *next* fire, ratcheting the mood up.
        let recent_isolation_count = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                if e.event_type != HappinessEventType::FeelingIsolated || e.days_ago > 90 {
                    return false;
                }
                // Drop our own companion markers — see emit site below.
                let is_companion = e
                    .context
                    .as_ref()
                    .and_then(|c| c.personal_adaptation_context.as_ref())
                    .map(|p| {
                        matches!(
                            p.kind,
                            PersonalAdaptationKind::StillStrugglingToSettle
                                | PersonalAdaptationKind::ReturnHomeAfterPoorAdaptation
                        )
                    })
                    .unwrap_or(false);
                !is_companion
            })
            .count() as u8;

        let adaptation_score = signals.adaptation_score;
        let poor_adaptation = adaptation_score > 0.0 && adaptation_score < 40.0;

        // Score the situation. Each signal is a small positive weight;
        // we fire once the cumulative weight clears the bar.
        let mut score: f32 = 0.0;
        if different_continent {
            score += 2.0;
        }
        if !speaks_local {
            score += 1.5;
        }
        if adapt <= 8.0 {
            score += 1.5;
        }
        if signals.same_language_or_nationality_teammates == 0 {
            score += 1.0;
        }
        if recent_isolation_count >= 2 {
            score += 1.5;
        }
        if signals.club_fit <= -3.0 {
            score += 1.0;
        }
        if poor_adaptation {
            score += 2.0;
        }
        if self.happiness.morale < 35.0 {
            score += 1.0;
        }

        // High professionalism delays acceptance of homesickness — pros
        // grit through. Doesn't fully suppress.
        if prof >= 16.0 {
            score -= 1.0;
        }
        // High loyalty similarly buys patience.
        let loyalty = self.attributes.loyalty.clamp(0.0, 20.0);
        if loyalty >= 16.0 {
            score -= 1.0;
        }

        // Favourite-club move: lift the bar — only the most extreme
        // failures (very low adaptation, deeply negative morale) get
        // through.
        if signals.destination_is_favourite {
            if !poor_adaptation || self.happiness.morale > 20.0 {
                return false;
            }
            score -= 2.0;
        }

        // Threshold — five signal-points clears it. A foreign player
        // with no language, no compatriots, low adaptability, and a
        // few isolation events crosses this naturally; a settled
        // foreign player with one or two checks does not.
        if score < 5.0 {
            return false;
        }

        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.wants_return_home;

        let mut desire_ctx =
            CareerDesireEventContext::new(CareerDesireKind::ReturnHomeAfterPoorAdaptation)
                .with_days_at_club(days_at_club.max(0) as u32);
        if adaptation_score > 0.0 {
            desire_ctx = desire_ctx.with_adaptation_score(adaptation_score);
        }
        if different_continent {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::DifferentContinent);
        }
        if !speaks_local {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::NoLocalLanguage);
        }
        if adapt <= 8.0 {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::LowAdaptability);
        }
        if signals.same_language_or_nationality_teammates == 0 {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::NoCompatriotSupport);
        }
        if poor_adaptation {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::PoorAdaptationScore);
        }
        if recent_isolation_count >= 2 {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::RepeatedIsolation);
        }
        if signals.club_fit <= -3.0 {
            desire_ctx = desire_ctx.with_evidence(CareerDesireEvidence::LowClubFit);
        }

        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::AdaptationIsolation,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_career_desire_context(desire_ctx)
        .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);

        // Also stage a `StillStrugglingToSettle` adaptation marker so
        // the renderer's existing personal-adaptation surface keeps a
        // contemporaneous "why" entry alongside the desire mood. Note
        // the kind is `StillStrugglingToSettle`, not the desire kind —
        // this companion must NOT feed back into the isolation count
        // that drives this helper (see filter above).
        let pactx = PersonalAdaptationEventContext::new(
            PersonalAdaptationKind::StillStrugglingToSettle,
            days_at_club.max(0) as u32,
        )
        .with_adaptability(self.attributes.adaptability)
        .with_local_language(speaks_local);

        self.happiness.add_event_with_context(
            HappinessEventType::WantsReturnHome,
            mag,
            None,
            happiness_ctx,
        );
        // Companion StillStrugglingToSettle adaptation marker — fires on
        // a 30d cooldown so the player feed can show one "settling"
        // anchor without the desire mood being duplicated every week.
        let still_struggling_ctx = HappinessEventContext::new(
            HappinessEventCause::AdaptationIsolation,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::DressingRoom,
        )
        .with_personal_adaptation_context(pactx);
        let _ = self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::FeelingIsolated,
            -1.0,
            None,
            still_struggling_ctx,
            30,
        );
        true
    }

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
        let mag = -1.5;
        let ctx = HappinessEventContext::new(
            HappinessEventCause::AdaptationIsolation,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::DressingRoom,
        )
        .with_evidence(HappinessEventEvidence::LanguageBarrier)
        .with_evidence(HappinessEventEvidence::NoInnerCircleYet)
        .with_follow_up(HappinessEventFollowUp::ManagerInterventionRisk);
        self.happiness
            .add_event_with_context(HappinessEventType::FeelingIsolated, mag, None, ctx);
    }
}

// ============================================================
// TransferEnvironmentProfile — weak↔elite / star↔weak gates
// ============================================================

/// Snapshot of the player's environmental situation right after a
/// transfer. Built once by `process_transfer_shock` and `process_transfer_environment_story`
/// from the staged [`PendingSigning`] + ctx.
///
/// All reputation fields share the 0..10000 scale. Derived coefficients
/// (`club_rep_gap`, `league_rep_gap`, `step_up_score`, `pressure_score`)
/// are exposed as methods so callers don't recompute.
///
/// Fields are intentionally narrow — each one is consumed by at least
/// one gate or emit-site magnitude scaler. Audit before adding new
/// fields: a write-only field becomes confusing fast.
#[derive(Debug, Clone, Copy)]
pub struct TransferEnvironmentProfile {
    pub source_club_rep: u16,
    pub dest_club_rep: u16,
    pub source_league_rep: u16,
    pub dest_league_rep: u16,
    pub player_world_rep: i16,
    pub player_current_rep: i16,
    pub player_ca: u8,
    pub age: u8,
    pub fee: f64,
    pub is_loan: bool,
    pub destination_is_favorite: bool,
    pub dest_position_depth_rank: Option<u8>,
    // Personality axes — captured so the cooldown helper doesn't have to
    // re-walk the player struct.
    pub ambition: f32,
    pub pressure: f32,
    pub professionalism: f32,
    pub loyalty: f32,
    pub adaptability: f32,
}

impl TransferEnvironmentProfile {
    /// Build from a freshly-consumed [`PendingSigning`] + current ctx.
    pub fn build(
        player: &Player,
        now: NaiveDate,
        pending: &PendingSigning,
        dest_club_rep_0_to_1: f32,
        dest_league_reputation: u16,
    ) -> Self {
        let dest_club_rep = (dest_club_rep_0_to_1.clamp(0.0, 1.0) * 10000.0) as u16;
        let destination_is_favorite = player.favorite_clubs.contains(&pending.destination_club_id);
        TransferEnvironmentProfile {
            source_club_rep: pending.source_club_reputation,
            dest_club_rep,
            source_league_rep: pending.source_league_reputation,
            dest_league_rep: dest_league_reputation,
            player_world_rep: player.player_attributes.world_reputation,
            player_current_rep: player.player_attributes.current_reputation,
            player_ca: player.player_attributes.current_ability,
            age: player.age(now),
            fee: pending.fee,
            is_loan: pending.is_loan,
            destination_is_favorite,
            dest_position_depth_rank: pending.dest_position_depth_rank,
            ambition: player.attributes.ambition,
            pressure: player.attributes.pressure,
            professionalism: player.attributes.professionalism,
            loyalty: player.attributes.loyalty,
            adaptability: player.attributes.adaptability,
        }
    }

    pub fn club_rep_gap(&self) -> i32 {
        self.dest_club_rep as i32 - self.source_club_rep as i32
    }

    pub fn league_rep_gap(&self) -> i32 {
        self.dest_league_rep as i32 - self.source_league_rep as i32
    }

    pub fn player_vs_dest_club(&self) -> i32 {
        self.player_world_rep as i32 - self.dest_club_rep as i32
    }

    /// Expected CA for the destination tier — `league_rep / 60` clamped
    /// to a sensible band. Used to detect "below standard" / "above
    /// standard" mismatches.
    pub fn expected_ca_for_dest(&self) -> i32 {
        ((self.dest_league_rep as i32) / 60).clamp(45, 175)
    }

    pub fn ability_vs_dest_tier(&self) -> i32 {
        self.player_ca as i32 - self.expected_ca_for_dest()
    }

    /// Heuristic step-up score in roughly [-1.5, 1.5]: weighted blend
    /// of club / league / ability gaps each normalised by ~10000 / ~10000
    /// / ~100. Positive = step up; negative = step down.
    pub fn step_up_score(&self) -> f32 {
        let club_norm = self.club_rep_gap() as f32 / 10000.0;
        let league_norm = self.league_rep_gap() as f32 / 10000.0;
        let ability_norm = self.ability_vs_dest_tier() as f32 / 100.0;
        0.45 * club_norm + 0.35 * league_norm + 0.20 * ability_norm
    }

    /// 0..1 pressure score — top-tier club + top-tier league + high
    /// personal reputation all contribute. Drives fan-expectation /
    /// media-spotlight gates.
    pub fn pressure_score(&self) -> f32 {
        let club = (self.dest_club_rep as f32 / 10000.0).clamp(0.0, 1.0);
        let league = (self.dest_league_rep as f32 / 10000.0).clamp(0.0, 1.0);
        let player = (self.player_current_rep.max(0) as f32 / 10000.0).clamp(0.0, 1.0);
        (0.50 * club + 0.30 * league + 0.20 * player).clamp(0.0, 1.0)
    }

    /// True when the destination club + league rep + player ability gap
    /// match the "weak player at elite club" narrative gate from the spec.
    pub fn is_weak_player_at_elite_club(&self) -> bool {
        if self.dest_club_rep < 7500 {
            return false;
        }
        let club_or_league_gap = self.club_rep_gap() >= 2500 || self.league_rep_gap() >= 2000;
        if !club_or_league_gap {
            return false;
        }
        let expected = self.expected_ca_for_dest();
        let ability_below = (self.player_ca as i32) < expected - 15;
        let rep_below = (self.player_world_rep as i32) < self.dest_club_rep as i32 - 2500;
        ability_below || rep_below
    }

    /// True when the destination is a clear step-down for a high-rep
    /// player — drives `TooGoodForLevel` / `StepDownEmbarrassment` gates.
    pub fn is_star_at_weak_club(&self) -> bool {
        let rep_gap = self.player_world_rep as i32 - self.dest_club_rep as i32 >= 3000;
        let ability_gap = self.ability_vs_dest_tier() >= 35;
        rep_gap || ability_gap
    }
}

/// Narrative role of an environment-story candidate. Caps in
/// [`Player::apply_first_tick_environment_events`] keep the arrival
/// feed readable: at most one Primary headline + one Flavor add-on.
/// `Universal` candidates emit alongside without competing for the cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvRole {
    /// Headline event — defines the arrival narrative. Cap: 1 per tick.
    Primary,
    /// Add-on flavour — colour for the headline. Cap: 1 per tick.
    Flavor,
    /// Orthogonal signal independent of the weak↔elite / star↔weak
    /// framing. Emits separately, doesn't consume the cap.
    Universal,
}

/// One candidate environment-story emission. Built by pure
/// `candidate_*` helpers and emitted via [`Player::emit_candidate`].
struct EnvCandidate {
    event_type: HappinessEventType,
    magnitude: f32,
    context: HappinessEventContext,
    role: EnvRole,
    /// Higher = preferred when multiple candidates of the same role
    /// pass their gates. Stable within a single tick.
    priority: u8,
    cooldown_days: u16,
}

/// Snapshot of the player's social-integration signals — used by
/// `SeniorMentorSupport` and the weekly adaptation score so we don't
/// pretend a defaulted `AdaptationSquadContext` is realistic.
#[derive(Debug, Clone, Copy)]
struct SocialSignals {
    same_language_teammates: u8,
    same_nationality_teammates: u8,
    team_chemistry: f32,
    manager_relation_level: f32,
}

impl SocialSignals {
    /// Read from `player.squad_social_view` (set by the team's weekly
    /// pre-tick) + `player.relations.get_team_chemistry()`. Returns
    /// neutral defaults only when the social view hasn't been populated
    /// yet — a recent transfer's first weekly tick falls in this bucket.
    fn from_player(player: &Player) -> Self {
        let view = player.squad_social_view.as_ref();
        SocialSignals {
            same_language_teammates: view.map(|v| v.same_language_teammates).unwrap_or(0),
            same_nationality_teammates: view.map(|v| v.same_nationality_teammates).unwrap_or(0),
            team_chemistry: player.relations.get_team_chemistry().clamp(0.0, 100.0),
            // No surfaced helper for highest staff-relation level today —
            // pass 0 (neutral) and rely on `adaptation_score`'s own
            // contribution floor. Tracked for a future helper rather than
            // pretending we have the data.
            manager_relation_level: 0.0,
        }
    }

    /// True when the player has at least one social anchor — a
    /// compatriot or a fluent shared-language teammate. Drives the
    /// `SeniorMentorSupport` gate.
    fn has_support_anchor(&self) -> bool {
        self.same_language_teammates >= 1 || self.same_nationality_teammates >= 1
    }

    /// Build an `AdaptationSquadContext` for the weekly tick. The
    /// `is_loan` / `is_favorite_club` flags are caller-supplied.
    fn adaptation_squad_context(
        &self,
        is_loan: bool,
        is_favorite_club: bool,
    ) -> AdaptationSquadContext {
        AdaptationSquadContext {
            same_language_teammates: self.same_language_teammates,
            same_nationality_teammates: self.same_nationality_teammates,
            mentor_quality: None,
            squad_chemistry: self.team_chemistry,
            manager_relation_level: self.manager_relation_level,
            is_loan,
            is_favorite_club,
        }
    }
}

impl Player {
    /// Apply the first-tick (post-shock) environment-realism events
    /// derived from the [`TransferEnvironmentProfile`].
    ///
    /// Build → rank → emit:
    ///   1. Each `candidate_*` helper inspects its gate and returns
    ///      `Some(EnvCandidate)` when it would fire.
    ///   2. Within `Primary` / `Flavor` roles, the highest-priority
    ///      candidate wins — emitting at most one of each role.
    ///   3. `Universal` candidates (loan-tier mismatch, media
    ///      spotlight) emit independently and don't compete for the cap.
    ///
    /// Existing first-tick shock events (DreamMove, JoiningElite,
    /// AmbitionShock, SalaryShock, FeelingIsolated, RoleMismatch) fire
    /// upstream in `process_transfer_shock` — this method only owns the
    /// new environment narrative.
    fn apply_first_tick_environment_events(
        &mut self,
        now: NaiveDate,
        profile: &TransferEnvironmentProfile,
    ) {
        let _ = now;
        let signals = SocialSignals::from_player(self);

        // Build the candidate pool. Each builder method is gate-checked
        // and returns None when the situation doesn't fit — so calling
        // every helper here is cheap.
        let mut pool = EnvCandidatePool::new();
        if profile.is_weak_player_at_elite_club() {
            pool.push_some(profile.top_club_opportunity_candidate());
            pool.push_some(profile.elite_training_lift_candidate());
            pool.push_some(profile.overawed_by_elite_club_candidate());
            pool.push_some(profile.role_path_blocked_candidate());
            pool.push_some(profile.dressing_room_status_shock_candidate());
            pool.push_some(profile.senior_mentor_support_candidate(&signals));
        }
        if profile.is_star_at_weak_club() {
            pool.push_some(profile.too_good_for_level_candidate());
            pool.push_some(profile.step_down_embarrassment_candidate());
            pool.push_some(profile.training_standard_frustration_candidate());
            pool.push_some(profile.fan_expectation_burden_first_tick_candidate());
            pool.push_some(profile.dressing_room_status_shock_candidate());
        }

        // Cap: 1 Primary + 1 Flavor. Pick highest priority per role.
        if let Some(primary) = pool.take_top_of(EnvRole::Primary) {
            self.emit_candidate(primary);
        }
        if let Some(flavor) = pool.take_top_of(EnvRole::Flavor) {
            self.emit_candidate(flavor);
        }

        // Universal signals — fire alongside the narrative cap.
        if profile.is_loan {
            if let Some(c) = profile.loan_level_mismatch_candidate() {
                self.emit_candidate(c);
            }
        }
        if let Some(c) = profile.media_spotlight_pressure_candidate() {
            self.emit_candidate(c);
        }
    }

    /// Push a built candidate onto happiness with its full context and
    /// per-event cooldown. Bool return mirrors the underlying happiness
    /// API but is currently unused by callers — the candidate's gate
    /// has already filtered noise.
    fn emit_candidate(&mut self, c: EnvCandidate) -> bool {
        self.happiness.add_event_with_context_and_cooldown(
            c.event_type,
            c.magnitude,
            None,
            c.context,
            c.cooldown_days,
        )
    }

    // ── Age / ambition / professionalism scaling ────────────────

    /// Spec multiplier: `1.20` for ≤23, `0.85` for ≥30, `1.0` otherwise.
    fn age_amplifier_for_top_club(age: u8) -> f32 {
        if age <= 23 {
            1.20
        } else if age >= 30 {
            0.85
        } else {
            1.0
        }
    }

    /// Spec: `0.85 + ambition / 20 * 0.45` → roughly [0.85, 1.30].
    fn ambition_amplifier_for_aspirational(ambition: f32) -> f32 {
        0.85 + (ambition.clamp(0.0, 20.0) / 20.0) * 0.45
    }

    /// Professionalism dampener on negatives: `1.0 - prof / 20 * 0.35`.
    fn professionalism_dampener_for_negatives(professionalism: f32) -> f32 {
        (1.0 - (professionalism.clamp(0.0, 20.0) / 20.0) * 0.35).max(0.5)
    }
}

/// Newtype wrapper around the candidate vector. Concentrates the
/// "push if Some, then pick the top" pattern into one place so both the
/// first-tick orchestrator and the weekly story orchestrator share the
/// same selection semantics.
struct EnvCandidatePool {
    items: Vec<EnvCandidate>,
}

impl EnvCandidatePool {
    fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Append a candidate if the builder returned `Some` — equivalent to
    /// `if let Some(c) = opt { pool.push(c) }` but reads as a single
    /// expression at the call site.
    fn push_some(&mut self, candidate: Option<EnvCandidate>) {
        if let Some(c) = candidate {
            self.items.push(c);
        }
    }

    /// Drain the highest-priority candidate of the given role out of
    /// the pool. Ties prefer the candidate that was inserted first —
    /// keeps emission order stable across reruns of the same situation.
    fn take_top_of(&mut self, role: EnvRole) -> Option<EnvCandidate> {
        let idx = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, c)| c.role == role)
            .max_by_key(|(i, c)| (c.priority, Reverse(*i)))
            .map(|(i, _)| i)?;
        Some(self.items.remove(idx))
    }

    /// Consume the pool, returning the single highest-priority
    /// candidate across all roles. Used by the weekly story tick where
    /// the cap is "one event per week", not split by role.
    fn into_top(self) -> Option<EnvCandidate> {
        self.items.into_iter().max_by_key(|c| c.priority)
    }
}

// ── Candidate builders: first-tick (post-shock) ─────────────────
//
// Methods on `TransferEnvironmentProfile`. Each one inspects a single
// gate, builds a fully-decorated `HappinessEventContext`, and returns
// `Some(EnvCandidate)` when the emission should happen. The orchestrator
// in `Player::apply_first_tick_environment_events` owns the &mut self
// emission via `emit_candidate`.

impl TransferEnvironmentProfile {
    fn top_club_opportunity_candidate(&self) -> Option<EnvCandidate> {
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.top_club_opportunity;
        let mag = base
            * Player::age_amplifier_for_top_club(self.age)
            * Player::ambition_amplifier_for_aspirational(self.ambition);
        let ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationAdmiration,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_evidence(HappinessEventEvidence::JoinedEliteClub)
        .with_evidence(HappinessEventEvidence::ReputationGap)
        .with_follow_up(HappinessEventFollowUp::PressureBuilding);
        Some(EnvCandidate {
            event_type: HappinessEventType::TopClubOpportunity,
            magnitude: mag,
            context: ctx,
            // Primary: the headline of the weak-to-elite narrative.
            role: EnvRole::Primary,
            priority: 90,
            cooldown_days: 120,
        })
    }

    fn elite_training_lift_candidate(&self) -> Option<EnvCandidate> {
        if (self.adaptability + self.professionalism) < 24.0 {
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.elite_training_lift;
        let mag = base * Player::ambition_amplifier_for_aspirational(self.ambition);
        let ctx = HappinessEventContext::new(
            HappinessEventCause::TrainingPartnership,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::TrainingGround,
        )
        .with_evidence(HappinessEventEvidence::JoinedEliteClub)
        .with_evidence(HappinessEventEvidence::HighProfessionalism)
        .with_follow_up(HappinessEventFollowUp::TrendImproving);
        Some(EnvCandidate {
            event_type: HappinessEventType::EliteTrainingLift,
            magnitude: mag,
            context: ctx,
            // Flavor: positive supplement to the headline.
            role: EnvRole::Flavor,
            priority: 55,
            cooldown_days: 60,
        })
    }

    fn overawed_by_elite_club_candidate(&self) -> Option<EnvCandidate> {
        let depth_blocked = self
            .dest_position_depth_rank
            .map(|r| r >= 4)
            .unwrap_or(false);
        if self.pressure > 8.0 && !depth_blocked {
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.overawed_by_elite_club;
        let mag = base
            * Player::age_amplifier_for_top_club(self.age)
            * Player::professionalism_dampener_for_negatives(self.professionalism);
        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationTension,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::DressingRoom,
        )
        .with_evidence(HappinessEventEvidence::JoinedEliteClub)
        .with_evidence(HappinessEventEvidence::BelowSquadStandard)
        .with_follow_up(HappinessEventFollowUp::SettlingInProgress);
        if self.pressure <= 8.0 {
            ctx = ctx.with_evidence(HappinessEventEvidence::LowPressurePersonality);
        }
        Some(EnvCandidate {
            event_type: HappinessEventType::OverawedByEliteClub,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Flavor,
            priority: 50,
            cooldown_days: 30,
        })
    }

    fn role_path_blocked_candidate(&self) -> Option<EnvCandidate> {
        let depth_blocked = self
            .dest_position_depth_rank
            .map(|r| r >= 4)
            .unwrap_or(false);
        if !depth_blocked {
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.role_path_blocked_at_elite_club;
        let mag = base
            * Player::age_amplifier_for_top_club(self.age)
            * Player::professionalism_dampener_for_negatives(self.professionalism);
        let ctx = HappinessEventContext::new(
            HappinessEventCause::PositionalRivalry,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::MatchDay,
        )
        .with_evidence(HappinessEventEvidence::BlockedByDepth)
        .with_evidence(HappinessEventEvidence::SamePositionCompetition)
        .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
        Some(EnvCandidate {
            event_type: HappinessEventType::RolePathBlockedAtEliteClub,
            magnitude: mag,
            context: ctx,
            // Higher Flavor priority than OverawedByEliteClub because
            // depth blocking is a concrete role signal, not a vibe.
            role: EnvRole::Flavor,
            priority: 65,
            cooldown_days: 30,
        })
    }

    /// `DressingRoomStatusShock` — previously-important player arrives
    /// at a much stronger club and is no longer the dressing-room star.
    /// Gates: source club was meaningfully below the destination
    /// (≥ 1500 rep gap), AND poor depth rank (≥ 3) OR clearly below
    /// the destination tier.
    fn dressing_room_status_shock_candidate(&self) -> Option<EnvCandidate> {
        // Player was a "name" at the source club only if source rep was
        // meaningfully below the destination — the bigger the gap, the
        // bigger the status drop.
        let upward_jump = (self.dest_club_rep as i32) - (self.source_club_rep as i32) >= 1500;
        if !upward_jump {
            return None;
        }
        let depth_blocked = self
            .dest_position_depth_rank
            .map(|r| r >= 3)
            .unwrap_or(false);
        let below_standard = self.ability_vs_dest_tier() <= -25;
        if !(depth_blocked || below_standard) {
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.dressing_room_status_shock;
        let mag = base * Player::professionalism_dampener_for_negatives(self.professionalism);
        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationTension,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::DressingRoom,
        )
        .with_evidence(HappinessEventEvidence::BelowSquadStandard)
        .with_follow_up(HappinessEventFollowUp::SettlingInProgress);
        if depth_blocked {
            ctx = ctx.with_evidence(HappinessEventEvidence::BlockedByDepth);
        }
        Some(EnvCandidate {
            event_type: HappinessEventType::DressingRoomStatusShock,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Flavor,
            priority: 60,
            cooldown_days: 45,
        })
    }

    /// `SeniorMentorSupport` — positive flavour for a struggling
    /// newcomer who has good social anchors: same-language teammates,
    /// compatriots, or a high-professionalism profile. Lifts only when
    /// the player needs the support (weak-at-elite or star-at-weak).
    fn senior_mentor_support_candidate(&self, s: &SocialSignals) -> Option<EnvCandidate> {
        if !s.has_support_anchor() && self.professionalism < 12.0 {
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.senior_mentor_support;
        let mut mul = 1.0;
        if s.same_nationality_teammates >= 1 {
            mul *= 1.15;
        }
        if s.same_language_teammates >= 2 {
            mul *= 1.10;
        }
        let mag = base * mul;
        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::DressingRoomLift,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::DressingRoom,
        )
        .with_follow_up(HappinessEventFollowUp::LikelyToSettle);
        if s.same_nationality_teammates >= 1 {
            ctx = ctx.with_evidence(HappinessEventEvidence::SharedNationality);
        }
        if s.same_language_teammates >= 1 {
            ctx = ctx.with_evidence(HappinessEventEvidence::NewSigningStillSettling);
        }
        if self.professionalism >= 14.0 {
            ctx = ctx.with_evidence(HappinessEventEvidence::HighProfessionalism);
        }
        Some(EnvCandidate {
            event_type: HappinessEventType::SeniorMentorSupport,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Flavor,
            // Lifted above the negative flavor (Overawed/Status-shock)
            // so a well-supported newcomer's first-tick story leads with
            // the positive framing rather than the pressure framing.
            priority: 70,
            cooldown_days: 60,
        })
    }

    fn too_good_for_level_candidate(&self) -> Option<EnvCandidate> {
        if self.destination_is_favorite && self.ambition < 16.0 {
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.too_good_for_level;
        let mut mul = if self.ambition >= 15.0 { 1.30 } else { 1.0 };
        if self.loyalty >= 15.0 && self.destination_is_favorite {
            mul *= 0.55;
        }
        let mag = base * mul;
        let ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationTension,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::MatchDay,
        )
        .with_evidence(HappinessEventEvidence::AboveSquadStandard)
        .with_evidence(HappinessEventEvidence::ReputationGap)
        .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
        Some(EnvCandidate {
            event_type: HappinessEventType::TooGoodForLevel,
            magnitude: mag,
            context: ctx,
            // Primary: headline of the star-to-weak narrative. Outranks
            // StepDownEmbarrassment because "too good for the level" is
            // the forward-looking framing; embarrassment is about
            // reputation, not playing fit.
            role: EnvRole::Primary,
            priority: 85,
            cooldown_days: 45,
        })
    }

    fn step_down_embarrassment_candidate(&self) -> Option<EnvCandidate> {
        let source_advantage = (self.source_club_rep as i32) - (self.dest_club_rep as i32) >= 2000
            || (self.source_league_rep as i32) - (self.dest_league_rep as i32) >= 1500;
        if !source_advantage {
            return None;
        }
        if self.destination_is_favorite && self.loyalty >= 15.0 {
            // Loyal favourite-club return suppresses the embarrassment
            // framing — surfaced as `DreamMove` elsewhere.
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.step_down_embarrassment;
        let mut mul = 1.0;
        if self.age >= 33 {
            mul *= 0.75;
        }
        if self.ambition >= 15.0 {
            mul *= 1.30;
        }
        let mag = base * mul;
        let ctx = HappinessEventContext::new(
            HappinessEventCause::MediaPressure,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Media,
        )
        .with_evidence(HappinessEventEvidence::AboveSquadStandard)
        .with_evidence(HappinessEventEvidence::MediaIncident)
        .with_follow_up(HappinessEventFollowUp::PressureBuilding);
        Some(EnvCandidate {
            event_type: HappinessEventType::StepDownEmbarrassment,
            magnitude: mag,
            context: ctx,
            // Primary: alternative headline for the star-to-weak
            // narrative. Lower priority than TooGoodForLevel — see
            // comment there.
            role: EnvRole::Primary,
            priority: 80,
            cooldown_days: 60,
        })
    }

    fn training_standard_frustration_candidate(&self) -> Option<EnvCandidate> {
        let setup_gap = (self.source_league_rep as i32) - (self.dest_league_rep as i32) >= 2500;
        if !setup_gap {
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.training_standard_frustration;
        let mul = if self.professionalism >= 15.0 {
            1.20
        } else {
            1.0
        };
        let mag = base * mul;
        let ctx = HappinessEventContext::new(
            HappinessEventCause::TrainingFriction,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::TrainingGround,
        )
        .with_evidence(HappinessEventEvidence::TrainingLevelGap)
        .with_evidence(HappinessEventEvidence::TrainingStandardsMismatch)
        .with_follow_up(HappinessEventFollowUp::PressureBuilding);
        Some(EnvCandidate {
            event_type: HappinessEventType::TrainingStandardFrustration,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Flavor,
            priority: 45,
            cooldown_days: 45,
        })
    }

    /// First-tick variant of `FanExpectationBurden`: no form data
    /// exists yet, so this gate fires purely on rep + fee + pressure
    /// personality. The weekly variant adds a form-sample requirement.
    fn fan_expectation_burden_first_tick_candidate(&self) -> Option<EnvCandidate> {
        if self.is_loan {
            return None;
        }
        if (self.player_current_rep as i32) < 6000 {
            return None;
        }
        let high_fee = self.fee >= 10_000_000.0;
        let high_rep_gap = self.player_world_rep as i32 - self.dest_club_rep as i32 >= 2000;
        if !(high_fee || high_rep_gap) {
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.fan_expectation_burden;
        let mul = if self.pressure <= 8.0 { 1.30 } else { 1.0 };
        let mag = base * mul;
        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::MediaPressure,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Media,
        )
        .with_evidence(HappinessEventEvidence::HighFeePressure)
        .with_follow_up(HappinessEventFollowUp::PressureBuilding);
        if self.pressure <= 8.0 {
            ctx = ctx.with_evidence(HappinessEventEvidence::LowPressurePersonality);
        }
        Some(EnvCandidate {
            event_type: HappinessEventType::FanExpectationBurden,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Flavor,
            priority: 40,
            cooldown_days: 45,
        })
    }

    fn media_spotlight_pressure_candidate(&self) -> Option<EnvCandidate> {
        if self.pressure_score() < 0.65 {
            return None;
        }
        let big_step_up = self.step_up_score() >= 0.30;
        let low_pressure = self.pressure <= 10.0;
        if !(big_step_up || low_pressure) {
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.media_spotlight_pressure;
        let mag = base * Player::professionalism_dampener_for_negatives(self.professionalism);
        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::MediaPressure,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Media,
        )
        .with_evidence(HappinessEventEvidence::MediaIncident)
        .with_follow_up(HappinessEventFollowUp::PressureBuilding);
        if low_pressure {
            ctx = ctx.with_evidence(HappinessEventEvidence::LowPressurePersonality);
        }
        Some(EnvCandidate {
            event_type: HappinessEventType::MediaSpotlightPressure,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Universal,
            priority: 30,
            cooldown_days: 30,
        })
    }

    fn loan_level_mismatch_candidate(&self) -> Option<EnvCandidate> {
        let down_mismatch = (self.player_world_rep as i32) - (self.dest_club_rep as i32) >= 2500;
        let up_mismatch = (self.dest_club_rep as i32) - (self.source_club_rep as i32) >= 2500;
        if !(down_mismatch || up_mismatch) {
            return None;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.loan_level_mismatch;
        let ctx = HappinessEventContext::new(
            HappinessEventCause::TacticalDisagreement,
            HappinessEventSeverity::from_magnitude(base),
            HappinessEventScope::MatchDay,
        )
        .with_evidence(HappinessEventEvidence::ReputationGap)
        .with_follow_up(HappinessEventFollowUp::SettlingInProgress);
        Some(EnvCandidate {
            event_type: HappinessEventType::LoanLevelMismatch,
            magnitude: base,
            context: ctx,
            role: EnvRole::Universal,
            priority: 20,
            cooldown_days: 60,
        })
    }
}

/// Per-tick weekly signals consumed by the weekly story candidates.
/// Built once at the top of [`Player::process_transfer_environment_story`]
/// so each candidate sees a consistent snapshot of form, adaptation,
/// and roll values.
struct WeeklyEnvSignals {
    days_since: i64,
    adaptation_score: f32,
    apps: f32,
    starts: f32,
    avg_rating: f32,
    league_reputation: u16,
    professionalism: f32,
    ambition: f32,
    pressure: f32,
    current_reputation: i16,
    had_recent_isolation: bool,
    roll: f32,
}

impl WeeklyEnvSignals {
    /// `AdaptationBreakthrough` — adaptation_score climbed to ≥ 65
    /// after a documented "hard start" (recent FeelingIsolated).
    fn adaptation_breakthrough_candidate(&self) -> Option<EnvCandidate> {
        if !(self.adaptation_score >= 65.0 && self.had_recent_isolation && self.roll < 0.50) {
            return None;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.adaptation_breakthrough;
        let ctx = HappinessEventContext::new(
            HappinessEventCause::AdaptationIsolation,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Personal,
        )
        .with_evidence(HappinessEventEvidence::NewSigningStillSettling)
        .with_follow_up(HappinessEventFollowUp::LikelyToSettle);
        Some(EnvCandidate {
            event_type: HappinessEventType::AdaptationBreakthrough,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Primary,
            priority: 90,
            cooldown_days: 60,
        })
    }

    /// `TrustedAfterStepUp` — manager trust: starter ratio ≥ 0.55 over
    /// 5+ apps + adaptation score ≥ 55.
    fn trusted_after_step_up_candidate(&self) -> Option<EnvCandidate> {
        if !(self.apps >= 5.0
            && self.starts / self.apps.max(1.0) >= 0.55
            && self.adaptation_score >= 55.0
            && self.roll < 0.55)
        {
            return None;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.trusted_after_step_up
            * Player::ambition_amplifier_for_aspirational(self.ambition);
        let ctx = HappinessEventContext::new(
            HappinessEventCause::ManagerSupport,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::MatchDay,
        )
        .with_evidence(HappinessEventEvidence::ManagerTrust)
        .with_follow_up(HappinessEventFollowUp::ManagerTrustRising);
        Some(EnvCandidate {
            event_type: HappinessEventType::TrustedAfterStepUp,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Primary,
            priority: 85,
            cooldown_days: 60,
        })
    }

    /// `ProvedLevelAfterMove` — strong form (avg ≥ 7.0) over a
    /// meaningful sample (≥ 6 apps).
    fn proved_level_after_move_candidate(&self) -> Option<EnvCandidate> {
        if !(self.apps >= 6.0 && self.avg_rating >= 7.0 && self.roll < 0.45) {
            return None;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.proved_level_after_move;
        let ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationAdmiration,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::MatchDay,
        )
        .with_evidence(HappinessEventEvidence::ExcellentPerformance)
        .with_follow_up(HappinessEventFollowUp::TrendImproving);
        Some(EnvCandidate {
            event_type: HappinessEventType::ProvedLevelAfterMove,
            magnitude: mag,
            context: ctx,
            // Highest of all weekly candidates — positive recovery
            // outranks every negative pressure event.
            role: EnvRole::Primary,
            priority: 95,
            cooldown_days: 60,
        })
    }

    /// `OverawedByEliteClub` (weekly) — adaptation_score < 45 in the
    /// first 60 days. Negative, so priority sits below the positive
    /// recovery candidates above.
    fn overawed_by_elite_club_candidate(&self) -> Option<EnvCandidate> {
        if !(self.days_since <= 60 && self.adaptation_score < 45.0 && self.roll < 0.40) {
            return None;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.overawed_by_elite_club
            * Player::professionalism_dampener_for_negatives(self.professionalism);
        let ctx = HappinessEventContext::new(
            HappinessEventCause::AdaptationIsolation,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::DressingRoom,
        )
        .with_evidence(HappinessEventEvidence::JoinedEliteClub)
        .with_evidence(HappinessEventEvidence::BelowSquadStandard)
        .with_follow_up(HappinessEventFollowUp::SettlingInProgress);
        Some(EnvCandidate {
            event_type: HappinessEventType::OverawedByEliteClub,
            magnitude: mag,
            context: ctx,
            // Below the positive recovery trio (90/85/95) so a player
            // who actually proves the move worked surfaces THAT before
            // another round of "still struggling".
            role: EnvRole::Primary,
            priority: 45,
            cooldown_days: 21,
        })
    }

    /// `TooGoodForLevel` (weekly). Two-mode gating:
    ///   - With 0–2 apps: fire on role-frustration framing (player
    ///     hasn't even been picked, which IS frustrating).
    ///   - With 3+ apps: require poor form (avg rating < 7.0).
    fn too_good_for_level_candidate(&self) -> Option<EnvCandidate> {
        if !(self.ambition >= 13.0 && self.roll < 0.35) {
            return None;
        }
        let no_minutes = self.apps < 3.0;
        if !no_minutes && self.avg_rating >= 7.0 {
            // Player has a proper sample and is performing — no event.
            return None;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.too_good_for_level;
        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationTension,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::MatchDay,
        )
        .with_evidence(HappinessEventEvidence::AboveSquadStandard)
        .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
        if no_minutes {
            ctx = ctx.with_evidence(HappinessEventEvidence::BlockedByDepth);
        } else {
            ctx = ctx.with_evidence(HappinessEventEvidence::ReputationGap);
        }
        Some(EnvCandidate {
            event_type: HappinessEventType::TooGoodForLevel,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Primary,
            priority: 50,
            cooldown_days: 30,
        })
    }

    /// `TrainingStandardFrustration` (weekly) — current league sits
    /// well below the expected tier for the player.
    fn training_standard_frustration_candidate(&self) -> Option<EnvCandidate> {
        if !((self.league_reputation as i32) < 4500 && self.roll < 0.30) {
            return None;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.training_standard_frustration;
        let ctx = HappinessEventContext::new(
            HappinessEventCause::TrainingFriction,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::TrainingGround,
        )
        .with_evidence(HappinessEventEvidence::TrainingLevelGap)
        .with_evidence(HappinessEventEvidence::TrainingStandardsMismatch)
        .with_follow_up(HappinessEventFollowUp::PressureBuilding);
        Some(EnvCandidate {
            event_type: HappinessEventType::TrainingStandardFrustration,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Flavor,
            priority: 35,
            cooldown_days: 30,
        })
    }

    /// `FanExpectationBurden` (weekly). Requires a real form sample
    /// before treating "no form" as "poor form" — without 3+ apps the
    /// fans haven't seen enough to react.
    fn fan_expectation_burden_candidate(&self) -> Option<EnvCandidate> {
        if !(self.current_reputation >= 6000
            && self.pressure <= 8.0
            && self.apps >= 3.0
            && self.avg_rating < 7.0
            && self.roll < 0.30)
        {
            return None;
        }
        let cfg = HappinessConfig::default();
        let mag = cfg.catalog.fan_expectation_burden;
        let ctx = HappinessEventContext::new(
            HappinessEventCause::MediaPressure,
            HappinessEventSeverity::from_magnitude(mag),
            HappinessEventScope::Media,
        )
        .with_evidence(HappinessEventEvidence::HighFeePressure)
        .with_evidence(HappinessEventEvidence::LowPressurePersonality)
        .with_follow_up(HappinessEventFollowUp::PressureBuilding);
        Some(EnvCandidate {
            event_type: HappinessEventType::FanExpectationBurden,
            magnitude: mag,
            context: ctx,
            role: EnvRole::Flavor,
            priority: 30,
            cooldown_days: 30,
        })
    }
}

// ============================================================
// Weekly transfer-environment story cadence (Phase 6)
// ============================================================

impl Player {
    /// Weekly tick — emits ongoing weak↔elite / star↔weak narrative
    /// events during the integration window (168 days post-transfer).
    /// Uses cooldowns + a deterministic per-player/per-week roll to
    /// limit spam (max 1 environment-story event per week).
    ///
    /// No-op when the player has no recent transfer or sits outside
    /// the integration window. `process_transfer_shock` already
    /// consumed `pending_signing` on day 0 — by the time this fires,
    /// the env info has to come from current ctx (dest reps) + the
    /// player's own attributes. Source-side reps are unavailable here
    /// (they lived on `PendingSigning`); the weekly tick infers the
    /// situation from the current player-vs-dest gap instead, which is
    /// what we want anyway for ongoing fit signals.
    pub fn process_transfer_environment_story(
        &mut self,
        now: NaiveDate,
        country_code: &str,
        club_rep_0_to_1: f32,
        league_reputation: u16,
        formation: Option<&[PlayerPositionType; 11]>,
    ) {
        let _ = formation;
        // Active window: 168 days (~24 weeks) post-transfer.
        let days_since = match self.days_since_transfer(now) {
            Some(d) if (0..=168).contains(&d) => d,
            _ => return,
        };

        // Cap at one environment-story event per calendar week.
        if self.has_recent_environment_story_event(7) {
            return;
        }

        // Deterministic per-player/per-week roll. Mirrors
        // `isolation_roll` but uses the ISO week number so events
        // don't fire every weekly tick of the same situation.
        let roll = environment_story_roll(self.id, now);

        let dest_club_rep = (club_rep_0_to_1.clamp(0.0, 1.0) * 10000.0) as u16;
        let player_world_rep = self.player_attributes.world_reputation as i32;
        let player_vs_dest = player_world_rep - dest_club_rep as i32;

        // Real adaptation context: language buddies + chemistry from the
        // squad social view written by the team's weekly pre-tick, not a
        // defaulted neutral context. `PlayerClubContract` doesn't carry
        // a club id, so the favourite-club flag is supplied as false
        // here — the loan-cap inside `adaptation_score` is the only
        // consumer and the weekly tick fires past the initial shock.
        let social = SocialSignals::from_player(self);
        let squad_ctx = social.adaptation_squad_context(self.is_on_loan(), false);
        let adaptation_score =
            self.adaptation_score(now, country_code, club_rep_0_to_1, None, &squad_ctx);

        // Snapshot the form + adaptation signals once so every weekly
        // candidate sees consistent data.
        let signals = WeeklyEnvSignals {
            days_since,
            adaptation_score,
            apps: self.statistics.played as f32 + self.statistics.played_subs as f32,
            starts: self.statistics.played as f32,
            avg_rating: self
                .statistics
                .average_rating_realistic(self.position().position_group()),
            league_reputation,
            professionalism: self.attributes.professionalism,
            ambition: self.attributes.ambition,
            pressure: self.attributes.pressure,
            current_reputation: self.player_attributes.current_reputation,
            had_recent_isolation: self
                .happiness
                .has_recent_event(&HappinessEventType::FeelingIsolated, 60),
            roll,
        };

        // Build the candidate pool — positive recovery candidates carry
        // priorities above their negative counterparts so a player who's
        // actually proving the move worked surfaces THAT before another
        // round of "still struggling."
        let mut pool = EnvCandidatePool::new();

        if dest_club_rep >= 7500 && player_vs_dest <= -2500 {
            pool.push_some(signals.proved_level_after_move_candidate());
            pool.push_some(signals.adaptation_breakthrough_candidate());
            pool.push_some(signals.trusted_after_step_up_candidate());
            pool.push_some(signals.overawed_by_elite_club_candidate());
        }
        if player_vs_dest >= 3500 {
            pool.push_some(signals.proved_level_after_move_candidate());
            pool.push_some(signals.too_good_for_level_candidate());
            pool.push_some(signals.training_standard_frustration_candidate());
            pool.push_some(signals.fan_expectation_burden_candidate());
        }

        // Single emission per week — highest priority wins across both
        // roles. Cap is "one event per week", not split by role.
        if let Some(winner) = pool.into_top() {
            self.emit_candidate(winner);
        }
    }

    /// True when any transfer-environment story event landed in the
    /// last `days` days. Used to enforce the "max one per week" cap on
    /// the weekly cadence.
    fn has_recent_environment_story_event(&self, days: u16) -> bool {
        for e in &self.happiness.recent_events {
            if e.days_ago > days {
                continue;
            }
            if matches!(
                e.event_type,
                HappinessEventType::TopClubOpportunity
                    | HappinessEventType::EliteTrainingLift
                    | HappinessEventType::AdaptationBreakthrough
                    | HappinessEventType::TrustedAfterStepUp
                    | HappinessEventType::ProvedLevelAfterMove
                    | HappinessEventType::SeniorMentorSupport
                    | HappinessEventType::OverawedByEliteClub
                    | HappinessEventType::RolePathBlockedAtEliteClub
                    | HappinessEventType::MediaSpotlightPressure
                    | HappinessEventType::DressingRoomStatusShock
                    | HappinessEventType::TooGoodForLevel
                    | HappinessEventType::TrainingStandardFrustration
                    | HappinessEventType::FanExpectationBurden
                    | HappinessEventType::StepDownEmbarrassment
                    | HappinessEventType::LoanLevelMismatch
            ) {
                return true;
            }
        }
        false
    }
}

/// Deterministic per-player/per-week roll, in `[0, 1)`. Same player +
/// same ISO week yields the same number, so re-running the weekly tick
/// is idempotent. Distinct from `isolation_roll` (per-day) so the two
/// systems don't share a seed and lock in correlated outcomes.
fn environment_story_roll(player_id: u32, date: NaiveDate) -> f32 {
    use chrono::Datelike;
    let week = date.iso_week().week();
    let year = date.iso_week().year();
    let h = (player_id as u64)
        .wrapping_mul(0xBF58_476D_1CE4_E5B9)
        .wrapping_add(week as u64)
        .wrapping_add((year as u64).wrapping_mul(54_321));
    let frac = ((h >> 17) as u32 as f32) / (u32::MAX as f32);
    frac.clamp(0.0, 0.999)
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

    fn dream_loan_count(p: &Player) -> usize {
        p.happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::DreamLoanOpportunity)
            .count()
    }

    fn home_return_count(p: &Player) -> usize {
        p.happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::HomeReturnOpportunity)
            .count()
    }

    /// Helper that exercises the source-aware permanent emit pathway —
    /// `passes_dream_move_gates` + `is_source_aware_step_up`.
    fn try_emit_dream_move_with_source(
        p: &mut Player,
        dest_club_rep_0_to_1: f32,
        dest_league_rep: u16,
        source_club_rep: u16,
        source_league_rep: u16,
        now: NaiveDate,
    ) {
        let dest_club_abs = (dest_club_rep_0_to_1 * 10000.0) as u16;
        let step_up = Player::is_source_aware_step_up(
            dest_club_abs,
            dest_league_rep,
            source_club_rep,
            source_league_rep,
        );
        p.emit_dream_move_with_source(dest_club_rep_0_to_1, step_up, 1.0, now);
    }

    #[test]
    fn pinsoglio_to_cittadella_does_not_fire_dream_move() {
        // 36yo high-rep keeper from Juventus (world_rep ~4500) joining
        // Cittadella (rep ~3000 → 0.30 normalised). Source-aware: from
        // Juventus (club rep 9000, Serie A 8500) → Cittadella (3000, 5000)
        // is a STEP DOWN, gate fails immediately.
        let mut p = player(36, 10.0, 4500);
        let now = d(2026, 4, 26);
        try_emit_dream_move_with_source(&mut p, 0.30, 5000, 9000, 8500, now);
        assert_eq!(dream_count(&p), 0);
    }

    #[test]
    fn step_down_at_any_age_does_not_fire_dream_move() {
        // 25yo player with world_rep 6000 joining a club at rep 4000.
        // Source club bigger than destination — source-aware gate fails.
        let mut p = player(25, 12.0, 6000);
        let now = d(2026, 4, 26);
        try_emit_dream_move_with_source(&mut p, 0.40, 5000, 6500, 6000, now);
        assert_eq!(dream_count(&p), 0);
    }

    #[test]
    fn young_prospect_to_top_club_fires_dream_move() {
        // 22yo with modest world_rep 2000 at a small club (rep 1500,
        // tier-3 league rep 2500) joining a top club (rep 8500, Premier
        // League ~9500). Source-aware gap is huge on both axes; legacy
        // gates also pass.
        let mut p = player(22, 10.0, 2000);
        let now = d(2026, 4, 26);
        try_emit_dream_move_with_source(&mut p, 0.85, 9500, 1500, 2500, now);
        assert_eq!(dream_count(&p), 1);
    }

    #[test]
    fn veteran_needs_extra_margin_for_dream_move() {
        // 33yo with world_rep 5000. Club at 6000 — only 1000 above.
        // 32+ requires 2500+ gap, so this should NOT fire.
        let mut p = player(33, 12.0, 5000);
        let now = d(2026, 4, 26);
        try_emit_dream_move_with_source(&mut p, 0.60, 6000, 3500, 4000, now);
        assert_eq!(dream_count(&p), 0);

        // Same player to a clearly elite club: world_rep 5000, club 8000.
        let mut p2 = player(33, 12.0, 5000);
        try_emit_dream_move_with_source(&mut p2, 0.80, 8000, 3500, 4000, now);
        assert_eq!(dream_count(&p2), 1);
    }

    #[test]
    fn over_35_requires_elite_destination() {
        // 36yo, world_rep 3000. Club at 6000 — 3000 above the player but
        // not elite (< 7500). 35+ gate should suppress.
        let mut p = player(36, 12.0, 3000);
        let now = d(2026, 4, 26);
        try_emit_dream_move_with_source(&mut p, 0.60, 6000, 2500, 3000, now);
        assert_eq!(dream_count(&p), 0);

        // Same player to genuinely elite club (rep 8500). Fires.
        let mut p2 = player(36, 12.0, 3000);
        try_emit_dream_move_with_source(&mut p2, 0.85, 9500, 2500, 3000, now);
        assert_eq!(dream_count(&p2), 1);
    }

    // ── Source-aware gate ──────────────────────────────────────────

    #[test]
    fn source_aware_step_up_requires_meaningful_gap() {
        // Both gaps under the threshold — fails.
        assert!(!Player::is_source_aware_step_up(5500, 6000, 4500, 5500));
        // Club gap clears (2000) — passes.
        assert!(Player::is_source_aware_step_up(6500, 6000, 4500, 5500));
        // League gap clears (1500), club gap doesn't — passes (either-axis).
        assert!(Player::is_source_aware_step_up(5500, 7000, 4500, 5500));
        // Zero source data (free agent, untagged) — fails.
        assert!(!Player::is_source_aware_step_up(9500, 9500, 0, 0));
    }

    #[test]
    fn spartak_to_dynamo_kyiv_is_not_a_dream_move() {
        // Lateral RU↔UA loan-or-permanent: Spartak Moscow (club ~7000,
        // RPL ~6500) ↔ Dynamo Kyiv (club ~6500, UPL ~5500). Source
        // bigger than destination — even a young Spartak prospect on
        // ambition 14 must NOT see this as a dream move.
        let mut p = player(20, 14.0, 4500);
        let now = d(2026, 4, 26);
        try_emit_dream_move_with_source(&mut p, 0.65, 5500, 7000, 6500, now);
        assert_eq!(dream_count(&p), 0);
    }

    // ── Loan-specific behaviour ────────────────────────────────────

    #[test]
    fn ordinary_loan_does_not_fire_dream_move() {
        // 19yo prospect from a small club loaned to a mid-table side.
        // Even with high ambition, the LOAN path must never emit
        // DreamMove — that copy is reserved for permanent moves.
        let mut p = player(19, 16.0, 1500);
        let now = d(2026, 4, 26);
        // Direct dream-move path is gated by source-aware step-up,
        // but the loan branch in process_transfer_shock skips it
        // entirely — confirm via the loan helper.
        p.emit_dream_loan_opportunity(0.55, 1500.0, true, 0.7, now);
        assert_eq!(dream_count(&p), 0);
        // Non-elite destination — loan event also doesn't fire.
        assert_eq!(dream_loan_count(&p), 0);
    }

    #[test]
    fn loan_to_elite_club_fires_dream_loan_opportunity() {
        // 19yo prospect at a small club, loaned to Real Madrid (club
        // rep ~9500). Source-aware gap is large, destination is elite,
        // player is young — the dedicated loan event fires.
        let mut p = player(19, 14.0, 1500);
        let now = d(2026, 4, 26);
        let step_up = Player::is_source_aware_step_up(9500, 9500, 1500, 2500);
        p.emit_dream_loan_opportunity(0.95, 1500.0, step_up, 0.7, now);
        assert_eq!(dream_count(&p), 0);
        assert_eq!(dream_loan_count(&p), 1);
    }

    #[test]
    fn veteran_loan_does_not_fire_dream_loan_opportunity() {
        // 35+ players go out on loan for minutes, not prestige —
        // the "opportunity" framing doesn't fit.
        let mut p = player(36, 14.0, 4000);
        let now = d(2026, 4, 26);
        let step_up = Player::is_source_aware_step_up(9500, 9500, 4000, 5000);
        p.emit_dream_loan_opportunity(0.95, 4000.0, step_up, 0.7, now);
        assert_eq!(dream_loan_count(&p), 0);
    }

    // ── Favourite-club handling ───────────────────────────────────

    #[test]
    fn favourite_club_loan_never_emits_dream_move() {
        // Loan landing at a favourite club: caller in
        // `process_transfer_shock` ALWAYS routes loans through
        // `emit_dream_loan_opportunity`, never through the dream-move
        // branch. Validate by exercising both helpers and confirming
        // neither writes a DreamMove for the loan profile.
        let mut p = player(20, 14.0, 3000);
        let now = d(2026, 4, 26);
        // Source-aware step-up but loan path:
        let step_up = Player::is_source_aware_step_up(9500, 9500, 3500, 4500);
        p.emit_dream_loan_opportunity(0.95, 3000.0, step_up, 0.7, now);
        assert_eq!(dream_count(&p), 0);
    }

    #[test]
    fn favourite_club_permanent_without_step_up_emits_homecoming() {
        // Permanent move to a favourite club whose reputation is lower
        // than where the player came from. Source-aware gate fails →
        // DreamMove suppressed → sentimental HomeReturnOpportunity
        // fires instead.
        let mut p = player(28, 12.0, 6000);
        let now = d(2026, 4, 26);
        let step_up = Player::is_source_aware_step_up(5500, 5500, 8000, 8500);
        // Branch logic mirrored from process_transfer_shock:
        if step_up && p.passes_dream_move_gates(0.55, now) {
            p.emit_dream_move_with_source(0.55, step_up, 1.0, now);
        } else {
            p.emit_favourite_club_homecoming(now);
        }
        assert_eq!(dream_count(&p), 0);
        assert_eq!(home_return_count(&p), 1);
    }

    #[test]
    fn favourite_club_permanent_with_real_step_up_still_emits_dream_move() {
        // Boyhood club happens to also be a clear step-up: small-club
        // talent rejoining a now-grown favourite. DreamMove DOES fire
        // because the move ALSO passes the real gates.
        let mut p = player(22, 14.0, 2000);
        let now = d(2026, 4, 26);
        let step_up = Player::is_source_aware_step_up(9000, 9500, 2000, 2500);
        if step_up && p.passes_dream_move_gates(0.90, now) {
            p.emit_dream_move_with_source(0.90, step_up, 1.0, now);
        } else {
            p.emit_favourite_club_homecoming(now);
        }
        assert_eq!(dream_count(&p), 1);
        assert_eq!(home_return_count(&p), 0);
    }

    /// Polish guard: a favourite-club move outside the home country and
    /// outside any failed-adaptation context must still emit the
    /// homecoming event, AND its CareerDesireEventContext must carry
    /// the dedicated `FavoriteClubHomecoming` kind — not the
    /// `ReturnHomeAfterPoorAdaptation` flavour the field reused before
    /// the polish pass.
    #[test]
    fn favourite_club_homecoming_uses_dedicated_desire_kind() {
        use crate::CareerDesireKind;
        let mut p = player(25, 12.0, 5000);
        let now = d(2026, 4, 26);
        p.emit_favourite_club_homecoming(now);
        assert_eq!(home_return_count(&p), 1);
        let ev = p
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::HomeReturnOpportunity)
            .expect("event present");
        let kind = ev
            .context
            .as_ref()
            .and_then(|c| c.career_desire_context.as_ref())
            .map(|cd| cd.kind);
        assert_eq!(
            kind,
            Some(CareerDesireKind::FavoriteClubHomecoming),
            "favourite-club move must not borrow the poor-adaptation flavour"
        );
    }
}

// ============================================================
// Transfer-environment realism tests
// ============================================================

#[cfg(test)]
mod transfer_environment_tests {
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

    fn person(
        ambition: f32,
        pressure: f32,
        professionalism: f32,
        loyalty: f32,
    ) -> PersonAttributes {
        PersonAttributes {
            adaptability: 10.0,
            ambition,
            controversy: 10.0,
            loyalty,
            pressure,
            professionalism,
            sportsmanship: 10.0,
            temperament: 10.0,
            consistency: 10.0,
            important_matches: 10.0,
            dirtiness: 10.0,
        }
    }

    fn player_with(
        age: u8,
        ambition: f32,
        world_rep: i16,
        current_rep: i16,
        ca: u8,
        pressure: f32,
        professionalism: f32,
        loyalty: f32,
    ) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.world_reputation = world_rep;
        attrs.current_reputation = current_rep;
        attrs.current_ability = ca;
        attrs.potential_ability = ca;
        let today = d(2026, 4, 26);
        let birth = today
            .checked_sub_signed(chrono::Duration::days(age as i64 * 365))
            .unwrap();
        PlayerBuilder::new()
            .id(7)
            .full_name(FullName::new("X".into(), "Y".into()))
            .birth_date(birth)
            .country_id(1)
            .attributes(person(ambition, pressure, professionalism, loyalty))
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    fn pending(
        dest_id: u32,
        fee: f64,
        is_loan: bool,
        src_club: u16,
        src_league: u16,
        depth_rank: Option<u8>,
    ) -> PendingSigning {
        PendingSigning {
            previous_salary: Some(50_000),
            fee,
            is_loan,
            destination_club_id: dest_id,
            had_return_home_desire: false,
            had_european_desire: false,
            had_libertadores_desire: false,
            source_club_reputation: src_club,
            source_league_reputation: src_league,
            dest_position_depth_rank: depth_rank,
        }
    }

    fn count(p: &Player, t: HappinessEventType) -> usize {
        p.happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == t)
            .count()
    }

    // ── Stage-helper smoke ───────────────────────────────────────

    #[test]
    fn stage_manual_pending_signing_records_carry_flags_and_metadata() {
        let mut p = player_with(24, 14.0, 4000, 4000, 130, 12.0, 14.0, 10.0);
        // Plant a homesickness mood so the snapshot picks it up.
        p.happiness
            .add_event(HappinessEventType::WantsReturnHome, -5.0);
        p.stage_manual_pending_signing(42, 7_500_000.0, false, 2500, 3000, Some(2));
        let staged = p.pending_signing.as_ref().expect("staged");
        assert_eq!(staged.destination_club_id, 42);
        assert_eq!(staged.fee, 7_500_000.0);
        assert!(!staged.is_loan);
        assert!(staged.had_return_home_desire);
        assert_eq!(staged.source_club_reputation, 2500);
        assert_eq!(staged.source_league_reputation, 3000);
        assert_eq!(staged.dest_position_depth_rank, Some(2));
        // Test player has no contract installed; the helper reads
        // `self.contract.as_ref().map(|c| c.salary)`, so previous_salary
        // is None. The integration path in actions/mod.rs holds the
        // source-club contract at this point and captures the real wage.
        assert_eq!(staged.previous_salary, None);
    }

    #[test]
    fn stage_manual_pending_signing_loan_marks_loan_flag() {
        let mut p = player_with(22, 12.0, 2500, 2500, 110, 10.0, 12.0, 10.0);
        p.stage_manual_pending_signing(99, 0.0, true, 6000, 7000, Some(3));
        let staged = p.pending_signing.as_ref().expect("staged");
        assert!(staged.is_loan);
        assert_eq!(staged.destination_club_id, 99);
    }

    // ── Weak player → elite club ─────────────────────────────────

    #[test]
    fn weak_player_at_elite_club_fires_top_club_opportunity() {
        // CA 80, world rep 1000, dest club rep 8500, league rep 9000.
        let mut p = player_with(24, 14.0, 1000, 1000, 80, 12.0, 14.0, 10.0);
        let pend = pending(100, 0.0, false, 2000, 2500, Some(2));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.85, 9000);
        assert!(profile.is_weak_player_at_elite_club());
        p.apply_first_tick_environment_events(d(2026, 4, 26), &profile);
        assert!(count(&p, HappinessEventType::TopClubOpportunity) >= 1);
    }

    #[test]
    fn weak_player_with_low_pressure_and_depth_block_fires_primary_plus_one_flavor() {
        // CA 80, world rep 1000, dest 8500 / league 9000, pressure 5,
        // depth rank 5. The cap allows 1 Primary + 1 Flavor — verify
        // the Primary is TopClubOpportunity and exactly one Flavor lands
        // (RolePathBlockedAtEliteClub wins by priority 65 vs Overawed's
        // 50). Professionalism set to 11.0 so SeniorMentorSupport's
        // `prof < 12.0 || has_anchor` gate fails (no squad_social_view
        // populated on the test player → no social anchor).
        let mut p = player_with(24, 12.0, 1000, 1000, 80, 5.0, 11.0, 10.0);
        let pend = pending(100, 0.0, false, 2000, 2500, Some(5));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.85, 9000);
        p.apply_first_tick_environment_events(d(2026, 4, 26), &profile);
        assert!(count(&p, HappinessEventType::TopClubOpportunity) >= 1);
        // Flavor cap: RolePathBlocked wins (priority 65) — Overawed
        // doesn't get a slot because we cap at 1 Flavor.
        assert!(count(&p, HappinessEventType::RolePathBlockedAtEliteClub) >= 1);
        assert_eq!(count(&p, HappinessEventType::OverawedByEliteClub), 0);
    }

    // ── Star → weak club ────────────────────────────────────────

    #[test]
    fn star_at_weak_club_fires_too_good_under_primary_cap() {
        // CA 170, world rep 8500, dest 2500 / league 3000. Source 9000 / 9500.
        // Both TooGoodForLevel and StepDownEmbarrassment are Primary
        // candidates; the cap allows only one. TooGoodForLevel wins by
        // priority (85 > 80).
        let mut p = player_with(28, 15.0, 8500, 8500, 170, 12.0, 14.0, 10.0);
        let pend = pending(100, 0.0, false, 9000, 9500, Some(1));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.25, 3000);
        assert!(profile.is_star_at_weak_club());
        p.apply_first_tick_environment_events(d(2026, 4, 26), &profile);
        assert_eq!(count(&p, HappinessEventType::TooGoodForLevel), 1);
        // StepDownEmbarrassment lost the Primary cap — won't fire at
        // first tick. (It can still surface later via direct emit sites
        // or the weekly cadence if the situation persists.)
        assert_eq!(count(&p, HappinessEventType::StepDownEmbarrassment), 0);
    }

    #[test]
    fn favorite_club_with_high_loyalty_suppresses_embarrassment() {
        // Same star-to-weak shape, but destination is a favourite and
        // loyalty is high — the embarrassment framing should be muted
        // and the TooGoodForLevel suppressed unless ambition is very high.
        let mut p = player_with(28, 13.0, 8500, 8500, 170, 12.0, 14.0, 16.0);
        p.favorite_clubs.push(100);
        let pend = pending(100, 0.0, false, 9000, 9500, Some(1));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.25, 3000);
        p.apply_first_tick_environment_events(d(2026, 4, 26), &profile);
        // With ambition 13 (< 16) and favourite-club destination, the
        // TooGoodForLevel gate is short-circuited.
        assert_eq!(count(&p, HappinessEventType::TooGoodForLevel), 0);
        // And loyalty 16 + favourite suppresses the embarrassment too.
        assert_eq!(count(&p, HappinessEventType::StepDownEmbarrassment), 0);
    }

    // ── Weekly cadence cooldown ─────────────────────────────────

    #[test]
    fn weekly_environment_story_caps_at_one_per_week() {
        // Player with a recent transfer has multiple env signals active.
        let mut p = player_with(24, 14.0, 1500, 1500, 85, 7.0, 12.0, 10.0);
        let now = d(2026, 4, 26);
        // Pretend the transfer landed 14 days ago.
        p.last_transfer_date = Some(now - chrono::Duration::days(14));
        // Drop one env event manually so the "max one per week" cap
        // triggers on the next tick.
        p.happiness
            .add_event(HappinessEventType::OverawedByEliteClub, -3.0);
        let before = p.happiness.recent_events.len();
        // Weekly call should now no-op for env-story events because
        // OverawedByEliteClub landed within 7 days.
        p.process_transfer_environment_story(now, "", 0.85, 9000, None);
        let after = p.happiness.recent_events.len();
        assert_eq!(
            after, before,
            "weekly cap should have prevented a second emission"
        );
    }

    // ── Profile derived coefficients ────────────────────────────

    #[test]
    fn profile_step_up_score_positive_for_weak_to_elite() {
        let p = player_with(24, 14.0, 1000, 1000, 80, 12.0, 14.0, 10.0);
        let pend = pending(100, 0.0, false, 2000, 2500, Some(2));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.85, 9000);
        assert!(profile.step_up_score() > 0.0);
    }

    #[test]
    fn profile_step_up_score_negative_for_star_to_weak() {
        let p = player_with(28, 15.0, 8500, 8500, 170, 12.0, 14.0, 10.0);
        let pend = pending(100, 0.0, false, 9000, 9500, Some(1));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.25, 3000);
        assert!(profile.step_up_score() < 0.0);
    }

    #[test]
    fn profile_pressure_score_high_for_elite_destination() {
        let p = player_with(26, 14.0, 6000, 6000, 150, 12.0, 14.0, 10.0);
        let pend = pending(100, 5_000_000.0, false, 5000, 6000, Some(1));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.95, 9500);
        assert!(profile.pressure_score() >= 0.7);
    }

    // ── First-tick cap ─────────────────────────────────────────

    fn count_env_first_tick_events(p: &Player) -> usize {
        p.happiness
            .recent_events
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type,
                    HappinessEventType::TopClubOpportunity
                        | HappinessEventType::EliteTrainingLift
                        | HappinessEventType::OverawedByEliteClub
                        | HappinessEventType::RolePathBlockedAtEliteClub
                        | HappinessEventType::DressingRoomStatusShock
                        | HappinessEventType::SeniorMentorSupport
                        | HappinessEventType::TooGoodForLevel
                        | HappinessEventType::StepDownEmbarrassment
                        | HappinessEventType::TrainingStandardFrustration
                        | HappinessEventType::FanExpectationBurden
                )
            })
            .count()
    }

    #[test]
    fn first_tick_caps_narrative_events_to_one_primary_plus_one_flavor() {
        // Weak-to-elite scenario with low pressure + blocked depth →
        // every weak-at-elite gate passes. Without the cap this would
        // spam 4-5 events; with the cap it's at most 2 narrative
        // events (Primary + Flavor) plus the orthogonal universals.
        let mut p = player_with(24, 14.0, 1000, 1000, 80, 5.0, 12.0, 10.0);
        let pend = pending(100, 0.0, false, 2000, 2500, Some(5));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.85, 9000);
        p.apply_first_tick_environment_events(d(2026, 4, 26), &profile);
        // At most: 1 Primary (TopClubOpportunity) + 1 Flavor (one of
        // RolePathBlocked / Overawed / SeniorMentor / EliteTrainingLift /
        // DressingRoomStatusShock).
        assert!(
            count_env_first_tick_events(&p) <= 2,
            "first-tick environment events should be capped at 2 (got {})",
            count_env_first_tick_events(&p),
        );
        // Primary should be TopClubOpportunity (highest-priority gate).
        assert_eq!(count(&p, HappinessEventType::TopClubOpportunity), 1);
    }

    #[test]
    fn first_tick_picks_role_path_blocked_as_top_flavor_when_depth_blocks() {
        // Depth rank 5 → RolePathBlockedAtEliteClub (priority 65) beats
        // OverawedByEliteClub (50) and SeniorMentorSupport (70) when
        // there is no support anchor + low professionalism.
        let mut p = player_with(24, 14.0, 1000, 1000, 80, 5.0, 10.0, 10.0);
        // No social anchors — squad_social_view is None → mentor-support
        // gate fails (low professionalism + no anchor).
        let pend = pending(100, 0.0, false, 2000, 2500, Some(5));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.85, 9000);
        p.apply_first_tick_environment_events(d(2026, 4, 26), &profile);
        assert_eq!(count(&p, HappinessEventType::RolePathBlockedAtEliteClub), 1);
    }

    #[test]
    fn senior_mentor_support_fires_when_player_has_compatriot_anchor() {
        // Weak-at-elite player WITH same-nationality teammates ≥ 1 →
        // SeniorMentorSupport beats RolePathBlocked / Overawed in the
        // Flavor slot (priority 70 vs 65/50).
        let mut p = player_with(24, 14.0, 1000, 1000, 80, 5.0, 10.0, 10.0);
        p.squad_social_view = Some(crate::club::player::core::player::SquadSocialView {
            same_nationality_teammates: 2,
            same_language_teammates: 2,
        });
        let pend = pending(100, 0.0, false, 2000, 2500, Some(5));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.85, 9000);
        p.apply_first_tick_environment_events(d(2026, 4, 26), &profile);
        assert_eq!(count(&p, HappinessEventType::SeniorMentorSupport), 1);
        // RolePathBlocked got out-ranked.
        assert_eq!(count(&p, HappinessEventType::RolePathBlockedAtEliteClub), 0);
    }

    #[test]
    fn dressing_room_status_shock_fires_on_upward_jump_with_depth_block() {
        // Modest player (source rep 2500) jumps to elite club (dest rep
        // 8500) with depth rank 3 → status shock candidate passes.
        let mut p = player_with(28, 12.0, 2500, 2500, 100, 12.0, 12.0, 10.0);
        let pend = pending(100, 0.0, false, 2500, 3000, Some(3));
        // Profile gate `is_weak_player_at_elite_club` needs player_ca
        // ≤ expected_ca - 15. Expected for league 9000 = 150. Player CA
        // 100 → ability_below = true. Confirmed.
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.85, 9000);
        assert!(profile.is_weak_player_at_elite_club());
        p.apply_first_tick_environment_events(d(2026, 4, 26), &profile);
        // DressingRoomStatusShock may or may not be picked (other
        // Flavor candidates compete) — assert at least its gate logic
        // surfaces a candidate by counting directly via the method.
        assert!(profile.dressing_room_status_shock_candidate().is_some());
    }

    // ── Loan-tier mismatch (universal) ──────────────────────────

    #[test]
    fn loan_level_mismatch_fires_for_extreme_tier_jump() {
        // Player rep 4000 loaned to dest 8500 → up_mismatch passes.
        // Universal — fires alongside the cap.
        let mut p = player_with(22, 13.0, 4000, 4000, 130, 12.0, 12.0, 10.0);
        let pend = pending(100, 0.0, true, 2000, 2500, Some(4));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.85, 9000);
        p.apply_first_tick_environment_events(d(2026, 4, 26), &profile);
        assert_eq!(count(&p, HappinessEventType::LoanLevelMismatch), 1);
    }

    // ── Weekly: positive recovery priority ──────────────────────

    #[test]
    fn weekly_positive_recovery_outranks_negative_pressure() {
        // Weak-at-elite player AFTER form has improved. Set 8 starts
        // with great form — ProvedLevelAfterMove (priority 95) should
        // outrank OverawedByEliteClub (priority 45) even though the
        // adaptation_score is < 45 and would otherwise trigger it.
        let mut p = player_with(24, 14.0, 1000, 1000, 80, 8.0, 12.0, 10.0);
        let now = d(2026, 4, 26);
        p.last_transfer_date = Some(now - chrono::Duration::days(30));
        // Stuff the rating ledger so realistic average is ≥ 7.0. The
        // canonical entry point is `record_match_rating`, which bumps
        // `played` AND feeds the rating ledger together.
        for _ in 0..8 {
            p.statistics.record_match_rating(7.5, 90, true);
        }
        p.process_transfer_environment_story(now, "", 0.85, 9000, None);
        // Either ProvedLevelAfterMove fired (positive priority), OR
        // the deterministic roll suppressed it — in which case nothing
        // else should have fired either, because the cap is one per
        // week and positives outrank negatives.
        let proved = count(&p, HappinessEventType::ProvedLevelAfterMove);
        let overawed = count(&p, HappinessEventType::OverawedByEliteClub);
        assert!(
            proved >= overawed,
            "positive recovery should outrank negative pressure (proved={}, overawed={})",
            proved,
            overawed,
        );
    }

    // ── Form/sample gating ─────────────────────────────────────

    #[test]
    fn weekly_fan_expectation_burden_does_not_fire_without_form_sample() {
        // Star-to-weak player with no appearances yet. FanExpectationBurden
        // weekly variant requires apps >= 3 before treating "no form" as
        // poor form — must NOT fire on a 0-app sample.
        let mut p = player_with(28, 14.0, 8500, 8500, 170, 6.0, 12.0, 10.0);
        let now = d(2026, 4, 26);
        p.last_transfer_date = Some(now - chrono::Duration::days(30));
        // Zero appearances by default.
        p.process_transfer_environment_story(now, "", 0.25, 3000, None);
        assert_eq!(count(&p, HappinessEventType::FanExpectationBurden), 0);
    }

    #[test]
    fn weekly_too_good_for_level_uses_no_minutes_framing_when_no_apps() {
        // Star-to-weak player, 0 apps → TooGoodForLevel weekly fires
        // with role-frustration (BlockedByDepth) framing instead of
        // poor-form framing.
        let mut p = player_with(28, 15.0, 8500, 8500, 170, 12.0, 12.0, 10.0);
        let now = d(2026, 4, 26);
        p.last_transfer_date = Some(now - chrono::Duration::days(30));
        // No apps. The deterministic roll for this player/week may
        // gate the emission; rather than asserting it fires, we
        // verify the no-minutes candidate is reachable via the
        // signals method when the gate passes.
        let signals = WeeklyEnvSignals {
            days_since: 30,
            adaptation_score: 50.0,
            apps: 0.0,
            starts: 0.0,
            avg_rating: 0.0,
            league_reputation: 3000,
            professionalism: 12.0,
            ambition: 15.0,
            pressure: 12.0,
            current_reputation: 8500,
            had_recent_isolation: false,
            roll: 0.10, // low enough to pass the 0.35 gate
        };
        let candidate = signals.too_good_for_level_candidate();
        assert!(candidate.is_some());
        let c = candidate.unwrap();
        // BlockedByDepth evidence reflects the no-minutes framing.
        assert!(
            c.context
                .evidence
                .contains(&HappinessEventEvidence::BlockedByDepth)
        );
    }

    #[test]
    fn weekly_proved_level_after_move_requires_min_5_apps() {
        // Strong avg rating but only 4 apps — sample too small.
        let signals = WeeklyEnvSignals {
            days_since: 30,
            adaptation_score: 70.0,
            apps: 4.0, // below the 6-apps minimum
            starts: 4.0,
            avg_rating: 8.0,
            league_reputation: 9000,
            professionalism: 12.0,
            ambition: 14.0,
            pressure: 12.0,
            current_reputation: 1000,
            had_recent_isolation: false,
            roll: 0.10,
        };
        assert!(signals.proved_level_after_move_candidate().is_none());
        // Boost to 6 apps — now fires.
        let signals_ok = WeeklyEnvSignals {
            apps: 6.0,
            ..signals
        };
        assert!(signals_ok.proved_level_after_move_candidate().is_some());
    }

    // ── EnvCandidatePool helper ─────────────────────────────────

    #[test]
    fn env_candidate_pool_take_top_picks_highest_priority_per_role() {
        let mut pool = EnvCandidatePool::new();
        let p = player_with(24, 14.0, 1000, 1000, 80, 5.0, 12.0, 10.0);
        let pend = pending(100, 0.0, false, 2000, 2500, Some(5));
        let profile = TransferEnvironmentProfile::build(&p, d(2026, 4, 26), &pend, 0.85, 9000);
        pool.push_some(profile.top_club_opportunity_candidate());
        pool.push_some(profile.role_path_blocked_candidate());
        pool.push_some(profile.overawed_by_elite_club_candidate());
        let primary = pool.take_top_of(EnvRole::Primary).expect("primary");
        assert_eq!(primary.event_type, HappinessEventType::TopClubOpportunity);
        let flavor = pool.take_top_of(EnvRole::Flavor).expect("flavor");
        // RolePathBlocked has priority 65, Overawed has 50.
        assert_eq!(
            flavor.event_type,
            HappinessEventType::RolePathBlockedAtEliteClub
        );
    }

    // ── Full process_transfer_shock pipeline (Spartak ↔ Dynamo Kyiv,
    //    elite-club loan, favourite-club permanent) ──────────────────

    #[test]
    fn spartak_loan_to_dynamo_kyiv_never_fires_dream_move() {
        // 19yo Spartak Moscow prospect (ambition 14, world rep 3500)
        // loaned to Dynamo Kyiv. Source reps: club 7000, RPL 6500.
        // Dest reps: club 0.65 → 6500, UPL 5500. Lateral cross-country
        // move — DreamMove must NOT fire, and the dedicated
        // DreamLoanOpportunity must also be silent (dest is not
        // elite-tier).
        let mut p = player_with(19, 14.0, 3500, 3500, 130, 12.0, 12.0, 10.0);
        let now = d(2026, 4, 26);
        let pend = pending(/* dest */ 200, 1_000_000.0, /* loan */ true, 7000, 6500, Some(2));
        p.pending_signing = Some(pend);
        p.process_transfer_shock(now, 0.65, 5500, "ua", None);
        assert_eq!(count(&p, HappinessEventType::DreamMove), 0);
        assert_eq!(count(&p, HappinessEventType::DreamLoanOpportunity), 0);
    }

    #[test]
    fn permanent_small_club_to_real_madrid_fires_dream_move() {
        // 22yo at a small club (source club 1500, league 2500) joining
        // Real Madrid (dest 0.95 → 9500, league 9500). Source-aware
        // gap is huge — DreamMove fires, DreamLoanOpportunity stays
        // silent on a permanent move.
        let mut p = player_with(22, 14.0, 2000, 2000, 130, 12.0, 12.0, 10.0);
        let now = d(2026, 4, 26);
        let pend = pending(
            /* dest */ 200,
            30_000_000.0,
            /* loan */ false,
            1500,
            2500,
            Some(2),
        );
        p.pending_signing = Some(pend);
        p.process_transfer_shock(now, 0.95, 9500, "es", None);
        assert_eq!(count(&p, HappinessEventType::DreamMove), 1);
        assert_eq!(count(&p, HappinessEventType::DreamLoanOpportunity), 0);
    }

    #[test]
    fn loan_small_club_to_real_madrid_fires_dream_loan_opportunity() {
        // Same young prospect, but on LOAN to Real Madrid. The dream
        // move framing is suppressed; the dedicated loan event fires
        // instead with its smaller magnitude.
        let mut p = player_with(20, 14.0, 2000, 2000, 130, 12.0, 12.0, 10.0);
        let now = d(2026, 4, 26);
        let pend = pending(
            /* dest */ 200,
            0.0,
            /* loan */ true,
            1500,
            2500,
            Some(4),
        );
        p.pending_signing = Some(pend);
        p.process_transfer_shock(now, 0.95, 9500, "es", None);
        assert_eq!(count(&p, HappinessEventType::DreamMove), 0);
        assert_eq!(count(&p, HappinessEventType::DreamLoanOpportunity), 1);
    }

    #[test]
    fn favourite_club_loan_does_not_emit_dream_move() {
        // Favourite-club LOAN. Even with the favourite flag set, the
        // loan path skips the DreamMove framing entirely.
        let mut p = player_with(20, 14.0, 3000, 3000, 130, 12.0, 12.0, 10.0);
        p.favorite_clubs.push(200);
        let now = d(2026, 4, 26);
        let pend = pending(/* dest */ 200, 0.0, /* loan */ true, 6000, 6500, Some(2));
        p.pending_signing = Some(pend);
        // Mid-tier destination — even the loan-opportunity event
        // should stay silent (not elite).
        p.process_transfer_shock(now, 0.55, 5500, "es", None);
        assert_eq!(count(&p, HappinessEventType::DreamMove), 0);
        assert_eq!(count(&p, HappinessEventType::DreamLoanOpportunity), 0);
    }
}
