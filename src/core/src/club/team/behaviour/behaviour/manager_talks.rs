//! Manager-driven passes: weekly talks to address transfer requests,
//! morale slumps, playing-time issues; player-initiated playing-time /
//! loan complaints; head-coach contract terminations for surplus
//! players. Tone selection (`pick_tone`) and personality-aware tone
//! modifiers also live here since they're only consumed by the talk
//! conduct functions.

use super::TeamBehaviour;
use crate::club::player::ManagerPromiseKind;
use crate::club::player::calculators::{
    AutomaticReleaseEligibility, FreeAgentReleaseReason, ReleaseEligibilityContext,
};
use crate::club::player::happiness::{PlayingTimeFrustrationConfig, PlayingTimeOpportunityContext};
use crate::club::player::interaction::{InteractionTone, InteractionTopic};
use crate::club::staff::CoachPlayerBond;
use crate::club::team::behaviour::topic_for_talk;
use crate::club::team::behaviour::{
    ContractTermination, ManagerTalkResult, ManagerTalkType, TeamBehaviourResult,
};
use crate::club::team::squad::SquadAssetContext;
use crate::context::GlobalContext;
use crate::utils::DateUtils;
use crate::{
    Player, PlayerCollection, PlayerFieldPositionGroup, PlayerSquadStatus, PlayerStatusType, Staff,
    StaffCollection,
};
use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;

impl TeamBehaviour {
    /// Date-aware. The interaction-log cooldown gate needs the
    /// simulation date so re-asking the same player about the same topic
    /// is throttled; pass `None` only from contexts where date isn't
    /// known and you accept that the cooldown gate becomes a no-op.
    pub(super) fn process_manager_player_talks_dated(
        players: &PlayerCollection,
        staffs: &StaffCollection,
        result: &mut TeamBehaviourResult,
        today: Option<NaiveDate>,
    ) {
        // Head-coach lookup: Manager → CaretakerManager → AssistantManager →
        // FirstTeamCoach → Coach. A caretaker / assistant can run morale and
        // preventive talks while the manager seat is open.
        let manager = match staffs.social_head_coach() {
            Some(m) => m,
            None => return,
        };

        // Identify players who need talks, sorted by priority
        let mut talk_candidates: Vec<(u32, ManagerTalkType, u8)> = Vec::new(); // (player_id, type, priority)

        for player in &players.players {
            let statuses = player.statuses.get();

            // Highest priority: transfer request
            if statuses.contains(&PlayerStatusType::Req) {
                talk_candidates.push((player.id, ManagerTalkType::TransferDiscussion, 100));
            }

            // High priority: unhappy players
            if statuses.contains(&PlayerStatusType::Unh) {
                // Decide between playing time talk and morale talk
                let talk_type = if player.happiness.factors.playing_time < -5.0 {
                    ManagerTalkType::PlayingTimeTalk
                } else {
                    ManagerTalkType::MoraleTalk
                };
                talk_candidates.push((player.id, talk_type, 90));
            }

            // Proactive: coach talks to high-ability players showing early playing time
            // frustration BEFORE they become fully unhappy — persuade them to stay patient.
            // Only for players with developed skills (CA >= 80).
            let ability = player.player_attributes.current_ability;
            if ability >= 80
                && player.happiness.factors.playing_time < -3.0
                && !statuses.contains(&PlayerStatusType::Unh)
                && !statuses.contains(&PlayerStatusType::Req)
            {
                // Higher ability = higher priority for proactive talk
                let priority = 75 + (ability.saturating_sub(80) / 10).min(15);
                talk_candidates.push((player.id, ManagerTalkType::PlayingTimeTalk, priority));
            }

            // Medium priority: very low morale
            if player.happiness.morale < 30.0
                && !statuses.contains(&PlayerStatusType::Unh)
                && !statuses.contains(&PlayerStatusType::Req)
            {
                talk_candidates.push((player.id, ManagerTalkType::Motivational, 70));
            }

            // Lower priority: praise good performers
            if player.behaviour.is_good() && player.happiness.morale < 80.0 {
                talk_candidates.push((player.id, ManagerTalkType::Praise, 30));
            }

            // Discipline for poor behaviour + high ability
            if player.behaviour.is_poor() && player.player_attributes.current_ability > 100 {
                talk_candidates.push((player.id, ManagerTalkType::Discipline, 60));
            }

            // Form-driven automatic talks — gate on the manager's personality.
            // A strong motivator spots hot streaks; a strong disciplinarian
            // spots slumps. Managers weak in both skip form-based talks.
            // Read the regressed form so a single barnstormer or stinker
            // doesn't summon a praise/discipline meeting after three games.
            let mgr_motivating = manager.staff_attributes.mental.motivating;
            let mgr_discipline = manager.staff_attributes.mental.discipline;
            let pos = player.position().position_group();
            let form = player.statistics.average_rating_realistic(pos);
            let apps = player.statistics.played + player.statistics.played_subs;
            if apps >= 3 {
                if mgr_motivating >= 14 && form >= 7.5 && player.happiness.morale < 85.0 {
                    talk_candidates.push((player.id, ManagerTalkType::Praise, 55));
                }
                if mgr_discipline >= 14
                    && form > 0.0
                    && form < 5.5
                    && player.player_attributes.current_ability >= 70
                {
                    talk_candidates.push((player.id, ManagerTalkType::Discipline, 55));
                }
            }

            // Polish task #5: conflict-risk gated preventive talks. The
            // bond's `conflict_risk` reads across staff relation,
            // promise track-record, low rapport, low authority and
            // personality controversy — so a player whose risk has
            // crept above 0.65 is showing the warning signs of a
            // dressing-room incident before anything formal has
            // happened. Catch it early.
            if let Some(date) = today {
                let bond = CoachPlayerBond::build(player, manager, date);
                if let Some(candidate) =
                    ConflictTalkGate::candidate_for(player.id, bond.conflict_risk)
                {
                    talk_candidates.push(candidate);
                }
            }
        }

        // Sort by priority (highest first)
        talk_candidates.sort_by(|a, b| b.2.cmp(&a.2));

        // Cooldown gate — drop a candidate if the same topic for the
        // same player is still on cooldown from a previous talk. We
        // can't deduplicate solely on (player, talk_type) because
        // different talk_types map to the same topic; we collapse via
        // `topic_for_talk`. Only emergency talks (priority ≥ 90) may
        // bypass the cooldown so genuinely unhappy players still get
        // attention.
        let cooled: Vec<(u32, ManagerTalkType, u8)> = talk_candidates
            .into_iter()
            .filter(|(player_id, talk_type, priority)| {
                if *priority >= 90 {
                    return true;
                }
                let Some(date) = today else { return true };
                let Some(player) = players.find(*player_id) else {
                    return true;
                };
                let topic = topic_for_talk(talk_type.clone());
                !player.interactions.topic_on_cooldown(topic, date)
            })
            .collect();

        // Dedup so we never emit two talks of the same topic to the same
        // player in one weekly batch.
        let mut talk_candidates = cooled;
        let mut seen: Vec<(u32, InteractionTopic)> = Vec::new();
        talk_candidates.retain(|(player_id, talk_type, _)| {
            let topic = topic_for_talk(talk_type.clone());
            if seen
                .iter()
                .any(|(pid, t)| *pid == *player_id && *t == topic)
            {
                return false;
            }
            seen.push((*player_id, topic));
            true
        });

        // Max 4 talks per week, +1 emergency slot for influential players
        let max_talks = 5.min(talk_candidates.len());

        for i in 0..max_talks {
            let (player_id, talk_type, _) = &talk_candidates[i];

            if let Some(player) = players.find(*player_id) {
                let talk_result = Self::conduct_manager_talk(manager, player, talk_type.clone());
                result.manager_talks.push(talk_result);
            }
        }
    }

