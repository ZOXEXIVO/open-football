//! Captaincy assignment + magnitude tuning.
//!
//! Each monthly tick the squad is scored on a realistic, deterministic
//! captaincy model (see [`CaptaincyModel`]) blending four blocks —
//! dressing-room leadership, club authority, reliability and seniority/fit —
//! on a 0..100 scale, then nudged by hard situational penalties (transfer
//! request, loan, long-term injury, …). The best eligible leader becomes
//! captain, the runner-up vice.
//!
//! Crucially the model is *sticky*. A real club does not hand the armband to
//! whoever edged 0.2 points ahead this month; captaincy is an earned,
//! dressing-room appointment that changes only on a clear sporting or
//! discipline reason. The incumbent is therefore retained unless a challenger
//! clears a hysteresis margin ([`CAPTAIN_HYSTERESIS`]) or the incumbent has a
//! disqualifying event (transfer request/listing, serious injury, leadership
//! collapse, or losing his place). See [`CaptaincyModel::select`].
//!
//! Every official captaincy write goes through the single chokepoint
//! [`CaptaincyAssigner::set_official_captain`] — never assign
//! `Team::captain_id` directly. The matchday armband (`matchday_leadership`)
//! is a separate, event-free concern and does not call into this module.
//!
//! The first appointment is narrated like any other: a fresh team starts
//! with no captain (`captain_id == None`, never persisted), so the first
//! monthly pick fires a `CaptaincyAwarded` the player can see on their
//! events page. Two guards keep the narration realistic:
//! 1. A 120-day cooldown on each emit type prevents recalculation
//!    oscillation from spamming armband-handover events.
//! 2. If the previous captain has left the squad (transfer / loan out /
//!    retirement), no `CaptaincyRemoved` is fired for them — the move
//!    itself is what unsettled them, not "stripping the armband" they no
//!    longer wear at this club.

use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::events::scaling::{
    criticism_amplifier, criticism_dampener, reputation_amplifier,
};
use crate::club::team::Team;
use crate::utils::DateUtils;
use crate::{
    ContractType, HappinessEventCause, HappinessEventContext, HappinessEventScope,
    HappinessEventSeverity, HappinessEventType, LeadershipEventContext, LeadershipEventKind,
    Player, PlayerPositionType, PlayerSquadStatus, PlayerStatusType,
};
use chrono::NaiveDate;
use std::cmp::Ordering;
use std::collections::HashMap;

/// Cooldown (days) on each captaincy event so monthly recalculation
/// oscillation around an evenly-matched leadership group doesn't spam
/// armband-handover narration.
const CAPTAINCY_EVENT_COOLDOWN_DAYS: u16 = 120;

/// Minimum leadership attribute (0..20 scale) required to be considered
/// for the captaincy ranking at all.
const MIN_LEADERSHIP_FOR_CAPTAINCY: f32 = 8.0;

// ---------------------------------------------------------------------------
// Final-blend block weights (sum = 1.0). Each block is scored on 0..100 and
// blended here; situational penalties are then added in absolute points.
// ---------------------------------------------------------------------------
const W_CORE_LEADERSHIP: f32 = 0.42;
const W_CLUB_AUTHORITY: f32 = 0.28;
const W_RELIABILITY: f32 = 0.16;
const W_SENIORITY_FIT: f32 = 0.14;

// ---------------------------------------------------------------------------
// Hysteresis. Real clubs rarely change captains monthly: the armband is an
// earned, season-long appointment, so a challenger must be *clearly* better
// (not a rounding-error better) to displace a sitting captain. Without this,
// a model that merely scores well still fails realism — it would re-hand the
// armband every time someone nudged ahead by a fraction of a point.
// ---------------------------------------------------------------------------
const CAPTAIN_HYSTERESIS: f32 = 8.0;
const VICE_HYSTERESIS: f32 = 5.0;

// ---------------------------------------------------------------------------
// Eligibility thresholds.
// ---------------------------------------------------------------------------
/// Youth-status player can only captain seniors if genuinely exceptional.
const YOUTH_EXCEPTION_LEADERSHIP: f32 = 17.0;
const YOUTH_EXCEPTION_PROFESSIONALISM: f32 = 15.0;
/// Top fraction of squad current-ability the youth exception demands.
const YOUTH_EXCEPTION_CA_TOP_FRACTION: f32 = 0.15;
/// A loan ending within this window counts as a short-term loan-in, which is
/// excluded from the primary pool (a six-month visitor isn't a club captain).
const SHORT_TERM_LOAN_DAYS: i64 = 182;
/// Long-term injury threshold (days remaining) for the hard penalty / the
/// "serious injury" incumbent-replacement trigger.
const LONG_TERM_INJURY_DAYS: u16 = 30;
/// Contract running down inside this window dampens authority.
const CONTRACT_EXPIRY_SOON_DAYS: i64 = 180;
/// Joined inside this window = not yet embedded in the dressing room.
const RECENT_SIGNING_DAYS: i64 = 90;
/// Enough squad starts logged that the season is genuinely under way, so an
/// incumbent on zero starts has demonstrably lost his place.
const LOST_PLACE_MIN_SQUAD_STARTS: u16 = 5;

// ---------------------------------------------------------------------------
// Hard situational penalties (absolute points on the 0..100 scale). These are
// dampeners, not bans: a marginal squad with no clean candidate can still end
// up with a transfer-listed captain rather than none.
// ---------------------------------------------------------------------------
const PEN_LONG_TERM_INJURY: f32 = -12.0;
const PEN_SUSPENDED: f32 = -6.0;
const PEN_TRANSFER_LISTED: f32 = -18.0;
const PEN_TRANSFER_REQUEST: f32 = -22.0;
const PEN_AGREED_TRANSFER: f32 = -22.0;
const PEN_UNHAPPY: f32 = -10.0;
const PEN_LOANED_IN: f32 = -20.0;
const PEN_CONTRACT_EXPIRING: f32 = -10.0;
const PEN_RECENT_SIGNING: f32 = -12.0;

