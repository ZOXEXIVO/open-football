//! Team-wide social weather — the single 0..100 number every downstream
//! system (training, match performance, board mood) reads when it wants
//! to know "is the dressing room a happy one?".
//!
//! Built once per weekly tick. The per-player [`crate::Relations`]
//! recalculation still produces a per-player chemistry score that's
//! useful as a local read, but the **squad** is a different object:
//! it owns squad-wide signals (captain, leadership core, manager
//! trust, recent turnover, integration) that any single
//! [`crate::Relations`] instance can't see. This snapshot rolls those
//! signals up so the answer is consistent across players.
//!
//! Why a separate type, not just an extra field on
//! `ChemistryContext`: ChemistryContext is **input** to the per-player
//! chemistry recalc. The snapshot is the **team-level output** —
//! downstream consumers should read it directly rather than averaging
//! 22 per-player chemistry numbers (those each carry their own
//! per-relation noise). Keeping the team number and the per-player
//! number on different stores makes their meanings unambiguous.

use crate::club::staff::CoachPlayerBond;
use crate::club::team::Team;
use crate::{Player, Staff, StaffPosition};
use chrono::Duration;
use chrono::NaiveDate;
use std::collections::{HashMap, HashSet};

/// Cutoff window for the "recent signing" turnover signal. Anyone whose
/// last transfer falls inside this window contributes to the turnover
/// penalty; on day-after-window 91 the contribution falls to zero,
/// matching real-world settling-in feel.
const RECENT_SIGNING_WINDOW_DAYS: i64 = 90;

/// Per-player snapshot of the manager bond — extracted from
/// [`CoachPlayerBond`] so the team-level averages stay aligned with
/// the per-player view the selection layer reads.
#[derive(Debug, Clone, Copy)]
struct ManagerBondSample {
    selection_trust: f32,
    tactical_buy_in: f32,
}

/// Rolled-up team chemistry. Every axis is on a 0..100 scale (with
/// `conflict_density` interpreted as "higher = worse"). `team_chemistry`
/// is the blended headline number callers consume.
#[derive(Debug, Clone, Copy)]
pub struct TeamSocialSnapshot {
    /// 0..100. Average per-pair PlayerRelation quality. 50 = neutral.
    pub avg_pair_harmony: f32,
    /// 0..100. Sum of conflict contributions divided by squad size,
    /// capped at 100. Higher = more dressing-room friction.
    pub conflict_density: f32,
    /// 0..100. Top-3 leadership core weighted by captain / vice
    /// status. Mirrors the per-player ChemistryContext input.
    pub leadership_quality: f32,
    /// 0..100. Average `CoachPlayerBond::selection_trust` across the
    /// squad, mapped onto a 0..100 axis. 50 = neutral.
    pub manager_trust_avg: f32,
    /// 0..100. Average `CoachPlayerBond::tactical_buy_in`, mapped.
    pub tactical_buy_in_avg: f32,
    /// 0..100. Nationality integration — share of the squad with at
    /// least one nationality compatriot, on a 0..100 axis. 50 = a
    /// neutral mix (large enough that most players have a compatriot
    /// without the dressing room being monocultural).
    pub integration_score: f32,
    /// Raw points subtracted in the blend formula. Scales with
    /// `recent_signings_90d` per [`TeamSocialSnapshot::turnover_penalty_for`].
    pub turnover_penalty: f32,
    /// New signings inside [`RECENT_SIGNING_WINDOW_DAYS`] — surfaced
    /// so callers (UI, debug) can read the underlying count instead of
    /// the derived penalty alone.
    pub recent_signings_90d: u8,
    /// 0..100. Blended team chemistry headline. Clamped to 0..100.
    pub team_chemistry: f32,
}

impl Default for TeamSocialSnapshot {
    fn default() -> Self {
        // Neutral defaults — every axis at the midpoint, no conflict, no
        // turnover, headline at 50. Matches the "freshly-built team"
        // starting state so the first weekly tick produces an honest
        // first read.
        TeamSocialSnapshot {
            avg_pair_harmony: 50.0,
            conflict_density: 0.0,
            leadership_quality: 50.0,
            manager_trust_avg: 50.0,
            tactical_buy_in_avg: 50.0,
            integration_score: 50.0,
            turnover_penalty: 0.0,
            recent_signings_90d: 0,
            team_chemistry: 50.0,
        }
    }
}

impl TeamSocialSnapshot {
    /// Build the snapshot from a `&Team` and today's date.
    pub fn build(team: &Team, today: NaiveDate) -> Self {
        let squad_size = team.players.players.len();
        if squad_size == 0 {
            return TeamSocialSnapshot::default();
        }

        let avg_pair_harmony = Self::compute_avg_pair_harmony(team);
        let conflict_density = Self::compute_conflict_density(team);
        let leadership_quality = Self::compute_leadership_quality(team);
        let bond_samples = Self::collect_manager_bond_samples(team, today);
        let manager_trust_avg = Self::compute_manager_trust_avg(&bond_samples);
        let tactical_buy_in_avg = Self::compute_tactical_buy_in_avg(&bond_samples);
        let integration_score = Self::compute_integration_score(team);
        let recent_signings_90d = Self::compute_recent_signings(team, today);
        let turnover_penalty = Self::turnover_penalty_for(recent_signings_90d);

        let team_chemistry = Self::blend(
            avg_pair_harmony,
            leadership_quality,
            manager_trust_avg,
            tactical_buy_in_avg,
            integration_score,
            conflict_density,
            turnover_penalty,
        );

        TeamSocialSnapshot {
            avg_pair_harmony,
            conflict_density,
            leadership_quality,
            manager_trust_avg,
            tactical_buy_in_avg,
            integration_score,
            turnover_penalty,
            recent_signings_90d,
            team_chemistry,
        }
    }

