//! Morale-shifting events: contract jealousy from a teammate's fresh
//! signing, monthly loan playing-time audits, controversy incidents, and
//! the periodic peer-wage envy sweep.

use super::TeamBehaviour;
use crate::PlayerHappiness;
use crate::club::person::Person;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::calculators::{
    ContractValuation, ValuationContext, expected_annual_value, package_inputs_from_contract,
};
use crate::club::player::contract::stalemate::{AffordabilityInput, ContractStalemate};
use crate::club::player::happiness::PlayingTimeFrustrationConfig;
use crate::club::player::lifecycle::CareerStageDetector;
use crate::context::GlobalContext;
use crate::utils::IntegerUtils;
use crate::{
    CareerDesireEventContext, CareerDesireEvidence, CareerDesireKind, ConflictLocation,
    HappinessEventCause, HappinessEventContext, HappinessEventEvidence, HappinessEventFollowUp,
    HappinessEventScope, HappinessEventSeverity, HappinessEventType, LoanConcernReason,
    LoanDevelopmentConcernReason, LoanEventContext, LoanEventKind, Player, PlayerClubContract,
    PlayerCollection, PlayerFieldPositionGroup, PlayerSquadStatus, TeammateConflictContext,
    TeammateConflictReason,
};
use chrono::{Datelike, NaiveDate};
use std::cmp::Ordering;
use std::collections::HashMap;

impl TeamBehaviour {
    /// When a teammate signs a notably bigger deal and this player earns
    /// meaningfully less, morale takes a hit — unless they're close friends.
    /// Fires at most once per player per signing window (the signer's
    /// `last_salary_negotiation` timestamp gates it). Gap threshold ≥25%.
    pub(super) fn process_contract_jealousy(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        // Cutoff: teammate's raise within the last 14 days counts as fresh news.
        let freshness_days = 14;

        // Collect fresh signers first (id, salary, last_negotiation) so we
        // don't clash borrows while mutating other players below.
        // Loaned-in players are excluded as signers — their parent club's
        // renewal isn't borrower-squad news, and the borrower's wage
        // hierarchy doesn't include them anyway.
        let signers: Vec<(u32, u32)> = players
            .players
            .iter()
            .filter(|p| !p.is_on_loan())
            .filter_map(|p| {
                let last = p.happiness.last_salary_negotiation?;
                let age_days = (today - last).num_days();
                if age_days >= 0 && age_days <= freshness_days {
                    p.contract.as_ref().map(|c| (p.id, c.salary))
                } else {
                    None
                }
            })
            .collect();

        if signers.is_empty() {
            return;
        }

        for (signer_id, signer_salary) in signers {
            if signer_salary == 0 {
                continue;
            }
            for player in players.players.iter_mut() {
                if player.id == signer_id {
                    continue;
                }
                // Loanees see star wages every day at a top club — they
                // know they're a temporary visitor on a different
                // contract structure (the loan deal), so a star
                // teammate's renewal isn't a personal slight.
                if player.is_on_loan() {
                    continue;
                }
                let own_salary = match player.contract.as_ref() {
                    Some(c) if c.salary > 0 => c.salary,
                    _ => continue,
                };
                // Only established players notice salary gaps. A reserve
                // or recent academy graduate at a top club isn't unsettled
                // when the star striker re-signs for ten times their wage —
                // they're grateful to be in the changing room. Without this
                // gate, a CA-60 squad filler at Real Madrid produces an
                // "unsettled by teammate's salary" event every renewal.
                if player.player_attributes.current_ability < 100
                    && player.player_attributes.world_reputation < 3000
                {
                    continue;
                }
                // Only noticed when the gap is ≥25%.
                let ratio = own_salary as f32 / signer_salary as f32;
                if ratio >= 0.75 {
                    continue;
                }

                // Close friends shrug it off.
                let friendship = player
                    .relations
                    .get_player(signer_id)
                    .map(|r| r.friendship)
                    .unwrap_or(30.0);
                if friendship >= 40.0 {
                    continue;
                }

                // Late-career fair-wage guard: a fading veteran already
                // paid fairly for his own age/ability (by his own
                // `ContractValuation`) isn't unsettled when a prime
                // teammate signs big — the gap is the market valuing
                // youth, not a personal slight. Mirrors the periodic
                // wage-envy gate so the two signals can't disagree about
                // whether a wage is genuinely low.
                let late_career_fair = player
                    .contract
                    .as_ref()
                    .map(|c| WageFairness::assess(player, c, today, ctx).late_career_wage_is_fair(player))
                    .unwrap_or(false);
                if late_career_fair {
                    continue;
                }

                // Magnitude scales with the gap: 25% gap → -1.5, 50% gap → -3.5, cap at -5.
                // Cooldown prevents a fresh raise refiring inside the
                // 14-day jealousy window from the same signer.
                let gap = (1.0 - ratio).clamp(0.25, 0.9);
                let magnitude = -((gap - 0.25) * 6.0 + 1.5).min(5.0);
                let context = HappinessEventContext::new(
                    HappinessEventCause::WageJealousy,
                    HappinessEventSeverity::from_magnitude(magnitude),
                    HappinessEventScope::DressingRoom,
                )
                .with_evidence(HappinessEventEvidence::WageGap)
                .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
                player.happiness.add_event_with_context_and_cooldown(
                    HappinessEventType::SalaryGapNoticed,
                    magnitude,
                    None,
                    context,
                    freshness_days as u16,
                );
            }
        }
    }

    /// Monthly audit of inbound loanees — did the borrowing club actually
    /// give them the minutes the loan contract required? If pace falls
    /// behind, open the recall window (parent may yank them back) and fire
    /// `LackOfPlayingTime` on the player. Runs on day 1 only.
    pub(super) fn process_loan_playing_time_audit(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return;
        }

        for player in players.players.iter_mut() {
            let (min_apps, loan_start, parent_club_id, loan_club_id, permanent_option) =
                match player.contract_loan.as_ref() {
                    Some(l) => match (l.loan_min_appearances, l.started) {
                        (Some(m), Some(s)) => (
                            m,
                            s,
                            l.loan_from_club_id,
                            l.loan_to_club_id,
                            l.loan_future_fee.is_some(),
                        ),
                        _ => continue,
                    },
                    None => continue,
                };
            // Too early to judge pace at all.
            let loan_days_elapsed = (today - loan_start).num_days();
            if loan_days_elapsed < 30 {
                continue;
            }

            // Match-opportunity model: judge the loan against the official
            // fixtures the borrowing club has actually played since the
            // player arrived — never elapsed calendar time. A loan window
            // crossing an international break / winter gap with no matches
            // leaves the audit silent (zero-match invariant), and the gate
            // also enforces the grace window + status-specific sample.
            let opp = player.playing_time_opportunity(today);
            let cfg = PlayingTimeFrustrationConfig::default();
            let status = player.contract.as_ref().map(|c| &c.squad_status);
            if opp
                .can_judge(status, &cfg, Some(min_apps))
                .is_none()
            {
                continue;
            }

            let eligible = opp.eligible_official_matches_since_join;
            let actual = player.statistics.played + player.statistics.played_subs;
            // Expected apps so far scale with matches actually played
            // (capped at the contractual season minimum): a loanee sent
            // out for minutes is expected to feature in the bulk of the
            // games the club plays.
            let expected_by_now = (((eligible as f32) * 0.6).floor() as u16).min(min_apps);
            if expected_by_now == 0 || actual >= expected_by_now {
                continue;
            }

            let deficit = expected_by_now.saturating_sub(actual);
            // Open the recall window for any meaningful shortfall.
            if let Some(loan) = player.contract_loan.as_mut() {
                if loan.loan_recall_available_after.is_none() {
                    loan.loan_recall_available_after = Some(today);
                }
            }
            // Morale hit scales with how badly we're trailing.
            let magnitude = -((deficit as f32 * 0.8).min(6.0) + 1.0);
            let lctx = LoanEventContext::new(LoanEventKind::LoanMinutesConcern);
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::Other,
                HappinessEventSeverity::from_magnitude(magnitude),
                HappinessEventScope::Boardroom,
            )
            .with_loan_context(lctx);
            player.happiness.add_event_with_context(
                HappinessEventType::LackOfPlayingTime,
                magnitude,
                None,
                happiness_ctx,
            );

