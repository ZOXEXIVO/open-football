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

use crate::club::staff::{CoachPlayerBond, CoachPlayerBondBreakdown};
use crate::club::team::Team;
use crate::{Player, PlayerCollection, Staff};
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
    /// Faction structure read from per-pair relations. A clique-y
    /// dressing room with one dominant faction reads differently from
    /// one fragmented into rival sub-groups.
    pub factions: SquadFactionSnapshot,
    /// 0..100. Blended team chemistry headline. Clamped to 0..100.
    pub team_chemistry: f32,
}

/// Captain mediation inputs collected once per squad. Shared by the
/// [`crate::club::team::behaviour::behaviour::conflict_escalation`]
/// pass (which scales per-player `conflict_risk` by the captain's
/// quality) and the debug snapshot (which exposes the mediation
/// score). Lives here so the squad-wide read can't drift between
/// consumers.
pub struct CaptainMediation {
    leader_support: f32,
    captain_id: Option<u32>,
    is_fallback: bool,
}

impl CaptainMediation {
    /// Build the mediation snapshot. Tries the formal captain first;
    /// if none, falls back to the highest-leadership / pro player at
    /// 60% strength. Returns a neutral instance when neither path
    /// produces a leader.
    pub fn for_squad(players: &PlayerCollection) -> Self {
        let formal_captain = players
            .iter()
            .filter(|p| !p.is_on_loan())
            .max_by_key(|p| {
                let leadership = p.skills.mental.leadership;
                let prof = p.attributes.professionalism;
                ((leadership * 2.0 + prof) * 10.0) as i64
            });
        let Some(captain) = formal_captain else {
            return Self {
                leader_support: 0.0,
                captain_id: None,
                is_fallback: true,
            };
        };
        let leader_support = Self::compute_leader_support(captain);
        Self {
            leader_support,
            captain_id: Some(captain.id),
            is_fallback: false,
        }
    }

    /// Build from an explicit captain id (e.g. `team.captain_id`).
    /// Falls back to the senior-leader path at 60% strength if the
    /// supplied id isn't on the active roster.
    pub fn for_captain(players: &PlayerCollection, captain_id: Option<u32>) -> Self {
        if let Some(id) = captain_id
            && let Some(captain) = players.iter().find(|p| p.id == id && !p.is_on_loan())
        {
            return Self {
                leader_support: Self::compute_leader_support(captain),
                captain_id: Some(id),
                is_fallback: false,
            };
        }
        let mut fallback = Self::for_squad(players);
        if !fallback.is_fallback {
            fallback.is_fallback = true;
            fallback.leader_support *= 0.6;
        }
        fallback
    }

    /// Apply mediation to a raw conflict_risk for a specific player.
    /// A strongly negative captain relation (level ≤ -25) backfires
    /// and raises the effective risk by 0.10 — a hostile captain is
    /// part of the problem, not the solution.
    pub fn effective_risk(&self, raw_risk: f32, player: &Player) -> f32 {
        let mut risk = raw_risk * (1.0 - self.leader_support * 0.35);
        if let Some(captain_id) = self.captain_id
            && player.id != captain_id
            && let Some(rel) = player.relations.get_player(captain_id)
            && rel.level <= -25.0
        {
            risk += 0.10;
        }
        risk.clamp(0.0, 1.0)
    }

    pub fn leader_support(&self) -> f32 {
        self.leader_support
    }

    pub fn captain_id(&self) -> Option<u32> {
        self.captain_id
    }

    pub fn is_fallback(&self) -> bool {
        self.is_fallback
    }

    fn compute_leader_support(captain: &Player) -> f32 {
        // Spec composite:
        //   0.45 * leadership
        // + 0.25 * professionalism
        // + 0.20 * average relation level toward teammates
        // + 0.10 * average outgoing influence
        let leadership = (captain.skills.mental.leadership / 20.0).clamp(0.0, 1.0);
        let professionalism = (captain.attributes.professionalism / 20.0).clamp(0.0, 1.0);

        let mut rel_acc = 0.0f32;
        let mut rel_count = 0u32;
        for (_, rel) in captain.relations.player_relations_iter() {
            rel_acc += (rel.level + 100.0) / 200.0;
            rel_count += 1;
        }
        let relation_avg = if rel_count == 0 {
            0.5
        } else {
            (rel_acc / rel_count as f32).clamp(0.0, 1.0)
        };

        let mut infl_acc = 0.0f32;
        let mut infl_count = 0u32;
        for (_, rel) in captain.relations.player_relations_iter() {
            infl_acc += (rel.influence / 100.0).clamp(0.0, 1.0);
            infl_count += 1;
        }
        let influence = if infl_count == 0 {
            0.0
        } else {
            (infl_acc / infl_count as f32).clamp(0.0, 1.0)
        };

        (0.45 * leadership + 0.25 * professionalism + 0.20 * relation_avg + 0.10 * influence)
            .clamp(0.0, 1.0)
    }
}

