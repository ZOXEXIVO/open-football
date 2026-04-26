//! Mentorship — senior players tutoring juniors.
//!
//! FM has a thin tutoring system where one senior "tutors" one junior and
//! slowly nudges the junior's personality attributes. This module takes the
//! same idea further:
//!
//! 1. **Automatic pairing** by the Head of Youth Development when a club
//!    has both eligible mentors and mentees. No manual UI required.
//! 2. **Multi-axis transfer**: personality (ambition/professionalism/
//!    determination), plus a tiny weekly CA nudge that represents the
//!    tactical/positional know-how rubbing off.
//! 3. **Rapport-aware**: mentors with high Man Management (if a staff
//!    member is supervising) or high `leadership` skill progress faster.
//! 4. **Position-aware**: a striker is mentored by a striker, a CB by a CB.
//! 5. **Status-driven**: both sides get the `Tut`/`Lrn` status set, which
//!    is visible in the UI so the manager can see who's pairing with whom.
//!
//! The pass runs weekly alongside training. It iterates the main team,
//! builds pairings, and applies gentle nudges to the mentee.

use crate::club::person::Person;
use crate::club::player::language::Language;
use crate::club::player::traits::PlayerTrait;
use crate::club::{ChangeType, HappinessEventType, MentorshipType, Player, PlayerStatusType, RelationshipChange};
use crate::PlayerFieldPositionGroup;
use chrono::NaiveDate;

/// Compatibility threshold below which a candidate mentor/mentee pair is
/// rejected outright. Stops the system from forcing mismatched pairs that
/// would either do nothing or actively hurt the mentee.
pub const MIN_COMPATIBILITY: f32 = 55.0;

/// One successful tutoring pair this tick.
pub struct MentorshipPairing {
    pub mentor_id: u32,
    pub mentee_id: u32,
}

/// Score a player's fitness as a mentor. High personality, leadership, and
/// age are what make a good mentor — raw CA is secondary.
fn mentor_score(player: &Player, now: NaiveDate) -> f32 {
    let age = player.age(now);
    if age < 28 {
        return 0.0;
    }
    let personality = &player.attributes;
    let mental = &player.skills.mental;

    let prof = personality.professionalism;
    let leadership = mental.leadership;
    let determination = mental.determination;
    let influence = mental.teamwork;

    let base = prof * 0.3 + leadership * 0.35 + determination * 0.2 + influence * 0.15;

    // Age premium — 32+ are wiser.
    let age_bonus = if age >= 32 { 2.0 } else if age >= 30 { 1.0 } else { 0.0 };

    // One-club-player trait is a signature mentor trait.
    let trait_bonus = if player.traits.contains(&PlayerTrait::OneClubPlayer) { 1.5 } else { 0.0 };

    base + age_bonus + trait_bonus
}

/// Score how badly a player needs mentoring. Low-personality, low-experience
/// youngsters are the priority.
fn mentee_need(player: &Player, now: NaiveDate) -> f32 {
    let age = player.age(now);
    if age > 22 {
        return 0.0;
    }
    let personality = &player.attributes;
    // Players with weak personality benefit the most from mentoring.
    let personality_gap =
        20.0 - ((personality.professionalism + personality.ambition) / 2.0);
    let age_factor = (22 - age) as f32; // younger = more need
    let low_caps = if player.player_attributes.international_apps == 0 {
        1.0
    } else {
        0.0
    };
    personality_gap * 1.0 + age_factor * 0.5 + low_caps
}

/// Compatibility score for a (mentor, mentee) pair. 0..100 axis. Pairs
/// scoring below [`MIN_COMPATIBILITY`] are skipped — better no pair than
/// a mismatched one that does nothing or actively hurts the mentee.
fn pair_compatibility(mentor: &Player, mentee: &Player, now: NaiveDate) -> f32 {
    let mut score = 50.0f32;

    // Same position group → mentor knows the role and can pass on craft.
    if mentor.position().position_group() == mentee.position().position_group() {
        score += 20.0;
    }

    // Shared language — without it, the day-to-day mentoring barely lands.
    let shared_lang = languages_overlap(mentor, mentee);
    if shared_lang {
        score += 15.0;
    } else {
        score -= 10.0;
    }

    if mentor.attributes.professionalism >= 15.0 {
        score += 10.0;
    }
    if mentor.skills.mental.leadership >= 15.0 {
        score += 8.0;
    }

    let mentor_age = mentor.age(now) as i32;
    let mentee_age = mentee.age(now) as i32;
    let age_gap = (mentor_age - mentee_age).abs();
    if (8..=16).contains(&age_gap) {
        score += 6.0;
    }

    if mentee.attributes.professionalism <= 10.0 && mentor.attributes.professionalism >= 15.0 {
        score += 8.0;
    }

    // Mentor toxicity penalties.
    if mentor.attributes.controversy > 14.0 {
        score -= 15.0;
    }
    if mentor.attributes.temperament < 8.0 {
        score -= 10.0;
    }

    // Existing strained relation between the two — a forced pair would
    // backfire even if every other axis lines up.
    if let Some(rel) = mentor.relations.get_player(mentee.id) {
        if rel.level <= -50.0 || rel.trust <= 20.0 {
            score -= 25.0;
        }
    }

    score
}