            // Recall request — the parent-club / player pressure layer
            // above the minutes-concern note. Fires only on a meaningful
            // shortfall (the deficit is real, not a quiet month), and is
            // cooldown-gated so a still-open recall window doesn't refire
            // it monthly. The minutes-concern event above remains the
            // ambient signal; this is the escalation.
            let meaningful = (expected_by_now >= 3 && deficit >= 2)
                || (expected_by_now > 0 && (actual as f32 / expected_by_now as f32) < 0.5);
            if meaningful {
                let recall_mag = -((deficit as f32 * 0.6).min(4.0) + 1.5);
                let mut recall_ctx = LoanEventContext::new(LoanEventKind::LoanRecallRequested)
                    .with_recall_reason(LoanConcernReason::InsufficientMinutes)
                    .with_expected_apps(expected_by_now)
                    .with_actual_apps(actual)
                    .with_deficit_apps(deficit)
                    .with_permanent_option(permanent_option)
                    .with_loan_days_elapsed(loan_days_elapsed.clamp(0, u16::MAX as i64) as u16);
                if let Some(pid) = parent_club_id {
                    recall_ctx = recall_ctx.with_parent_club(pid);
                }
                if let Some(lid) = loan_club_id {
                    recall_ctx = recall_ctx.with_loan_club(lid);
                }
                let recall_happiness = HappinessEventContext::new(
                    HappinessEventCause::Other,
                    HappinessEventSeverity::from_magnitude(recall_mag),
                    HappinessEventScope::Boardroom,
                )
                .with_loan_context(recall_ctx);
                player.happiness.add_event_with_context_and_cooldown(
                    HappinessEventType::LoanRecallRequested,
                    recall_mag,
                    None,
                    recall_happiness,
                    45,
                );
            }
        }
    }

    /// Monthly development audit for loaned youngsters. Where the
    /// minutes-concern / recall audit above is about *action* (open the
    /// recall window), this is about *progress* — a loan can fail even
    /// with some minutes if the player is misused, at the wrong level, in
    /// a weak training environment, or simply not developing. Several weak
    /// signals are aggregated into one warning so it stays meaningful
    /// rather than firing after one quiet month. Runs on day 1 only.
    pub(super) fn process_loan_development_audit(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return;
        }

        for player in players.players.iter_mut() {
            if !player.is_on_loan() {
                continue;
            }
            let loan_start = match player.contract_loan.as_ref().and_then(|l| l.started) {
                Some(s) => s,
                None => continue,
            };
            let loan_days_elapsed = (today - loan_start).num_days();
            if loan_days_elapsed < 60 {
                continue;
            }

            let age = player.age(today);
            let status = player.contract.as_ref().map(|c| c.squad_status.clone());
            let is_prospect = matches!(
                status,
                Some(PlayerSquadStatus::HotProspectForTheFuture)
                    | Some(PlayerSquadStatus::DecentYoungster)
            );
            // Development loans are for youngsters and flagged prospects;
            // an established senior on loan is judged by the recall audit.
            if age > 23 && !is_prospect {
                continue;
            }
            if player
                .happiness
                .has_recent_event(&HappinessEventType::LoanDevelopmentConcern, 60)
            {
                continue;
            }

            // Opportunity gate — same as the minutes audit. Enforces the
            // grace window, status sample, and the zero-official-match
            // invariant, so an injured / break-stranded loanee is never
            // unfairly judged as a failed development.
            let opp = player.playing_time_opportunity(today);
            let cfg = PlayingTimeFrustrationConfig::default();
            if opp.can_judge(status.as_ref(), &cfg, None).is_none() {
                continue;
            }

            let eligible = opp.eligible_official_matches_since_join;
            let starts = opp.player_starts_since_join;
            let minutes_share = if eligible > 0 {
                (starts as f32) / (eligible as f32)
            } else {
                0.0
            };

            let mut score = 0i32;
            let mut reasons: Vec<LoanDevelopmentConcernReason> = Vec::new();

            // Insufficient minutes — the loanee isn't starting often enough.
            if eligible >= 4 && minutes_share < 0.4 {
                score += 2;
                reasons.push(LoanDevelopmentConcernReason::InsufficientMinutes);
            }
            // An active loan-tier mismatch is a strong development signal.
            if player
                .happiness
                .has_recent_event(&HappinessEventType::LoanLevelMismatch, 90)
            {
                score += 2;
                reasons.push(LoanDevelopmentConcernReason::LevelMismatch);
            }
            // Played out of position.
            if player
                .happiness
                .has_recent_event(&HappinessEventType::RoleMismatch, 90)
            {
                score += 2;
                reasons.push(LoanDevelopmentConcernReason::WrongRole);
            }
            // Repeated poor training environment.
            let poor_training = player
                .happiness
                .recent_events
                .iter()
                .filter(|e| {
                    e.event_type == HappinessEventType::PoorTraining && e.days_ago <= 90
                })
                .count();
            if poor_training >= 2 {
                score += 1;
                reasons.push(LoanDevelopmentConcernReason::PoorTrainingEnvironment);
            }
            // Poor match performances despite featuring.
            let apps = player.statistics.played + player.statistics.played_subs;
            if apps >= 3 && player.statistics.average_rating > 0.0
                && player.statistics.average_rating < 6.5
            {
                score += 1;
                reasons.push(LoanDevelopmentConcernReason::PoorMatchPerformance);
            }

            if score < 3 {
                continue;
            }

            // Base -2.5, escalating with the number of failing signals.
            let mut magnitude = if score >= 6 {
                -4.5
            } else if score >= 4 {
                -3.5
            } else {
                HappinessConfig::default().catalog.loan_development_concern
            };
            // A high-professional still getting some starts is coping —
            // dampen. Elite parent-club prospects feel a wasted loan more.
            if player.attributes.professionalism >= 15.0 && starts >= 1 {
                magnitude *= 0.75;
            }
            if matches!(status, Some(PlayerSquadStatus::HotProspectForTheFuture)) {
                magnitude *= 1.20;
            }

            let (parent_club_id, loan_club_id, permanent_option) = player
                .contract_loan
                .as_ref()
                .map(|l| {
                    (
                        l.loan_from_club_id,
                        l.loan_to_club_id,
                        l.loan_future_fee.is_some(),
                    )
                })
                .unwrap_or((None, None, false));

            let mut lctx = LoanEventContext::new(LoanEventKind::LoanDevelopmentConcern)
                .with_minutes_share(minutes_share)
                .with_permanent_option(permanent_option)
                .with_loan_days_elapsed(loan_days_elapsed.clamp(0, u16::MAX as i64) as u16);
            if let Some(pid) = parent_club_id {
                lctx = lctx.with_parent_club(pid);
            }
            if let Some(lid) = loan_club_id {
                lctx = lctx.with_loan_club(lid);
            }
            for reason in reasons {
                lctx = lctx.with_development_reason(reason);
            }

            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::Other,
                HappinessEventSeverity::from_magnitude(magnitude),
                HappinessEventScope::Boardroom,
            )
            .with_loan_context(lctx)
            .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
            player.happiness.add_event_with_context(
                HappinessEventType::LoanDevelopmentConcern,
                magnitude,
                None,
                happiness_ctx,
            );
        }
    }

    /// Monthly controversy roll — high-controversy players with poor
    /// temperament occasionally find themselves in incidents: a dressing-
    /// room row, a media storm, a training-ground scrap. Fires a morale
    /// hit on the player + a relationship drag against a random teammate.
    /// Scaled so a calm, sportsmanlike star ~never triggers, while a hot-
    /// head with controversy >15 and temperament <8 fires frequently.
    pub(super) fn process_controversy_incidents(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return; // Monthly cadence
        }

        // Collect potential troublemakers (immutable pass).
        let candidates: Vec<(u32, u32, f32)> = players
            .players
            .iter()
            .filter_map(|p| {
                let controversy = p.attributes.controversy;
                let temperament = p.attributes.temperament;
                let sportsmanship = p.attributes.sportsmanship;
                if controversy < 12.0 {
                    return None;
                }
                // Risk score: big when controversial + hot-tempered + unsporting
                let risk = controversy + (20.0 - temperament) * 0.6 + (20.0 - sportsmanship) * 0.4;
                if risk < 35.0 {
                    return None;
                }
                // Convert to 0-100 trigger chance this month.
                let chance = ((risk - 35.0) * 1.8).clamp(0.0, 60.0);
                let roll = IntegerUtils::random(0, 100) as f32;
                if roll > chance {
                    return None;
                }
                Some((p.id, 0u32, controversy))
            })
            .collect();

        if candidates.is_empty() {
            return;
        }

        // Pick a nearby teammate (low-friendship, different age bracket) to
        // be involved in the spat. Only one per incident.
        let all_ids: Vec<u32> = players.players.iter().map(|p| p.id).collect();

        for (offender_id, _, controversy) in candidates {
            // Find a candidate teammate — scan for low-friendship relation.
            let victim_id = {
                let offender = match players.find(offender_id) {
                    Some(p) => p,
                    None => continue,
                };
                let mut picked: Option<u32> = None;
                for tid in &all_ids {
                    if *tid == offender_id {
                        continue;
                    }
                    let friendship = offender
                        .relations
                        .get_player(*tid)
                        .map(|r| r.friendship)
                        .unwrap_or(30.0);
                    if friendship < 35.0 {
                        picked = Some(*tid);
                        break;
                    }
                }
                picked
            };

            // Fire the incident event on the offender.
            if let Some(offender) = players.players.iter_mut().find(|p| p.id == offender_id) {
                let magnitude = -(3.0 + ((controversy - 12.0) * 0.3).clamp(0.0, 4.0));
                let bust_up_eligible =
                    controversy >= 15.0 && offender.attributes.temperament <= 8.0;
                // Hot-tempered offenders → training-ground scope; calmer
                // controversies still surface as media noise.
                let scope = if bust_up_eligible {
                    HappinessEventScope::TrainingGround
                } else {
                    HappinessEventScope::Media
                };
                let cause = if bust_up_eligible {
                    HappinessEventCause::PersonalityClash
                } else {
                    HappinessEventCause::MediaPressure
                };

                // Personality-shaped evidence — the user reads "high
                // controversy + low temperament" as the why, not the
                // catch-all "controversy incident".
                let mut offender_evidence: Vec<HappinessEventEvidence> = Vec::new();
                if controversy >= 12.0 {
                    offender_evidence.push(HappinessEventEvidence::HighControversy);
                }
                if offender.attributes.temperament <= 8.0 {
                    offender_evidence.push(HappinessEventEvidence::LowTemperament);
                }
                if offender.attributes.sportsmanship <= 8.0 {
                    offender_evidence.push(HappinessEventEvidence::LowSportsmanship);
                }
                offender_evidence.push(if bust_up_eligible {
                    HappinessEventEvidence::TrainingGroundIncident
                } else {
                    HappinessEventEvidence::MediaIncident
                });

                let incident_ctx = HappinessEventContext::new(
                    cause,
                    HappinessEventSeverity::from_magnitude(magnitude),
                    scope,
                )
                .with_evidence_iter(offender_evidence.iter().copied())
                .with_follow_up(HappinessEventFollowUp::DressingRoomDamageRisk);
                offender.happiness.add_event_with_context(
                    HappinessEventType::ControversyIncident,
                    magnitude,
                    None,
                    incident_ctx,
                );

                // Bigger incidents (training-ground bust-ups) — only the
                // hottest combinations trigger them, never every routine
                // controversy. Cooldown 60d so a recurring offender's
                // history is layered, not flooded.
                if bust_up_eligible {
                    let prof_dampen = crate::club::player::events::scaling::criticism_dampener(
                        offender.attributes.professionalism,
                    );
                    let cfg = HappinessConfig::default();
                    let mag = cfg.catalog.training_ground_bust_up * prof_dampen;
                    let bust_up_ctx = HappinessEventContext::new(
                        HappinessEventCause::PersonalityClash,
                        HappinessEventSeverity::from_magnitude(mag),
                        HappinessEventScope::TrainingGround,
                    )
                    .with_evidence(HappinessEventEvidence::TrainingGroundIncident)
                    .with_evidence(HappinessEventEvidence::HighControversy)
                    .with_evidence(HappinessEventEvidence::LowTemperament)
                    .with_follow_up(HappinessEventFollowUp::ManagerInterventionRisk);
                    offender.happiness.add_event_with_context_and_cooldown(
                        HappinessEventType::TrainingGroundBustUp,
                        mag,
                        None,
                        bust_up_ctx,
                        60,
                    );
                }

                // Public apology — well-adjusted controversial players walk
                // back the worst of the fallout. Soft positive (+1.0).
                if offender.attributes.professionalism >= 14.0 && controversy <= 12.0 {
                    offender
                        .happiness
                        .add_event_default_with_cooldown(HappinessEventType::PublicApology, 90);
                }
            }
            // Ripple on the teammate (if one was found). The victim was
            // picked precisely because friendship was low — surface that
            // as evidence so the UI explains why this teammate, not the
            // generic "argued with a teammate".
            if let Some(vid) = victim_id {
                if let Some(victim) = players.players.iter_mut().find(|p| p.id == vid) {
                    let snapshot = victim
                        .relations
                        .get_player(offender_id)
                        .map(|r| (r.level, r.trust, r.friendship, r.professional_respect));
                    let mut victim_evidence: Vec<HappinessEventEvidence> =
                        vec![HappinessEventEvidence::DressingRoomRow];
                    if let Some((level, trust, friendship, prof)) = snapshot {
                        if friendship <= 35.0 {
                            victim_evidence.push(HappinessEventEvidence::LowFriendship);
                        }
                        if trust <= 35.0 {
                            victim_evidence.push(HappinessEventEvidence::LowTrust);
                        }
                        if prof <= 35.0 {
                            victim_evidence.push(HappinessEventEvidence::LowProfessionalRespect);
                        }
                        if level <= -25.0 {
                            victim_evidence
                                .push(HappinessEventEvidence::AlreadyStrainedRelationship);
                        }
                    }
                    let mut conflict_ctx = HappinessEventContext::new(
                        HappinessEventCause::PersonalityClash,
                        HappinessEventSeverity::from_magnitude(-2.0),
                        HappinessEventScope::DressingRoom,
                    )
                    .with_evidence_iter(victim_evidence.iter().copied())
                    .with_follow_up(HappinessEventFollowUp::DressingRoomDamageRisk)
                    .with_teammate_conflict_context(TeammateConflictContext::new(
                        TeammateConflictReason::PersonalityClash,
                        ConflictLocation::DressingRoom,
                    ));
                    if let Some((level, trust, friendship, prof)) = snapshot {
                        conflict_ctx = conflict_ctx
                            .with_relationship_levels(level, level)
                            .with_relationship_axes(trust, friendship, prof);
                    }
                    // Shared same-tick budget + 45-day partner
                    // cooldown via the central helper, so the
                    // controversy emit can't quietly leapfrog the
                    // behaviour-pass cap or refire on a recurring
                    // offender/victim pair.
                    victim
                        .happiness
                        .try_add_partner_context_with_same_tick_budget(
                            HappinessEventType::ConflictWithTeammate,
                            -2.0,
                            offender_id,
                            conflict_ctx,
                            45,
                            PlayerHappiness::MAX_CONFLICT_WITH_TEAMMATE_PER_TICK,
                        );
                }
            }
        }
    }

    /// Monthly squad-wide wage audit: compare every player's salary to the
    /// top earner at their position group. If they're a starter earning
    /// <60% of the top salary in their slot, fire a gentle recurring
    /// `SalaryGapNoticed` event. Complements `process_contract_jealousy`,
    /// which only fires on fresh raises.
    pub(super) fn process_periodic_wage_envy(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return; // Monthly only
        }

        // Build the top-earner-by-position map from permanent squad
        // contracts only. Loanees' parent contracts may be huge (a Real
        // Madrid loanee carrying a Madrid wage) or tiny (a youth loanee
        // from a lower-league parent) and neither belongs in the
        // borrower's wage structure.
        let mut top_by_group: HashMap<PlayerFieldPositionGroup, u32> = HashMap::new();
        for p in &players.players {
            if p.is_on_loan() {
                continue;
            }
            let Some(contract) = p.contract.as_ref() else {
                continue;
            };
            if contract.salary == 0 {
                continue;
            }
            let group = p.position().position_group();
            let entry = top_by_group.entry(group).or_insert(0);
            if contract.salary > *entry {
                *entry = contract.salary;
            }
        }

        for player in players.players.iter_mut() {
            // Loanees know their wage at the borrower is the loan deal —
            // not the parent contract — and that their stay is temporary.
            // Comparing the parent salary to the borrower's stars is
            // doubly nonsensical and produces the "low-CA loanee
            // unsettled by stars" bug.
            if player.is_on_loan() {
                continue;
            }
            let Some(contract) = player.contract.as_ref() else {
                continue;
            };
            if contract.salary == 0 {
                continue;
            }
            // Only players who play a meaningful role care about the gap —
            // the third-choice keeper being underpaid vs the #1 is the way
            // the world works.
            if !matches!(
                contract.squad_status,
                PlayerSquadStatus::KeyPlayer
                    | PlayerSquadStatus::FirstTeamRegular
                    | PlayerSquadStatus::FirstTeamSquadRotation
            ) {
                continue;
            }
            // Reputation gate (mirror of `process_contract_jealousy`).
            // Squad-status alone isn't enough — a top club may slot a
            // CA-60 youth into rotation as cover, and that player has no
            // business being unsettled by the star earner's wages.
            if player.player_attributes.current_ability < 100
                && player.player_attributes.world_reputation < 3000
            {
                continue;
            }
            // Just-signed grace window: a player who agreed their wage
            // in the last 90 days cannot credibly resent the squad's
            // pay structure — they negotiated their slot in it. A
            // youth-team graduate signing his first senior deal was
            // previously getting an "Unsettled by squad wage
            // hierarchy" event in the same month as the contract.
            //
            // For contracts without a stored start date (older save
            // files, certain generator-produced contracts), fall back
            // to the player's most recent transfer date — every
            // signing path that mutates the senior contract also
            // populates `last_transfer_date`. If both are missing the
            // contract is treated as long-installed (the legacy
            // behaviour), which is the safe default since the
            // appearance gate below still keeps fresh graduates out.
            let contract_age_days = contract
                .started
                .or(player.last_transfer_date)
                .map(|d| (today - d).num_days())
                .unwrap_or(i64::MAX);
            if contract_age_days < 90 {
                continue;
            }
            // Matchday-inclusion gate: even after grace, a player
            // needs a genuine track record at the club before the
            // squad-wide wage hierarchy becomes a personal grievance.
            // Requires ≥8 eligible matches the club has played since
            // the player joined AND ≥3 matchday inclusions
            // (started / sub appearance / named to the bench — being
            // travelled-with counts as "the manager saw fit to take
            // me to the game"). Hot prospects who haven't been
            // included yet, or established players whose first weeks
            // fell across an international break, aren't candidates
            // for this morale signal yet.
            let opp = player.playing_time_opportunity(today);
            if opp.eligible_official_matches_since_join < 8 {
                continue;
            }
            let matchday_inclusion = opp.player_starts_since_join
                + opp.player_sub_apps_since_join
                + opp.player_unused_bench_since_join;
            if matchday_inclusion < 3 {
                continue;
            }
            let group = player.position().position_group();
            let top = match top_by_group.get(&group) {
                Some(t) if *t > 0 => *t,
                _ => continue,
            };
            if player.id == 0 || contract.salary >= top {
                continue;
            }
            // The raw peer ratio is only the entry signal. The full gate
            // weighs it against the player's own fair valuation, age,
            // current role, ambition and temperament — so a fairly-paid
            // veteran merely trailing a prime star is never flagged, while
            // a genuinely underpaid important player still is. The
            // returned magnitude is already late-career-capped.
            let Some(profile) = WageEnvyProfile::evaluate(player, contract, top, today, ctx) else {
                continue;
            };
            // 28-day cooldown so the monthly audit doesn't re-fire the
            // same player while last month's wage-envy event is still
            // visible in the history.
            let context = HappinessEventContext::new(
                HappinessEventCause::WageJealousy,
                HappinessEventSeverity::from_magnitude(profile.magnitude),
                HappinessEventScope::DressingRoom,
            )
            .with_evidence(HappinessEventEvidence::WageGap)
            .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
            player.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::SalaryGapNoticed,
                profile.magnitude,
                None,
                context,
                28,
            );
        }
    }

    /// Monthly squad-ambition audit. An ambitious star who is clearly
    /// above the level of the squad around him — or who has just seen a
    /// top teammate leave unreplaced — pushes the board to strengthen
    /// before he commits his future. This is a pressure signal, not a
    /// transfer request; it becomes dangerous only stacked with title /
    /// European ambition or stalled contract talks. Runs on day 1 only.
    pub(super) fn process_squad_ambition_audit(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return;
        }

        // Squad baseline from permanent (non-loan) players, plus per-unit
        // averages / counts so we can name the weakest unit around a star.
        let mut sum: u32 = 0;
        let mut count: u32 = 0;
        let mut by_group: HashMap<PlayerFieldPositionGroup, (u32, u32)> = HashMap::new();
        for p in &players.players {
            if p.is_on_loan() {
                continue;
            }
            let ca = p.player_attributes.current_ability as u32;
            sum += ca;
            count += 1;
            let group = p.position().position_group();
            let entry = by_group.entry(group).or_insert((0, 0));
            entry.0 += ca;
            entry.1 += 1;
        }
        if count < 5 {
            return;
        }
        let squad_avg = (sum / count) as u8;

        // Weakest unit overall — lowest average ability among groups that
        // actually have players. Used as context + a thin-depth signal.
        let weakest_unit = by_group
            .iter()
            .filter(|(_, (_, c))| *c > 0)
            .min_by(|a, b| {
                let aa = a.1.0 as f32 / a.1.1 as f32;
                let bb = b.1.0 as f32 / b.1.1 as f32;
                aa.partial_cmp(&bb).unwrap_or(Ordering::Equal)
            })
            .map(|(g, _)| *g);

        for player in players.players.iter_mut() {
            if player.is_on_loan() {
                continue;
            }
            let ambition = player.attributes.ambition;
            if ambition < 14.0 {
                continue;
            }
            let ca = player.player_attributes.current_ability;
            let world_rep = player.player_attributes.world_reputation.max(0);
            if ca < 130 && world_rep < 4500 {
                continue;
            }
            // Only key figures carry this weight — a backup wanting the
            // board to sign stars is not a realistic pressure source.
            let is_key = player
                .contract
                .as_ref()
                .map(|c| {
                    matches!(
                        c.squad_status,
                        PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
                    )
                })
                .unwrap_or(false);
            if !is_key {
                continue;
            }
            if player
                .happiness
                .has_recent_event(&HappinessEventType::WantsStrongerSquad, 90)
            {
                continue;
            }

            // Squad-weakness triggers: the star is meaningfully above the
            // squad's level, or a close teammate / mentor just left and
            // wasn't replaced.
            let above_squad = (ca as i32) - (squad_avg as i32) >= 10;
            let key_sold = player
                .happiness
                .has_recent_event(&HappinessEventType::CloseFriendSold, 60)
                || player
                    .happiness
                    .has_recent_event(&HappinessEventType::MentorDeparted, 60);
            if !above_squad && !key_sold {
                continue;
            }

            let player_group = player.position().position_group();
            let unit_thin = by_group
                .get(&player_group)
                .map(|(_, c)| *c < 3)
                .unwrap_or(true);

            let mut desire = CareerDesireEventContext::new(CareerDesireKind::StrongerSquadAmbition)
                .with_squad_average_ability(squad_avg)
                .with_player_ability(ca)
                .with_evidence(CareerDesireEvidence::HighAmbition);
            if let Some(unit) = weakest_unit {
                desire = desire.with_weakest_unit(unit);
            }
            if above_squad {
                desire = desire
                    .with_evidence(CareerDesireEvidence::SquadQualityBelowPlayerLevel)
                    .with_evidence(CareerDesireEvidence::PlayerAboveClubLevel);
            }
            if key_sold {
                desire = desire.with_evidence(CareerDesireEvidence::KeyPlayerSold);
            }
            if unit_thin {
                desire = desire.with_evidence(CareerDesireEvidence::WeakDepthInPlayerUnit);
            }

            let magnitude = HappinessConfig::default().catalog.wants_stronger_squad;
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::ReputationAdmiration,
                HappinessEventSeverity::from_magnitude(magnitude),
                HappinessEventScope::Boardroom,
            )
            .with_career_desire_context(desire)
            .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
            player.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::WantsStrongerSquad,
                magnitude,
                None,
                happiness_ctx,
                90,
            );
        }
    }

    /// Monthly contract-stalemate audit. Where the deterministic country
    /// pipeline reasons about *listing*, this surfaces the player-facing
    /// [`ContractTalksStalled`] signal once a renewal has genuinely broken
    /// down (`Severe` / `Exhausted`). Affordability isn't known in the
    /// squad-behaviour pass, so the assessment falls back to its
    /// rejection-count rules (the assess helper treats unknown headroom
    /// as "don't over-escalate"). Loaned-in players are skipped — the
    /// parent contract owns the renewal. Runs on day 1 only.
    ///
    /// [`ContractTalksStalled`]: HappinessEventType::ContractTalksStalled
    pub(super) fn process_contract_stalemate_audit(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return;
        }
        for player in players.players.iter_mut() {
            if player.is_on_loan() || player.is_retired() {
                continue;
            }
            let current_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
            if current_salary == 0 {
                continue;
            }
            let stalemate = ContractStalemate::assess(
                player,
                today,
                AffordabilityInput {
                    wage_budget_headroom: None,
                    current_salary,
                },
            );
            player.maybe_emit_contract_talks_stalled(&stalemate, today);
        }
    }

    /// Monthly title-ambition audit. Elite, ambitious players at a club
    /// that is visibly off the title pace want to play for a genuine
    /// challenger — a more specific frustration than wanting European
    /// football. Reads league-table context off the [`ClubContext`]
    /// (`ctx.club`): position, season progress, division tier, club
    /// reputation. Rare and mostly affects stars; a runaway version would
    /// strip every mid-table side of talent, so the gates are tight.
    /// Runs on day 1 only.
    ///
    /// [`ClubContext`]: crate::club::context::ClubContext
    pub(super) fn process_title_ambition_audit(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return;
        }
        let Some(club) = ctx.club.as_ref() else {
            return;
        };
        // Top flight only, unless a lower division carries elite prestige.
        if club.main_league_tier > 1 && club.league_reputation < 8000 {
            return;
        }
        if club.total_league_matches == 0 || club.league_position == 0 {
            return;
        }
        // Need enough of the season gone for the table to mean something.
        let progress =
            club.league_matches_played as f32 / club.total_league_matches.max(1) as f32;
        if progress < 0.4 {
            return;
        }
        // Inside the top four is a realistic title shot — no grievance.
        let league_position = club.league_position;
        if league_position <= 4 {
            return;
        }

        let club_id = club.id;
        let club_reputation = club.main_team_reputation;

        for player in players.players.iter_mut() {
            if player.is_on_loan() {
                continue;
            }
            let ambition = player.attributes.ambition;
            if ambition < 16.0 {
                continue;
            }
            let age = player.age(today);
            let world_rep = player.player_attributes.world_reputation.max(0) as u16;
            let ca = player.player_attributes.current_ability;
            // Prime window, or a reputation big enough to transcend age.
            if !((24..=31).contains(&age) || world_rep >= 6500) {
                continue;
            }
            // Only genuine top-tier talent generates this mood.
            if ca < 145 && world_rep < 6000 {
                continue;
            }
            // A fresh arrival hasn't earned the right to grumble yet.
            if player
                .days_since_transfer(today)
                .map(|d| d < 180)
                .unwrap_or(false)
            {
                continue;
            }
            if player
                .happiness
                .has_recent_event(&HappinessEventType::WantsTitleChallenge, 120)
            {
                continue;
            }
            // Loyal club legends at a favourite club give the project time.
            if player.attributes.loyalty >= 17.0 && player.favorite_clubs.contains(&club_id) {
                continue;
            }
            // If the club can't even offer Europe, that ambition is the
            // primary grievance — don't stack a title demand on top.
            if player
                .happiness
                .has_recent_event(&HappinessEventType::WantsEuropeanCompetition, 120)
            {
                continue;
            }

            let mut desire = CareerDesireEventContext::new(CareerDesireKind::TitleChallengeAmbition)
                .with_league_position(league_position)
                .with_club_reputation(club_reputation)
                .with_player_ability(ca)
                .with_evidence(CareerDesireEvidence::HighAmbition)
                .with_evidence(CareerDesireEvidence::CurrentClubNotTitleContender);
            if (24..=31).contains(&age) {
                desire = desire.with_evidence(CareerDesireEvidence::PrimeCareerWindow);
            }
            if (world_rep as i32) > (club_reputation as i32) + 1000 {
                desire = desire.with_evidence(CareerDesireEvidence::PlayerAboveClubLevel);
            }

            let magnitude = HappinessConfig::default().catalog.wants_title_challenge;
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::ReputationAdmiration,
                HappinessEventSeverity::from_magnitude(magnitude),
                HappinessEventScope::Personal,
            )
            .with_career_desire_context(desire)
            .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
            player.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::WantsTitleChallenge,
                magnitude,
                None,
                happiness_ctx,
                120,
            );
        }
    }

    /// Monthly reserve-ambition audit. A senior player parked in a
    /// B / Reserve / Second squad plays real football every week — but at
    /// the wrong level, so the minutes-based playing-time machinery never
    /// sees a grievance. This audit supplies the missing mood: he dreams
    /// of genuine first-team football, at his own club or somewhere he'd
    /// actually start. The weekly complaint pass escalates the lingering
    /// mood into a loan / transfer request. Runs on day 1 only, and only
    /// when the behaviour pass is executing for a senior reserve squad —
    /// age-restricted youth teams (U18..U23) are the normal development
    /// pathway, where being young in the youth side is a career on
    /// track, not a career stuck.
    pub(super) fn process_reserve_ambition_audit(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return;
        }
        let is_senior_reserve = ctx
            .team
            .as_ref()
            .and_then(|t| t.team_type)
            .map(|t| t.is_senior_reserve())
            .unwrap_or(false);
        if !is_senior_reserve {
            return;
        }

        for player in players.players.iter_mut() {
            // Loanees belong to their parent club — their development
            // grievances are the loan audits' concern.
            if player.is_on_loan() {
                continue;
            }
            let age = player.age(today);
            // Teenagers in a reserve squad are still on a development
            // arc; the dream starts once adulthood makes it a career.
            if age < 20 {
                continue;
            }
            // NotNeeded players have accepted their fate — the listing /
            // release systems own their exit, not an ambition dream.
            if player
                .contract
                .as_ref()
                .map(|c| matches!(c.squad_status, PlayerSquadStatus::NotNeeded))
                .unwrap_or(false)
            {
                continue;
            }
            // A fresh arrival is still settling in — no stuck-career
            // story yet. Homegrown players (never transferred) pass.
            if player
                .days_since_transfer(today)
                .map(|d| d < 300)
                .unwrap_or(false)
            {
                continue;
            }
            let ambition = player.attributes.ambition;
            // A low-ambition veteran making a living in the reserves is
            // a realistic career: he doesn't dream, he plays.
            if ambition < 8.0 && age >= 29 {
                continue;
            }

            let days_at_club = player
                .days_since_transfer(today)
                .map(|d| d.clamp(0, u32::MAX as i64) as u32)
                .unwrap_or(0);

            let mut desire =
                CareerDesireEventContext::new(CareerDesireKind::FirstTeamBreakthroughAmbition)
                    .with_player_ability(player.player_attributes.current_ability)
                    .with_evidence(CareerDesireEvidence::StuckInReserveSquad);
            if days_at_club > 0 {
                desire = desire.with_days_at_club(days_at_club);
            }
            if ambition >= 14.0 {
                desire = desire.with_evidence(CareerDesireEvidence::HighAmbition);
            }
            if (24..=31).contains(&age) {
                desire = desire.with_evidence(CareerDesireEvidence::PrimeCareerWindow);
            }

            let magnitude = HappinessConfig::default().catalog.wants_first_team_football;
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::ReputationAdmiration,
                HappinessEventSeverity::from_magnitude(magnitude),
                HappinessEventScope::Personal,
            )
            .with_career_desire_context(desire)
            .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
            player.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::WantsFirstTeamFootball,
                magnitude,
                None,
                happiness_ctx,
                60,
            );
        }
    }

    /// Monthly late-career audit for contracted players. Older players
    /// whose role has faded begin to weigh up retirement; veteran leaders
    /// with the right temperament signal interest in coaching. Both gates
    /// live in [`CareerStageDetector`]; this just walks the squad on the
    /// monthly cadence. Loaned-in players are the parent club's concern.
    pub(super) fn process_veteran_career_stage_audit(
        players: &mut PlayerCollection,
        ctx: &GlobalContext<'_>,
    ) {
        let today = ctx.simulation.date.date();
        if today.day() != 1 {
            return;
        }
        for player in players.players.iter_mut() {
            if player.is_on_loan() {
                continue;
            }
            CareerStageDetector::maybe_consider_retirement(player, today);
            CareerStageDetector::maybe_show_coaching_interest(player, today);
        }
    }
}