    fn conduct_manager_talk(
        manager: &Staff,
        player: &Player,
        talk_type: ManagerTalkType,
    ) -> ManagerTalkResult {
        // Success chance formula
        let man_management = manager.staff_attributes.mental.man_management as f32;
        let motivating = manager.staff_attributes.mental.motivating as f32;
        let discipline = manager.staff_attributes.mental.discipline as f32;
        let temperament = player.attributes.temperament;
        let professionalism = player.attributes.professionalism;
        let loyalty = player.attributes.loyalty;

        // Relationship bonus from existing relationship
        let relationship_bonus = player
            .relations
            .get_staff(manager.id)
            .map(|r| (r.level / 100.0) * 0.2)
            .unwrap_or(0.0);

        // Rapport, kept-promise track record, and lived experience of
        // the coach's competence move success_chance on top of the raw
        // attribute formula. Trusted coaches have an easier time
        // landing every kind of talk; an exposed coach (low credibility,
        // broken promises) is fighting uphill.
        let rapport_score = player.rapport.score(manager.id) as f32;
        let rapport_chance = (rapport_score / 400.0).clamp(-0.125, 0.25);
        let promise_chance = player.happiness.factors.promise_trust / 100.0;
        let credibility_chance = player.happiness.factors.coach_credibility / 120.0;

        let success_chance = (0.5 + man_management / 40.0 + motivating / 60.0 - temperament / 60.0
            + professionalism / 80.0
            + loyalty / 80.0
            + relationship_bonus
            + rapport_chance
            + promise_chance
            + credibility_chance)
            .clamp(0.1, 0.95);

        let success = rand::random::<f32>() < success_chance;

        // For transfer discussion, the talk succeeding doesn't guarantee the player
        // withdraws the request — there's only a 30% chance of that happening.
        let actual_success = if talk_type == ManagerTalkType::TransferDiscussion && success {
            rand::random::<f32>() < 0.3
        } else {
            success
        };

        // Pick a tone based on talk_type + manager personality. A
        // discipline-heavy coach reaches for Authoritarian; a high
        // man-management coach for Supportive. Honest framing engages
        // when the underlying squad situation contradicts a successful
        // outcome — e.g. "you'll play more" for a player at the bottom
        // of the depth chart. The picker prefers honesty when the
        // manager has high man_management; weak coaches lie.
        let topic = topic_for_talk(talk_type.clone());
        let tone = pick_tone(&talk_type, manager, player);
        let credibility =
            player.promise_credibility(ManagerPromiseKind::PlayingTime, Some(manager.id));

        // Honest framing if the talk is a "you'll play more" / "we'll
        // sort it out" promise but the squad situation makes that hard
        // to deliver, AND the manager is honest enough to admit it.
        let promise_topic = matches!(
            topic,
            InteractionTopic::PlayingTime
                | InteractionTopic::TransferRequest
                | InteractionTopic::ContractStatus
        );
        let honest_framing =
            promise_topic && credibility < 50 && (man_management + discipline) / 2.0 >= 12.0;

        // Outcomes — base table.
        let (mut morale_change, mut relationship_change) = match (&talk_type, actual_success) {
            (ManagerTalkType::PlayingTimeTalk, true) => (10.0, 0.3),
            (ManagerTalkType::PlayingTimeTalk, false) => (-5.0, -0.1),
            (ManagerTalkType::MoraleTalk, true) => (8.0, 0.3),
            (ManagerTalkType::MoraleTalk, false) => (-3.0, -0.2),
            (ManagerTalkType::TransferDiscussion, true) => (5.0, 0.2),
            (ManagerTalkType::TransferDiscussion, false) => (0.0, 0.0),
            (ManagerTalkType::Praise, true) => (5.0, 0.5),
            (ManagerTalkType::Praise, false) => (1.0, 0.1),
            (ManagerTalkType::Discipline, true) => (-3.0, 0.1),
            (ManagerTalkType::Discipline, false) => (-8.0, -0.5),
            (ManagerTalkType::Motivational, true) => (6.0, 0.2),
            (ManagerTalkType::Motivational, false) => (-2.0, -0.1),
            (ManagerTalkType::PlayingTimeRequest, true) => (8.0, 0.3),
            (ManagerTalkType::PlayingTimeRequest, false) => (-5.0, -0.2),
            (ManagerTalkType::LoanRequest, true) => (5.0, 0.2),
            (ManagerTalkType::LoanRequest, false) => (-3.0, -0.1),
        };

        // Credibility scaling for promise-bearing successful talks. A
        // promise the player half-believed lifts morale less; a
        // not-credible promise barely helps and primes a hard fall when
        // it breaks.
        if actual_success && promise_topic {
            let cred_factor = 0.4 + (credibility as f32 / 100.0) * 0.8; // 0.4..1.2
            morale_change *= cred_factor;
            // Honest framing softens the lift but improves rapport — the
            // player respects being told the truth.
            if honest_framing {
                morale_change *= 0.7;
                relationship_change += 0.15;
            }
        }

        // Tone modifier: matching tone with player personality lifts the
        // outcome; mismatch dampens or backfires.
        let (tone_morale_mul, tone_rel_mul) = tone_modifier(tone, player);
        morale_change *= tone_morale_mul;
        relationship_change *= tone_rel_mul;

        // Rapport reception — praise from a trusted coach lands harder,
        // criticism from an untrusted coach lands much harder. Decide
        // tone from the *outcome* (morale_change sign) rather than the
        // talk type alone: a failed playing-time talk that drops morale
        // should read as criticism even though the talk type is
        // nominally "positive". Discipline always reads negative unless
        // the disciplined player ended up morale-positive (rare — e.g.
        // a pro who wanted clear standards).
        let positive_tone = if matches!(talk_type, ManagerTalkType::Discipline) {
            morale_change > 0.0
        } else {
            morale_change >= 0.0
        };
        let rapport_mult = player
            .rapport
            .talk_reception_multiplier(manager.id, positive_tone);
        morale_change *= rapport_mult;

        debug!(
            "Manager talk: {} with player {} - type {:?}, tone {:?}, honest {}, success: {}, cred {}",
            manager.full_name,
            player.full_name,
            talk_type,
            tone,
            honest_framing,
            actual_success,
            credibility
        );

        ManagerTalkResult {
            player_id: player.id,
            staff_id: manager.id,
            talk_type,
            success: actual_success,
            morale_change,
            relationship_change,
            tone,
            honest_framing,
            mood_before: player.happiness.morale,
        }
    }