/// Dressing-room faction read. Built by walking the per-pair harmony
/// graph (`>= 60` edges) and finding connected components — each
/// component is a faction. Surfaced to the blend so a fragmented squad
/// pays a chemistry tax, and a unified squad gets a small bonus.
#[derive(Debug, Clone, Copy, Default)]
pub struct SquadFactionSnapshot {
    /// Number of distinct factions (connected components of the
    /// "strong-bond" graph) in the active squad.
    pub faction_count: u8,
    /// Share (0..1) of the active squad in the largest faction. A
    /// unified dressing room reads close to 1.0; a fragmented squad
    /// drops toward 1 / `faction_count`.
    pub largest_faction_share: f32,
    /// Number of players who belong to no faction at all — they're
    /// in a 1-member component (no `>= 60` bond with anyone).
    pub isolated_players: u8,
    /// 0..1. Hostility *between* factions: how many cross-component
    /// hostile pairs exist relative to the upper bound. Drives the
    /// chemistry tax.
    pub faction_tension: f32,
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
            factions: SquadFactionSnapshot::default(),
            team_chemistry: 50.0,
        }
    }
}

impl TeamSocialSnapshot {
    /// Build the snapshot from a `&Team` and today's date.
    pub fn build(team: &Team, today: NaiveDate) -> Self {
        let active = Self::active_social_players(team);
        if active.is_empty() {
            return TeamSocialSnapshot::default();
        }

        let avg_pair_harmony = Self::compute_avg_pair_harmony(team);
        let conflict_density = Self::compute_conflict_density(team);
        let leadership_quality = Self::compute_leadership_quality(team);
        let bond_samples = Self::collect_manager_bond_samples(team, today);
        let manager_trust_avg = Self::compute_manager_trust_avg(&bond_samples);
        let tactical_buy_in_avg = Self::compute_tactical_buy_in_avg(&bond_samples);
        let integration_score = Self::compute_integration_score(team, today);
        let recent_signings_90d = Self::compute_recent_signings(team, today);
        let turnover_penalty = Self::turnover_penalty_for(recent_signings_90d);
        let factions = FactionWalk::collect(&active).into_snapshot();

        let team_chemistry = Self::blend_with_factions(
            avg_pair_harmony,
            leadership_quality,
            manager_trust_avg,
            tactical_buy_in_avg,
            integration_score,
            conflict_density,
            turnover_penalty,
            &factions,
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
            factions,
            team_chemistry,
        }
    }

    /// Compose the headline number per the design spec. Public so
    /// tests can reproduce the formula at unit-test scale without
    /// constructing a full `Team`. Faction-agnostic — `build` calls
    /// [`Self::blend_with_factions`] which folds the faction signal
    /// in on top of this base.
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

    /// Faction-aware blend used by `build`. Adds bounded contributions:
    ///   * `+ largest_faction_share` bonus up to **+3** when faction
    ///     tension is low — a unified squad rallies around its core.
    ///   * `- isolated_players * 0.8` capped at **-6** — every
    ///     dressing-room outsider drags the headline a touch.
    ///   * `- faction_tension * 0.15` capped at **-10** — cross-faction
    ///     hostility is the costliest signal here.
    #[allow(clippy::too_many_arguments)]
    pub fn blend_with_factions(
        avg_pair_harmony: f32,
        leadership_quality: f32,
        manager_trust_avg: f32,
        tactical_buy_in_avg: f32,
        integration_score: f32,
        conflict_density: f32,
        turnover_penalty: f32,
        factions: &SquadFactionSnapshot,
    ) -> f32 {
        let base = Self::blend(
            avg_pair_harmony,
            leadership_quality,
            manager_trust_avg,
            tactical_buy_in_avg,
            integration_score,
            conflict_density,
            turnover_penalty,
        );
        // Low tension unlocks the unity bonus; high tension cancels it.
        let unity_bonus = if factions.faction_tension < 0.20 {
            (factions.largest_faction_share * 3.0).min(3.0)
        } else {
            0.0
        };
        let isolation_penalty = (factions.isolated_players as f32 * 0.8).min(6.0);
        let tension_penalty = (factions.faction_tension * 15.0).min(10.0);
        (base + unity_bonus - isolation_penalty - tension_penalty).clamp(0.0, 100.0)
    }