/// A player's own fair-wage picture, shared by the periodic wage-envy
/// sweep ([`WageEnvyProfile`]) and the fresh-renewal jealousy path
/// ([`TeamBehaviour::process_contract_jealousy`]) so the two can never
/// disagree about whether a wage is actually low.
///
/// `fair_ratio` compares the player's *effective* annual package (base
/// plus realistically-weighted bonuses, via the shared
/// [`expected_annual_value`] helper) against what [`ContractValuation`]
/// says he should earn given his age, ability, status, club and league —
/// the same curve the salary-happiness factor uses. Because the
/// valuation bakes in the age decline, a fading veteran is measured
/// against a *lower* bar, so a wage that merely trails a prime star is
/// not treated as underpayment.
struct WageFairness {
    age: u8,
    is_goalkeeper: bool,
    /// 34+ for an outfielder, 37+ for a goalkeeper.
    is_late_career: bool,
    /// effective package / fair expected wage. ≥1.0 means paid at or
    /// above his own valuation; well below 1.0 is genuine underpayment.
    fair_ratio: f32,
}

impl WageFairness {
    fn assess(
        player: &Player,
        contract: &PlayerClubContract,
        today: NaiveDate,
        ctx: &GlobalContext<'_>,
    ) -> WageFairness {
        let age = player.age(today);
        let is_goalkeeper = player.position().is_goalkeeper();
        let is_late_career = if is_goalkeeper { age >= 37 } else { age >= 34 };

        // Club / league reputation feed the valuation curve. Fall back to
        // a neutral mid-tier baseline when the club context is absent or
        // unpopulated (older saves), mirroring `calculate_salary_factor`.
        let (club_reputation_score, league_reputation) =
            ctx.club.as_ref().map_or((0.5_f32, 5_000_u16), |club| {
                let rep = if club.main_team_reputation > 0 {
                    (club.main_team_reputation as f32 / 10_000.0).clamp(0.0, 1.0)
                } else {
                    0.5
                };
                let league = if club.league_reputation > 0 {
                    club.league_reputation
                } else {
                    5_000
                };
                (rep, league)
            });

        // Approximate months left; only widens the valuation's leverage
        // band, never the expected wage itself.
        let months_remaining = ((contract.expiration - today).num_days() / 30) as i32;
        let valuation_ctx = ValuationContext::happiness_default(
            player,
            age,
            contract.squad_status.clone(),
            club_reputation_score,
            league_reputation,
            months_remaining,
        );
        let expected_wage = ContractValuation::evaluate(player, &valuation_ctx).expected_wage as f32;
        let effective_salary =
            expected_annual_value(&package_inputs_from_contract(contract, player)) as f32;

        // No meaningful expectation (youth / amateur edge) → treat as
        // fairly paid so the gate can never fire on a divide-by-zero.
        let fair_ratio = if expected_wage >= 1.0 {
            effective_salary / expected_wage
        } else {
            1.0
        };

        WageFairness {
            age,
            is_goalkeeper,
            is_late_career,
            fair_ratio,
        }
    }