    pub(super) fn process_playing_time_complaints(
        players: &PlayerCollection,
        staffs: &StaffCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        let manager = match staffs.social_head_coach() {
            Some(m) => m,
            None => return,
        };

        let current_date = ctx.simulation.date.date();

        // Collect complaint candidates with priority score for sorting
        let mut candidates: Vec<(u32, ManagerTalkType, u32)> = Vec::new();

        for player in &players.players {
            if player.player_attributes.is_injured {
                continue;
            }

            // Manager-pinned players don't initiate loan / playing-time
            // grievances — being a guaranteed starter is exactly what these
            // talks would be asking for.
            if player.is_force_match_selection {
                continue;
            }

            let age = DateUtils::age(player.birth_date, current_date);
            if age < 16 {
                continue;
            }

            // Already has a transfer request or loan status
            let statuses = player.statuses.get();
            if statuses.contains(&PlayerStatusType::Req)
                || statuses.contains(&PlayerStatusType::Loa)
            {
                continue;
            }

            // Skip players already on loan from another club
            if player.is_on_loan() {
                continue;
            }

            let ability = player.player_attributes.current_ability;
            let ambition = player.attributes.ambition;
            let determination = player.skills.mental.determination;

            // Skip players marked as NotNeeded (they accept their fate)
            let squad_status = player.contract.as_ref().map(|c| &c.squad_status);
            if matches!(squad_status, Some(PlayerSquadStatus::NotNeeded)) {
                continue;
            }

            // Match-opportunity gate — every playing-time grievance is
            // judged on the official fixtures the club has actually played
            // since the player joined, never on calendar days. The gate is
            // `None` while there are zero eligible matches, during the hard
            // grace window, or below the status-specific sample.
            let cfg = PlayingTimeFrustrationConfig::default();
            let opp = player.playing_time_opportunity(current_date);
            let loan_min = player
                .contract_loan
                .as_ref()
                .and_then(|c| c.loan_min_appearances);
            let gate = opp.can_judge(squad_status, &cfg, loan_min);

            // ── Check 1: Youth prospect wants real football (loan request) ──
            // Young players with prospect status who aren't getting meaningful
            // first-team football should request loans for development.
            let is_prospect = matches!(
                squad_status,
                Some(PlayerSquadStatus::HotProspectForTheFuture)
                    | Some(PlayerSquadStatus::DecentYoungster)
            );

            if is_prospect && age >= 19 && age <= 23 {
                // Priority increases with age — a 22yo prospect is more urgent than a 19yo
                let age_urgency = (age as f32 - 18.0) / 5.0; // 0.2 at 19, 0.8 at 22
                let ambition_factor = ambition / 20.0; // 0-1
                let determination_factor = determination / 20.0;

                // Ambitious, determined prospects request loans sooner
                let desire =
                    age_urgency * 0.4 + ambition_factor * 0.35 + determination_factor * 0.25;

                // At age 21+ with decent ambition (>10), almost always request
                // At age 19-20, need high ambition (>14) or long wait
                let threshold = if age >= 21 {
                    0.35 // Lower bar — most 21+ prospects want real football
                } else {
                    0.55 // Higher bar — 19-20 year olds need more drive
                };

                // A prospect only pushes for a loan once he has had real
                // first-team match opportunities at the parent club — or
                // has sat through a long idle stretch (60+ days) while the
                // season is clearly under way. Calendar days alone, with
                // the club having played nothing, never trigger it.
                let had_opportunity = opp.eligible_official_matches_since_join > 0;
                let season_active = ctx
                    .club
                    .as_ref()
                    .map(|c| c.league_matches_played > 0)
                    .unwrap_or(false);
                let long_idle = opp.days_since_join >= 60 && season_active && had_opportunity;

                if (desire > threshold && had_opportunity)
                    || (age >= 21 && had_opportunity && gate.is_some())
                    || long_idle
                {
                    let priority = (desire * 100.0) as u32 + age as u32 * 10;
                    candidates.push((player.id, ManagerTalkType::LoanRequest, priority));
                    continue;
                }
            }

            // ── Check 2: Playing time complaints ──
            // Only skilled players complain.
            if ability < 60 {
                continue;
            }

            // No eligible matches / still in grace / below sample → the
            // player has no playing-time grievance to raise yet.
            let Some(frustration_mult) = gate else {
                continue;
            };

            let playing_time_factor =
                calculate_playing_time_factor_for_complaint(player, &opp, &cfg) * frustration_mult;

            if playing_time_factor <= cfg.complaint_threshold {
                // A young player normally asks for a development LOAN, not a
                // permanent exit. But a settled long-term backup — at the
                // club a couple of seasons already, old enough that another
                // loan back to the same bench won't unstick him — needs a
                // PERMANENT move to actually start somewhere. Without this a
                // perpetual #2 (e.g. a young keeper behind an entrenched
                // starter) loans out and returns to the bench indefinitely,
                // never settling anywhere he plays. The talk can still be
                // resolved by giving him minutes; only a genuine, unfixable
                // block escalates to Req → a permanent move.
                let settled_long_term_backup = age >= 21 && opp.days_since_join >= 540;
                let talk_type = if age < 23 && !settled_long_term_backup {
                    ManagerTalkType::LoanRequest
                } else {
                    ManagerTalkType::PlayingTimeRequest
                };

                let priority = opp.eligible_official_matches_since_join as u32 + 50;
                candidates.push((player.id, talk_type, priority));
            }
        }

        // Sort by priority descending (most urgent first)
        candidates.sort_by(|a, b| b.2.cmp(&a.2));

        // Max 2 complaints per week
        let max_complaints = 2.min(candidates.len());

        for i in 0..max_complaints {
            let (player_id, talk_type, _) = &candidates[i];

            if let Some(player) = players.find(*player_id) {
                let talk_result =
                    Self::conduct_loan_or_playing_time_talk(manager, player, talk_type.clone());
                result.manager_talks.push(talk_result);
            }
        }
    }

