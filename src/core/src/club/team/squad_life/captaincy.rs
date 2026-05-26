//! Captaincy assignment + magnitude tuning.
//!
//! Each monthly tick the squad is ranked by leadership × loyalty × tenure ×
//! reputation. The top scorer becomes captain, the runner-up vice. Captaincy
//! changes carry morale consequences: a stripped former captain takes a hit,
//! a new appointee gets a lift.
//!
//! Three guards keep the events realistic:
//! 1. The very first captain pick on a freshly-loaded team is silent
//!    (`captaincy_initialized` flag) — it's save-file setup, not a
//!    decision the player remembers.
//! 2. A 120-day cooldown on each emit type prevents recalculation
//!    oscillation from spamming armband-handover events.
//! 3. If the previous captain has left the squad (transfer / loan out /
//!    retirement), no `CaptaincyRemoved` is fired for them — the move
//!    itself is what unsettled them, not "stripping the armband" they no
//!    longer wear at this club.

use crate::club::team::Team;
use crate::utils::DateUtils;
use crate::{
    HappinessEventCause, HappinessEventContext, HappinessEventScope, HappinessEventSeverity,
    HappinessEventType, LeadershipEventContext, LeadershipEventKind, Player,
};
use chrono::{Datelike, NaiveDate};

/// Cooldown (days) on each captaincy event so monthly recalculation
/// oscillation around an evenly-matched leadership group doesn't spam
/// armband-handover narration.
const CAPTAINCY_EVENT_COOLDOWN_DAYS: u16 = 120;

/// Minimum leadership attribute (0..20 scale) required to be considered
/// for the captaincy ranking at all.
const MIN_LEADERSHIP_FOR_CAPTAINCY: f32 = 8.0;

pub struct CaptaincyAssigner;

impl CaptaincyAssigner {
    /// Rank the squad, pin the new captain/vice-captain, and emit the
    /// morale events for any handover.
    pub fn assign(team: &mut Team, date: NaiveDate) {
        let ranked = Self::rank_candidates(team, date);

        if ranked.is_empty() {
            team.captain_id = None;
            team.vice_captain_id = None;
            return;
        }

        let new_captain = ranked.first().map(|(id, _)| *id);
        let new_vice = ranked.get(1).map(|(id, _)| *id);
        let was_initialized = team.captaincy_initialized;

        if team.captain_id != new_captain && was_initialized {
            Self::emit_handover_events(team, team.captain_id, new_captain);
        }

        team.captain_id = new_captain;
        team.vice_captain_id = new_vice;
        team.captaincy_initialized = true;
    }

    /// Rank captaincy candidates by leadership × loyalty × tenure × reputation.
    /// Age bell curve peaks around 29-31.
    fn rank_candidates(team: &Team, date: NaiveDate) -> Vec<(u32, f32)> {
        let now_year = date.year();
        let mut ranked: Vec<(u32, f32)> = team
            .players
            .iter()
            .filter(|p| p.skills.mental.leadership >= MIN_LEADERSHIP_FOR_CAPTAINCY)
            .filter_map(|p| {
                let contract = p.contract.as_ref()?;
                let tenure_years = contract
                    .started
                    .map(|s| (now_year - s.year()).max(0) as f32)
                    .unwrap_or(0.0);
                let age = DateUtils::age(p.birth_date, date) as f32;
                let age_factor = if age < 23.0 {
                    0.5
                } else if age >= 23.0 && age <= 34.0 {
                    1.0 + ((age - 28.0).abs() * -0.05).max(-0.25)
                } else {
                    0.7
                };
                let score = p.skills.mental.leadership * 1.5
                    + p.attributes.loyalty * 0.8
                    + p.attributes.professionalism * 0.4
                    + tenure_years.min(10.0) * 0.6
                    + p.player_attributes.current_reputation as f32 / 2500.0;
                Some((p.id, score * age_factor))
            })
            .collect();

        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked
    }