    /// Compose the headline number per the design spec. Public so
    /// tests can reproduce the formula at unit-test scale without
    /// constructing a full `Team`.
    pub fn blend(
        avg_pair_harmony: f32,
        leadership_quality: f32,
        manager_trust_avg: f32,
        tactical_buy_in_avg: f32,
        integration_score: f32,
        conflict_density: f32,
        turnover_penalty: f32,
    ) -> f32 {
        let raw = 50.0
            + (avg_pair_harmony - 50.0) * 0.25
            + (leadership_quality - 50.0) * 0.20
            + (manager_trust_avg - 50.0) * 0.20
            + (tactical_buy_in_avg - 50.0) * 0.15
            + (integration_score - 50.0) * 0.10
            - conflict_density * 0.40
            - turnover_penalty;
        raw.clamp(0.0, 100.0)
    }

    /// Step turnover penalty. Mirrors the per-player chemistry
    /// `turnover_penalty_for` so the two reads agree. Public so the
    /// builder can reuse the same function; consumers don't need to
    /// know the curve.
    pub fn turnover_penalty_for(recent_signings: u8) -> f32 {
        match recent_signings {
            0 => 0.0,
            1 => 2.0,
            2..=3 => 5.0,
            4..=6 => 10.0,
            _ => 18.0,
        }
    }

    fn compute_avg_pair_harmony(team: &Team) -> f32 {
        let pairs = PairWalk::collect(team);
        pairs.avg_harmony()
    }

    fn compute_conflict_density(team: &Team) -> f32 {
        let pairs = PairWalk::collect(team);
        pairs.conflict_density()
    }

    /// Leadership quality per the spec:
    ///   0.40 * captain_leadership +
    ///   0.20 * vice_captain_leadership +
    ///   0.25 * top_3_senior_leaders_avg (excluding captain/vice) +
    ///   0.15 * squad_average_professionalism
    ///
    /// When no captain exists, the 0.40 weight is redistributed onto
    /// top senior leaders. Same for vice — so an under-organised squad
    /// without an armband still gets a sensible read from its strongest
    /// available core.
    ///
    /// Critically — captain weighting reads `team.captain_id` /
    /// `team.vice_captain_id` directly. The pre-polish version only
    /// considered the top-3 leadership scorers and applied captain
    /// weights from inside that list, so a captain whose leadership
    /// attribute had dipped below the third-ranked squad member silently
    /// lost his weight.
    fn compute_leadership_quality(team: &Team) -> f32 {
        if team.players.players.is_empty() {
            return 50.0;
        }
        let parts = LeadershipParts::collect(team);
        parts.compose()
    }

    fn collect_manager_bond_samples(
        team: &Team,
        today: NaiveDate,
    ) -> Vec<ManagerBondSample> {
        let Some(manager) = ManagerLookup::for_snapshot(team) else {
            return Vec::new();
        };
        team.players
            .players
            .iter()
            .filter(|p| !p.is_on_loan())
            .map(|p| {
                let bond = CoachPlayerBond::build(p, manager, today);
                ManagerBondSample {
                    selection_trust: bond.selection_trust,
                    tactical_buy_in: bond.tactical_buy_in,
                }
            })
            .collect()
    }

    fn compute_manager_trust_avg(samples: &[ManagerBondSample]) -> f32 {
        if samples.is_empty() {
            return 50.0;
        }
        let avg = samples.iter().map(|s| s.selection_trust).sum::<f32>() / samples.len() as f32;
        (avg * 100.0).clamp(0.0, 100.0)
    }

    fn compute_tactical_buy_in_avg(samples: &[ManagerBondSample]) -> f32 {
        if samples.is_empty() {
            return 50.0;
        }
        let avg = samples.iter().map(|s| s.tactical_buy_in).sum::<f32>() / samples.len() as f32;
        (avg * 100.0).clamp(0.0, 100.0)
    }

    /// Four-channel integration score per the polish spec:
    ///
    /// * 0.45 nationality_support — share of the squad with at least one
    ///   nationality compatriot.
    /// * 0.35 language_support — share of the squad with at least one
    ///   chat-ready language partner (per `SquadSocialView`).
    /// * 0.10 squad_tenure_blend — mean tenure since most recent move,
    ///   normalised on a 0..1 year-since-arrival ramp.
    /// * 0.10 personality_adaptability — squad-mean adaptability attribute.
    ///
    /// Nationality is dampened when the supporting channels (language /
    /// tenure) are weak — a monocultural squad whose players can't talk
    /// to each other and just got there doesn't get a free pass to the
    /// top of the band. Per spec: "Avoid making mono-national squads
    /// automatically perfect."
    fn compute_integration_score(team: &Team) -> f32 {
        let players = &team.players.players;
        if players.is_empty() {
            return 50.0;
        }
        let parts = IntegrationParts::collect(team);
        // Cap nationality if the two support channels are weak. Average
        // of (language_support, tenure_blend) ≤ 0.5 dampens nationality
        // by 0.5; smooth ramp between weak and strong.
        let support_avg = (parts.language_support + parts.tenure_blend) * 0.5;
        let nat_dampener = (0.5 + support_avg).clamp(0.5, 1.0);
        let nat_capped = parts.nationality_support * nat_dampener;
        (0.45 * nat_capped
            + 0.35 * parts.language_support * 100.0
            + 0.10 * parts.tenure_blend * 100.0
            + 0.10 * parts.adaptability * 100.0)
            .clamp(0.0, 100.0)
    }