    /// Head coach reviews the squad for genuinely surplus players whose
    /// contracts can be torn up. This is now gated by the single central
    /// release policy [`AutomaticReleaseEligibility`] — the same gate the
    /// season-start surplus trim and the unresolved-salary fallback use —
    /// so a contracted senior is only mutually terminated when EVERY exit
    /// guard agrees: not on loan / pinned / listed, classified as genuine
    /// squad surplus, clearly below team level (or an old declining
    /// veteran), with negligible market value and a severance the club can
    /// shrug off. A player the market would pay for, an expensive contract,
    /// or anyone still useful is left for the sale / loan / listing systems
    /// instead of being walked for free.
    pub(super) fn process_coach_contract_terminations(
        players: &PlayerCollection,
        staffs: &StaffCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        if staffs.social_head_coach().is_none() {
            return;
        }

        let date = ctx.simulation.date.date();

        // Classify every player against the squad the head coach manages
        // (this team's roster). For the main team that is the first-team
        // squad; a reserve / youth coach measures his own deadwood against
        // his own squad's level. Built once and shared across candidates.
        let asset_ctx = SquadAssetContext::for_squad(players);
        let squad_avg_ability = asset_ctx.squad_avg_ability();

        // Evidence gate. At the very start of a season — before the club has
        // played a meaningful number of official matches — the head coach has
        // not yet seen this squad perform, and the full team-behaviour update
        // fires on the first simulated day (`last_full_update` is `None`) with
        // no warm-up. A mutual termination is a one-way door: the player is
        // walked for free. So it must wait for the season to produce evidence
        // rather than tear up a recognised senior's deal before a ball is
        // kicked. The asset classifier is deliberately early-season-robust for
        // *classification* (it reads prior-season minutes, never the tiny
        // current sample); this guards the *decision* to act on it. Once the
        // squad has accumulated enough matches the pass resumes normally and
        // the surplus is cleared then.
        if asset_ctx.is_early_season() {
            return;
        }

        // Severance / market-value caps scale with this squad's annual
        // wage bill (loanees excluded — they belong to their parent club).
        let annual_wage_bill: u32 = players
            .players
            .iter()
            .filter(|p| !p.is_on_loan())
            .filter_map(|p| p.contract.as_ref().map(|c| c.salary))
            .sum();

        // Reputation inputs for pricing the player exactly as the country
        // listing pass would: the real league reputation and the club's
        // blended market-value reputation, both carried on the context.
        let league_reputation = ctx.club_league_reputation();
        let club_reputation = ctx.club_main_reputation();

        // Per-position-group active headcount on this squad. A free release
        // must never thin a position group below safe match-day cover — the
        // "does not create unsafe squad depth" gate. Decremented as
        // terminations are emitted so two cuts in the same group this week
        // still respect the floor. Loanees / contractless players don't
        // count (they aren't this club's usable depth).
        let mut group_active: HashMap<PlayerFieldPositionGroup, usize> = HashMap::new();
        for p in &players.players {
            if p.is_on_loan() || p.contract.is_none() {
                continue;
            }
            *group_active
                .entry(p.position().position_group())
                .or_default() += 1;
        }
        // Releasing a player must leave at least this many in his group.
        const MIN_GROUP_AFTER_RELEASE: usize = 2;

        const MAX_TERMINATIONS_PER_WEEK: usize = 2;
        let mut emitted = 0;

        for player in &players.players {
            if emitted >= MAX_TERMINATIONS_PER_WEEK {
                break;
            }
            // Depth guard: never strip a position group below safe cover.
            let group = player.position().position_group();
            let active_in_group = group_active.get(&group).copied().unwrap_or(0);
            if active_in_group <= MIN_GROUP_AFTER_RELEASE {
                continue;
            }
            let market_value = player.value(date, league_reputation, club_reputation);
            let release_ctx = ReleaseEligibilityContext {
                date,
                squad_avg_ability,
                market_value,
                annual_wage_bill,
                asset_class: asset_ctx.classify(player, date),
            };
            if let Some(termination) = CoachTerminationReview::evaluate(player, date, &release_ctx) {
                result.contract_terminations.push(termination);
                if let Some(count) = group_active.get_mut(&group) {
                    *count = count.saturating_sub(1);
                }
                emitted += 1;
            }
        }
    }