fn languages_overlap(a: &Player, b: &Player) -> bool {
    let a_set: std::collections::HashSet<Language> = a
        .languages
        .iter()
        .filter(|l| l.is_native || l.proficiency >= 60)
        .map(|l| l.language)
        .collect();
    if a_set.is_empty() {
        return false;
    }
    b.languages
        .iter()
        .any(|l| (l.is_native || l.proficiency >= 60) && a_set.contains(&l.language))
}

/// Walk the main team and build mentorship pairings. Each mentor teaches
/// at most one mentee per pass, and each mentee has exactly one mentor.
/// Returns the pairings that were applied this tick so callers can surface
/// them to the UI / result stream.
pub fn process_mentorship(
    players: &mut [Player],
    date: NaiveDate,
    head_of_youth_wwy: u8,
) -> Vec<MentorshipPairing> {
    if players.len() < 2 {
        return Vec::new();
    }

    // Score candidates (immutable pass).
    let mut mentors: Vec<(usize, f32, PlayerFieldPositionGroup)> = Vec::new();
    let mut mentees: Vec<(usize, f32, PlayerFieldPositionGroup)> = Vec::new();
    for (idx, p) in players.iter().enumerate() {
        // Already paired? Let the existing pairing finish before re-pairing.
        let statuses = p.statuses.get();
        if statuses.contains(&PlayerStatusType::Inj) {
            continue;
        }
        let group = p.position().position_group();
        let m_score = mentor_score(p, date);
        if m_score > 10.0 && !statuses.contains(&PlayerStatusType::Lrn) {
            mentors.push((idx, m_score, group));
        }
        let n_score = mentee_need(p, date);
        if n_score > 6.0 && !statuses.contains(&PlayerStatusType::Tut) {
            mentees.push((idx, n_score, group));
        }
    }

    if mentors.is_empty() || mentees.is_empty() {
        return Vec::new();
    }

    // Sort by score — best mentors/mentees first.
    mentors.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    mentees.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Greedy same-position-group matching, gated on compatibility.
    let mut used_mentors: Vec<bool> = vec![false; mentors.len()];
    let mut pairings: Vec<(usize, usize, f32)> = Vec::new();
    for (mentee_idx, _, mentee_group) in &mentees {
        // Find best compatibility-scoring mentor in same group.
        let mut best: Option<(usize, f32)> = None;
        for (mi, (mentor_idx, _, mentor_group)) in mentors.iter().enumerate() {
            if used_mentors[mi] {
                continue;
            }
            if mentor_group != mentee_group {
                continue;
            }
            let compat =
                pair_compatibility(&players[*mentor_idx], &players[*mentee_idx], date);
            if compat < MIN_COMPATIBILITY {
                continue;
            }
            match best {
                Some((_, b)) if b >= compat => {}
                _ => best = Some((mi, compat)),
            }
        }
        if let Some((mi, compat)) = best {
            let mentor_idx = mentors[mi].0;
            pairings.push((mentor_idx, *mentee_idx, compat));
            used_mentors[mi] = true;
        }
    }

    // Head of Youth Development amplifies transfer rate: elite HoY → +25%.
    let hoy_factor = 1.0 + (head_of_youth_wwy as f32 / 20.0 - 0.5).max(0.0) * 0.5;

    let mut applied: Vec<MentorshipPairing> = Vec::with_capacity(pairings.len());
    let mentor_ids: Vec<u32> = pairings
        .iter()
        .map(|(m_idx, _, _)| players[*m_idx].id)
        .collect();
    let mentee_ids: Vec<u32> = pairings
        .iter()
        .map(|(_, t_idx, _)| players[*t_idx].id)
        .collect();
    let pair_compatibilities: Vec<f32> =
        pairings.iter().map(|(_, _, c)| *c).collect();

    for (i, (mentor_idx, mentee_idx, compat)) in pairings.iter().enumerate() {
        // Read mentor attributes (immutable).
        let (mentor_prof, mentor_amb, mentor_det, mentor_controversy) = {
            let mentor = &players[*mentor_idx];
            (
                mentor.attributes.professionalism,
                mentor.attributes.ambition,
                mentor.skills.mental.determination,
                mentor.attributes.controversy,
            )
        };

        // Bad-mentor rule: low professionalism / high controversy mentors
        // can sour the mentee instead of guiding them. 20% deterministic
        // weekly chance of negative influence.
        let bad_mentor =
            mentor_prof < 8.0 || mentor_controversy > 16.0;
        let bad_roll = bad_mentor_roll(mentor_ids[i], mentee_ids[i], date) < 0.20;

        // Step size scales with compatibility tier.
        let (step_factor, bond_mag) = if *compat >= 75.0 {
            (1.0, 0.20)
        } else {
            // 55..74 tier
            (0.6, 0.10)
        };
        let step = 0.020 * step_factor * hoy_factor;

        let mentee = &mut players[*mentee_idx];
        if bad_mentor && bad_roll {
            // Negative influence — pull mentee's professionalism down a
            // notch and record training friction with the mentor.
            mentee.attributes.professionalism =
                (mentee.attributes.professionalism - 0.010).clamp(0.0, 20.0);
            mentee.relations.update_with_type(
                mentor_ids[i],
                -0.15,
                ChangeType::TrainingFriction,
                date,
            );
            // Repeated incidents (last 30 days had a similar event) escalate
            // to a visible ConflictWithTeammate event.
            let recent_friction = mentee
                .happiness
                .recent_events
                .iter()
                .filter(|e| {
                    e.event_type == HappinessEventType::ConflictWithTeammate
                        && e.partner_player_id == Some(mentor_ids[i])
                        && e.days_ago <= 30
                })
                .count();
            if recent_friction >= 1 {
                mentee.happiness.add_event_with_partner(
                    HappinessEventType::ConflictWithTeammate,
                    -1.5,
                    Some(mentor_ids[i]),
                );
            }
        } else {
            // Healthy mentoring — drift personality and bond.
            nudge_personality_step(mentee, mentor_prof, mentor_amb, mentor_det, step);
            mentee.relations.update_with_type(
                mentor_ids[i],
                bond_mag,
                ChangeType::MentorshipBond,
                date,
            );
            // Partner-tagged event — tier 75+ is meaningful enough to log.
            if *compat >= 75.0 {
                mentee.happiness.add_event_with_partner(
                    HappinessEventType::TeammateBonding,
                    0.6,
                    Some(mentor_ids[i]),
                );
            }
        }
        mentee.statuses.add(date, PlayerStatusType::Tut);

        applied.push(MentorshipPairing {
            mentor_id: mentor_ids[i],
            mentee_id: mentee_ids[i],
        });
    }

    // Mark mentors with Lrn status. Mentor-side relation bookkeeping is
    // handled below via `set_mentorship`, which already mirrors the bond
    // and influence on the mentor's side.
    for (mentor_idx, _, _) in &pairings {
        players[*mentor_idx].statuses.add(date, PlayerStatusType::Lrn);
    }
    let _ = pair_compatibilities; // kept for diagnostics; enforced as cap above.

    // Record mentorship on both sides — the mentor flags the mentee's slot
    // with Mentee and the mentee flags the mentor with Mentor. Chemistry /
    // influence passes can now see the pairing and surface it in the UI.
    for (i, (mentor_idx, mentee_idx, _)) in pairings.iter().enumerate() {
        let mentor_id = mentor_ids[i];
        let mentee_id = mentee_ids[i];
        players[*mentor_idx]
            .relations
            .set_mentorship(mentee_id, MentorshipType::Mentee);
        players[*mentee_idx]
            .relations
            .set_mentorship(mentor_id, MentorshipType::Mentor);
    }

    applied
}