    /// Map the headline `team_chemistry` (0..100) onto the per-player
    /// match-rating shift any downstream consumer should fold in. The
    /// shift is capped at ±0.10 — strong dressing-room weather can tilt
    /// a 7.0 night into a 7.1 or 6.9 evening, but never enough to
    /// override raw ability or form. Centred on 50 so a neutral
    /// chemistry produces a zero shift; positive at high chemistry,
    /// negative at low.
    ///
    /// Single source of truth for the rating cap per the polish spec
    /// (task #10): "No social system should dominate raw ability/form
    /// in one tick." Consumers must call this rather than rolling
    /// their own scale so the cap can't drift.
    pub fn team_chemistry_rating_shift(&self) -> f32 {
        // 50 chemistry → 0, 100 → +0.10, 0 → -0.10.
        let centered = (self.team_chemistry - 50.0) / 50.0;
        (centered * 0.10).clamp(-0.10, 0.10)
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
        let active = Self::active_social_players(team);
        let pairs = PairWalk::collect(&active);
        pairs.avg_harmony()
    }

    fn compute_conflict_density(team: &Team) -> f32 {
        let active = Self::active_social_players(team);
        let pairs = PairWalk::collect(&active);
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
        let active = Self::active_social_players(team);
        if active.is_empty() {
            return 50.0;
        }
        let parts = LeadershipParts::collect(team, &active);
        parts.compose()
    }

