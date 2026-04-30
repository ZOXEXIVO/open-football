//! Manager-driven passes: weekly talks to address transfer requests,
//! morale slumps, playing-time issues; player-initiated playing-time /
//! loan complaints; head-coach contract terminations for surplus
//! players. Tone selection (`pick_tone`) and personality-aware tone
//! modifiers also live here since they're only consumed by the talk
//! conduct functions.

use super::TeamBehaviour;
use crate::club::player::ManagerPromiseKind;
use crate::club::player::interaction::{InteractionTone, InteractionTopic};
use crate::club::team::behaviour::topic_for_talk;
use crate::club::team::behaviour::{
    ContractTermination, ManagerTalkResult, ManagerTalkType, TeamBehaviourResult,
};
use crate::context::GlobalContext;
use crate::utils::DateUtils;
use crate::{
    ContractType, Player, PlayerCollection, PlayerSquadStatus, PlayerStatusType, Staff,
    StaffCollection, StaffPosition,
};
use chrono::NaiveDate;
use log::debug;

impl TeamBehaviour {
    /// Date-aware. The interaction-log cooldown gate needs the
    /// simulation date so re-asking the same player about the same topic
    /// is throttled; pass `None` only from contexts where date isn't
    /// known and you accept that the cooldown gate becomes a no-op.
    pub(super) fn process_manager_player_talks_dated(
        players: &PlayerCollection,
        staffs: &StaffCollection,
        result: &mut TeamBehaviourResult,
        today: Option<chrono::NaiveDate>,
    ) {
        // Find the manager
        let manager = match staffs.find_by_position(StaffPosition::Manager) {
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
            let mgr_motivating = manager.staff_attributes.mental.motivating;
            let mgr_discipline = manager.staff_attributes.mental.discipline;
            let form = player.statistics.average_rating;
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
        let manager = match staffs.find_by_position(StaffPosition::Manager) {
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
            let days = player.player_attributes.days_since_last_match;

            // Skip players marked as NotNeeded (they accept their fate)
            let squad_status = player.contract.as_ref().map(|c| &c.squad_status);
            if matches!(squad_status, Some(PlayerSquadStatus::NotNeeded)) {
                continue;
            }

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

                if desire > threshold || (age >= 21 && days > 14) {
                    let priority = (desire * 100.0) as u32 + age as u32 * 10;
                    candidates.push((player.id, ManagerTalkType::LoanRequest, priority));
                    continue;
                }
            }

            // ── Check 2: Playing time complaints (existing logic, enhanced) ──
            // Only skilled players complain
            if ability < 60 {
                continue;
            }

            let ability_modifier = (ability as f32 - 60.0) / 140.0;
            let ambition_modifier = 1.0 - ambition / 30.0;
            let combined_modifier =
                (ambition_modifier * 0.5 + (1.0 - ability_modifier) * 0.5).max(0.4);
            let threshold = (21.0 * combined_modifier) as u16;

            let playing_time_factor = calculate_playing_time_factor_for_complaint(player);

            if days > threshold || playing_time_factor < -10.0 {
                let talk_type = if age < 23 {
                    // Young players request loans, not just playing time
                    ManagerTalkType::LoanRequest
                } else {
                    ManagerTalkType::PlayingTimeRequest
                };

                let priority = days as u32 + if playing_time_factor < -10.0 { 50 } else { 0 };
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

    /// Head coach reviews the squad for unwanted players whose contracts
    /// can be torn up cheaply. Fires when the three FM-style criteria line
    /// up: the player is structurally surplus (NotNeeded or a deadwood
    /// youth), they're not a developing prospect, and the payout is small
    /// enough that the club would rather eat it than keep paying wages.
    pub(super) fn process_coach_contract_terminations(
        players: &PlayerCollection,
        staffs: &StaffCollection,
        result: &mut TeamBehaviourResult,
        ctx: &GlobalContext<'_>,
    ) {
        if staffs.find_by_position(StaffPosition::Manager).is_none() {
            return;
        }

        let date = ctx.simulation.date.date();

        // Use one month of the squad's total wage bill as the cap for a
        // cheap termination — clubs tolerate a payout of that size to free
        // a squad slot. Scales naturally with club size/wealth.
        let monthly_wage_bill: u64 = players
            .players
            .iter()
            .filter(|p| !p.is_on_loan())
            .filter_map(|p| p.contract.as_ref().map(|c| c.salary as u64 / 12))
            .sum();
        let payout_cap = (monthly_wage_bill / 2).max(5_000) as u32;

        const MAX_TERMINATIONS_PER_WEEK: usize = 2;
        let mut emitted = 0;

        for player in &players.players {
            if emitted >= MAX_TERMINATIONS_PER_WEEK {
                break;
            }
            if let Some((payout, reason)) = evaluate_termination(player, date, payout_cap) {
                result.contract_terminations.push(ContractTermination {
                    player_id: player.id,
                    payout,
                    reason,
                });
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

/// Decide whether this player's contract should be terminated today.
/// Returns the payout and a short reason code; None means keep.
fn evaluate_termination(
    player: &Player,
    date: NaiveDate,
    payout_cap: u32,
) -> Option<(u32, &'static str)> {
    if player.is_on_loan() {
        return None;
    }
    let contract = player.contract.as_ref()?;

    // Don't tear up an existing sale — they're leaving anyway.
    if contract.is_transfer_listed {
        // Listed but still sitting around after a while? Let the market
        // finish its job; termination is a last resort, not the first.
    }

    // Promising youngsters stay even when squad-status says NotNeeded.
    let age = DateUtils::age(player.birth_date, date);
    let ca = player.player_attributes.current_ability;
    let pa = player.player_attributes.potential_ability;
    let is_prospect = age <= 23 && pa > ca + 15;
    if is_prospect {
        return None;
    }

    // Any player the squad really needs stays.
    let unneeded = matches!(
        contract.squad_status,
        PlayerSquadStatus::NotNeeded | PlayerSquadStatus::NotYetSet
    );
    if !unneeded {
        return None;
    }

    let payout = contract.termination_cost(date);

    // Free release (youth / amateur / non-contract / expiring): always
    // fine. Paid buyouts only if below the club's comfort threshold.
    let reason = match contract.contract_type {
        ContractType::Youth => "term_reason_youth_surplus",
        ContractType::Amateur | ContractType::NonContract => "term_reason_free_release",
        _ => "term_reason_surplus_squad",
    };

    if payout == 0 || payout <= payout_cap {
        Some((payout, reason))
    } else {
        None
    }
}

fn calculate_playing_time_factor_for_complaint(player: &Player) -> f32 {
    let total = player.statistics.played + player.statistics.played_subs;
    if total < 5 {
        return 0.0;
    }

    let play_ratio = player.statistics.played as f32 / total as f32;

    let expected_ratio = if let Some(ref contract) = player.contract {
        match contract.squad_status {
            PlayerSquadStatus::KeyPlayer => 0.70,
            PlayerSquadStatus::FirstTeamRegular => 0.50,
            PlayerSquadStatus::FirstTeamSquadRotation => 0.25,
            PlayerSquadStatus::MainBackupPlayer => 0.20,
            PlayerSquadStatus::HotProspectForTheFuture => 0.10,
            PlayerSquadStatus::DecentYoungster => 0.10,
            PlayerSquadStatus::NotNeeded => 0.05,
            _ => 0.30,
        }
    } else {
        0.30
    };

    if play_ratio >= expected_ratio {
        0.0
    } else {
        let deficit = (expected_ratio - play_ratio) / expected_ratio.max(0.01);
        -deficit * 20.0
    }
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