    /// Active player count surfaced for `ChemistryContextBuilder`. Mirrors
    /// the iteration scope used by `PairWalk::collect`, kept as a public
    /// thin accessor so callers don't replicate the filter inline.
    pub fn squad_size(team: &Team) -> usize {
        team.players.players.len()
    }

    /// Recent-signings count for the 90-day turnover window. Public so
    /// `ChemistryContextBuilder` can call the same helper and the two
    /// consumers can't drift apart — see polish task #10's
    /// "dedupe leadership/turnover" requirement.
    pub fn compute_recent_signings(team: &Team, today: NaiveDate) -> u8 {
        let cutoff = today - Duration::days(RECENT_SIGNING_WINDOW_DAYS);
        // `last_transfer_date` is `Some` only for actual moves, so a
        // player who has been at the club from birth doesn't count
        // toward turnover.
        let count = team
            .players
            .players
            .iter()
            .filter(|p| p.last_transfer_date.map(|d| d >= cutoff).unwrap_or(false))
            .count();
        count.min(u8::MAX as usize) as u8
    }
}

/// Captain weight in the leadership composite. Spec value 0.40.
const CAPTAIN_LEADERSHIP_WEIGHT: f32 = 0.40;
/// Vice-captain weight. Spec value 0.20.
const VICE_CAPTAIN_LEADERSHIP_WEIGHT: f32 = 0.20;
/// Top-3 senior leaders (excluding captain/vice) weight. Spec value 0.25.
const SENIOR_LEADERS_WEIGHT: f32 = 0.25;
/// Squad professionalism average weight. Spec value 0.15.
const SQUAD_PROFESSIONALISM_WEIGHT: f32 = 0.15;
/// Years of tenure at which the integration tenure_blend ramp saturates.
/// Two seasons of continuity is treated as a fully-settled core.
const TENURE_FULL_YEARS: f32 = 2.0;
const TENURE_FULL_DAYS: f32 = TENURE_FULL_YEARS * 365.0;

/// Walks the squad's pair-relation graph exactly once, deduping
/// directional entries into unordered pairs, filtering out departed
/// players. Both `avg_pair_harmony` and `conflict_density` read from
/// this single walk so the two figures can't disagree about which
/// pairs exist.
///
/// Per polish task #8:
///   * filter relation targets to current squad ids,
///   * average bidirectional A↔B entries into one quality reading,
///   * normalise conflict density by active pair count, not squad size.
struct PairWalk {
    /// Each entry is an active unordered pair: averaged harmony quality
    /// (0..100) + conflict contribution (raw points). Built once during
    /// `collect`.
    pairs: Vec<PairSample>,
}

/// One unordered-pair record produced by `PairWalk::collect`.
#[derive(Clone, Copy)]
struct PairSample {
    /// Averaged harmony quality across the directions that exist for
    /// this pair (one or two, depending on whether both sides have a
    /// `PlayerRelation` entry for the other).
    harmony: f32,
    /// Sum of per-direction conflict contributions (worse direction
    /// dominates because severity > 0 caps quickly).
    conflict: f32,
}

impl PairWalk {
    fn collect(team: &Team) -> Self {
        let squad_ids: HashSet<u32> = team.players.players.iter().map(|p| p.id).collect();
        // Map keyed on the canonical ordered (lo, hi) pair so a
        // direction recorded on either side updates the same bucket.
        let mut buckets: HashMap<(u32, u32), [Option<DirectionSample>; 2]> = HashMap::new();
        for p in team.players.players.iter() {
            for (target_id, rel) in p.relations.player_relations_iter() {
                if !squad_ids.contains(target_id) {
                    // Departed players (transferred / retired) are
                    // ignored — their relation rows are stale.
                    continue;
                }
                if *target_id == p.id {
                    continue; // defensive: self-relations would skew the average.
                }
                let key = if p.id < *target_id {
                    (p.id, *target_id)
                } else {
                    (*target_id, p.id)
                };
                let slot = if p.id < *target_id { 0 } else { 1 };
                let entry = buckets.entry(key).or_insert([None, None]);
                entry[slot] = Some(DirectionSample::from_relation(rel));
            }
        }

        let pairs = buckets
            .into_values()
            .map(|sides| {
                let mut harmony_acc = 0.0f32;
                let mut conflict_acc = 0.0f32;
                let mut directions = 0u8;
                for s in sides.iter().flatten() {
                    harmony_acc += s.harmony;
                    conflict_acc += s.conflict;
                    directions += 1;
                }
                if directions == 0 {
                    PairSample {
                        harmony: 50.0,
                        conflict: 0.0,
                    }
                } else {
                    let harmony = harmony_acc / directions as f32;
                    // Conflict from BOTH directions counts — a mutual
                    // hostility is genuinely worse than a one-sided
                    // grudge — but the spec wants a per-pair density
                    // normalised against the active pair count, so we
                    // keep the sum here and divide by active pairs at
                    // the team level.
                    PairSample {
                        harmony: harmony.clamp(0.0, 100.0),
                        conflict: conflict_acc,
                    }
                }
            })
            .collect();
        Self { pairs }
    }

    fn avg_harmony(&self) -> f32 {
        if self.pairs.is_empty() {
            return 50.0;
        }
        let sum: f32 = self.pairs.iter().map(|p| p.harmony).sum();
        (sum / self.pairs.len() as f32).clamp(0.0, 100.0)
    }