pub struct CaptaincyAssigner;

impl CaptaincyAssigner {
    /// Monthly reappointment entry point. Score the squad with the realistic
    /// [`CaptaincyModel`], then route the resulting captain / vice through the
    /// [`set_official_captain`] chokepoint so any genuine handover narrates
    /// its morale events. Incumbents are retained unless a challenger clears
    /// the hysteresis margin or the incumbent is disqualified — captaincy is
    /// stable by design.
    ///
    /// An empty ranking (no eligible leaders) resolves to `None` and still
    /// flows through the chokepoint, so a captain who drops below the
    /// eligibility threshold — or whose squad loses every qualifying
    /// leader — is properly stripped rather than silently cleared.
    ///
    /// [`set_official_captain`]: CaptaincyAssigner::set_official_captain
    pub fn assign(team: &mut Team, date: NaiveDate) {
        let model = CaptaincyModel::new(team, date);
        let ranked = model.ranked();

        let new_captain = model.select(&ranked, team.captain_id, CAPTAIN_HYSTERESIS);

        let new_vice = match new_captain {
            Some(cap) => {
                let pool: Vec<ScoredCandidate> =
                    ranked.iter().copied().filter(|c| c.id != cap).collect();
                let incumbent_vice = team.vice_captain_id.filter(|v| *v != cap);
                model.select(&pool, incumbent_vice, VICE_HYSTERESIS)
            }
            None => None,
        };

        Self::set_official_captain(team, new_captain, new_vice);
    }