    fn collect_manager_bond_samples(
        team: &Team,
        today: NaiveDate,
    ) -> Vec<ManagerBondSample> {
        let Some(manager) = ManagerLookup::for_snapshot(team) else {
            return Vec::new();
        };
        Self::active_social_players(team)
            .into_iter()
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
    fn compute_integration_score(team: &Team, today: NaiveDate) -> f32 {
        let active = Self::active_social_players(team);
        if active.is_empty() {
            return 50.0;
        }
        let parts = IntegrationParts::collect(team, today, &active);
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
        Self::active_social_players(team).len()
    }

    /// Single source of truth for "who is socially present at this club
    /// this week". Excluded:
    ///   * players out on loan (they're not in this dressing room),
    ///   * long-term unavailable (injury > 90 days — they're physically
    ///     out of the building, so their per-pair relations are stale).
    ///
    /// All squad-level reads (pair harmony, conflict density, leadership,
    /// integration, manager-trust averages, factions) must filter through
    /// this helper so the social weather is consistent across consumers.
    pub fn active_social_players(team: &Team) -> Vec<&Player> {
        team.players
            .players
            .iter()
            .filter(|p| !p.is_on_loan())
            .filter(|p| {
                !(p.player_attributes.is_injured
                    && p.player_attributes.injury_days_remaining > 90)
            })
            .collect()
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
    fn collect(active: &[&Player]) -> Self {
        let squad_ids: HashSet<u32> = active.iter().map(|p| p.id).collect();
        // Map keyed on the canonical ordered (lo, hi) pair so a
        // direction recorded on either side updates the same bucket.
        let mut buckets: HashMap<(u32, u32), [Option<DirectionSample>; 2]> = HashMap::new();
        for p in active.iter() {
            for (target_id, rel) in p.relations.player_relations_iter() {
                if !squad_ids.contains(target_id) {
                    // Departed players (transferred / retired / out on
                    // loan / long-term injured) are ignored — their
                    // relation rows don't reflect this week's dressing
                    // room.
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

/// Strong-bond edge threshold for the faction graph. Pairs whose
/// per-direction averaged harmony hits this bar form a clique edge.
const FACTION_BOND_EDGE: f32 = 60.0;
/// Hostility threshold for the "cross-faction tension" reading. A
/// pair below this on the harmony scale counts as hostile when their
/// endpoints land in different factions.
const FACTION_HOSTILITY_THRESHOLD: f32 = 25.0;

/// Builds the squad-faction structure from the active pair walk.
///
/// Faction = connected component of the "strong-bond" graph
/// (`PairSample::harmony >= 60`). A player with no strong-bond edge is
/// its own component → counted as `isolated_players`. Cross-component
/// hostile pairs (harmony < 25) drive `faction_tension`.
///
/// Implementation note: a plain union-find keeps the build O(E·α(N))
/// without pulling in a graph crate. Active squads are small enough
/// (≤32 typically) that the constant overhead doesn't matter.
struct FactionWalk {
    /// One entry per active player, mapping player_id → component_root.
    roots: HashMap<u32, u32>,
    /// Hostile cross-component edge count.
    hostile_cross: u32,
    /// Total hostile pair count (cross + within) used to scale tension.
    hostile_total: u32,
    /// Active player count — used to compute the largest-faction share.
    active_count: u32,
}

impl FactionWalk {
    fn collect(active: &[&Player]) -> Self {
        let mut uf = UnionFind::new(active.iter().map(|p| p.id));
        let squad_ids: HashSet<u32> = active.iter().map(|p| p.id).collect();

        // Build undirected pair table from the same data PairWalk
        // walks, but average per-direction harmony BEFORE thresholding
        // so a one-sided strong bond doesn't elevate an otherwise weak
        // pair into a clique edge.
        let mut pair_table: HashMap<(u32, u32), [Option<f32>; 2]> = HashMap::new();
        for p in active.iter() {
            for (target_id, rel) in p.relations.player_relations_iter() {
                if !squad_ids.contains(target_id) || *target_id == p.id {
                    continue;
                }
                let sample = DirectionSample::from_relation(rel);
                let key = if p.id < *target_id {
                    (p.id, *target_id)
                } else {
                    (*target_id, p.id)
                };
                let slot = if p.id < *target_id { 0 } else { 1 };
                let entry = pair_table.entry(key).or_insert([None, None]);
                entry[slot] = Some(sample.harmony);
            }
        }

        let mut hostile_total = 0u32;
        let mut hostile_pairs: Vec<(u32, u32)> = Vec::new();
        for ((a, b), sides) in pair_table.iter() {
            let mut acc = 0.0f32;
            let mut count = 0u32;
            for s in sides.iter().flatten() {
                acc += s;
                count += 1;
            }
            if count == 0 {
                continue;
            }
            let avg = acc / count as f32;
            if avg >= FACTION_BOND_EDGE {
                uf.union(*a, *b);
            }
            if avg < FACTION_HOSTILITY_THRESHOLD {
                hostile_total += 1;
                hostile_pairs.push((*a, *b));
            }
        }

        let mut hostile_cross = 0u32;
        for (a, b) in hostile_pairs {
            if uf.find(a) != uf.find(b) {
                hostile_cross += 1;
            }
        }

        let mut roots: HashMap<u32, u32> = HashMap::new();
        for &id in squad_ids.iter() {
            roots.insert(id, uf.find(id));
        }

        Self {
            roots,
            hostile_cross,
            hostile_total,
            active_count: active.len() as u32,
        }
    }

    fn into_snapshot(self) -> SquadFactionSnapshot {
        if self.active_count == 0 {
            return SquadFactionSnapshot::default();
        }

        // Group by root to enumerate components.
        let mut comp_sizes: HashMap<u32, u32> = HashMap::new();
        for &root in self.roots.values() {
            *comp_sizes.entry(root).or_insert(0) += 1;
        }

        let largest = comp_sizes.values().copied().max().unwrap_or(0);
        let isolated = comp_sizes.values().filter(|&&s| s == 1).count() as u32;
        let faction_count = comp_sizes.len() as u32;
        let largest_share = if self.active_count == 0 {
            0.0
        } else {
            (largest as f32) / (self.active_count as f32)
        };

        // Tension: share of hostile pairs that crossed faction lines,
        // amplified by the cross count's contribution to total
        // hostility. Bounded 0..1.
        let tension = if self.hostile_total == 0 {
            0.0
        } else {
            (self.hostile_cross as f32 / self.hostile_total as f32).clamp(0.0, 1.0)
        };

        SquadFactionSnapshot {
            faction_count: faction_count.min(u8::MAX as u32) as u8,
            largest_faction_share: largest_share.clamp(0.0, 1.0),
            isolated_players: isolated.min(u8::MAX as u32) as u8,
            faction_tension: tension,
        }
    }
}

/// Minimal union-find used by the faction walk. Path-compressed find,
/// rank-tied union. Sized for active rosters (~32), so a Vec-backed
/// dense store isn't worth the index translation cost.
struct UnionFind {
    parent: HashMap<u32, u32>,
    rank: HashMap<u32, u32>,
}

impl UnionFind {
    fn new<I: IntoIterator<Item = u32>>(ids: I) -> Self {
        let mut parent = HashMap::new();
        let mut rank = HashMap::new();
        for id in ids {
            parent.insert(id, id);
            rank.insert(id, 0);
        }
        Self { parent, rank }
    }

    fn find(&mut self, mut x: u32) -> u32 {
        // Walk to the root, then collapse the path.
        let mut path: Vec<u32> = Vec::new();
        while let Some(&p) = self.parent.get(&x) {
            if p == x {
                break;
            }
            path.push(x);
            x = p;
        }
        for node in path {
            self.parent.insert(node, x);
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        let rank_a = *self.rank.get(&ra).unwrap_or(&0);
        let rank_b = *self.rank.get(&rb).unwrap_or(&0);
        match rank_a.cmp(&rank_b) {
            std::cmp::Ordering::Less => {
                self.parent.insert(ra, rb);
            }
            std::cmp::Ordering::Greater => {
                self.parent.insert(rb, ra);
            }
            std::cmp::Ordering::Equal => {
                self.parent.insert(rb, ra);
                self.rank.insert(ra, rank_a + 1);
            }
        }
    }
}

/// Head-coach lookup with the spec's full fallback chain: Manager →
/// CaretakerManager → AssistantManager → any contracted coach. Returns
/// `None` only when every seat is vacant (an unlikely simulator state
/// — the manager market normally fills the head-coach role first).
struct ManagerLookup;

impl ManagerLookup {
    /// Pick the staff member the snapshot should treat as "the head
    /// coach". Delegates to [`crate::StaffCollection::social_head_coach`]
    /// so every social system (talks, credibility, snapshot, conflict
    /// gating) reads from the same fallback chain.
    fn for_snapshot(team: &Team) -> Option<&Staff> {
        team.staffs.social_head_coach()
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
    fn collect(team: &Team, today: NaiveDate, active: &[&Player]) -> Self {
        let n = active.len() as f32;
        if n <= 0.0 {
            return Self {
                nationality_support: 50.0,
                language_support: 0.5,
                tenure_blend: 0.5,
                adaptability: 0.5,
            };
        }

        let mut nat_counts: HashMap<u32, u16> = HashMap::new();
        for p in active.iter() {
            *nat_counts.entry(p.country_id).or_insert(0) += 1;
        }
        let with_compatriots = active
            .iter()
            .filter(|p| nat_counts.get(&p.country_id).copied().unwrap_or(0) >= 2)
            .count() as f32;
        let nationality_support = (with_compatriots / n * 100.0).clamp(0.0, 100.0);

        let with_lang = active
            .iter()
            .filter(|p| {
                p.squad_social_view
                    .as_ref()
                    .map(|v| v.same_language_teammates > 0)
                    .unwrap_or(false)
            })
            .count() as f32;
        let language_support = (with_lang / n).clamp(0.0, 1.0);

        let tenure_blend = Self::squad_tenure_blend(active, today);
        let adaptability_sum: f32 = active
            .iter()
            .map(|p| (p.attributes.adaptability / 20.0).clamp(0.0, 1.0))
            .sum();
        let adaptability = (adaptability_sum / n).clamp(0.0, 1.0);

        let _ = team;
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
    /// linearly to 1.0 over [`TENURE_FULL_DAYS`].
    ///
    /// The reference is `today` (simulation date), not the squad's most
    /// recent transfer — an old, settled squad reads correctly even when
    /// every player joined in the same window.
    pub(crate) fn squad_tenure_blend(active: &[&Player], today: NaiveDate) -> f32 {
        if active.is_empty() {
            return 0.5;
        }
        let sum: f32 = active
            .iter()
            .map(|p| match p.last_transfer_date {
                None => 1.0,
                Some(joined) => {
                    let days = (today - joined).num_days().max(0) as f32;
                    (days / TENURE_FULL_DAYS).clamp(0.0, 1.0)
                }
            })
            .sum();
        (sum / active.len() as f32).clamp(0.0, 1.0)
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
    fn collect(team: &Team, active: &[&Player]) -> Self {
        let captain_leadership = team
            .captain_id
            .and_then(|id| active.iter().find(|p| p.id == id))
            .map(|p| p.skills.mental.leadership.clamp(0.0, 20.0));
        let vice_leadership = team
            .vice_captain_id
            .and_then(|id| active.iter().find(|p| p.id == id))
            .map(|p| p.skills.mental.leadership.clamp(0.0, 20.0));

        let mut senior_pool: Vec<f32> = active
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

        let prof_sum: f32 = active
            .iter()
            .map(|p| (p.attributes.professionalism).clamp(0.0, 20.0))
            .sum();
        let squad_professionalism = if active.is_empty() {
            10.0
        } else {
            prof_sum / active.len() as f32
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

/// Debug / LLM-facing read of the team's social weather. Bundles the
/// headline snapshot, the captain mediation score, top-3 conflict-risk
/// players (with the reason-side bond breakdown), and top-3 isolated
/// players. The shape is stable so a save-state debug pane or an
/// LLM-prompt narrator can read each field by name without having to
/// rebuild any of the underlying signals itself.
#[derive(Debug, Clone)]
pub struct TeamSocialDebug {
    pub snapshot: TeamSocialSnapshot,
    pub captain_mediation_score: f32,
    pub captain_mediation_is_fallback: bool,
    pub captain_id: Option<u32>,
    pub top_conflict_risk_players: Vec<ConflictRiskDebugEntry>,
    pub top_isolated_players: Vec<u32>,
}

/// Per-player conflict-risk debug entry. Surfaces the four-axis
/// `CoachPlayerBond` numbers AND the underlying `BondInputs` so a
/// narrator can attribute *why* the player is at risk.
#[derive(Debug, Clone, Copy)]
pub struct ConflictRiskDebugEntry {
    pub player_id: u32,
    pub effective_conflict_risk: f32,
    pub raw_conflict_risk: f32,
    pub selection_trust: f32,
    pub training_receptiveness: f32,
    pub tactical_buy_in: f32,
    pub breakdown: CoachPlayerBondBreakdown,
}

impl TeamSocialDebug {
    /// Build the debug snapshot. Pure read — no mutations, safe to
    /// call from any UI / debug / LLM-prompt path. Returns the
    /// neutral snapshot when the team has no head coach (every
    /// per-player bond would be a neutral fallback in that case).
    pub fn build(team: &Team, today: NaiveDate) -> Self {
        let snapshot = TeamSocialSnapshot::build(team, today);
        let mediation = CaptainMediation::for_captain(&team.players, team.captain_id);

        let Some(coach) = team.staffs.social_head_coach() else {
            return Self {
                snapshot,
                captain_mediation_score: mediation.leader_support(),
                captain_mediation_is_fallback: mediation.is_fallback(),
                captain_id: mediation.captain_id(),
                top_conflict_risk_players: Vec::new(),
                top_isolated_players: Vec::new(),
            };
        };

        let active = TeamSocialSnapshot::active_social_players(team);
        let mut risk_entries: Vec<ConflictRiskDebugEntry> = active
            .iter()
            .map(|p| {
                let (bond, breakdown) =
                    CoachPlayerBond::build_with_breakdown(p, coach, today);
                let effective = mediation.effective_risk(bond.conflict_risk, p);
                ConflictRiskDebugEntry {
                    player_id: p.id,
                    effective_conflict_risk: effective,
                    raw_conflict_risk: bond.conflict_risk,
                    selection_trust: bond.selection_trust,
                    training_receptiveness: bond.training_receptiveness,
                    tactical_buy_in: bond.tactical_buy_in,
                    breakdown,
                }
            })
            .collect();
        risk_entries.sort_by(|a, b| {
            b.effective_conflict_risk
                .partial_cmp(&a.effective_conflict_risk)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        risk_entries.truncate(3);

        // Isolation read: re-walk the faction graph to find one-member
        // components. We only return ids, in faction-walk discovery
        // order — the snapshot's `isolated_players` count already says
        // how many there are.
        let isolated = Self::isolated_player_ids(&active);

        Self {
            snapshot,
            captain_mediation_score: mediation.leader_support(),
            captain_mediation_is_fallback: mediation.is_fallback(),
            captain_id: mediation.captain_id(),
            top_conflict_risk_players: risk_entries,
            top_isolated_players: isolated,
        }
    }

    fn isolated_player_ids(active: &[&Player]) -> Vec<u32> {
        let walk = FactionWalk::collect(active);
        let mut comp_size: HashMap<u32, u32> = HashMap::new();
        for &root in walk.roots.values() {
            *comp_size.entry(root).or_insert(0) += 1;
        }
        let mut isolated: Vec<u32> = walk
            .roots
            .iter()
            .filter(|(_, root)| comp_size.get(root).copied().unwrap_or(0) == 1)
            .map(|(id, _)| *id)
            .collect();
        // Stable ordering — by id — so the LLM-facing render doesn't
        // flicker between runs.
        isolated.sort();
        isolated.truncate(3);
        isolated
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

    // ── Improvement-pass tests ─────────────────────────────────────

    #[test]
    fn old_settled_squad_reads_high_tenure_blend() {
        // Polish task #2: per-player days-since-arrival rampto 730d.
        // A squad whose players all arrived 3 years ago must read
        // tenure_blend > 0.95 — every player is past the 2-year ramp.
        let today = SnapshotFixture::today();
        let three_years_ago = today - Duration::days(3 * 365);
        let players: Vec<&Player> = Vec::new();
        let blend_empty = IntegrationParts::squad_tenure_blend(&players, today);
        assert!(
            (blend_empty - 0.5).abs() < 1e-3,
            "empty squad must return 0.5 neutral"
        );

        let owned: Vec<Player> = (1..=5)
            .map(|id| {
                let mut p = SnapshotFixture::player(id, 10.0, 1);
                p.last_transfer_date = Some(three_years_ago);
                p
            })
            .collect();
        let active: Vec<&Player> = owned.iter().collect();
        let blend = IntegrationParts::squad_tenure_blend(&active, today);
        assert!(
            blend > 0.95,
            "3-year settled squad tenure_blend = {} (expected > 0.95)",
            blend
        );

        // Fresh-arrival squad → < 0.05.
        let owned_fresh: Vec<Player> = (1..=5)
            .map(|id| {
                let mut p = SnapshotFixture::player(id, 10.0, 1);
                p.last_transfer_date = Some(today);
                p
            })
            .collect();
        let active_fresh: Vec<&Player> = owned_fresh.iter().collect();
        let blend_fresh = IntegrationParts::squad_tenure_blend(&active_fresh, today);
        assert!(
            blend_fresh < 0.05,
            "fresh-arrival squad tenure_blend = {} (expected < 0.05)",
            blend_fresh
        );

        // Mixed: half settled, half fresh — should land midway.
        let mut mixed = owned;
        mixed[0].last_transfer_date = Some(today);
        mixed[1].last_transfer_date = Some(today);
        let active_mixed: Vec<&Player> = mixed.iter().collect();
        let blend_mixed = IntegrationParts::squad_tenure_blend(&active_mixed, today);
        assert!(
            blend_mixed > 0.4 && blend_mixed < 0.7,
            "mixed squad tenure_blend = {} (expected midpoint band)",
            blend_mixed
        );
    }

    #[test]
    fn out_on_loan_hostile_player_ignored_by_snapshot() {
        // Polish task #9: a player out on loan must not influence
        // pair harmony / conflict density. Install a hostile pair on
        // a squad, then mark one side out on loan, and assert the
        // densities snap back to the peaceful baseline.
        use crate::ChangeType;
        use crate::PlayerClubContract;
        use crate::RelationshipChange;
        let date = SnapshotFixture::today();

        let mut players: Vec<Player> = (1..=4)
            .map(|id| SnapshotFixture::player(id, 10.0, 1))
            .collect();
        let baseline_team = SnapshotFixture::build_team(players.clone());
        let baseline = TeamSocialSnapshot::build(&baseline_team, date);

        // Mirror `install_rivalry`: drive PersonalConflict updates AND
        // mark a CompetitionRivalry so the conflict_contribution lights
        // up regardless of exact level threshold.
        for _ in 0..15 {
            players[0].relations.update_with_type(
                2,
                -0.8,
                ChangeType::PersonalConflict,
                date,
            );
            players[1].relations.update_with_type(
                1,
                -0.8,
                ChangeType::PersonalConflict,
                date,
            );
        }
        players[1].relations.update_player_relationship(
            1,
            RelationshipChange::negative(ChangeType::CompetitionRivalry, 0.9),
            date,
        );

        // First confirm the hostility registers when both players are
        // active — guards the test from a no-op state.
        let hostile_team = SnapshotFixture::build_team(players.clone());
        let hostile_snap = TeamSocialSnapshot::build(&hostile_team, date);
        assert!(
            hostile_snap.conflict_density > baseline.conflict_density,
            "test precondition: hostile pair must lift conflict_density (base={} hostile={})",
            baseline.conflict_density,
            hostile_snap.conflict_density
        );

        // Flag player 1 out on loan; pair walk should skip the pair.
        players[0].contract_loan = Some(PlayerClubContract::new(
            5_000,
            date + Duration::days(180),
        ));
        let loaned_team = SnapshotFixture::build_team(players);
        let snap = TeamSocialSnapshot::build(&loaned_team, date);

        assert!(
            (snap.conflict_density - baseline.conflict_density).abs() < 1e-3,
            "loanee hostile pair must not affect conflict_density (base={} got={})",
            baseline.conflict_density,
            snap.conflict_density
        );
    }

    #[test]
    fn faction_tension_lowers_team_chemistry() {
        // Polish task #8: cross-faction hostility taxes chemistry.
        // We seed the relation graph directly so the faction walk
        // sees clearly-defined intra-faction bonds and one explicit
        // cross-faction hostile pair — avoiding fragile update arithmetic.
        let date = SnapshotFixture::today();

        let owned: Vec<Player> = (1..=6)
            .map(|id| SnapshotFixture::player(id, 10.0, 1))
            .collect();
        let peaceful = SnapshotFixture::build_team(owned.clone());
        let peaceful_snap = TeamSocialSnapshot::build(&peaceful, date);

        let mut hostile_players = owned;
        // Strong intra-faction bonds: harmony lands well above the
        // FACTION_BOND_EDGE = 60 threshold for both factions.
        let bond_pairs = [(1u32, 2u32), (2, 3), (1, 3), (4, 5), (5, 6), (4, 6)];
        for &(a, b) in bond_pairs.iter() {
            for &(src, dst) in &[(a, b), (b, a)] {
                if let Some(p) = hostile_players.iter_mut().find(|p| p.id == src) {
                    p.relations.update_simple(dst, 100.0);
                    if let Some(rel) = p.relations.get_player(dst) {
                        // Snapshot reads trust + professional_respect too;
                        // bond writes only level so we top those manually.
                        let _ = rel;
                    }
                    // update_simple only bumps level; reach inside via the
                    // public RelationshipChange API to lift trust/prof.
                    use crate::{ChangeType, RelationshipChange};
                    p.relations.update_player_relationship(
                        dst,
                        RelationshipChange::positive(ChangeType::TeamSuccess, 100.0),
                        date,
                    );
                }
            }
        }
        // One explicit cross-faction hostile pair (1 ↔ 4) — drive
        // level deep negative so harmony lands below
        // FACTION_HOSTILITY_THRESHOLD = 25.
        for &(src, dst) in &[(1u32, 4u32), (4, 1)] {
            if let Some(p) = hostile_players.iter_mut().find(|p| p.id == src) {
                p.relations.update_simple(dst, -100.0);
                use crate::{ChangeType, RelationshipChange};
                p.relations.update_player_relationship(
                    dst,
                    RelationshipChange::negative(ChangeType::PersonalConflict, 100.0),
                    date,
                );
            }
        }

        let hostile_team = SnapshotFixture::build_team(hostile_players);
        let hostile_snap = TeamSocialSnapshot::build(&hostile_team, date);

        assert!(
            hostile_snap.factions.faction_count >= 2,
            "expected ≥2 factions, got {}",
            hostile_snap.factions.faction_count
        );
        assert!(
            hostile_snap.factions.faction_tension > 0.0,
            "expected non-zero faction tension (got {})",
            hostile_snap.factions.faction_tension
        );
        assert!(
            hostile_snap.team_chemistry < peaceful_snap.team_chemistry,
            "faction tension must lower team chemistry (peace={} hostile={})",
            peaceful_snap.team_chemistry,
            hostile_snap.team_chemistry
        );
    }
}