    /// Per polish task #8: normalise by *active pair count* (the pairs
    /// that actually carry a relation), not raw squad size. A 25-player
    /// squad with two genuine rivalries should read the same density
    /// as a 12-player squad with the same two rivalries — sample size
    /// is the right denominator, not roster headcount.
    fn conflict_density(&self) -> f32 {
        let active = self.pairs.len().max(1) as f32;
        let total: f32 = self.pairs.iter().map(|p| p.conflict).sum();
        (total / active).min(100.0)
    }
}

/// One direction of a pair relation, captured as the same numbers
/// `PlayerRelation::quality_score` / `conflict_contribution` would
/// produce — so the pair walk and the per-pair display agree about
/// what counts as "good" and "hostile".
#[derive(Clone, Copy)]
struct DirectionSample {
    harmony: f32,
    conflict: f32,
}

impl DirectionSample {
    fn from_relation(rel: &crate::club::relations::PlayerRelation) -> Self {
        let level_axis = (rel.level + 100.0) / 2.0;
        let harmony = (level_axis * 0.4 + rel.trust * 0.3 + rel.professional_respect * 0.3)
            .clamp(0.0, 100.0);

        let mut conflict = 0.0f32;
        let is_rivalry = !rel.rivalry_with.is_empty();
        if rel.level <= -75.0 {
            conflict += 25.0;
        } else if rel.level <= -50.0 {
            conflict += 15.0;
        } else if rel.level <= -25.0 || rel.trust <= 20.0 {
            conflict += 8.0;
        }
        if is_rivalry {
            conflict += 6.0;
        }
        Self { harmony, conflict }
    }
}

/// Head-coach lookup with the spec's full fallback chain: Manager →
/// CaretakerManager → AssistantManager → any contracted coach. Returns
/// `None` only when every seat is vacant (an unlikely simulator state
/// — the manager market normally fills the head-coach role first).
struct ManagerLookup;

impl ManagerLookup {
    /// Pick the staff member the snapshot should treat as "the head
    /// coach". Borrows `team.staffs.head_coach()` which already walks
    /// Manager → CaretakerManager → AssistantManager; if that returns
    /// the internal stub (`id == 0`, every seat vacant), fall back to
    /// any contracted coaching staff.
    fn for_snapshot(team: &Team) -> Option<&Staff> {
        let head = team.staffs.head_coach();
        if head.id != 0 {
            return Some(head);
        }
        // Final fallback: the head_coach stub is in play, so look for
        // the highest-priority alternative role still on the books.
        // Iterating once over the staff vec keeps the worst case O(n).
        team.staffs.find_by_any_position(&[
            StaffPosition::Manager,
            StaffPosition::CaretakerManager,
            StaffPosition::AssistantManager,
            StaffPosition::Coach,
            StaffPosition::FirstTeamCoach,
        ])
    }
}

/// Per-channel raw signals that feed the four-channel integration
/// formula. Built once per `compute_integration_score` so the four
/// channels are sourced from the same squad iteration.
struct IntegrationParts {
    /// Share of squad with at least one nationality compatriot
    /// (rescaled 0..100 for the spec blend).
    nationality_support: f32,
    /// Share of squad with at least one chat-ready language partner
    /// (0..1). Read from per-player `SquadSocialView`.
    language_support: f32,
    /// Mean normalised tenure (0..1, saturating at
    /// [`TENURE_FULL_YEARS`]). A long-settled squad scores high here.
    tenure_blend: f32,
    /// Mean adaptability attribute (0..1).
    adaptability: f32,
}

impl IntegrationParts {
    fn collect(team: &Team) -> Self {
        let players = &team.players.players;
        let n = players.len() as f32;
        if n <= 0.0 {
            return Self {
                nationality_support: 50.0,
                language_support: 0.5,
                tenure_blend: 0.5,
                adaptability: 0.5,
            };
        }

        let mut nat_counts: HashMap<u32, u16> = HashMap::new();
        for p in players.iter() {
            *nat_counts.entry(p.country_id).or_insert(0) += 1;
        }
        let with_compatriots = players
            .iter()
            .filter(|p| nat_counts.get(&p.country_id).copied().unwrap_or(0) >= 2)
            .count() as f32;
        let nationality_support = (with_compatriots / n * 100.0).clamp(0.0, 100.0);

        let with_lang = players
            .iter()
            .filter(|p| {
                p.squad_social_view
                    .as_ref()
                    .map(|v| v.same_language_teammates > 0)
                    .unwrap_or(false)
            })
            .count() as f32;
        let language_support = (with_lang / n).clamp(0.0, 1.0);

        let tenure_blend = Self::squad_tenure_blend(players);
        let adaptability_sum: f32 = players
            .iter()
            .map(|p| (p.attributes.adaptability / 20.0).clamp(0.0, 1.0))
            .sum();
        let adaptability = (adaptability_sum / n).clamp(0.0, 1.0);

        Self {
            nationality_support,
            language_support,
            tenure_blend,
            adaptability,
        }
    }