    /// True when a late-career veteran is paid fairly enough relative to
    /// his *own* declining valuation that a teammate's fresh raise — or
    /// the squad wage hierarchy — should not unsettle him. Prime-age
    /// players are never suppressed here; the caller's full gate handles
    /// them.
    fn late_career_wage_is_fair(&self, player: &Player) -> bool {
        if !self.is_late_career {
            return false;
        }
        // Within ~30% of his own fair value is not "underpaid" for a
        // fading player.
        if self.fair_ratio >= 0.70 {
            return true;
        }
        // A loyal, professional veteran in the fair band lets it go.
        player.attributes.loyalty >= 16.0
            && player.attributes.professionalism >= 14.0
            && self.fair_ratio >= 0.65
    }
}

/// Outcome of the periodic wage-envy gate for one player. Built (via
/// [`WageEnvyProfile::evaluate`]) only when the player is genuinely and
/// materially underpaid for someone of his importance, age and current
/// role — never merely because a prime-age star out-earns him. The
/// embedded `magnitude` is the final, late-career-capped morale hit.
struct WageEnvyProfile {
    magnitude: f32,
}

impl WageEnvyProfile {
    /// Decide whether `player` should resent the squad wage hierarchy
    /// this month, and size the hit. Returns `None` (no event) unless
    /// every gate passes. `top_group_salary` is the highest permanent
    /// wage at the player's position group; the caller has already
    /// applied the cheap squad-status / reputation / grace / appearance
    /// pre-gates.
    fn evaluate(
        player: &Player,
        contract: &PlayerClubContract,
        top_group_salary: u32,
        today: NaiveDate,
        ctx: &GlobalContext<'_>,
    ) -> Option<WageEnvyProfile> {
        if top_group_salary == 0 || contract.salary == 0 {
            return None;
        }

        let peer_ratio = contract.salary as f32 / top_group_salary as f32;

        let fairness = WageFairness::assess(player, contract, today, ctx);
        let fair_ratio = fairness.fair_ratio;
        let is_late_career = fairness.is_late_career;

        let squad_status = &contract.squad_status;
        let is_key = matches!(squad_status, PlayerSquadStatus::KeyPlayer);
        let starter_ratio = player.happiness.starter_ratio;
        let ambition = player.attributes.ambition;
        let loyalty = player.attributes.loyalty;
        let professionalism = player.attributes.professionalism;

        // ── Score ──────────────────────────────────────────────
        let peer_gap = ((0.60 - peer_ratio) / 0.60).clamp(0.0, 1.0);
        let fair_gap = ((0.82 - fair_ratio) / 0.82).clamp(0.0, 1.0);
        let role_weight = match squad_status {
            PlayerSquadStatus::KeyPlayer => 1.20,
            PlayerSquadStatus::FirstTeamRegular => 1.00,
            PlayerSquadStatus::FirstTeamSquadRotation => 0.70,
            _ => 0.0,
        };
        let current_use_weight = 0.65 + 0.70 * starter_ratio.clamp(0.0, 1.0);
        let ambition_weight = 0.70 + 0.60 * (ambition / 20.0);
        let loyalty_damp = 1.0 - 0.25 * (loyalty / 20.0);
        let professionalism_damp = 1.0 - 0.15 * (professionalism / 20.0);
        let late_career_damp = Self::late_career_damp(&fairness, is_key, starter_ratio);

        let salary_gap_score = (0.45 * peer_gap + 0.55 * fair_gap)
            * role_weight
            * current_use_weight
            * ambition_weight
            * loyalty_damp
            * professionalism_damp
            * late_career_damp;

        // ── Gate ───────────────────────────────────────────────
        if peer_ratio >= 0.60 || fair_ratio >= 0.82 || salary_gap_score < 1.0 {
            return None;
        }

        // A player weighing retirement isn't chasing a new wage band —
        // unless he's still a genuine, regularly-playing key man.
        let recent_retirement = player
            .happiness
            .has_recent_event(&HappinessEventType::RetirementConsidering, 180);
        if recent_retirement && !(is_key && starter_ratio >= 0.60) {
            return None;
        }

        if is_late_career {
            // Tighter bar: only a clearly-underpaid, still-ambitious,
            // still-playing veteran is unsettled by the hierarchy.
            if fair_ratio >= 0.70 || ambition < 14.0 {
                return None;
            }
            if starter_ratio < 0.50 && !is_key {
                return None;
            }
            // A loyal, professional veteran in the fair band lets it go.
            if loyalty >= 16.0 && professionalism >= 14.0 && fair_ratio >= 0.65 {
                return None;
            }
        }

        // ── Magnitude ──────────────────────────────────────────
        let mut magnitude = -(1.25 + salary_gap_score * 4.0).clamp(1.25, 5.0);
        // A fading non-key veteran feels it, but it never becomes a
        // dressing-room crisis.
        if is_late_career && !is_key {
            magnitude = magnitude.max(-2.5);
        }
        // Someone already mulling retirement shrugs most of it off.
        if recent_retirement {
            magnitude = magnitude.max(-1.5);
        }

        Some(WageEnvyProfile { magnitude })
    }