    /// Emit `CaptaincyRemoved` for the outgoing captain (only if still in
    /// squad) and `CaptaincyAwarded` for the incoming one.
    fn emit_handover_events(team: &mut Team, old_captain: Option<u32>, new_captain: Option<u32>) {
        // A captain who left the club should not get a `CaptaincyRemoved`
        // event applied to their morale at their next club; the transfer
        // pipeline handles the move itself, and pinning a "stripped of
        // armband" event on a departed player would be doubly wrong.
        if let Some(old_id) = old_captain {
            if let Some(p) = team.players.players.iter_mut().find(|p| p.id == old_id) {
                let mag = CaptaincyMagnitude::removed(p);
                let lctx = LeadershipEventContext::new(LeadershipEventKind::CaptaincyRemoved)
                    .with_leadership_attribute(p.skills.mental.leadership);
                let happiness_ctx = HappinessEventContext::new(
                    HappinessEventCause::Other,
                    HappinessEventSeverity::from_magnitude(mag),
                    HappinessEventScope::DressingRoom,
                )
                .with_leadership_context(lctx);
                p.happiness.add_event_with_context_and_cooldown(
                    HappinessEventType::CaptaincyRemoved,
                    mag,
                    None,
                    happiness_ctx,
                    CAPTAINCY_EVENT_COOLDOWN_DAYS,
                );
            }
        }
        if let Some(new_id) = new_captain {
            if let Some(p) = team.players.players.iter_mut().find(|p| p.id == new_id) {
                let mag = CaptaincyMagnitude::awarded(p);
                let lctx = LeadershipEventContext::new(LeadershipEventKind::CaptaincyAwarded)
                    .with_leadership_attribute(p.skills.mental.leadership);
                let happiness_ctx = HappinessEventContext::new(
                    HappinessEventCause::Other,
                    HappinessEventSeverity::from_magnitude(mag),
                    HappinessEventScope::DressingRoom,
                )
                .with_leadership_context(lctx);
                p.happiness.add_event_with_context_and_cooldown(
                    HappinessEventType::CaptaincyAwarded,
                    mag,
                    None,
                    happiness_ctx,
                    CAPTAINCY_EVENT_COOLDOWN_DAYS,
                );
            }
        }
    }
}

/// Magnitude calculators for the captaincy morale events. Catalog defaults
/// shaped by the affected player's traits — leadership/loyalty/reputation
/// amplify positive lift, and reputation/temperament amplify the sting of
/// a public stripping (with professionalism dampening it).
pub struct CaptaincyMagnitude;

impl CaptaincyMagnitude {
    /// Magnitude for `CaptaincyAwarded`. Catalog default amplified by the
    /// player's leadership traits, loyalty, reputation, and tempered by
    /// age (a 19-year-old handed the armband feels it less viscerally
    /// than a 30-year-old club legend who's earned it). Returns a value
    /// near the catalog default (7.0) but in the band ~5..10.
    pub fn awarded(p: &Player) -> f32 {
        let cfg = crate::club::player::behaviour_config::HappinessConfig::default();
        let base = cfg.catalog.captaincy_awarded;
        let leadership_lift = (p.skills.mental.leadership.clamp(0.0, 20.0) / 20.0) * 0.30;
        let loyalty_lift = (p.attributes.loyalty.clamp(0.0, 20.0) / 20.0) * 0.20;
        // Reputation amplifier — a star getting the armband at a marquee
        // club feels it carry more weight (pressure plus prestige).
        let rep_lift =
            (p.player_attributes.current_reputation as f32 / 10_000.0).clamp(0.0, 1.0) * 0.20;
        let mul = (1.0 + leadership_lift + loyalty_lift + rep_lift).clamp(0.7, 1.6);
        base * mul
    }

    /// Magnitude for `CaptaincyRemoved`. Catalog default (-7.0) amplified
    /// by reputation and reactive personality (controversy / low
    /// temperament read this as a public humiliation), softened by
    /// professionalism (high-pro players keep it together).
    pub fn removed(p: &Player) -> f32 {
        let cfg = crate::club::player::behaviour_config::HappinessConfig::default();
        let base = cfg.catalog.captaincy_removed;
        let rep_amp = crate::club::player::events::scaling::reputation_amplifier(
            p.player_attributes.current_reputation,
        );
        let provoke_amp = crate::club::player::events::scaling::criticism_amplifier(
            p.attributes.controversy,
            p.attributes.temperament,
        );
        let prof_dampen =
            crate::club::player::events::scaling::criticism_dampener(p.attributes.professionalism);
        // base is negative; multiplying by these factors keeps the sign.
        base * rep_amp * provoke_amp * prof_dampen
    }
}