    /// Loan/playing-time talk with enhanced success logic.
    /// For LoanRequest: success depends heavily on player ambition, determination,
    /// and manager's man_management. Ambitious players are harder to convince to stay.
    fn conduct_loan_or_playing_time_talk(
        manager: &Staff,
        player: &Player,
        talk_type: ManagerTalkType,
    ) -> ManagerTalkResult {
        let man_management = manager.staff_attributes.mental.man_management as f32;
        let motivating = manager.staff_attributes.mental.motivating as f32;
        let professionalism = player.attributes.professionalism;
        let loyalty = player.attributes.loyalty;
        let ambition = player.attributes.ambition;
        let determination = player.skills.mental.determination;

        let relationship_bonus = player
            .relations
            .get_staff(manager.id)
            .map(|r| (r.level / 100.0) * 0.2)
            .unwrap_or(0.0);

        if talk_type == ManagerTalkType::LoanRequest {
            // For loan requests, "success" means the manager AGREES to loan the player.
            // High ambition/determination players are MORE convincing (harder to deny).
            // Good man_management coaches are more likely to agree to a sensible loan.
            let player_conviction = ambition / 20.0 * 0.4
                + determination / 20.0 * 0.3
                + professionalism / 20.0 * 0.2
                + 0.1;
            let coach_willingness = man_management / 20.0 * 0.5 + motivating / 20.0 * 0.3;

            // Same trio of modifiers as conduct_manager_talk: rapport
            // history, kept-promise track record, and lived coach
            // credibility. A player who trusts the coach tilts the
            // negotiation; a player who doesn't fights uphill.
            let rapport_score = player.rapport.score(manager.id) as f32;
            let rapport_chance = (rapport_score / 400.0).clamp(-0.125, 0.25);
            let promise_chance = player.happiness.factors.promise_trust / 100.0;
            let credibility_chance = player.happiness.factors.coach_credibility / 120.0;

            // Base: 50% chance. Player conviction pushes it up, loyalty pulls it down.
            // Final clamp matches conduct_manager_talk so the two paths stay aligned.
            let success_chance = (0.50
                + player_conviction * 0.25
                + coach_willingness * 0.15
                - loyalty / 40.0  // loyal players are less insistent
                + relationship_bonus
                + rapport_chance
                + promise_chance
                + credibility_chance)
                .clamp(0.10, 0.95);

            let success = rand::random::<f32>() < success_chance;

            let (mut morale_change, rel_change) = if success {
                (5.0, 0.2) // Player happy — loan agreed
            } else {
                // Denied loan — ambitious players take it harder
                let morale_hit = -3.0 - (ambition / 20.0) * 4.0; // -3 to -7
                (morale_hit, -0.15)
            };

            // Rapport reception — agreement from a trusted coach lands
            // harder; refusal from an untrusted coach lands much harder
            // (the player reads it as confirmation that the coach
            // doesn't rate them).
            let positive_tone = success;
            let rapport_mult = player
                .rapport
                .talk_reception_multiplier(manager.id, positive_tone);
            morale_change *= rapport_mult;

            let tone = pick_tone(&talk_type, manager, player);
            ManagerTalkResult {
                player_id: player.id,
                staff_id: manager.id,
                talk_type,
                success,
                morale_change,
                relationship_change: rel_change,
                tone,
                honest_framing: !success && manager.staff_attributes.mental.man_management >= 12,
                mood_before: player.happiness.morale,
            }
        } else {
            // Standard playing time talk — use existing logic
            Self::conduct_manager_talk(manager, player, talk_type)
        }
    }
}

/// Head-coach squad-cleanup wrapper around the central release policy.
/// Builds nothing of its own: it defers entirely to
/// [`AutomaticReleaseEligibility`] (the single gate every club-driven
/// release path shares) and only converts a clean pass into a mutual
/// termination. A blocked player is left untouched here — the surplus
/// trim, the unresolved-salary fallback, and the country listing pass own
/// the sell / loan / keep-and-seek-a-buyer outcomes for him, so the coach
/// pass never tears up a deal those systems would rather monetise.
struct CoachTerminationReview;

impl CoachTerminationReview {
    /// Returns a [`ContractTermination`] only when the player is genuinely
    /// releasable for free; `None` keeps him (still useful, sellable,
    /// expensive to settle, listed, pinned, on loan, or contractless).
    fn evaluate(
        player: &Player,
        date: NaiveDate,
        release_ctx: &ReleaseEligibilityContext,
    ) -> Option<ContractTermination> {
        let contract = player.contract.as_ref()?;

        // Already on the market — the sale process owns this player. Never
        // convert a standing listing into a free walk-out.
        if contract.is_transfer_listed {
            return None;
        }

        // Central policy. Anything it blocks (protected role / asset,
        // near team level, valuable, expensive severance, on loan, pinned)
        // is not a free-release candidate.
        if let Some(block) = AutomaticReleaseEligibility::assess(player, release_ctx) {
            debug!(
                "coach termination skipped for {} (id={}): {:?} — CA {} vs squad avg {}, \
                 value={:.0}, severance={} → leaving for the sale/loan/listing systems",
                player.full_name,
                player.id,
                block,
                player.player_attributes.current_ability,
                release_ctx.squad_avg_ability,
                release_ctx.market_value,
                contract.termination_cost(date),
            );
            return None;
        }

        Some(ContractTermination {
            player_id: player.id,
            payout: contract.termination_cost(date),
            reason: FreeAgentReleaseReason::MutualTermination,
        })
    }
}

/// Playing-time deficit for a complaint, on the match-opportunity model.
/// Compares the player's weighted involvement against what his squad
/// status leads him to expect across the eligible official matches the
/// club has actually played since he joined. Returns a value in
/// `[max_negative, 0]`; `0` when he's meeting or beating expectations.
/// The caller scales this by the grace `frustration_multiplier`.
fn calculate_playing_time_factor_for_complaint(
    player: &Player,
    opp: &PlayingTimeOpportunityContext,
    cfg: &PlayingTimeFrustrationConfig,
) -> f32 {
    let eligible = opp.eligible_official_matches_since_join as f32;
    if eligible <= 0.0 {
        return 0.0;
    }
    let status = player.contract.as_ref().map(|c| &c.squad_status);
    let expected_share = PlayingTimeFrustrationConfig::expected_start_share(status);
    let expected_raw = eligible * expected_share;
    let expected = expected_raw.max(1.0);
    let actual = opp.actual_involvement_score(cfg);
    if actual >= expected_raw {
        return 0.0;
    }
    let deficit_ratio = ((expected_raw - actual) / expected).clamp(0.0, 1.0);
    (cfg.max_negative_playing_time_factor * deficit_ratio)
        .clamp(cfg.max_negative_playing_time_factor, 0.0)
}