    /// Age-banded damp on the wage-envy score for late-career players.
    /// Non-veterans are unaffected (1.0). A still-central key player who
    /// is playing regularly keeps most of his voice (0.85); everyone else
    /// is steeply discounted as they wind down (0.45 deep into the
    /// decline, 0.65 at its start).
    fn late_career_damp(fairness: &WageFairness, is_key: bool, starter_ratio: f32) -> f32 {
        if !fairness.is_late_career {
            return 1.0;
        }
        if is_key && starter_ratio >= 0.65 {
            return 0.85;
        }
        let steep = if fairness.is_goalkeeper {
            fairness.age >= 39
        } else {
            fairness.age >= 36
        };
        if steep { 0.45 } else { 0.65 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::context::ClubContext;
    use crate::club::player::builder::PlayerBuilder;
    use crate::context::SimulationContext;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerClubContract, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, TeamContext, TeamType,
    };
    use chrono::NaiveDate;

    fn first_of_month(y: i32, m: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, 1).unwrap()
    }

    fn month_ctx<'a>(date: NaiveDate) -> GlobalContext<'a> {
        let dt = date.and_hms_opt(0, 0, 0).unwrap();
        GlobalContext::new(SimulationContext::new(dt))
    }

    fn attrs(ambition: f32) -> PersonAttributes {
        PersonAttributes {
            adaptability: 12.0,
            ambition,
            controversy: 5.0,
            loyalty: 10.0,
            pressure: 12.0,
            professionalism: 12.0,
            sportsmanship: 12.0,
            temperament: 12.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    fn build_player(id: u32, birth: NaiveDate, ca: u8, world_rep: i16, ambition: f32) -> Player {
        let mut pa = PlayerAttributes::default();
        pa.current_ability = ca;
        pa.world_reputation = world_rep;
        pa.current_reputation = world_rep;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".into(), id.to_string()))
            .birth_date(birth)
            .country_id(1)
            .attributes(attrs(ambition))
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(pa)
            .build()
            .unwrap()
    }

    fn with_contract(mut p: Player, status: PlayerSquadStatus) -> Player {
        let mut c = PlayerClubContract::new(50_000, NaiveDate::from_ymd_opt(2028, 6, 30).unwrap());
        c.squad_status = status;
        p.contract = Some(c);
        p
    }

    fn count(player: &Player, kind: HappinessEventType) -> usize {
        player
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == kind)
            .count()
    }

    // ── LoanRecallRequested ─────────────────────────────────────

    /// A young loanee who started 90 days ago, was given the chance to
    /// feature in plenty of matches, but barely played.
    fn make_starved_loanee(today: NaiveDate) -> Player {
        let mut p = build_player(1, NaiveDate::from_ymd_opt(2004, 1, 1).unwrap(), 120, 2_000, 12.0);
        p = with_contract(p, PlayerSquadStatus::FirstTeamRegular);
        let loan_start = today - chrono::Duration::days(90);
        let mut loan = PlayerClubContract::new_loan(
            40_000,
            NaiveDate::from_ymd_opt(2027, 6, 30).unwrap(),
            100,
            101,
            200,
        );
        loan.started = Some(loan_start);
        loan.loan_min_appearances = Some(8);
        p.contract_loan = Some(loan);
        // Plenty of matches available, almost none played.
        p.happiness.eligible_official_matches_since_join = 12;
        p.happiness.starts_since_join = 1;
        p.statistics.played = 1;
        p.statistics.played_subs = 0;
        p
    }

    #[test]
    fn failing_loan_opens_recall_and_requests_it() {
        let today = first_of_month(2026, 6);
        let mut players = PlayerCollection::new(vec![make_starved_loanee(today)]);
        TeamBehaviour::process_loan_playing_time_audit(&mut players, &month_ctx(today));

        let p = &players.players[0];
        assert_eq!(
            count(p, HappinessEventType::LoanRecallRequested),
            1,
            "a clearly failing loan must request a recall"
        );
        assert!(
            p.contract_loan
                .as_ref()
                .unwrap()
                .loan_recall_available_after
                .is_some(),
            "the recall window must be opened"
        );
        // The existing minutes-concern signal still fires alongside it.
        assert!(count(p, HappinessEventType::LackOfPlayingTime) >= 1);
    }

    #[test]
    fn recall_respects_cooldown() {
        let today = first_of_month(2026, 6);
        let mut players = PlayerCollection::new(vec![make_starved_loanee(today)]);
        TeamBehaviour::process_loan_playing_time_audit(&mut players, &month_ctx(today));
        // Re-run the same monthly audit immediately — the 45-day cooldown
        // must keep it from re-firing.
        TeamBehaviour::process_loan_playing_time_audit(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::LoanRecallRequested),
            1,
            "recall request must not refire inside its cooldown"
        );
    }