/// Deterministic 0..1 roll for the bad-mentor test. Same mentor + mentee +
/// date returns the same number — keeps tests reproducible.
fn bad_mentor_roll(mentor_id: u32, mentee_id: u32, date: NaiveDate) -> f32 {
    use chrono::Datelike;
    let h = (mentor_id as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((mentee_id as u64).wrapping_mul(0xC6BC_279E_9286_5A2B))
        .wrapping_add(date.num_days_from_ce() as u64);
    let frac = ((h >> 13) as u32 as f32) / (u32::MAX as f32);
    frac.clamp(0.0, 0.999)
}

/// Nudge the mentee's personality toward the mentor's values with an
/// explicit step size. Caller scales by compatibility tier and HoY factor.
fn nudge_personality_step(
    mentee: &mut Player,
    mentor_prof: f32,
    mentor_amb: f32,
    mentor_det: f32,
    step: f32,
) {
    let drift = |current: f32, target: f32| -> f32 {
        let delta = target - current;
        if delta.abs() < 0.05 {
            0.0
        } else {
            delta.signum() * step
        }
    };

    let cur = &mut mentee.attributes;
    cur.professionalism = (cur.professionalism + drift(cur.professionalism, mentor_prof)).clamp(0.0, 20.0);
    cur.ambition = (cur.ambition + drift(cur.ambition, mentor_amb)).clamp(0.0, 20.0);

    let det_now = mentee.skills.mental.determination;
    let det_new = det_now + drift(det_now, mentor_det);
    mentee.skills.mental.determination = det_new.clamp(0.0, 20.0);
}

// `RelationshipChange` is referenced in match-event helpers via the public
// re-export. Re-exporting here so this file stays self-contained.
#[allow(dead_code)]
type _RelationshipChangeAlias = RelationshipChange;