/// Choose a tone for a manager-player talk based on the talk type and
/// the manager's mental attributes. The picker prefers tones the
/// manager is comfortable with: a high-discipline coach reaches for
/// `Authoritarian`, a high-man-management coach for `Supportive`. The
/// player's personality isn't a hard gate here — the *modifier* below
/// applies the matchup penalty instead.
fn pick_tone(talk_type: &ManagerTalkType, manager: &Staff, _player: &Player) -> InteractionTone {
    let man_mgmt = manager.staff_attributes.mental.man_management;
    let discipline = manager.staff_attributes.mental.discipline;
    let motivating = manager.staff_attributes.mental.motivating;

    match talk_type {
        ManagerTalkType::Discipline => {
            if discipline >= 15 {
                InteractionTone::Authoritarian
            } else {
                InteractionTone::Demanding
            }
        }
        ManagerTalkType::Praise => {
            if motivating >= 14 {
                InteractionTone::Supportive
            } else {
                InteractionTone::Calm
            }
        }
        ManagerTalkType::Motivational => {
            if motivating >= 14 {
                InteractionTone::Supportive
            } else if discipline >= 14 {
                InteractionTone::Demanding
            } else {
                InteractionTone::Calm
            }
        }
        ManagerTalkType::PlayingTimeTalk
        | ManagerTalkType::PlayingTimeRequest
        | ManagerTalkType::TransferDiscussion
        | ManagerTalkType::LoanRequest => {
            if man_mgmt >= 15 {
                InteractionTone::Honest
            } else if man_mgmt >= 11 {
                InteractionTone::Calm
            } else {
                InteractionTone::Evasive
            }
        }
        ManagerTalkType::MoraleTalk => {
            if man_mgmt >= 13 {
                InteractionTone::Supportive
            } else {
                InteractionTone::Calm
            }
        }
    }
}

/// Multipliers applied to (morale_change, relationship_change) given the
/// chosen tone and the player's personality. Approximations of how each
/// player type reacts: hot-headed temperaments rebel against
/// authoritarian, low-pressure personalities retreat under demanding,
/// honest tones land regardless of personality, evasive tones always
/// blunt morale and corrode rapport.
fn tone_modifier(tone: InteractionTone, player: &Player) -> (f32, f32) {
    let temperament = player.attributes.temperament;
    let pressure = player.attributes.pressure;
    let professionalism = player.attributes.professionalism;
    match tone {
        InteractionTone::Calm => (1.0, 1.0),
        InteractionTone::Demanding => {
            if professionalism >= 14.0 {
                (1.1, 1.05)
            } else if pressure <= 8.0 {
                (0.7, 0.7)
            } else {
                (0.95, 0.9)
            }
        }
        InteractionTone::Supportive => {
            if pressure <= 10.0 {
                (1.15, 1.1)
            } else {
                (1.05, 1.05)
            }
        }
        InteractionTone::Honest => (0.9, 1.2),
        InteractionTone::Evasive => (0.6, 0.5),
        InteractionTone::Authoritarian => {
            if temperament <= 8.0 {
                (0.7, 0.5)
            } else if professionalism >= 15.0 {
                (1.05, 0.95)
            } else {
                (0.85, 0.8)
            }
        }
        InteractionTone::Apologetic => (1.05, 1.15),
    }
}

#[cfg(test)]
mod tests {
    use crate::club::player::rapport::PlayerRapport;
    use chrono::NaiveDate;

    fn d() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
    }

    /// Polarity of `positive_tone` in `conduct_manager_talk` is now
    /// driven by the actual sign of the talk outcome, not the talk
    /// type. A failed PlayingTimeTalk produces morale_change = -5.0,
    /// which should read as negative tone — and through the rapport
    /// multiplier, low rapport should amplify the hit.
    #[test]
    fn failed_playing_time_talk_with_low_rapport_hurts_more_than_neutral() {
        let mut low_rapport = PlayerRapport::new();
        // 30 negative ticks → ramp lands the score around -45 (the
        // floor), the lowest meaningful rapport.
        low_rapport.on_negative(99, d(), 30);

        let neutral = PlayerRapport::new();

        // Replicate the polarity decision from conduct_manager_talk
        // for a failed playing-time talk.
        let morale_change: f32 = -5.0;
        let positive_tone = morale_change >= 0.0;
        assert!(!positive_tone);

        let low_rapport_mult = low_rapport.talk_reception_multiplier(99, positive_tone);
        let neutral_mult = neutral.talk_reception_multiplier(99, positive_tone);

        let low_rapport_morale = morale_change * low_rapport_mult;
        let neutral_morale = morale_change * neutral_mult;

        assert!(
            low_rapport_morale < neutral_morale,
            "failed talk with low rapport should hurt more ({} vs {})",
            low_rapport_morale,
            neutral_morale
        );
    }

    /// Refused LoanRequest with low rapport should likewise hurt
    /// more than the neutral-rapport baseline. The LoanRequest path
    /// already uses `let positive_tone = success;` so the multiplier
    /// is queried with `false` for refusals.
    #[test]
    fn refused_loan_request_with_low_rapport_hurts_more_than_neutral() {
        let mut low_rapport = PlayerRapport::new();
        low_rapport.on_negative(99, d(), 30);
        let neutral = PlayerRapport::new();

        // Default refused-loan morale hit for an ambition=14 player:
        // -3 - (14/20)*4 = -5.8.
        let morale_change: f32 = -5.8;
        let positive_tone = false;

        let low_rapport_morale =
            morale_change * low_rapport.talk_reception_multiplier(99, positive_tone);
        let neutral_morale = morale_change * neutral.talk_reception_multiplier(99, positive_tone);

        assert!(
            low_rapport_morale < neutral_morale,
            "refused loan with low rapport should hurt more ({} vs {})",
            low_rapport_morale,
            neutral_morale
        );
    }

    /// Discipline criticism still uses negative rapport reception
    /// even when the talk "succeeds" (morale_change = -3.0 in the
    /// successful Discipline branch). This is the polarity carve-out
    /// for Discipline — a stern talk that the player accepts is
    /// still criticism.
    #[test]
    fn successful_discipline_still_reads_as_criticism() {
        // Replicate the polarity decision from conduct_manager_talk
        // for a successful Discipline talk (morale_change = -3.0).
        let morale_change: f32 = -3.0;
        let is_discipline = true;
        let positive_tone = if is_discipline {
            morale_change > 0.0
        } else {
            morale_change >= 0.0
        };
        assert!(
            !positive_tone,
            "successful Discipline should still read negative"
        );
    }
}

/// Conflict-risk gated candidate generator for the weekly talk picker.
/// Translates the spec thresholds (>0.65 private talk, >0.80 morale
/// rescue) into the existing candidate format. Bundled under a named
/// type so the per-player branch in the talk-picker loop stays compact.
struct ConflictTalkGate;

impl ConflictTalkGate {
    /// Priority for a "soft warning" preventive talk — sits between
    /// the medium-priority Motivational candidate (70) and the
    /// proactive playing-time candidate band (75-90), reflecting "the
    /// coach has noticed but it's not yet on fire".
    const SOFT_WARNING_PRIORITY: u8 = 72;
    /// Priority for an "elevated risk" intervention — high enough to
    /// land above proactive playing-time but below the Unh / Req
    /// emergency band (90+). The coach is reading early signs of a
    /// genuine breakdown.
    const ELEVATED_RISK_PRIORITY: u8 = 85;