    #[test]
    fn early_loan_does_not_request_recall() {
        let today = first_of_month(2026, 6);
        let mut p = make_starved_loanee(today);
        // Only 10 days into the loan — inside the 30-day grace.
        p.contract_loan.as_mut().unwrap().started = Some(today - chrono::Duration::days(10));
        let mut players = PlayerCollection::new(vec![p]);
        TeamBehaviour::process_loan_playing_time_audit(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::LoanRecallRequested),
            0,
            "no recall request inside the 30-day grace window"
        );
    }

    // ── LoanDevelopmentConcern ──────────────────────────────────

    #[test]
    fn young_failing_loan_emits_development_concern() {
        let today = first_of_month(2026, 6);
        let mut p = make_starved_loanee(today);
        // Featured a little, but poorly rated — adds the performance signal.
        p.statistics.played = 3;
        p.statistics.average_rating = 6.0;
        let mut players = PlayerCollection::new(vec![p]);
        TeamBehaviour::process_loan_development_audit(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::LoanDevelopmentConcern),
            1,
            "a young loanee with low minutes and poor performances is a development concern"
        );
    }

    #[test]
    fn senior_loanee_no_development_concern() {
        let today = first_of_month(2026, 6);
        let mut p = make_starved_loanee(today);
        // A 30-year-old, not a prospect — judged by minutes, not development.
        p.birth_date = NaiveDate::from_ymd_opt(1996, 1, 1).unwrap();
        p.contract = Some({
            let mut c =
                PlayerClubContract::new(50_000, NaiveDate::from_ymd_opt(2028, 6, 30).unwrap());
            c.squad_status = PlayerSquadStatus::FirstTeamRegular;
            c
        });
        let mut players = PlayerCollection::new(vec![p]);
        TeamBehaviour::process_loan_development_audit(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::LoanDevelopmentConcern),
            0,
            "an established senior is not a development-loan candidate"
        );
    }

    // ── WantsStrongerSquad ──────────────────────────────────────

    #[test]
    fn star_far_above_squad_wants_stronger_squad() {
        let today = first_of_month(2026, 6);
        let birth = NaiveDate::from_ymd_opt(1998, 1, 1).unwrap();
        let star = with_contract(
            build_player(1, birth, 150, 5_000, 16.0),
            PlayerSquadStatus::KeyPlayer,
        );
        let mut squad = vec![star];
        for id in 2..=5u32 {
            squad.push(with_contract(
                build_player(id, birth, 100, 1_000, 10.0),
                PlayerSquadStatus::MainBackupPlayer,
            ));
        }
        let mut players = PlayerCollection::new(squad);
        TeamBehaviour::process_squad_ambition_audit(&mut players, &month_ctx(today));

        assert_eq!(
            count(&players.players[0], HappinessEventType::WantsStrongerSquad),
            1,
            "the ambitious star far above the squad average should speak up"
        );
        // The weak, low-ambition squad players do not.
        for p in &players.players[1..] {
            assert_eq!(count(p, HappinessEventType::WantsStrongerSquad), 0);
        }
    }

    #[test]
    fn squad_ambition_respects_cooldown() {
        let today = first_of_month(2026, 6);
        let birth = NaiveDate::from_ymd_opt(1998, 1, 1).unwrap();
        let star = with_contract(
            build_player(1, birth, 150, 5_000, 16.0),
            PlayerSquadStatus::KeyPlayer,
        );
        let mut squad = vec![star];
        for id in 2..=5u32 {
            squad.push(with_contract(
                build_player(id, birth, 100, 1_000, 10.0),
                PlayerSquadStatus::MainBackupPlayer,
            ));
        }
        let mut players = PlayerCollection::new(squad);
        TeamBehaviour::process_squad_ambition_audit(&mut players, &month_ctx(today));
        TeamBehaviour::process_squad_ambition_audit(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::WantsStrongerSquad),
            1,
            "monthly re-run inside cooldown must not double-fire"
        );
    }

    // ── WantsTitleChallenge ─────────────────────────────────────

    fn title_ctx<'a>(date: NaiveDate, name: &'a str, position: u8) -> GlobalContext<'a> {
        let mut ctx = month_ctx(date);
        let cc = ClubContext::new(1, name)
            .with_league_position(position, 20, 38, 22)
            .with_main_league_tier(1)
            .with_reputations(5_000, 5_000, 6_000, 6_000);
        ctx.club = Some(cc);
        ctx
    }

    #[test]
    fn elite_star_at_midtable_wants_title_challenge() {
        let today = first_of_month(2026, 6);
        let name = "Club".to_string();
        let star = with_contract(
            build_player(1, NaiveDate::from_ymd_opt(1998, 1, 1).unwrap(), 150, 7_000, 16.0),
            PlayerSquadStatus::KeyPlayer,
        );
        let mut players = PlayerCollection::new(vec![star]);
        TeamBehaviour::process_title_ambition_audit(&mut players, &title_ctx(today, &name, 10));
        assert_eq!(
            count(&players.players[0], HappinessEventType::WantsTitleChallenge),
            1,
            "an elite ambitious star at a mid-table club after mid-season should want a title challenger"
        );
    }

    #[test]
    fn elite_star_at_title_contender_does_not() {
        let today = first_of_month(2026, 6);
        let name = "Club".to_string();
        let star = with_contract(
            build_player(1, NaiveDate::from_ymd_opt(1998, 1, 1).unwrap(), 150, 7_000, 16.0),
            PlayerSquadStatus::KeyPlayer,
        );
        let mut players = PlayerCollection::new(vec![star]);
        // Sitting 2nd — a realistic title shot.
        TeamBehaviour::process_title_ambition_audit(&mut players, &title_ctx(today, &name, 2));
        assert_eq!(
            count(&players.players[0], HappinessEventType::WantsTitleChallenge),
            0,
            "a top-four side is a realistic title challenge — no grievance"
        );
    }

    // ── WantsFirstTeamFootball ──────────────────────────────────

    fn reserve_ctx<'a>(date: NaiveDate, team_type: TeamType) -> GlobalContext<'a> {
        let mut ctx = month_ctx(date);
        ctx.team = Some(TeamContext::new(1).with_type(team_type));
        ctx
    }

    /// A 24-year-old homegrown regular of the second squad — never
    /// transferred, decent ambition, not written off. The canonical
    /// "stuck in the reserves for seasons" case.
    fn stuck_reserve_player() -> Player {
        with_contract(
            build_player(1, NaiveDate::from_ymd_opt(2002, 1, 1).unwrap(), 90, 500, 12.0),
            PlayerSquadStatus::MainBackupPlayer,
        )
    }

    #[test]
    fn stuck_senior_reserve_dreams_of_first_team() {
        let today = first_of_month(2026, 6);
        let mut players = PlayerCollection::new(vec![stuck_reserve_player()]);
        TeamBehaviour::process_reserve_ambition_audit(
            &mut players,
            &reserve_ctx(today, TeamType::Second),
        );
        assert_eq!(
            count(&players.players[0], HappinessEventType::WantsFirstTeamFootball),
            1,
            "a settled senior in the second squad should dream of first-team football"
        );
    }

    #[test]
    fn main_team_player_does_not_dream_of_first_team() {
        let today = first_of_month(2026, 6);
        let mut players = PlayerCollection::new(vec![stuck_reserve_player()]);
        TeamBehaviour::process_reserve_ambition_audit(
            &mut players,
            &reserve_ctx(today, TeamType::Main),
        );
        assert_eq!(
            count(&players.players[0], HappinessEventType::WantsFirstTeamFootball),
            0,
            "the audit only applies to senior reserve squads"
        );
    }

    #[test]
    fn youth_squad_player_does_not_dream_yet() {
        let today = first_of_month(2026, 6);
        // 18-year-old in the second squad — still a development arc.
        let teen = with_contract(
            build_player(1, NaiveDate::from_ymd_opt(2008, 2, 1).unwrap(), 90, 500, 12.0),
            PlayerSquadStatus::DecentYoungster,
        );
        let mut players = PlayerCollection::new(vec![teen]);
        TeamBehaviour::process_reserve_ambition_audit(
            &mut players,
            &reserve_ctx(today, TeamType::Second),
        );
        assert_eq!(
            count(&players.players[0], HappinessEventType::WantsFirstTeamFootball),
            0,
            "teenagers in a reserve squad are on the normal development pathway"
        );
    }

    #[test]
    fn fresh_signing_in_reserves_does_not_dream_yet() {
        let today = first_of_month(2026, 6);
        let mut p = stuck_reserve_player();
        // Arrived 60 days ago — still settling in.
        p.last_transfer_date = Some(today - chrono::Duration::days(60));
        let mut players = PlayerCollection::new(vec![p]);
        TeamBehaviour::process_reserve_ambition_audit(
            &mut players,
            &reserve_ctx(today, TeamType::Second),
        );
        assert_eq!(
            count(&players.players[0], HappinessEventType::WantsFirstTeamFootball),
            0,
            "a fresh arrival has no stuck-career story yet"
        );
    }

    #[test]
    fn reserve_ambition_respects_cooldown() {
        let today = first_of_month(2026, 6);
        let mut players = PlayerCollection::new(vec![stuck_reserve_player()]);
        let ctx = reserve_ctx(today, TeamType::Second);
        TeamBehaviour::process_reserve_ambition_audit(&mut players, &ctx);
        TeamBehaviour::process_reserve_ambition_audit(&mut players, &ctx);
        assert_eq!(
            count(&players.players[0], HappinessEventType::WantsFirstTeamFootball),
            1,
            "monthly re-run inside the 60-day cooldown must not double-fire"
        );
    }

    // ── SalaryGapNoticed grace + appearance gate ────────────────

    /// Build a squad that pairs a single starter earning a fraction of
    /// the top wage (gap easily satisfies the 60% threshold) with a
    /// top-earning star at the same position group. The starter's
    /// match-opportunity counters are saturated so the appearance gate
    /// passes — every test then varies only the grace state we want
    /// to exercise.
    fn build_wage_envy_pair(starter_contract_started: Option<NaiveDate>) -> Vec<Player> {
        let birth = NaiveDate::from_ymd_opt(2002, 1, 1).unwrap();
        // A prime-age (24), ambitious, ever-present regular earning a tiny
        // fraction of his own fair value (20k vs a multi-million expected
        // wage) — the canonical "genuinely underpaid important player" who
        // *should* fire. The high ambition + starter_ratio carry the score
        // past the new fair-value gate, so these grace / appearance tests
        // actually exercise those gates rather than passing trivially
        // because the scoring fell short.
        let mut starter = build_player(1, birth, 130, 5_000, 16.0);
        let mut starter_contract = PlayerClubContract::new(
            20_000,
            NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
        );
        starter_contract.squad_status = PlayerSquadStatus::FirstTeamRegular;
        starter_contract.started = starter_contract_started;
        starter.contract = Some(starter_contract);
        starter.happiness.eligible_official_matches_since_join = 12;
        starter.happiness.starts_since_join = 6;
        starter.happiness.starter_ratio = 0.9;

        let star = with_contract(
            build_player(2, birth, 160, 8_000, 14.0),
            PlayerSquadStatus::KeyPlayer,
        );
        // Override the star wage so the ratio gap clearly trips the
        // 60% threshold (20k / 200k = 0.10).
        let mut star = star;
        if let Some(c) = star.contract.as_mut() {
            c.salary = 200_000;
        }
        vec![starter, star]
    }

    #[test]
    fn new_contract_skips_periodic_wage_envy_until_grace_expires() {
        let today = first_of_month(2026, 6);
        // Contract signed 30 days ago — inside the 90-day grace.
        let fresh_squad = build_wage_envy_pair(Some(today - chrono::Duration::days(30)));
        let mut players = PlayerCollection::new(fresh_squad);
        TeamBehaviour::process_periodic_wage_envy(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::SalaryGapNoticed),
            0,
            "a contract started 30 days ago must sit inside the grace window"
        );

        // Same setup, but the contract is now 120 days old — outside
        // grace, appearance gate already satisfied, so the envy
        // signal should fire.
        let aged_squad = build_wage_envy_pair(Some(today - chrono::Duration::days(120)));
        let mut players = PlayerCollection::new(aged_squad);
        TeamBehaviour::process_periodic_wage_envy(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::SalaryGapNoticed),
            1,
            "an established player on a 120-day-old contract should notice the gap"
        );
    }

    #[test]
    fn wage_envy_requires_minimum_appearance_track_record() {
        let today = first_of_month(2026, 6);
        // Contract well outside grace, but the starter has not played
        // enough eligible matches yet — the appearance gate must
        // suppress the audit.
        let mut squad = build_wage_envy_pair(Some(today - chrono::Duration::days(200)));
        squad[0].happiness.eligible_official_matches_since_join = 3;
        squad[0].happiness.starts_since_join = 1;
        let mut players = PlayerCollection::new(squad);
        TeamBehaviour::process_periodic_wage_envy(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::SalaryGapNoticed),
            0,
            "a fresh arrival with only 3 eligible matches must not yet resent the wage hierarchy"
        );
    }

    // ── SalaryGapNoticed late-career / fair-wage gate ───────────

    /// Builds a wage-envy scenario: the underpaid candidate (id 1) plus a
    /// top-earning peer (id 2) in the same position group, so the
    /// periodic sweep has a real wage ceiling to compare against. The
    /// candidate always clears the cheap pre-gates (reputation, 90-day
    /// grace, ≥8 eligible matches, ≥3 matchday inclusions); each test then
    /// varies only the age / status / wage / temperament that the new gate
    /// reasons about.
    struct WageEnvyScenario {
        age_years: i64,
        is_goalkeeper: bool,
        squad_status: PlayerSquadStatus,
        ca: u8,
        world_rep: i16,
        salary: u32,
        top_salary: u32,
        starter_ratio: f32,
        ambition: f32,
        loyalty: f32,
        professionalism: f32,
        recent_retirement: bool,
    }

    impl WageEnvyScenario {
        /// A drastically-underpaid, ever-present, prime-age (24) regular —
        /// the canonical "genuinely underpaid important player".
        fn prime_regular() -> Self {
            WageEnvyScenario {
                age_years: 24,
                is_goalkeeper: false,
                squad_status: PlayerSquadStatus::FirstTeamRegular,
                ca: 140,
                world_rep: 5_000,
                salary: 20_000,
                top_salary: 200_000,
                starter_ratio: 0.90,
                ambition: 16.0,
                loyalty: 10.0,
                professionalism: 12.0,
                recent_retirement: false,
            }
        }

        /// A 35-year-old outfield veteran; defaults otherwise as
        /// [`Self::prime_regular`].
        fn late_career_outfield() -> Self {
            WageEnvyScenario {
                age_years: 35,
                ..Self::prime_regular()
            }
        }

        fn position(&self) -> PlayerPositionType {
            if self.is_goalkeeper {
                PlayerPositionType::Goalkeeper
            } else {
                PlayerPositionType::Striker
            }
        }

        fn squad(&self, today: NaiveDate) -> PlayerCollection {
            let birth = today - chrono::Duration::days(self.age_years * 365);
            let position = self.position();

            let mut candidate = build_player(1, birth, self.ca, self.world_rep, self.ambition);
            candidate.attributes.loyalty = self.loyalty;
            candidate.attributes.professionalism = self.professionalism;
            candidate.positions = PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 20,
                }],
            };
            let mut contract =
                PlayerClubContract::new(self.salary, NaiveDate::from_ymd_opt(2032, 6, 30).unwrap());
            contract.squad_status = self.squad_status.clone();
            // Well outside the 90-day grace window.
            contract.started = Some(today - chrono::Duration::days(200));
            candidate.contract = Some(contract);
            // Saturate the appearance gate; starter_ratio is set
            // independently so a benched veteran can carry a long history
            // of starts yet a low recent ratio.
            candidate.happiness.eligible_official_matches_since_join = 12;
            candidate.happiness.starts_since_join = 9;
            candidate.happiness.starter_ratio = self.starter_ratio;
            if self.recent_retirement {
                candidate
                    .happiness
                    .add_event(HappinessEventType::RetirementConsidering, -2.0);
            }

            // Prime-age key man on the ceiling wage in the same group.
            // Never fires (its salary == the group top).
            let mut peer = build_player(
                2,
                today - chrono::Duration::days(26 * 365),
                165,
                9_000,
                14.0,
            );
            peer.positions = PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 20,
                }],
            };
            let mut peer_contract = PlayerClubContract::new(
                self.top_salary,
                NaiveDate::from_ymd_opt(2032, 6, 30).unwrap(),
            );
            peer_contract.squad_status = PlayerSquadStatus::KeyPlayer;
            peer.contract = Some(peer_contract);

            PlayerCollection::new(vec![candidate, peer])
        }
    }

    /// A veteran paid *fairly for his age* — he earns half the prime
    /// star's wage but well above his own age-suppressed valuation
    /// (fair_ratio ≥ 0.82). The squad wage hierarchy is not a grievance.
    #[test]
    fn fairly_paid_veteran_does_not_notice_wage_gap() {
        let today = first_of_month(2026, 6);
        let mut players = WageEnvyScenario {
            salary: 1_400_000,
            top_salary: 3_000_000,
            ..WageEnvyScenario::late_career_outfield()
        }
        .squad(today);
        TeamBehaviour::process_periodic_wage_envy(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::SalaryGapNoticed),
            0,
            "a veteran paid above his own fair value must not resent a prime star earning more"
        );
    }

    /// A loyal, professional, benched 35-year-old who *is* somewhat
    /// underpaid (fair_ratio ~0.68) still lets it go: a fading reserve is
    /// not unsettled by the squad wage hierarchy.
    #[test]
    fn loyal_benched_veteran_does_not_notice_wage_gap() {
        let today = first_of_month(2026, 6);
        let mut players = WageEnvyScenario {
            salary: 650_000,
            top_salary: 1_400_000,
            starter_ratio: 0.30,
            ambition: 15.0,
            loyalty: 17.0,
            professionalism: 15.0,
            ..WageEnvyScenario::late_career_outfield()
        }
        .squad(today);
        TeamBehaviour::process_periodic_wage_envy(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::SalaryGapNoticed),
            0,
            "a loyal, benched late-career veteran must not be flagged"
        );
    }

    /// An elite, still-central, still-ambitious 35-year-old key player who
    /// is genuinely and badly underpaid (fair_ratio well below 0.70) is a
    /// legitimate grievance — the event still fires.
    #[test]
    fn underpaid_elite_veteran_keyplayer_notices_wage_gap() {
        let today = first_of_month(2026, 6);
        let mut players = WageEnvyScenario {
            squad_status: PlayerSquadStatus::KeyPlayer,
            ca: 165,
            world_rep: 9_000,
            starter_ratio: 0.75,
            ambition: 18.0,
            ..WageEnvyScenario::late_career_outfield()
        }
        .squad(today);
        TeamBehaviour::process_periodic_wage_envy(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::SalaryGapNoticed),
            1,
            "a genuinely-underpaid, still-key, ambitious veteran is a real grievance"
        );
    }

    /// A drastically-underpaid prime-age regular fires — the gate must not
    /// over-suppress the very players it is meant to protect.
    #[test]
    fn underpaid_prime_regular_notices_wage_gap() {
        let today = first_of_month(2026, 6);
        let mut players = WageEnvyScenario::prime_regular().squad(today);
        TeamBehaviour::process_periodic_wage_envy(&mut players, &month_ctx(today));
        assert_eq!(
            count(&players.players[0], HappinessEventType::SalaryGapNoticed),
            1,
            "a drastically-underpaid prime regular should still notice the gap"
        );
    }

    /// Someone already weighing retirement is suppressed — unless he's a
    /// genuine, regularly-playing key man, in which case the hit still
    /// lands but is capped soft (≥ -1.5).
    #[test]
    fn retirement_considering_suppresses_wage_envy_unless_genuine_keyplayer() {
        let today = first_of_month(2026, 6);

        // Rotation-ish key player (starter_ratio 0.55 < 0.60) mulling
        // retirement → suppressed despite a firing-strength gap.
        let mut winding_down = WageEnvyScenario {
            squad_status: PlayerSquadStatus::KeyPlayer,
            starter_ratio: 0.55,
            recent_retirement: true,
            ..WageEnvyScenario::prime_regular()
        }
        .squad(today);
        TeamBehaviour::process_periodic_wage_envy(&mut winding_down, &month_ctx(today));
        assert_eq!(
            count(&winding_down.players[0], HappinessEventType::SalaryGapNoticed),
            0,
            "a player considering retirement and no longer a regular starter is suppressed"
        );

        // Genuine, ever-present key man considering retirement → still
        // fires, but the magnitude is capped soft.
        let mut still_key = WageEnvyScenario {
            squad_status: PlayerSquadStatus::KeyPlayer,
            starter_ratio: 0.70,
            recent_retirement: true,
            ..WageEnvyScenario::prime_regular()
        }
        .squad(today);
        TeamBehaviour::process_periodic_wage_envy(&mut still_key, &month_ctx(today));
        assert_eq!(
            count(&still_key.players[0], HappinessEventType::SalaryGapNoticed),
            1,
            "a still-key regular starter weighing retirement may still feel underpaid"
        );
        let capped = still_key.players[0]
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::SalaryGapNoticed)
            .map(|e| e.magnitude);
        assert_eq!(
            capped,
            Some(-1.5),
            "the retirement-window hit must be capped soft at -1.5"
        );
    }

    /// The late-career boundary is position-specific: at 36 an outfield
    /// player is "late career" (≥34) and suppressed, while a goalkeeper is
    /// not yet (GK threshold is 37) and a genuinely underpaid one still
    /// fires. Same age, same wage, same temperament — only the position
    /// differs.
    #[test]
    fn late_career_boundary_is_position_specific() {
        let today = first_of_month(2026, 6);

        let mut gk = WageEnvyScenario {
            age_years: 36,
            is_goalkeeper: true,
            ..WageEnvyScenario::prime_regular()
        }
        .squad(today);
        TeamBehaviour::process_periodic_wage_envy(&mut gk, &month_ctx(today));
        assert_eq!(
            count(&gk.players[0], HappinessEventType::SalaryGapNoticed),
            1,
            "a 36-year-old keeper is not yet late-career, so a genuine gap still fires"
        );

        let mut outfield = WageEnvyScenario {
            age_years: 36,
            is_goalkeeper: false,
            ..WageEnvyScenario::prime_regular()
        }
        .squad(today);
        TeamBehaviour::process_periodic_wage_envy(&mut outfield, &month_ctx(today));
        assert_eq!(
            count(&outfield.players[0], HappinessEventType::SalaryGapNoticed),
            0,
            "a 36-year-old outfield player is late-career and steeply damped — suppressed"
        );
    }

    /// Fresh-renewal jealousy ([`process_contract_jealousy`]) must not
    /// unsettle a late-career veteran whose own valuation already says his
    /// wage is fair — while a prime, underpaid teammate is still rattled by
    /// the same signing.
    #[test]
    fn fresh_renewal_spares_fairly_paid_veteran_but_not_prime_teammate() {
        let today = first_of_month(2026, 6);

        // Veteran (id 1): 35, paid fairly for his age (900k vs his
        // age-suppressed valuation → fair_ratio ≥ 0.70).
        let mut veteran = build_player(
            1,
            today - chrono::Duration::days(35 * 365),
            140,
            4_000,
            12.0,
        );
        let mut vet_contract =
            PlayerClubContract::new(900_000, NaiveDate::from_ymd_opt(2032, 6, 30).unwrap());
        vet_contract.squad_status = PlayerSquadStatus::FirstTeamRegular;
        veteran.contract = Some(vet_contract);

        // Signer (id 2): a prime star who just agreed a huge fresh deal.
        let mut signer = build_player(
            2,
            today - chrono::Duration::days(25 * 365),
            165,
            9_000,
            14.0,
        );
        let mut signer_contract =
            PlayerClubContract::new(2_000_000, NaiveDate::from_ymd_opt(2032, 6, 30).unwrap());
        signer_contract.squad_status = PlayerSquadStatus::KeyPlayer;
        signer.contract = Some(signer_contract);
        signer.happiness.last_salary_negotiation = Some(today);

        // Prime teammate (id 3): drastically underpaid, not late-career.
        let mut prime = build_player(
            3,
            today - chrono::Duration::days(24 * 365),
            140,
            5_000,
            14.0,
        );
        let mut prime_contract =
            PlayerClubContract::new(200_000, NaiveDate::from_ymd_opt(2032, 6, 30).unwrap());
        prime_contract.squad_status = PlayerSquadStatus::FirstTeamRegular;
        prime.contract = Some(prime_contract);

        let mut players = PlayerCollection::new(vec![veteran, signer, prime]);
        TeamBehaviour::process_contract_jealousy(&mut players, &month_ctx(today));

        assert_eq!(
            count(&players.players[0], HappinessEventType::SalaryGapNoticed),
            0,
            "a fairly-paid late-career veteran should shrug off a teammate's fresh raise"
        );
        assert_eq!(
            count(&players.players[2], HappinessEventType::SalaryGapNoticed),
            1,
            "a drastically-underpaid prime teammate is still unsettled by the same signing"
        );
    }
}