    /// The single safe chokepoint for writing the official club captaincy.
    /// Every official captain write must go through here — never assign
    /// `Team::captain_id` directly.
    ///
    /// A genuine change emits `CaptaincyRemoved` for the displaced captain
    /// (only if still in the squad — see [`emit_handover_events`]) and
    /// `CaptaincyAwarded` for the incoming one, both subject to the per-type
    /// cooldown. An unchanged captain emits nothing, so repeated monthly
    /// reviews never duplicate. The first appointment on a fresh team
    /// (`captain_id` starts `None`) therefore fires a single visible award.
    ///
    /// Vice-captaincy changes ride the same captaincy event types at
    /// reduced magnitude ([`Self::VICE_MAGNITUDE_SCALE`]) — the deputy
    /// role is a real status, but quieter than the armband itself.
    /// Matchday armband resolution (`matchday_leadership`) is a
    /// separate concern that never calls here.
    ///
    /// [`emit_handover_events`]: CaptaincyAssigner::emit_handover_events
    pub fn set_official_captain(team: &mut Team, new_captain: Option<u32>, new_vice: Option<u32>) {
        if team.captain_id != new_captain {
            Self::emit_handover_events(team, team.captain_id, new_captain);
        }
        if team.vice_captain_id != new_vice {
            Self::emit_vice_handover_events(team, team.vice_captain_id, new_vice, new_captain);
        }

        team.captain_id = new_captain;
        team.vice_captain_id = new_vice;
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

    /// Vice events are quieter than the full armband but real: the
    /// deputy role is club status. Scale keeps them clearly below a
    /// captaincy change in the morale ledger.
    const VICE_MAGNITUDE_SCALE: f32 = 0.4;

    /// Emit reduced-magnitude captaincy events for a vice-captaincy
    /// change. Suppressed where a captain-level event already tells
    /// the bigger story: a vice promoted to captain only gets the
    /// award, a captain demoted to vice only gets the removal. Called
    /// before the id fields are updated, so `team.captain_id` still
    /// holds the outgoing captain.
    fn emit_vice_handover_events(
        team: &mut Team,
        old_vice: Option<u32>,
        new_vice: Option<u32>,
        new_captain: Option<u32>,
    ) {
        let old_captain = team.captain_id;
        if let Some(old_id) = old_vice {
            if Some(old_id) != new_captain {
                if let Some(p) = team.players.players.iter_mut().find(|p| p.id == old_id) {
                    let mag = CaptaincyMagnitude::removed(p) * Self::VICE_MAGNITUDE_SCALE;
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
        }
        if let Some(new_id) = new_vice {
            if Some(new_id) != old_captain {
                if let Some(p) = team.players.players.iter_mut().find(|p| p.id == new_id) {
                    let mag = CaptaincyMagnitude::awarded(p) * Self::VICE_MAGNITUDE_SCALE;
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
}

/// A scored captaincy candidate. Carries the score plus the raw signals the
/// deterministic tie-breaks consult, so ranking never depends on iteration
/// order or floating-point noise.
#[derive(Debug, Clone, Copy)]
struct ScoredCandidate {
    id: u32,
    score: f32,
    leadership: f32,
    tenure_years: f32,
    squad_status_rank: u8,
    reputation: i16,
    age: u8,
}

/// The realistic captaincy scorer. Borrows the squad and precomputes the
/// squad-relative context (start/appearance leaders, current-ability spread,
/// nationality blocs) once, so every candidate is scored against the same
/// denominators.
struct CaptaincyModel<'a> {
    team: &'a Team,
    date: NaiveDate,
    max_starts: u16,
    max_apps: u16,
    min_ca: u8,
    max_ca: u8,
    ca_values: Vec<u8>,
    nationality_counts: HashMap<u32, usize>,
    squad_size: usize,
}

impl<'a> CaptaincyModel<'a> {
    fn new(team: &'a Team, date: NaiveDate) -> Self {
        let mut max_starts = 0u16;
        let mut max_apps = 0u16;
        let mut min_ca = u8::MAX;
        let mut max_ca = u8::MIN;
        let mut ca_values = Vec::with_capacity(team.players.players.len());
        let mut nationality_counts: HashMap<u32, usize> = HashMap::new();

        for p in team.players.iter() {
            let starts = p.statistics.played;
            let apps = p.statistics.played + p.statistics.played_subs;
            max_starts = max_starts.max(starts);
            max_apps = max_apps.max(apps);

            let ca = p.player_attributes.current_ability;
            min_ca = min_ca.min(ca);
            max_ca = max_ca.max(ca);
            ca_values.push(ca);

            *nationality_counts.entry(p.country_id).or_insert(0) += 1;
        }

        CaptaincyModel {
            team,
            date,
            max_starts,
            max_apps,
            min_ca: min_ca.min(max_ca),
            max_ca,
            ca_values,
            nationality_counts,
            squad_size: team.players.players.len(),
        }
    }

    /// The ranked candidate pool, best first. Prefers the *primary* pool of
    /// fully clean candidates; only if that is empty does it fall back to the
    /// base-eligible set (so a squad of only transfer-listed / on-loan
    /// leaders still gets a captain rather than none).
    fn ranked(&self) -> Vec<ScoredCandidate> {
        let mut primary: Vec<ScoredCandidate> = self
            .team
            .players
            .iter()
            .filter(|p| self.is_primary_eligible(p))
            .map(|p| self.evaluate(p))
            .collect();

        let mut pool = if primary.is_empty() {
            self.team
                .players
                .iter()
                .filter(|p| self.is_base_eligible(p))
                .map(|p| self.evaluate(p))
                .collect()
        } else {
            std::mem::take(&mut primary)
        };

        pool.sort_by(Self::cmp_candidates);
        pool
    }

    /// Resolve one armband slot under hysteresis. A sitting holder keeps the
    /// role unless he has left the pool (ineligible / departed), has a
    /// disqualifying event, or the top challenger clears the margin.
    fn select(
        &self,
        candidates: &[ScoredCandidate],
        incumbent: Option<u32>,
        hysteresis: f32,
    ) -> Option<u32> {
        let challenger = candidates.first()?;

        match incumbent.and_then(|id| candidates.iter().find(|c| c.id == id)) {
            // No incumbent, or the incumbent is no longer in the pool (lost
            // eligibility / left the squad) — appoint the best challenger.
            None => Some(challenger.id),
            Some(inc) => {
                if self.must_replace_incumbent(inc.id) {
                    return Some(challenger.id);
                }
                if challenger.score >= inc.score + hysteresis {
                    Some(challenger.id)
                } else {
                    Some(inc.id)
                }
            }
        }
    }

    /// Hard reasons an incumbent loses the armband regardless of hysteresis:
    /// he wants out, is being sold, is seriously hurt, his leadership has
    /// collapsed, or he has clearly lost his starting place.
    fn must_replace_incumbent(&self, id: u32) -> bool {
        let Some(p) = self.team.players.iter().find(|p| p.id == id) else {
            return true;
        };
        if p.skills.mental.leadership < MIN_LEADERSHIP_FOR_CAPTAINCY {
            return true;
        }
        if self.is_transfer_listed(p) || self.has_transfer_request(p) || self.has_agreed_transfer(p)
        {
            return true;
        }
        if self.is_long_term_injured(p) {
            return true;
        }
        // Lost his place: nothing started all season while the squad is
        // demonstrably playing matches.
        p.statistics.played == 0 && self.max_starts >= LOST_PLACE_MIN_SQUAD_STARTS
    }

    // -- Eligibility ----------------------------------------------------------

    /// Base bar: under contract, leadership ≥ floor, not retired.
    fn is_base_eligible(&self, p: &Player) -> bool {
        p.contract.is_some()
            && p.skills.mental.leadership >= MIN_LEADERSHIP_FOR_CAPTAINCY
            && !p.is_retired()
    }

    /// Clean candidate: base-eligible and free of the disqualifiers that only
    /// relax when no clean candidate exists (transfer flux, short-term loan,
    /// youth status without exceptional standing).
    fn is_primary_eligible(&self, p: &Player) -> bool {
        self.is_base_eligible(p)
            && !self.is_transfer_listed(p)
            && !self.has_transfer_request(p)
            && !self.has_agreed_transfer(p)
            && !self.is_short_term_loan(p)
            && !(self.is_loaned_in_candidate(p) && self.has_permanent_base_eligible_leader())
            && (!self.is_youth_only(p) || self.clears_youth_exception(p))
    }

    /// Transfer-listed by the contract flag *or* the `Lst` squad status —
    /// either way the club is shopping him, so he's no clean captain.
    fn is_transfer_listed(&self, p: &Player) -> bool {
        p.contract
            .as_ref()
            .map(|c| c.is_transfer_listed)
            .unwrap_or(false)
            || p.statuses.has(PlayerStatusType::Lst)
    }

    fn has_transfer_request(&self, p: &Player) -> bool {
        p.statuses.has(PlayerStatusType::Req)
    }

    /// He has agreed a move to another club (`Trn`) and is gone the moment
    /// the window opens — not a credible long-term captain.
    fn has_agreed_transfer(&self, p: &Player) -> bool {
        p.statuses.has(PlayerStatusType::Trn)
    }

    fn is_unhappy(&self, p: &Player) -> bool {
        p.statuses.has(PlayerStatusType::Unh)
    }

    fn is_long_term_injured(&self, p: &Player) -> bool {
        p.player_attributes.is_injured
            && p.player_attributes.injury_days_remaining > LONG_TERM_INJURY_DAYS
    }

    fn is_short_term_loan(&self, p: &Player) -> bool {
        if !p.is_on_loan() {
            return false;
        }
        // A loan with little time left (or with no end date on record) is a
        // short-term visitor, never a club captain.
        p.contract_loan
            .as_ref()
            .map(|c| (c.expiration - self.date).num_days() <= SHORT_TERM_LOAN_DAYS)
            .unwrap_or(true)
    }

    /// Physically here on a loan agreement (borrowing-side view). While the
    /// club has a permanent leader to turn to, a visitor — however good —
    /// isn't the club captain.
    fn is_loaned_in_candidate(&self, p: &Player) -> bool {
        p.contract_loan.is_some()
    }

    /// Does the squad hold at least one permanent (non-loaned-in)
    /// base-eligible leader? If so, loaned-in candidates stay out of the
    /// primary pool; if not, the fallback may still reach for a loanee so
    /// a squad of only borrowed leaders is captained rather than left bare.
    fn has_permanent_base_eligible_leader(&self) -> bool {
        self.team
            .players
            .iter()
            .any(|p| self.is_base_eligible(p) && !self.is_loaned_in_candidate(p))
    }

    fn is_youth_only(&self, p: &Player) -> bool {
        let Some(c) = p.contract.as_ref() else {
            return false;
        };
        matches!(
            c.squad_status,
            PlayerSquadStatus::DecentYoungster | PlayerSquadStatus::HotProspectForTheFuture
        ) || c.contract_type == ContractType::Youth
    }

    fn clears_youth_exception(&self, p: &Player) -> bool {
        p.skills.mental.leadership >= YOUTH_EXCEPTION_LEADERSHIP
            && p.attributes.professionalism >= YOUTH_EXCEPTION_PROFESSIONALISM
            && self.ca_top_fraction(p) <= YOUTH_EXCEPTION_CA_TOP_FRACTION
    }

    /// Fraction of the squad with strictly higher current ability — 0.0 means
    /// the best in the squad, so "top 15%" is `<= 0.15`.
    fn ca_top_fraction(&self, p: &Player) -> f32 {
        if self.squad_size == 0 {
            return 1.0;
        }
        let ca = p.player_attributes.current_ability;
        let better = self.ca_values.iter().filter(|&&c| c > ca).count();
        better as f32 / self.squad_size as f32
    }

    // -- Scoring --------------------------------------------------------------

    fn evaluate(&self, p: &Player) -> ScoredCandidate {
        let score = (self.core_leadership(p) * W_CORE_LEADERSHIP
            + self.club_authority(p) * W_CLUB_AUTHORITY
            + self.reliability(p) * W_RELIABILITY
            + self.seniority_fit(p) * W_SENIORITY_FIT
            + self.penalties(p))
        .clamp(0.0, 100.0);

        ScoredCandidate {
            id: p.id,
            score,
            leadership: p.skills.mental.leadership,
            tenure_years: self.tenure_years(p),
            squad_status_rank: Self::squad_status_rank(p),
            reputation: p.player_attributes.current_reputation,
            age: DateUtils::age(p.birth_date, self.date),
        }
    }

    /// Dressing-room authority earned through character and on-pitch
    /// leadership traits. 0..100.
    fn core_leadership(&self, p: &Player) -> f32 {
        let m = &p.skills.mental;
        let a = &p.attributes;
        100.0
            * (norm(m.leadership) * 0.30
                + norm(a.professionalism) * 0.13
                + norm(m.teamwork) * 0.11
                + norm(m.determination) * 0.10
                + norm(m.composure) * 0.09
                + norm(a.pressure) * 0.08
                + norm(m.decisions) * 0.06
                + norm(a.consistency) * 0.05
                + norm(a.important_matches) * 0.04
                + norm(a.temperament) * 0.025
                + norm(a.sportsmanship) * 0.015)
    }

    /// Standing within the club: tenure, squad importance, playing share,
    /// reputation, quality rank and loyalty. 0..100.
    fn club_authority(&self, p: &Player) -> f32 {
        let tenure = (self.tenure_years(p) / 6.0).min(1.0);
        let status = Self::squad_status_factor(p);
        let starts = p.statistics.played as f32 / self.max_starts.max(1) as f32;
        let apps =
            (p.statistics.played + p.statistics.played_subs) as f32 / self.max_apps.max(1) as f32;
        let reputation = (p.player_attributes.current_reputation.max(0) as f32 / 10_000.0)
            .sqrt()
            .clamp(0.0, 1.0);
        let ca_rank = self.ca_rank_factor(p);
        let loyalty = norm(p.attributes.loyalty);

        100.0
            * (tenure * 0.24
                + status * 0.18
                + starts.clamp(0.0, 1.0) * 0.16
                + apps.clamp(0.0, 1.0) * 0.10
                + reputation * 0.12
                + ca_rank * 0.10
                + loyalty * 0.10)
    }

    /// Can he be trusted to deliver week-in week-out and stay out of trouble.
    /// 0..100.
    fn reliability(&self, p: &Player) -> f32 {
        // No meaningful sample yet (pre-season / new arrival) reads as neutral
        // rather than as a damning 0, so it doesn't distort the ranking. The
        // sample-size-regressed rating keeps a small-sample 8.2 from reading
        // as flawless reliability.
        let rating = p
            .statistics
            .average_rating_realistic(p.position().position_group());
        let rating_factor = if rating < 1.0 {
            0.5
        } else {
            ((rating - 6.2) / (7.4 - 6.2)).clamp(0.0, 1.0)
        };
        let condition = p.player_attributes.condition_percentage() as f32 / 100.0;
        let low_jadedness =
            (1.0 - p.player_attributes.jadedness.max(0) as f32 / 10_000.0).clamp(0.0, 1.0);
        let discipline = self.low_discipline_risk(p);
        // Three behaviour tiers (Good / Normal / Poor) → 1.0 / 0.65 / 0.15.
        let behaviour = if p.behaviour.is_good() {
            1.0
        } else if p.behaviour.is_poor() {
            0.15
        } else {
            0.65
        };

        100.0
            * (rating_factor * 0.35
                + condition.clamp(0.0, 1.0) * 0.15
                + low_jadedness * 0.10
                + discipline * 0.20
                + behaviour * 0.20)
    }

    /// Low discipline risk: high temperament & sportsmanship, low dirtiness,
    /// and a clean card record relative to games played. 0..1.
    fn low_discipline_risk(&self, p: &Player) -> f32 {
        let temperament = norm(p.attributes.temperament);
        let sportsmanship = norm(p.attributes.sportsmanship);
        let low_dirtiness = 1.0 - norm(p.attributes.dirtiness);
        let apps = (p.statistics.played + p.statistics.played_subs).max(5) as f32;
        let card_load =
            (p.statistics.yellow_cards as f32 + p.statistics.red_cards as f32 * 3.0) / apps;
        let card_clean = (1.0 - card_load).clamp(0.0, 1.0);

        temperament * 0.30 + sportsmanship * 0.20 + low_dirtiness * 0.25 + card_clean * 0.25
    }

    /// Age profile, positional convention, social integration and overall
    /// dressing-room stability. 0..100.
    fn seniority_fit(&self, p: &Player) -> f32 {
        let age = Self::age_curve(DateUtils::age(p.birth_date, self.date) as f32);
        let position = Self::position_factor(p.position());
        // Prefer the real squad social snapshot (compatriot / shared-language
        // teammate counts) when the weekly pre-tick has built one — a leader
        // the dressing room can actually talk to integrates it. Fall back to
        // the nationality-bloc share and the adaptability proxy otherwise.
        let (nationality, language) = match p.squad_social_view.as_ref() {
            Some(view) => (
                (view.same_nationality_teammates as f32 / 5.0).min(1.0),
                (view.same_language_teammates as f32 / 5.0).min(1.0),
            ),
            None => (
                self.nationality_integration(p),
                norm(p.attributes.adaptability),
            ),
        };
        let stability = 0.40 * (1.0 - norm(p.attributes.controversy))
            + 0.30 * norm(p.attributes.loyalty)
            + 0.30 * norm(p.attributes.professionalism);

        100.0
            * (age * 0.35
                + position * 0.15
                + nationality * 0.10
                + language * 0.10
                + stability * 0.30)
    }

    /// Sum of hard situational penalties (absolute points, all ≤ 0).
    fn penalties(&self, p: &Player) -> f32 {
        let mut pen = 0.0;
        if self.is_long_term_injured(p) {
            pen += PEN_LONG_TERM_INJURY;
        }
        if p.player_attributes.is_banned {
            pen += PEN_SUSPENDED;
        }
        if self.is_transfer_listed(p) {
            pen += PEN_TRANSFER_LISTED;
        }
        if self.has_transfer_request(p) {
            pen += PEN_TRANSFER_REQUEST;
        }
        if self.has_agreed_transfer(p) {
            pen += PEN_AGREED_TRANSFER;
        }
        if self.is_unhappy(p) {
            pen += PEN_UNHAPPY;
        }
        if p.is_on_loan() {
            pen += PEN_LOANED_IN;
        }
        if let Some(c) = p.contract.as_ref() {
            if (c.expiration - self.date).num_days() <= CONTRACT_EXPIRY_SOON_DAYS {
                pen += PEN_CONTRACT_EXPIRING;
            }
            if let Some(started) = c.started {
                if (self.date - started).num_days() < RECENT_SIGNING_DAYS {
                    pen += PEN_RECENT_SIGNING;
                }
            }
        }
        pen
    }

    // -- Squad-relative helpers ----------------------------------------------

    fn tenure_years(&self, p: &Player) -> f32 {
        p.contract
            .as_ref()
            .and_then(|c| c.started)
            .map(|s| ((self.date - s).num_days() as f32 / 365.25).max(0.0))
            .unwrap_or(0.0)
    }

    /// 1.0 for the highest current ability in the squad, 0.0 for the lowest.
    fn ca_rank_factor(&self, p: &Player) -> f32 {
        if self.max_ca <= self.min_ca {
            return 1.0;
        }
        let ca = p.player_attributes.current_ability;
        ((ca.saturating_sub(self.min_ca)) as f32 / (self.max_ca - self.min_ca) as f32)
            .clamp(0.0, 1.0)
    }

    /// How embedded a player is in the squad's dominant nationality bloc. A
    /// homegrown-majority leader integrates the dressing room; an isolated
    /// nationality scores low. Self-contained (no club-country lookup needed).
    fn nationality_integration(&self, p: &Player) -> f32 {
        if self.squad_size == 0 {
            return 0.0;
        }
        let same = self
            .nationality_counts
            .get(&p.country_id)
            .copied()
            .unwrap_or(1);
        let share = same as f32 / self.squad_size as f32;
        (share / 0.5).min(1.0)
    }

    // -- Static lookup tables -------------------------------------------------

    fn squad_status_factor(p: &Player) -> f32 {
        match p.contract.as_ref().map(|c| &c.squad_status) {
            Some(PlayerSquadStatus::KeyPlayer) => 1.00,
            Some(PlayerSquadStatus::FirstTeamRegular) => 0.85,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 0.60,
            Some(PlayerSquadStatus::MainBackupPlayer) => 0.35,
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 0.25,
            Some(PlayerSquadStatus::DecentYoungster) => 0.20,
            Some(PlayerSquadStatus::NotNeeded) => 0.05,
            _ => 0.10,
        }
    }

    fn squad_status_rank(p: &Player) -> u8 {
        match p.contract.as_ref().map(|c| &c.squad_status) {
            Some(PlayerSquadStatus::KeyPlayer) => 6,
            Some(PlayerSquadStatus::FirstTeamRegular) => 5,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 4,
            Some(PlayerSquadStatus::MainBackupPlayer) => 3,
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 2,
            Some(PlayerSquadStatus::DecentYoungster) => 1,
            _ => 0,
        }
    }

    /// Captaincy age curve: callow at the extremes, peak authority 25–31.
    fn age_curve(age: f32) -> f32 {
        if age < 22.0 {
            0.25
        } else if age < 25.0 {
            0.55
        } else if age < 32.0 {
            1.00
        } else if age < 36.0 {
            0.85
        } else {
            0.55
        }
    }

    /// Positional captaincy convention: spine roles (GK, central defence, DM,
    /// central midfield) see the whole pitch and lead naturally; wide and
    /// advanced roles less so.
    fn position_factor(pos: PlayerPositionType) -> f32 {
        use PlayerPositionType::*;
        match pos {
            Goalkeeper => 1.00,
            Sweeper | DefenderCenterLeft | DefenderCenter | DefenderCenterRight => 1.00,
            DefensiveMidfielder => 1.00,
            MidfielderCenterLeft | MidfielderCenter | MidfielderCenterRight => 1.00,
            DefenderLeft | DefenderRight | WingbackLeft | WingbackRight => 0.85,
            AttackingMidfielderLeft | AttackingMidfielderCenter | AttackingMidfielderRight => 0.65,
            MidfielderLeft | MidfielderRight => 0.65,
            Striker | ForwardLeft | ForwardCenter | ForwardRight => 0.75,
        }
    }

    /// Deterministic best-first ordering: score, then the fixed tie-break
    /// chain (leadership, tenure, squad status, reputation, age, lower id).
    fn cmp_candidates(a: &ScoredCandidate, b: &ScoredCandidate) -> Ordering {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                b.leadership
                    .partial_cmp(&a.leadership)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| {
                b.tenure_years
                    .partial_cmp(&a.tenure_years)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| b.squad_status_rank.cmp(&a.squad_status_rank))
            .then_with(|| b.reputation.cmp(&a.reputation))
            .then_with(|| b.age.cmp(&a.age))
            .then_with(|| a.id.cmp(&b.id))
    }
}

/// Normalise a 0..20 attribute to a 0..1 factor.
fn norm(attr: f32) -> f32 {
    (attr / 20.0).clamp(0.0, 1.0)
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
        let cfg = HappinessConfig::default();
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
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.captaincy_removed;
        let rep_amp = reputation_amplifier(p.player_attributes.current_reputation);
        let provoke_amp = criticism_amplifier(p.attributes.controversy, p.attributes.temperament);
        let prof_dampen = criticism_dampener(p.attributes.professionalism);
        // base is negative; multiplying by these factors keeps the sign.
        base * rep_amp * provoke_amp * prof_dampen
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptaincyAssigner, CaptaincyModel};
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::core::player::SquadSocialView;
    use crate::club::team::squad_life::matchday_leadership::{
        LeadershipCandidate, MatchdayLeadership,
    };
    use crate::shared::fullname::FullName;
    use crate::{
        HappinessEventType, PersonAttributes, Player, PlayerAttributes, PlayerClubContract,
        PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
        PlayerSquadStatus, PlayerStatusType, StaffCollection, Team, TeamBuilder, TeamReputation,
        TeamType, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn today() -> NaiveDate {
        day(2025, 6, 1)
    }

    fn training() -> TrainingSchedule {
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        )
    }

    /// A clean, well-rounded captaincy candidate: peak age, long tenure, key
    /// player, full availability. Tests tweak only the one signal under test.
    fn strong(id: u32, leadership: f32) -> Player {
        let mut p = PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".to_string(), format!("P{}", id)))
            .birth_date(day(1995, 1, 1)) // 30 at `today()` → peak age band
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();

        p.skills.mental.leadership = leadership;
        p.skills.mental.teamwork = 14.0;
        p.skills.mental.determination = 14.0;
        p.skills.mental.composure = 14.0;
        p.skills.mental.decisions = 14.0;
        p.skills.mental.concentration = 14.0;

        p.attributes.professionalism = 15.0;
        p.attributes.loyalty = 15.0;
        p.attributes.pressure = 14.0;
        p.attributes.consistency = 14.0;
        p.attributes.important_matches = 14.0;
        p.attributes.temperament = 15.0;
        p.attributes.sportsmanship = 14.0;
        p.attributes.controversy = 3.0;
        p.attributes.adaptability = 14.0;
        p.attributes.dirtiness = 4.0;

        p.player_attributes.current_ability = 150;
        p.player_attributes.current_reputation = 6000;
        p.player_attributes.condition = 10_000;
        p.player_attributes.jadedness = 0;

        p.statistics.played = 28;
        p.statistics.average_rating = 7.2;

        let mut c = PlayerClubContract::new(50_000, day(2030, 6, 30));
        c.started = Some(day(2017, 7, 1)); // ~8 years tenure
        c.squad_status = PlayerSquadStatus::KeyPlayer;
        p.contract = Some(c);

        p
    }

    fn team_of(players: Vec<Player>) -> Team {
        TeamBuilder::new()
            .id(1)
            .league_id(Some(1))
            .club_id(1)
            .name("Test".to_string())
            .slug("test".to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(training())
            .build()
            .unwrap()
    }

    /// Matchday-armband test fixtures. Wrapped in a unit struct so the
    /// captaincy tests share one named home for their XI helpers rather
    /// than leaking a free fn into the module namespace.
    struct MatchdayFixture;

    impl MatchdayFixture {
        /// Neutral XI candidate parked at the peak age band (28) so the
        /// armband fallback's age curve and penalty bands don't muddy
        /// attribute-driven assertions.
        fn xi_candidate(id: u32, leadership: f32) -> LeadershipCandidate {
            LeadershipCandidate {
                id,
                age: 28,
                position: PlayerPositionType::MidfielderCenter,
                leadership,
                teamwork: 12.0,
                determination: 12.0,
                composure: 12.0,
                professionalism: 12.0,
                loyalty: 12.0,
                pressure: 12.0,
                consistency: 12.0,
                important_matches: 12.0,
                temperament: 12.0,
                sportsmanship: 12.0,
                dirtiness: 6.0,
                controversy: 6.0,
                reputation: 3000.0,
                experience: 0.0,
                condition_pct: 100.0,
                jadedness: 0.0,
            }
        }
    }

    /// A brilliant new arrival should not seize the armband from an
    /// established captain: low tenure / apps and the recent-signing penalty
    /// keep him well short, and hysteresis would block him even if close.
    #[test]
    fn high_leadership_new_signing_does_not_displace_established_captain() {
        let incumbent = strong(1, 14.0);
        let mut signing = strong(2, 20.0);
        if let Some(c) = signing.contract.as_mut() {
            c.started = Some(day(2025, 4, 1)); // joined ~60 days ago
            c.squad_status = PlayerSquadStatus::FirstTeamRegular;
        }
        signing.statistics.played = 3;

        let mut team = team_of(vec![incumbent, signing]);
        team.captain_id = Some(1);
        team.vice_captain_id = Some(2);

        CaptaincyAssigner::assign(&mut team, today());

        assert_eq!(team.captain_id, Some(1));
    }

    /// The core of the design: a marginally better challenger must not flip
    /// the armband, and repeated monthly recalculations must not oscillate.
    #[test]
    fn captain_stays_stable_when_candidates_are_close() {
        let incumbent = strong(1, 14.0);
        let challenger = strong(2, 15.0); // a hair better, everything else equal
        let mut team = team_of(vec![incumbent, challenger]);
        team.captain_id = Some(1);
        team.vice_captain_id = Some(2);

        CaptaincyAssigner::assign(&mut team, today());
        assert_eq!(
            team.captain_id,
            Some(1),
            "a small edge must not flip the armband"
        );

        CaptaincyAssigner::assign(&mut team, today());
        assert_eq!(
            team.captain_id,
            Some(1),
            "captaincy must not oscillate monthly"
        );
    }

    /// A captain who has handed in a transfer request is no longer a credible
    /// leader and must be replaced by the next eligible one.
    #[test]
    fn transfer_requesting_captain_is_replaced_by_next_leader() {
        let mut wantaway = strong(1, 16.0);
        wantaway
            .statuses
            .add(day(2025, 1, 1), PlayerStatusType::Req);
        let successor = strong(2, 14.0);

        let mut team = team_of(vec![wantaway, successor]);
        team.captain_id = Some(1);

        CaptaincyAssigner::assign(&mut team, today());
        assert_eq!(team.captain_id, Some(2));
    }

    /// A dazzling short-term loanee is a guest, not a club captain, while a
    /// permanent senior leader is available.
    #[test]
    fn short_term_loanee_is_not_made_captain_over_permanent_leader() {
        let permanent = strong(1, 14.0);
        let mut loanee = strong(2, 20.0);
        let mut loan = PlayerClubContract::new(40_000, day(2025, 9, 1)); // ends in ~3 months
        loan.started = Some(day(2025, 1, 15));
        loanee.contract_loan = Some(loan);

        let mut team = team_of(vec![permanent, loanee]);

        CaptaincyAssigner::assign(&mut team, today());
        assert_eq!(team.captain_id, Some(1));
        assert_ne!(team.captain_id, Some(2));
    }

    /// The vice is the best leader excluding the captain, by the same model.
    #[test]
    fn vice_captain_excludes_captain_and_ranks_realistically() {
        let mut team = team_of(vec![strong(1, 16.0), strong(2, 14.0), strong(3, 12.0)]);

        CaptaincyAssigner::assign(&mut team, today());
        assert_eq!(team.captain_id, Some(1));
        assert_eq!(team.vice_captain_id, Some(2));
        assert_ne!(team.captain_id, team.vice_captain_id);
    }

    /// Official captaincy and the matchday armband stay distinct: when the
    /// club captain is benched, the vice wears it for that match.
    #[test]
    fn matchday_armband_falls_to_vice_when_official_captain_benched() {
        let mut team = team_of(vec![strong(1, 16.0), strong(2, 14.0), strong(3, 12.0)]);
        CaptaincyAssigner::assign(&mut team, today());
        assert_eq!(team.captain_id, Some(1));
        assert_eq!(team.vice_captain_id, Some(2));

        // Official captain (1) is benched — only 2 and 3 are in the XI.
        let xi = vec![
            MatchdayFixture::xi_candidate(2, 14.0),
            MatchdayFixture::xi_candidate(3, 12.0),
        ];
        let armband = MatchdayLeadership::resolve(team.captain_id, team.vice_captain_id, &xi);
        assert_eq!(armband.captain_id, Some(2));
    }

    /// When no one clears the eligibility bar, the captain is stripped and the
    /// outgoing captain — still in the squad — gets a removal event.
    #[test]
    fn no_eligible_leaders_clears_captain_and_emits_removal() {
        let weak1 = strong(1, 5.0); // below the leadership floor
        let weak2 = strong(2, 4.0);
        let mut team = team_of(vec![weak1, weak2]);
        team.captain_id = Some(1);

        CaptaincyAssigner::assign(&mut team, today());

        assert_eq!(team.captain_id, None);
        let stripped = team.players.players.iter().find(|p| p.id == 1).unwrap();
        let removals = stripped
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::CaptaincyRemoved)
            .count();
        assert!(
            removals >= 1,
            "a stripped captain should receive a CaptaincyRemoved event"
        );
    }

    /// A one-cap 8.2 must not read as flawless reliability: the realistic
    /// (sample-size-regressed) rating pulls a tiny sample back toward the
    /// positional neutral, so a single-game superstar does not out-score a
    /// season-long performer on the reliability block.
    #[test]
    fn small_sample_high_rating_does_not_dominate_reliability() {
        let mut small_sample = strong(1, 14.0);
        small_sample.statistics.played = 1;
        small_sample.statistics.average_rating = 8.2;

        let proven = strong(2, 14.0); // strong() defaults: 28 starts, 7.2 avg

        let team = team_of(vec![small_sample, proven]);
        let model = CaptaincyModel::new(&team, today());

        let small = team.players.players.iter().find(|p| p.id == 1).unwrap();
        let big = team.players.players.iter().find(|p| p.id == 2).unwrap();
        assert!(
            model.reliability(small) < model.reliability(big),
            "a one-cap 8.2 must not out-reliability a proven 28-game 7.2"
        );
    }

    /// A `Lst` (transfer-listed) squad status keeps a player out of the
    /// primary captaincy pool even with the squad's best leadership, so a
    /// clean candidate takes the armband.
    #[test]
    fn transfer_listed_status_blocks_primary_captaincy() {
        let clean = strong(1, 14.0);
        let mut listed = strong(2, 18.0); // stronger leader, but listed
        listed.statuses.add(day(2025, 1, 1), PlayerStatusType::Lst);

        let mut team = team_of(vec![clean, listed]);
        CaptaincyAssigner::assign(&mut team, today());

        assert_eq!(team.captain_id, Some(1));
        assert_ne!(team.captain_id, Some(2));
    }

    /// An incumbent who has agreed a transfer (`Trn`) is gone the moment the
    /// window opens and must hand the armband to the next eligible leader.
    #[test]
    fn agreed_transfer_status_forces_incumbent_replacement() {
        let mut leaving = strong(1, 16.0);
        leaving.statuses.add(day(2025, 1, 1), PlayerStatusType::Trn);
        let successor = strong(2, 14.0);

        let mut team = team_of(vec![leaving, successor]);
        team.captain_id = Some(1);

        CaptaincyAssigner::assign(&mut team, today());
        assert_eq!(team.captain_id, Some(2));
    }

    /// A hot prospect is youth-status: he can only captain seniors if he
    /// clears the exception (leadership ≥ 17, professionalism ≥ 15, top 15%
    /// CA). Short of that, an ordinary senior leader keeps the armband.
    #[test]
    fn hot_prospect_youth_does_not_captain_without_exception() {
        let senior = strong(1, 14.0);
        let mut prospect = strong(2, 16.0); // higher leadership, but a prospect
        if let Some(c) = prospect.contract.as_mut() {
            c.squad_status = PlayerSquadStatus::HotProspectForTheFuture;
        }

        let mut team = team_of(vec![senior, prospect]);
        CaptaincyAssigner::assign(&mut team, today());

        assert_eq!(team.captain_id, Some(1));
        assert_ne!(team.captain_id, Some(2));
    }

    /// A long-term loaned-in star is still a visitor: while a permanent
    /// base-eligible leader is on the books, the borrowed player stays out
    /// of the primary pool even though his loan isn't short-term.
    #[test]
    fn long_term_loaned_in_star_does_not_beat_permanent_leader() {
        let permanent = strong(1, 14.0);
        let mut loanee = strong(2, 20.0); // brilliant leader, but on a long loan
        let mut loan = PlayerClubContract::new(60_000, day(2027, 6, 30)); // ~2y out
        loan.started = Some(day(2025, 1, 1));
        loanee.contract_loan = Some(loan);

        let mut team = team_of(vec![permanent, loanee]);
        CaptaincyAssigner::assign(&mut team, today());

        assert_eq!(team.captain_id, Some(1));
        assert_ne!(team.captain_id, Some(2));
    }

    /// Two otherwise-identical leaders differ only in dressing-room
    /// integration. The better-integrated one — deliberately the HIGHER id,
    /// so the id tie-break would otherwise favour the other — takes the
    /// armband once `squad_social_view` feeds the seniority/fit block.
    #[test]
    fn squad_social_view_breaks_close_tie_for_integrated_leader() {
        let isolated = strong(1, 14.0);
        let integrated = strong(2, 14.0);

        let mut team = team_of(vec![isolated, integrated]);
        for p in team.players.players.iter_mut() {
            p.squad_social_view = Some(if p.id == 2 {
                SquadSocialView {
                    same_nationality_teammates: 5,
                    same_language_teammates: 5,
                }
            } else {
                SquadSocialView {
                    same_nationality_teammates: 0,
                    same_language_teammates: 0,
                }
            });
        }

        CaptaincyAssigner::assign(&mut team, today());
        assert_eq!(team.captain_id, Some(2));
    }
}