    /// Average normalised tenure across the squad. A player with no
    /// `last_transfer_date` (academy product who's never moved) reads
    /// as fully settled (1.0). Otherwise, days-since-arrival ramps
    /// linearly to 1.0 over `TENURE_FULL_DAYS`.
    fn squad_tenure_blend(players: &[Player]) -> f32 {
        if players.is_empty() {
            return 0.5;
        }
        let today_proxy_required = players
            .iter()
            .any(|p| p.last_transfer_date.is_some());
        if !today_proxy_required {
            return 1.0;
        }
        // We don't take `today` here — the snapshot's tenure read is a
        // relative "how long have they been around" signal, so the
        // newest transfer in the squad becomes the reference and the
        // others are ramped relative to it. This keeps the read stable
        // across save reloads without threading a date through.
        let latest = players
            .iter()
            .filter_map(|p| p.last_transfer_date)
            .max();
        let Some(latest) = latest else {
            return 1.0;
        };
        let sum: f32 = players
            .iter()
            .map(|p| {
                let d = p
                    .last_transfer_date
                    .map(|d| (latest - d).num_days())
                    .unwrap_or(TENURE_FULL_DAYS as i64);
                ((d as f32) / TENURE_FULL_DAYS).clamp(0.0, 1.0)
            })
            .sum();
        (sum / players.len() as f32).clamp(0.0, 1.0)
    }
}

/// Per-channel inputs for the spec's leadership formula. Built once
/// per `compute_leadership_quality` so captain weight and the
/// senior-leaders avg are sourced from the same squad iteration.
struct LeadershipParts {
    /// Captain's leadership attribute (0..20), or `None` if no captain.
    captain_leadership: Option<f32>,
    /// Vice-captain's leadership attribute (0..20), or `None`.
    vice_leadership: Option<f32>,
    /// Mean of the top-3 leadership scores in the squad **excluding**
    /// captain and vice — these are the supporting voices in the
    /// dressing room.
    top_senior_avg: f32,
    /// Squad mean of personality.professionalism (0..20).
    squad_professionalism: f32,
}

impl LeadershipParts {
    fn collect(team: &Team) -> Self {
        let captain_leadership = team
            .captain_id
            .and_then(|id| team.players.players.iter().find(|p| p.id == id))
            .map(|p| p.skills.mental.leadership.clamp(0.0, 20.0));
        let vice_leadership = team
            .vice_captain_id
            .and_then(|id| team.players.players.iter().find(|p| p.id == id))
            .map(|p| p.skills.mental.leadership.clamp(0.0, 20.0));

        let mut senior_pool: Vec<f32> = team
            .players
            .players
            .iter()
            .filter(|p| Some(p.id) != team.captain_id && Some(p.id) != team.vice_captain_id)
            .map(|p| p.skills.mental.leadership.clamp(0.0, 20.0))
            .collect();
        senior_pool.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let top_senior_avg = if senior_pool.is_empty() {
            10.0
        } else {
            let take = senior_pool.len().min(3);
            senior_pool.iter().take(take).sum::<f32>() / take as f32
        };

        let prof_sum: f32 = team
            .players
            .players
            .iter()
            .map(|p| (p.attributes.professionalism).clamp(0.0, 20.0))
            .sum();
        let squad_professionalism = if team.players.players.is_empty() {
            10.0
        } else {
            prof_sum / team.players.players.len() as f32
        };

        Self {
            captain_leadership,
            vice_leadership,
            top_senior_avg,
            squad_professionalism,
        }
    }

    /// Blend per the spec; if captain / vice is missing, redistribute
    /// the unused weight onto the senior-leaders channel so an
    /// armband-less squad still gets a sensible read from its strongest
    /// core voices.
    fn compose(&self) -> f32 {
        // Map raw 0..20 scores onto 0..100 axes.
        let cap = self.captain_leadership.map(|v| v / 20.0 * 100.0);
        let vice = self.vice_leadership.map(|v| v / 20.0 * 100.0);
        let senior = (self.top_senior_avg / 20.0 * 100.0).clamp(0.0, 100.0);
        let prof = (self.squad_professionalism / 20.0 * 100.0).clamp(0.0, 100.0);

        // Redistribute unused captain/vice weights onto the senior pool.
        let mut senior_weight = SENIOR_LEADERS_WEIGHT;
        let mut score = SQUAD_PROFESSIONALISM_WEIGHT * prof;
        let mut applied_weight = SQUAD_PROFESSIONALISM_WEIGHT;
        match cap {
            Some(v) => {
                score += CAPTAIN_LEADERSHIP_WEIGHT * v;
                applied_weight += CAPTAIN_LEADERSHIP_WEIGHT;
            }
            None => {
                senior_weight += CAPTAIN_LEADERSHIP_WEIGHT;
            }
        }
        match vice {
            Some(v) => {
                score += VICE_CAPTAIN_LEADERSHIP_WEIGHT * v;
                applied_weight += VICE_CAPTAIN_LEADERSHIP_WEIGHT;
            }
            None => {
                senior_weight += VICE_CAPTAIN_LEADERSHIP_WEIGHT;
            }
        }
        score += senior_weight * senior;
        applied_weight += senior_weight;
        if applied_weight <= 0.0 {
            50.0
        } else {
            (score / applied_weight).clamp(0.0, 100.0)
        }
    }
}

#[cfg(test)]
mod tests {
    //! Spec-driven tests for the team-level chemistry snapshot.
    //!
    //! Covers: new-signing turnover penalty fades after 90 days, and
    //! a strong leadership core (captain + vice-captain) lifts
    //! team_chemistry vs an equivalent squad with no recognised
    //! leaders. The per-player relation update -> conflict_density
    //! plumbing is exercised indirectly by the chemistry test.
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::team::TeamBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, StaffCollection, TeamReputation,
        TeamType, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    struct SnapshotFixture;

    impl SnapshotFixture {
        fn today() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        }