    /// Map `conflict_risk` onto a (player_id, talk_type, priority)
    /// candidate, or `None` when the risk sits in the suppress band
    /// (< 0.65). Routes through `MoraleTalk` so the talk picker's
    /// existing morale-talk delivery code (tone selection, outcome
    /// recording, follow-up promise) handles the conversation; the
    /// distinction is just that this candidate fires from a bond
    /// reading rather than from a status flag.
    fn candidate_for(player_id: u32, conflict_risk: f32) -> Option<(u32, ManagerTalkType, u8)> {
        if conflict_risk > 0.80 {
            Some((
                player_id,
                ManagerTalkType::MoraleTalk,
                Self::ELEVATED_RISK_PRIORITY,
            ))
        } else if conflict_risk > 0.65 {
            Some((
                player_id,
                ManagerTalkType::MoraleTalk,
                Self::SOFT_WARNING_PRIORITY,
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod coach_termination_tests {
    //! The head-coach contract-cleanup pass now routes every decision
    //! through the central [`AutomaticReleaseEligibility`] gate, so a
    //! contracted senior is mutually terminated only when he is genuine,
    //! cheap, clearly-below-level surplus — and is otherwise left for the
    //! sale / loan / listing systems instead of being walked for free.
    use super::*;
    use crate::club::StaffStub;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::club::staff::{StaffClubContract, StaffPosition, StaffStatus};
    use crate::club::team::squad::SquadAssetClass;
    use crate::context::SimulationContext;
    use crate::shared::fullname::FullName;
    use crate::{
        ContractType, PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };
    use chrono::{Datelike, Duration};

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 12).unwrap()
        }

        /// Squad context with avg ability 100 and a 1.2M wage bill — the
        /// same calibration the release-gate fixtures use, so the
        /// FullTime severance cap sits at ~12.5K and the market-value cap
        /// at the 100K floor.
        fn ctx(market_value: f64, asset_class: SquadAssetClass) -> ReleaseEligibilityContext {
            ReleaseEligibilityContext {
                date: Self::date(),
                squad_avg_ability: 100,
                market_value,
                annual_wage_bill: 1_200_000,
                asset_class,
            }
        }

        fn contract(
            salary: u32,
            contract_type: ContractType,
            months_remaining: u32,
            status: PlayerSquadStatus,
        ) -> PlayerClubContract {
            let expiration = Self::date() + Duration::days(months_remaining as i64 * 30);
            let mut c = PlayerClubContract::new(salary, expiration);
            c.contract_type = contract_type;
            c.squad_status = status;
            c
        }

        fn player(ability: u8, age: u8, contract: Option<PlayerClubContract>) -> Player {
            Self::player_with_id(1, ability, age, contract)
        }

        fn player_with_id(
            id: u32,
            ability: u8,
            age: u8,
            contract: Option<PlayerClubContract>,
        ) -> Player {
            let birth_year = Self::date().year() - age as i32;
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), "Player".to_string()))
                .birth_date(NaiveDate::from_ymd_opt(birth_year, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(attrs)
                .contract(contract)
                .build()
                .unwrap()
        }

        /// A staff collection holding only a head coach (Manager seat) — the
        /// minimum `process_coach_contract_terminations` needs to run.
        fn head_coach_only() -> StaffCollection {
            let mut staff = StaffStub::default();
            staff.id = 1;
            staff.contract = Some(StaffClubContract::new(
                50_000,
                NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
                StaffPosition::Manager,
                StaffStatus::Active,
            ));
            StaffCollection::new(vec![staff])
        }

        /// A bare global context at the fixture date — no club attached, so
        /// the reputation lookups return 0, which is all the termination pass
        /// needs for pricing a near-worthless veteran.
        fn ctx_global<'a>() -> GlobalContext<'a> {
            let dt = Self::date().and_hms_opt(0, 0, 0).unwrap();
            GlobalContext::new(SimulationContext::new(dt))
        }

        /// A squad with one genuine cheap-old-surplus senior (id 10, NotNeeded,
        /// CA 60, age 36) behind two first-team regulars in the same position
        /// group. The regulars raise the squad level (so the veteran is clearly
        /// below it) and the wage bill (so the value / severance caps have
        /// headroom), and keep the group ≥3 so a release still leaves safe
        /// cover. `busiest_played` sets one regular's official appearances —
        /// the season-evidence proxy that drives the early-season gate.
        fn surplus_squad(busiest_played: u16) -> PlayerCollection {
            let surplus = Self::player_with_id(
                10,
                60,
                36,
                Some(Self::contract(
                    15_000,
                    ContractType::FullTime,
                    3,
                    PlayerSquadStatus::NotNeeded,
                )),
            );
            let mut reg_a = Self::player_with_id(
                11,
                130,
                25,
                Some(Self::contract(
                    5_000_000,
                    ContractType::FullTime,
                    24,
                    PlayerSquadStatus::FirstTeamRegular,
                )),
            );
            reg_a.statistics.played = busiest_played;
            let reg_b = Self::player_with_id(
                12,
                130,
                25,
                Some(Self::contract(
                    5_000_000,
                    ContractType::FullTime,
                    24,
                    PlayerSquadStatus::FirstTeamRegular,
                )),
            );
            PlayerCollection::new(vec![surplus, reg_a, reg_b])
        }
    }

    // Scenario 3: cheap old declining true-surplus senior — every gate
    // passes, so the coach negotiates a mutual termination, the contract
    // is cleared, Frt is stamped, and the explicit reason is recorded.
    #[test]
    fn cheap_declining_surplus_is_mutually_terminated() {
        let player = Fx::player(
            60,
            33,
            Some(Fx::contract(
                15_000,
                ContractType::FullTime,
                3,
                PlayerSquadStatus::NotNeeded,
            )),
        );
        let ctx = Fx::ctx(20_000.0, SquadAssetClass::TrueSurplus);

        let termination = CoachTerminationReview::evaluate(&player, Fx::date(), &ctx)
            .expect("cheap declining true-surplus senior must be terminable");
        assert_eq!(termination.player_id, player.id);
        assert_eq!(termination.reason, FreeAgentReleaseReason::MutualTermination);

        // Committing the termination clears the contract, stamps Frt, and
        // records the explicit reason in state + decision history.
        let mut p = player;
        p.on_contract_terminated(Fx::date(), termination.reason);
        assert!(p.contract.is_none(), "termination must clear the contract");
        assert!(
            p.statuses.get().contains(&PlayerStatusType::Frt),
            "termination must stamp Frt for the free-agent sweep"
        );
        assert_eq!(
            p.release_reason(),
            Some(FreeAgentReleaseReason::MutualTermination),
            "termination must record the explicit mutual-termination reason"
        );
        assert!(
            p.decision_history
                .items
                .iter()
                .any(|d| d.decision == "dec_reason_released_free"),
            "termination must be explained in decision history"
        );
    }

    // Scenario 2: a high-salary full-time player with expensive severance
    // is NOT torn up — the club would have to pay a settlement it won't.
    #[test]
    fn expensive_full_time_contract_is_not_terminated() {
        let player = Fx::player(
            60,
            33,
            Some(Fx::contract(
                2_000_000,
                ContractType::FullTime,
                24,
                PlayerSquadStatus::NotNeeded,
            )),
        );
        let ctx = Fx::ctx(20_000.0, SquadAssetClass::TrueSurplus);
        assert!(
            CoachTerminationReview::evaluate(&player, Fx::date(), &ctx).is_none(),
            "an expensive-to-settle contract must never be auto-terminated"
        );
    }

    // Scenario 1: a valuable surplus senior is a sale, not a free walk-out.
    #[test]
    fn valuable_surplus_senior_is_not_terminated() {
        let player = Fx::player(
            60,
            29,
            Some(Fx::contract(
                15_000,
                ContractType::FullTime,
                24,
                PlayerSquadStatus::NotNeeded,
            )),
        );
        // Market value above the cap → the gate blocks a free release.
        let ctx = Fx::ctx(400_000.0, SquadAssetClass::TrueSurplus);
        assert!(
            CoachTerminationReview::evaluate(&player, Fx::date(), &ctx).is_none(),
            "a player the market would pay for must be sold, not released"
        );
    }

    // Scenario 4: an already transfer-listed player is owned by the sale
    // process — the coach pass must never convert the listing into a free
    // termination, even when the numbers would otherwise clear the gate.
    #[test]
    fn transfer_listed_player_is_not_terminated() {
        let mut contract =
            Fx::contract(15_000, ContractType::FullTime, 3, PlayerSquadStatus::NotNeeded);
        contract.is_transfer_listed = true;
        let player = Fx::player(60, 33, Some(contract));
        let ctx = Fx::ctx(20_000.0, SquadAssetClass::TrueSurplus);
        assert!(
            CoachTerminationReview::evaluate(&player, Fx::date(), &ctx).is_none(),
            "a transfer-listed player's contract must not be torn up"
        );
    }

    // Scenario 7: a loaned-in player belongs to his parent club; the
    // borrowing club can never release him.
    #[test]
    fn loaned_player_is_not_terminated() {
        let mut player = Fx::player(
            60,
            33,
            Some(Fx::contract(
                15_000,
                ContractType::FullTime,
                3,
                PlayerSquadStatus::NotNeeded,
            )),
        );
        let mut loan = PlayerClubContract::new(15_000, Fx::date());
        loan.loan_from_club_id = Some(999);
        player.contract_loan = Some(loan);
        let ctx = Fx::ctx(20_000.0, SquadAssetClass::TrueSurplus);
        assert!(
            CoachTerminationReview::evaluate(&player, Fx::date(), &ctx).is_none(),
            "a loaned-in player must never be released by the borrowing club"
        );
    }

    // Scenario 8: a manager-pinned player is never auto-released.
    #[test]
    fn force_selected_player_is_not_terminated() {
        let mut player = Fx::player(
            60,
            33,
            Some(Fx::contract(
                15_000,
                ContractType::FullTime,
                3,
                PlayerSquadStatus::NotNeeded,
            )),
        );
        player.is_force_match_selection = true;
        let ctx = Fx::ctx(20_000.0, SquadAssetClass::TrueSurplus);
        assert!(
            CoachTerminationReview::evaluate(&player, Fx::date(), &ctx).is_none(),
            "a force-selected player must never be auto-terminated"
        );
    }

    // A still-useful, non-surplus senior (classified anything other than
    // TrueSurplus) is protected even when his bare numbers look thin — the
    // Zobnin guard, at the coach-termination layer.
    #[test]
    fn non_surplus_asset_class_is_not_terminated() {
        let player = Fx::player(
            60,
            29,
            Some(Fx::contract(
                15_000,
                ContractType::FullTime,
                3,
                PlayerSquadStatus::NotYetSet,
            )),
        );
        let ctx = Fx::ctx(20_000.0, SquadAssetClass::UnknownNeedsEvaluation);
        assert!(
            CoachTerminationReview::evaluate(&player, Fx::date(), &ctx).is_none(),
            "a not-yet-evaluated senior must not be walked for free"
        );
    }

    // The early-season evidence gate: a genuine cheap-old-surplus senior who
    // WOULD pass every release gate is nonetheless left alone while the season
    // is too young to judge the squad (no official matches played), and is
    // only mutually terminated once enough matches have accumulated. This is
    // the "veteran walked for free on simulated day 1" guard, exercised
    // through the full `process_coach_contract_terminations` pass.
    #[test]
    fn early_season_window_defers_coach_terminations_until_evidence() {
        let staffs = Fx::head_coach_only();
        let ctx = Fx::ctx_global();

        // Day 1 / low-evidence window — busiest player has zero appearances.
        let early = Fx::surplus_squad(0);
        let mut result = TeamBehaviourResult::new();
        TeamBehaviour::process_coach_contract_terminations(&early, &staffs, &mut result, &ctx);
        assert!(
            result.contract_terminations.is_empty(),
            "no contract may be torn up before the season has produced evidence"
        );

        // Established season — a squad-mate now has a full appearance sample,
        // so the evidence window has closed and the same veteran is released.
        let established = Fx::surplus_squad(20);
        let mut result = TeamBehaviourResult::new();
        TeamBehaviour::process_coach_contract_terminations(&established, &staffs, &mut result, &ctx);
        assert_eq!(
            result.contract_terminations.len(),
            1,
            "once the season has evidence the surplus veteran is mutually terminated"
        );
        assert_eq!(result.contract_terminations[0].player_id, 10);
        assert_eq!(
            result.contract_terminations[0].reason,
            FreeAgentReleaseReason::MutualTermination
        );
    }
}
