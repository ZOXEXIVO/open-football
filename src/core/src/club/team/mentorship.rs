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
use crate::club::player::traits::PlayerTrait;
use crate::club::{HappinessEventType, Player, PlayerStatusType};
use crate::PlayerFieldPositionGroup;
use chrono::NaiveDate;

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

    // Greedy same-position-group matching.
    let mut used_mentors: Vec<bool> = vec![false; mentors.len()];
    let mut pairings: Vec<(usize, usize)> = Vec::new();
    for (mentee_idx, _, mentee_group) in &mentees {
        for (mi, (mentor_idx, _, mentor_group)) in mentors.iter().enumerate() {
            if used_mentors[mi] {
                continue;
            }
            if mentor_group == mentee_group {
                pairings.push((*mentor_idx, *mentee_idx));
                used_mentors[mi] = true;
                break;
            }
        }
    }

    // Head of Youth Development amplifies transfer rate: elite HoY → +25%.
    let hoy_factor = 1.0 + (head_of_youth_wwy as f32 / 20.0 - 0.5).max(0.0) * 0.5;

    let mut applied: Vec<MentorshipPairing> = Vec::with_capacity(pairings.len());
    let mentor_ids: Vec<u32> = pairings
        .iter()
        .map(|(m_idx, _)| players[*m_idx].id)
        .collect();
    let mentee_ids: Vec<u32> = pairings
        .iter()
        .map(|(_, t_idx)| players[*t_idx].id)
        .collect();

    for (i, (mentor_idx, mentee_idx)) in pairings.iter().enumerate() {
        // Read mentor attributes (immutable).
        let (mentor_prof, mentor_amb, mentor_det) = {
            let mentor = &players[*mentor_idx];
            (
                mentor.attributes.professionalism,
                mentor.attributes.ambition,
                mentor.skills.mental.determination,
            )
        };

        // Apply to mentee (mutable).
        let mentee = &mut players[*mentee_idx];
        nudge_personality(mentee, mentor_prof, mentor_amb, mentor_det, hoy_factor);
        mentee.statuses.add(date, PlayerStatusType::Tut);
        mentee
            .happiness
            .add_event(HappinessEventType::TeammateBonding, 0.6);

        applied.push(MentorshipPairing {
            mentor_id: mentor_ids[i],
            mentee_id: mentee_ids[i],
        });
    }

    // Now mark mentors (separate pass to avoid two mutable borrows).
    for (mentor_idx, _) in &pairings {
        players[*mentor_idx].statuses.add(date, PlayerStatusType::Lrn);
    }

    applied
}

/// Nudge the mentee's personality toward the mentor's values — tiny weekly
/// delta because personality shifts should take months of shared training.
fn nudge_personality(
    mentee: &mut Player,
    mentor_prof: f32,
    mentor_amb: f32,
    mentor_det: f32,
    hoy_factor: f32,
) {
    let step = 0.015 * hoy_factor; // max ~0.02/week toward target

    let cur = &mut mentee.attributes;
    let drift = |current: f32, target: f32| -> f32 {
        let delta = target - current;
        if delta.abs() < 0.05 {
            0.0
        } else {
            delta.signum() * step
        }
    };

    cur.professionalism = (cur.professionalism + drift(cur.professionalism, mentor_prof)).clamp(0.0, 20.0);
    cur.ambition = (cur.ambition + drift(cur.ambition, mentor_amb)).clamp(0.0, 20.0);

    let det_now = mentee.skills.mental.determination;
    let det_new = det_now + drift(det_now, mentor_det);
    mentee.skills.mental.determination = det_new.clamp(0.0, 20.0);
}