        fn player(id: u32, leadership: f32, country_id: u32) -> Player {
            let mut skills = PlayerSkills::default();
            skills.mental.leadership = leadership;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("S".into(), id.to_string()))
                .birth_date(NaiveDate::from_ymd_opt(1998, 1, 1).unwrap())
                .country_id(country_id)
                .attributes(PersonAttributes::default())
                .skills(skills)
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap()
        }

        fn build_team(players: Vec<Player>) -> Team {
            TeamBuilder::new()
                .id(1)
                .league_id(Some(1))
                .club_id(1)
                .name("Snap FC".into())
                .slug("snap-fc".into())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(100, 100, 200))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }
    }

    #[test]
    fn turnover_penalty_for_curve_matches_spec_steps() {
        // The piecewise curve is the spec — assert each step directly.
        assert_eq!(TeamSocialSnapshot::turnover_penalty_for(0), 0.0);
        assert_eq!(TeamSocialSnapshot::turnover_penalty_for(1), 2.0);
        assert_eq!(TeamSocialSnapshot::turnover_penalty_for(3), 5.0);
        assert_eq!(TeamSocialSnapshot::turnover_penalty_for(5), 10.0);
        assert_eq!(TeamSocialSnapshot::turnover_penalty_for(8), 18.0);
    }

    #[test]
    fn new_signing_turnover_penalty_fades_after_90_days() {
        // Spec test: a new signing applies a turnover penalty; after
        // 90 days the penalty fades to zero because the player is no
        // longer "recent". The chemistry headline rises accordingly.
        let mut players: Vec<Player> = (1..=4)
            .map(|id| SnapshotFixture::player(id, 10.0, 1))
            .collect();
        players[0].last_transfer_date = Some(SnapshotFixture::today());

        let team = SnapshotFixture::build_team(players.clone());
        let fresh = TeamSocialSnapshot::build(&team, SnapshotFixture::today());

        // Same team but the new signing arrived 91 days ago.
        let mut later_players = players;
        later_players[0].last_transfer_date =
            Some(SnapshotFixture::today() - Duration::days(91));
        let later_team = SnapshotFixture::build_team(later_players);
        let settled = TeamSocialSnapshot::build(&later_team, SnapshotFixture::today());

        assert!(
            fresh.turnover_penalty > 0.0,
            "fresh signing should produce a non-zero turnover penalty (was {})",
            fresh.turnover_penalty
        );
        assert_eq!(
            settled.turnover_penalty, 0.0,
            "90+ days later the turnover penalty must fade to zero"
        );
        assert!(
            settled.team_chemistry > fresh.team_chemistry,
            "settled chemistry ({}) should be higher than fresh-signing chemistry ({})",
            settled.team_chemistry,
            fresh.team_chemistry
        );
        assert_eq!(fresh.recent_signings_90d, 1);
        assert_eq!(settled.recent_signings_90d, 0);
    }

    #[test]
    fn high_leadership_captain_lifts_team_chemistry() {
        // Spec test: "High leadership captain reduces conflict effect."
        // We split this into a clean leadership-quality test (no
        // conflict noise): a squad with a recognised captain whose
        // leadership is 18 reads higher than the same squad with no
        // captain installed.
        let mut players: Vec<Player> = (1..=4)
            .map(|id| SnapshotFixture::player(id, 6.0, 1))
            .collect();
        if let Some(p) = players.iter_mut().find(|p| p.id == 1) {
            p.skills.mental.leadership = 18.0;
        }
        let without_captain = SnapshotFixture::build_team(players.clone());
        let no_armband = TeamSocialSnapshot::build(&without_captain, SnapshotFixture::today());

        let mut with_captain = SnapshotFixture::build_team(players);
        with_captain.captain_id = Some(1);
        let armband = TeamSocialSnapshot::build(&with_captain, SnapshotFixture::today());

        assert!(
            armband.leadership_quality > no_armband.leadership_quality,
            "captain weighting must raise leadership_quality ({} → {})",
            no_armband.leadership_quality,
            armband.leadership_quality
        );
        assert!(
            armband.team_chemistry >= no_armband.team_chemistry,
            "team chemistry should not fall when a high-leadership captain is recognised ({} vs {})",
            armband.team_chemistry,
            no_armband.team_chemistry
        );
    }

    #[test]
    fn nationality_compatriots_lift_integration_score() {
        // A monocultural squad scores high integration; a squad of
        // isolated nationalities scores low.
        let homogeneous: Vec<Player> = (1..=4)
            .map(|id| SnapshotFixture::player(id, 10.0, 1))
            .collect();
        let team_h = SnapshotFixture::build_team(homogeneous);
        let snap_h = TeamSocialSnapshot::build(&team_h, SnapshotFixture::today());

        let isolated: Vec<Player> = (1..=4)
            .map(|id| SnapshotFixture::player(id, 10.0, id))
            .collect();
        let team_i = SnapshotFixture::build_team(isolated);
        let snap_i = TeamSocialSnapshot::build(&team_i, SnapshotFixture::today());

        assert!(
            snap_h.integration_score > snap_i.integration_score,
            "compatriot-heavy squad should integrate better ({} vs {})",
            snap_h.integration_score,
            snap_i.integration_score
        );
    }

    #[test]
    fn empty_team_reads_neutral() {
        let team = SnapshotFixture::build_team(Vec::new());
        let snap = TeamSocialSnapshot::build(&team, SnapshotFixture::today());
        assert_eq!(snap.team_chemistry, 50.0);
    }

    #[test]
    fn blend_formula_matches_spec_coefficients() {
        // Direct coefficient check — gates against an accidental
        // rebalance. Each input set to its midpoint should produce a
        // neutral 50, and known inputs should produce the predicted
        // headline.
        let neutral = TeamSocialSnapshot::blend(50.0, 50.0, 50.0, 50.0, 50.0, 0.0, 0.0);
        assert!((neutral - 50.0).abs() < 1e-3, "neutral blend {}", neutral);

        // All positives at +20 from neutral; no conflict, no turnover.
        // Expected raw = 50 + (0.25+0.20+0.20+0.15+0.10)*20 = 50 + 18 = 68.
        let lifted = TeamSocialSnapshot::blend(70.0, 70.0, 70.0, 70.0, 70.0, 0.0, 0.0);
        assert!(
            (lifted - 68.0).abs() < 1e-2,
            "lifted blend {} expected 68",
            lifted
        );

        // 40 conflict density alone subtracts 16; turnover 10 subtracts 10.
        let stressed = TeamSocialSnapshot::blend(50.0, 50.0, 50.0, 50.0, 50.0, 40.0, 10.0);
        assert!(
            (stressed - 24.0).abs() < 1e-2,
            "stressed blend {} expected 24",
            stressed
        );
    }

    // ── Polish task #11 integration tests ─────────────────────────

    /// Push a synthetic rivalry / hostile pair into both directions of
    /// the relation graph. The pair walk picks both sides up and the
    /// conflict_density read shows the resulting density.
    fn install_rivalry(team: &mut Team, a_id: u32, b_id: u32) {
        use crate::ChangeType;
        use crate::RelationshipChange;
        let date = SnapshotFixture::today();
        if let Some(p) = team.players.players.iter_mut().find(|p| p.id == a_id) {
            for _ in 0..10 {
                p.relations.update_with_type(
                    b_id,
                    -0.8,
                    ChangeType::PersonalConflict,
                    date,
                );
            }
        }
        if let Some(p) = team.players.players.iter_mut().find(|p| p.id == b_id) {
            for _ in 0..10 {
                p.relations.update_with_type(
                    a_id,
                    -0.8,
                    ChangeType::PersonalConflict,
                    date,
                );
            }
            // Mark the rivalry symmetrically so conflict_contribution
            // picks it up.
            let change = RelationshipChange::negative(ChangeType::CompetitionRivalry, 0.9);
            p.relations.update_player_relationship(a_id, change, date);
        }
    }

    #[test]
    fn high_conflict_density_lowers_team_chemistry() {
        let players: Vec<Player> = (1..=4)
            .map(|id| SnapshotFixture::player(id, 10.0, 1))
            .collect();
        let peaceful = SnapshotFixture::build_team(players.clone());
        let peaceful_snap = TeamSocialSnapshot::build(&peaceful, SnapshotFixture::today());

        let mut hostile = SnapshotFixture::build_team(players);
        install_rivalry(&mut hostile, 1, 2);
        install_rivalry(&mut hostile, 3, 4);
        let hostile_snap = TeamSocialSnapshot::build(&hostile, SnapshotFixture::today());

        assert!(
            hostile_snap.conflict_density > peaceful_snap.conflict_density,
            "rivalries must lift conflict_density (peace={}, hostile={})",
            peaceful_snap.conflict_density,
            hostile_snap.conflict_density
        );
        assert!(
            hostile_snap.team_chemistry < peaceful_snap.team_chemistry,
            "rivalries must drop team_chemistry (peace={}, hostile={})",
            peaceful_snap.team_chemistry,
            hostile_snap.team_chemistry
        );
    }

    #[test]
    fn stale_relation_to_departed_player_is_ignored() {
        // Polish task #8: relation rows pointing at players no longer
        // in the squad must not show up in pair harmony / conflict
        // density. We seed a strong hostile relation toward an id that
        // is *not* in the team and assert the densities stay at the
        // neutral baseline.
        use crate::ChangeType;
        let players: Vec<Player> = (1..=3)
            .map(|id| SnapshotFixture::player(id, 10.0, 1))
            .collect();
        let baseline_team = SnapshotFixture::build_team(players.clone());
        let baseline = TeamSocialSnapshot::build(&baseline_team, SnapshotFixture::today());

        let mut team = SnapshotFixture::build_team(players);
        // 999 is a departed player id — not present on the team.
        let departed_id = 999u32;
        if let Some(p) = team.players.players.iter_mut().find(|p| p.id == 1) {
            for _ in 0..10 {
                p.relations.update_with_type(
                    departed_id,
                    -0.8,
                    ChangeType::PersonalConflict,
                    SnapshotFixture::today(),
                );
            }
        }
        let after = TeamSocialSnapshot::build(&team, SnapshotFixture::today());

        assert!(
            (after.conflict_density - baseline.conflict_density).abs() < 1e-3,
            "stale relation to departed player must not change conflict_density (baseline={}, after={})",
            baseline.conflict_density,
            after.conflict_density
        );
        assert!(
            (after.avg_pair_harmony - baseline.avg_pair_harmony).abs() < 1e-3,
            "stale relation to departed player must not change avg_pair_harmony"
        );
    }

    #[test]
    fn strong_captain_and_manager_trust_raise_team_chemistry() {
        // Polish task #11 spec test: a recognised high-leadership
        // captain combined with a positive staff relation across the
        // squad lifts team_chemistry vs the same squad with no captain
        // and a neutral staff relation.
        use crate::ChangeType;
        use crate::RelationshipChange;
        use crate::club::staff::StaffStub;
        use crate::{Staff, StaffClubContract, StaffPosition};

        fn build_manager(id: u32) -> Staff {
            let mut staff = StaffStub::default();
            staff.id = id;
            staff.contract = Some(StaffClubContract::new(
                50_000,
                SnapshotFixture::today() + chrono::Duration::days(365),
                StaffPosition::Manager,
                crate::club::staff::StaffStatus::Active,
            ));
            staff
        }

        let players_base: Vec<Player> = (1..=4)
            .map(|id| SnapshotFixture::player(id, 6.0, 1))
            .collect();

        // Baseline: no captain, no staff relations.
        let mut base_team = SnapshotFixture::build_team(players_base.clone());
        let manager = build_manager(101);
        base_team.staffs.staffs.push(manager.clone());
        let baseline = TeamSocialSnapshot::build(&base_team, SnapshotFixture::today());

        // Lifted: captain installed (leadership 18) + every player has a
        // positive staff relation with the manager.
        let mut lifted_players = players_base;
        if let Some(p) = lifted_players.iter_mut().find(|p| p.id == 1) {
            p.skills.mental.leadership = 18.0;
        }
        let mut lifted_team = SnapshotFixture::build_team(lifted_players);
        lifted_team.staffs.staffs.push(manager);
        lifted_team.captain_id = Some(1);
        for p in lifted_team.players.players.iter_mut() {
            for _ in 0..5 {
                p.relations.update_staff_relationship(
                    101,
                    RelationshipChange::positive(ChangeType::CoachingSuccess, 3.0),
                    SnapshotFixture::today(),
                );
            }
        }
        let lifted = TeamSocialSnapshot::build(&lifted_team, SnapshotFixture::today());

        assert!(
            lifted.team_chemistry > baseline.team_chemistry,
            "captain + manager trust should lift team_chemistry (base={} lifted={})",
            baseline.team_chemistry,
            lifted.team_chemistry
        );
        assert!(
            lifted.manager_trust_avg > baseline.manager_trust_avg,
            "manager trust avg must rise with positive staff relations (base={} lifted={})",
            baseline.manager_trust_avg,
            lifted.manager_trust_avg
        );
        assert!(
            lifted.leadership_quality > baseline.leadership_quality,
            "captain weighting must lift leadership_quality"
        );
    }

    #[test]
    fn caretaker_manager_contributes_to_manager_trust_avg() {
        // Polish task #7: ManagerLookup walks Manager → CaretakerManager
        // → AssistantManager → first available coach. A team with only
        // a CaretakerManager must still compute manager_trust_avg
        // (it should not collapse to the neutral fallback).
        use crate::ChangeType;
        use crate::RelationshipChange;
        use crate::club::staff::StaffStub;
        use crate::{Staff, StaffClubContract, StaffPosition};

        fn caretaker(id: u32) -> Staff {
            let mut staff = StaffStub::default();
            staff.id = id;
            staff.contract = Some(StaffClubContract::new(
                30_000,
                SnapshotFixture::today() + chrono::Duration::days(180),
                StaffPosition::CaretakerManager,
                crate::club::staff::StaffStatus::Active,
            ));
            staff
        }

        let mut players: Vec<Player> = (1..=4)
            .map(|id| SnapshotFixture::player(id, 10.0, 1))
            .collect();
        // Boost the staff relation so manager_trust_avg moves off neutral
        // — if the lookup falls through to the no-manager fallback,
        // manager_trust_avg sticks at the default 50.
        for p in players.iter_mut() {
            for _ in 0..5 {
                p.relations.update_staff_relationship(
                    202,
                    RelationshipChange::positive(ChangeType::CoachingSuccess, 3.0),
                    SnapshotFixture::today(),
                );
            }
        }
        let mut team = SnapshotFixture::build_team(players);
        team.staffs.staffs.push(caretaker(202));
        let snap = TeamSocialSnapshot::build(&team, SnapshotFixture::today());
        // The lift is small because the bond dampens single-axis
        // updates — what we're proving is "the caretaker was found and
        // his per-player bonds were sampled", not a specific magnitude.
        // 50.0 is the no-manager fallback; anything above it confirms
        // the lookup walked through the CaretakerManager seat.
        assert!(
            snap.manager_trust_avg > 51.0,
            "caretaker manager should still contribute to manager_trust_avg (got {}, expected > 51 vs no-manager fallback of 50)",
            snap.manager_trust_avg
        );
    }

    #[test]
    fn monocultural_squad_without_language_or_tenure_is_not_perfect() {
        // Polish task #9: a freshly-arrived monocultural squad with no
        // squad_social_view (so language_support = 0) and recent
        // transfers should not max out integration_score on nationality
        // alone — the nationality contribution must be dampened.
        let players: Vec<Player> = (1..=6)
            .map(|id| {
                let mut p = SnapshotFixture::player(id, 10.0, 1);
                // Recent transfer wipes tenure
                p.last_transfer_date = Some(SnapshotFixture::today());
                // Default squad_social_view is None, so language_support = 0
                p
            })
            .collect();
        let team = SnapshotFixture::build_team(players);
        let snap = TeamSocialSnapshot::build(&team, SnapshotFixture::today());
        // With nationality_support ≈ 100, language_support = 0,
        // tenure_blend ≈ 0 (all recent), adaptability mid-range:
        // capped nationality contribution ≈ 0.45 * 100 * 0.5 = 22.5
        // language: 0
        // tenure: 0
        // adaptability ≈ 0.10 * 50 = 5
        // → ~27.5 not ~100
        assert!(
            snap.integration_score < 60.0,
            "monocultural fresh-arrival squad should not max integration ({})",
            snap.integration_score
        );
    }
}

