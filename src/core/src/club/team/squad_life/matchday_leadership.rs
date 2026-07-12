//! Matchday armband resolution.
//!
//! Separate from the persistent club captaincy (`captaincy.rs`, assigned
//! monthly by `CaptaincyAssigner` and stored on `Team.captain_id` /
//! `vice_captain_id`). That hierarchy answers "who is the club captain this
//! month"; this module answers "who actually wears the armband for *this*
//! match", which depends on who was selected.
//!
//! The rules:
//!   1. The persistent club captain wears it **if he started** (is in the
//!      selected XI) and isn't hard-invalid (leadership collapsed below the
//!      floor). His age is not checked — an official young captain keeps the
//!      armband whenever the club has appointed him.
//!   2. Otherwise the persistent vice-captain wears it under the same
//!      filter.
//!   3. Otherwise the best leader in the selected XI is chosen, first from
//!      the *mature* leadership-floor pool (`leadership >=
//!      MIN_LEADERSHIP_FOR_FALLBACK && (age >= MATURITY_AGE ||
//!      young_exception)`), then from the full leadership-floor pool, then
//!      from the entire XI if no starter clears the floor at all.
//! The vice slot is filled by the same priority over the remaining XI,
//! excluding whoever took the captaincy.
//!
//! The candidate pool is *always* the on-field set passed in — so a benched
//! captain never wears the armband, and the same function re-resolves the
//! armband when the captain leaves the pitch (substitution / red card): pass
//! the still-active players and the persistent hierarchy, and the vice (if
//! still on) or the best remaining leader is returned.
//!
//! National teams have no persistent hierarchy, so they pass `None, None` and
//! fall straight through to the best-leader rule. The richer score also
//! makes the rotated-XI / national-team path align with the persistent
//! captaincy model: it reads pressure, consistency, important-matches,
//! discipline, condition, jadedness, positional convention *and an age /
//! seniority block*, so a hot-running 20-year-old doesn't out-stand a
//! 28-year-old senior on a rotated cup XI or a national pick.

use crate::Player;
use crate::PlayerPositionType;
use crate::r#match::MatchPlayer;
use crate::utils::DateUtils;
use chrono::NaiveDate;
use std::cmp::Ordering;

/// Minimum leadership (0..20) for the primary best-leader pool. Mirrors
/// the same floor the persistent `CaptaincyAssigner` uses, so the
/// matchday fallback only reaches for genuine dressing-room voices.
const MIN_LEADERSHIP_FOR_FALLBACK: f32 = 8.0;

/// Leadership floor below which a persistent captain / vice loses the
/// armband even though he started. A conservative hard-invalidity gate:
/// in practice the persistent captaincy model already strips players
/// below the eligibility threshold, so this only fires when something
/// catastrophic has happened to a sitting captain between monthly ticks.
const HARD_INVALIDITY_LEADERSHIP: f32 = 6.0;

/// Age (years) at which a player is considered a mature dressing-room voice
/// and so belongs in the *primary* fallback pool by default. Younger
/// players can still get in via [`LeadershipCandidate::young_exception`].
const MATURITY_AGE: u8 = 23;

/// Tie-break granularity on the 0..100 score. Scores are quantised onto
/// this grid before comparison, so sub-grid differences (floating-point
/// noise) fall through to the deterministic tie-break chain. Quantising —
/// rather than an `|a - b| <= eps` window — keeps the comparator
/// transitive: an epsilon window is not transitive, and `max_by` with a
/// non-transitive comparator is order-sensitive, meaning the same XI
/// listed in a different order could wear a different armband.
const SCORE_EPSILON: f32 = 0.01;

// ---------------------------------------------------------------------------
// Final-blend block weights (sum = 1.0). Each block is scored on 0..1 and
// scaled to 0..100 in the final blend; matchday penalties are then added
// in absolute points.
// ---------------------------------------------------------------------------
const W_CORE_VOICE: f32 = 0.47;
const W_RELIABILITY: f32 = 0.18;
const W_STATURE: f32 = 0.16;
const W_SENIORITY: f32 = 0.11;
const W_READINESS: f32 = 0.08;

// ---------------------------------------------------------------------------
// Matchday penalties (absolute points on the 0..100 scale). These are
// matchday-specific — a wrecked condition / heavy jadedness / discipline
// risk that would have a club captain rested or substituted. They never
// fire on the persistent monthly captaincy.
// ---------------------------------------------------------------------------
const PEN_CONDITION_LOW: f32 = -12.0;
const PEN_CONDITION_TIRED: f32 = -5.0;
const PEN_JADEDNESS_HEAVY: f32 = -8.0;
const PEN_JADEDNESS_MODERATE: f32 = -4.0;
const PEN_LEADERSHIP_LOW: f32 = -10.0;
const PEN_LEADERSHIP_FLOOR: f32 = -20.0;
const PEN_VOLATILE: f32 = -6.0;

// Age penalties (absolute points). Fallback ranking only — never apply to
// the persistent captain / vice priority (those still go through
// `valid_present`, which only checks the hard-invalidity floor).
const PEN_AGE_TEEN: f32 = -18.0;
const PEN_AGE_19_20: f32 = -12.0;
const PEN_AGE_19_20_EXCEPTIONAL: f32 = -4.0;
const PEN_AGE_21_22: f32 = -6.0;
const PEN_AGE_21_22_EXCEPTIONAL: f32 = 0.0;

/// Resolved armband holders for a single match, by player id. Always point
/// at players in the candidate pool they were resolved from (never a benched
/// or unselected player).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchdayLeadership {
    pub captain_id: Option<u32>,
    pub vice_captain_id: Option<u32>,
}

/// Leadership-scored candidate drawn from a selected squad. Holds the
/// scalars the matchday ranking needs, so the resolver is independent of
/// whether the source was a persisted `Player` (club / national selection,
/// post-match roster) or an in-match `MatchPlayer`.
///
/// The signal set mirrors the persistent `CaptaincyModel` so rotated-XI and
/// national-team fallbacks (where no official captain/vice starts) make
/// the same kind of pick the monthly assigner would: leadership + the
/// supporting mental block, club authority via reputation / position fit
/// / loyalty / experience, matchday reliability via discipline and
/// consistency, an explicit age/seniority block so a hot-running 20-year-
/// old doesn't out-stand a 28-year-old senior, and a readiness term that
/// quietly demotes a wrecked or heavy-legged starter.
#[derive(Debug, Clone, Copy)]
pub struct LeadershipCandidate {
    pub id: u32,
    pub age: u8,
    pub position: PlayerPositionType,
    pub leadership: f32,
    pub teamwork: f32,
    pub determination: f32,
    pub composure: f32,
    pub professionalism: f32,
    pub loyalty: f32,
    pub pressure: f32,
    pub consistency: f32,
    pub important_matches: f32,
    pub temperament: f32,
    pub sportsmanship: f32,
    pub dirtiness: f32,
    pub controversy: f32,
    /// Current reputation on the 0..~10_000 scale.
    pub reputation: f32,
    /// International appearances, used as an experience proxy.
    pub experience: f32,
    /// Condition as a 0..100 percentage at kickoff (or at the moment the
    /// armband is re-resolved). Drives the matchday readiness block and
    /// the low-condition penalty.
    pub condition_pct: f32,
    /// Accumulated jadedness on the 0..10_000 scale. Reads as deep fatigue
    /// — penalises a flogged starter even with a fresh-looking condition.
    pub jadedness: f32,
}

impl LeadershipCandidate {
    /// Build a candidate from a persisted `Player` evaluated at `date`
    /// (club / national selection, post-match roster lookups).
    pub fn from_player_at(player: &Player, date: NaiveDate) -> Self {
        LeadershipCandidate {
            id: player.id,
            age: DateUtils::age(player.birth_date, date),
            position: player.position(),
            leadership: player.skills.mental.leadership,
            teamwork: player.skills.mental.teamwork,
            determination: player.skills.mental.determination,
            composure: player.skills.mental.composure,
            professionalism: player.attributes.professionalism,
            loyalty: player.attributes.loyalty,
            pressure: player.attributes.pressure,
            consistency: player.attributes.consistency,
            important_matches: player.attributes.important_matches,
            temperament: player.attributes.temperament,
            sportsmanship: player.attributes.sportsmanship,
            dirtiness: player.attributes.dirtiness,
            controversy: player.attributes.controversy,
            reputation: player.player_attributes.current_reputation as f32,
            experience: player.player_attributes.international_apps as f32,
            condition_pct: player.player_attributes.condition_percentage() as f32,
            jadedness: player.player_attributes.jadedness.max(0) as f32,
        }
    }

    /// Build a candidate from a selected `MatchPlayer` evaluated at `date`.
    /// Used by the squad builders, which only carry the match-side player
    /// view.
    pub fn from_match_player_at(player: &MatchPlayer, date: NaiveDate) -> Self {
        LeadershipCandidate {
            id: player.id,
            age: DateUtils::age(player.birth_date, date),
            position: player.tactical_position.current_position,
            leadership: player.skills.mental.leadership,
            teamwork: player.skills.mental.teamwork,
            determination: player.skills.mental.determination,
            composure: player.skills.mental.composure,
            professionalism: player.attributes.professionalism,
            loyalty: player.attributes.loyalty,
            pressure: player.attributes.pressure,
            consistency: player.attributes.consistency,
            important_matches: player.attributes.important_matches,
            temperament: player.attributes.temperament,
            sportsmanship: player.attributes.sportsmanship,
            dirtiness: player.attributes.dirtiness,
            controversy: player.attributes.controversy,
            reputation: player.player_attributes.current_reputation as f32,
            experience: player.player_attributes.international_apps as f32,
            condition_pct: player.player_attributes.condition_percentage() as f32,
            jadedness: player.player_attributes.jadedness.max(0) as f32,
        }
    }

    /// Normalise a 0..20 attribute to a 0..1 factor. Associated rather than
    /// a free fn so all matchday-leadership helpers stay namespaced under
    /// the candidate type.
    fn n20(attr: f32) -> f32 {
        (attr / 20.0).clamp(0.0, 1.0)
    }

    /// Composite matchday leader score on 0..100, after blending the five
    /// blocks and applying the matchday penalties. Larger is better.
    fn score(&self) -> f32 {
        let core = self.core_voice();
        let rel = self.reliability();
        let sta = self.stature();
        let sen = self.seniority();
        let rdy = self.readiness();

        let blended = core * W_CORE_VOICE
            + rel * W_RELIABILITY
            + sta * W_STATURE
            + sen * W_SENIORITY
            + rdy * W_READINESS;
        (100.0 * blended + self.matchday_penalties()).clamp(0.0, 100.0)
    }

    /// Dressing-room voice: leadership and the supporting mental block,
    /// with pressure / consistency / important-matches priming the matchday
    /// view. 0..1.
    fn core_voice(&self) -> f32 {
        Self::n20(self.leadership) * 0.36
            + Self::n20(self.professionalism) * 0.14
            + Self::n20(self.teamwork) * 0.12
            + Self::n20(self.determination) * 0.12
            + Self::n20(self.composure) * 0.10
            + Self::n20(self.pressure) * 0.08
            + Self::n20(self.consistency) * 0.04
            + Self::n20(self.important_matches) * 0.04
    }

    /// Can he be trusted to lead through 90 minutes — clean discipline,
    /// steady output, big-match temperament, body holding up. 0..1.
    fn reliability(&self) -> f32 {
        self.discipline01() * 0.40
            + Self::n20(self.consistency) * 0.25
            + Self::n20(self.important_matches) * 0.15
            + self.condition01() * 0.10
            + self.low_jaded01() * 0.10
    }

    /// Standing on the pitch: reputation, positional captaincy convention,
    /// loyalty to the badge, and (lightly) international experience. The
    /// experience term is kept small here so the dedicated seniority block
    /// can carry the age/career-arc signal without double-counting.
    /// 0..1.
    fn stature(&self) -> f32 {
        self.rep01() * 0.40
            + self.exp01() * 0.15
            + self.position_factor() * 0.25
            + Self::n20(self.loyalty) * 0.20
    }

    /// Seniority: age maturity dominates, lightly topped up by
    /// international experience. 0..1. Keeps the matchday fallback away
    /// from young players unless they clear the explicit
    /// [`young_exception`] gate.
    ///
    /// [`young_exception`]: LeadershipCandidate::young_exception
    fn seniority(&self) -> f32 {
        Self::age_factor(self.age) * 0.75 + self.exp01() * 0.25
    }

    /// Captaincy age curve: 25..31 peak authority, callow at the extremes.
    /// Mirrors the persistent monthly assigner's curve so the matchday
    /// fallback and the season-long appointment agree on who reads as a
    /// senior figure. Associated rather than a free fn so every matchday-
    /// leadership helper stays namespaced under the candidate type.
    fn age_factor(age: u8) -> f32 {
        match age {
            0..=18 => 0.08,
            19..=20 => 0.22,
            21..=22 => 0.45,
            23..=24 => 0.70,
            25..=31 => 1.00,
            32..=34 => 0.90,
            35..=36 => 0.70,
            _ => 0.50,
        }
    }

    /// Readiness for the matchday — fresh body, fresh legs, and the
    /// mental match-day attributes that decide whether he can drive a
    /// team through a tough 90. 0..1.
    fn readiness(&self) -> f32 {
        self.condition01() * 0.45
            + self.low_jaded01() * 0.30
            + Self::n20(self.pressure) * 0.15
            + Self::n20(self.composure) * 0.10
    }

    /// Sum of hard matchday penalties (absolute points on the 0..100 scale).
    /// These are matchday-specific — the persistent captaincy model already
    /// owns the season-long penalties (transfer flux, long-term injury, etc.).
    fn matchday_penalties(&self) -> f32 {
        let mut pen = 0.0;
        if self.condition_pct < 45.0 {
            pen += PEN_CONDITION_LOW;
        } else if self.condition_pct < 60.0 {
            pen += PEN_CONDITION_TIRED;
        }
        if self.jadedness > 8500.0 {
            pen += PEN_JADEDNESS_HEAVY;
        } else if self.jadedness > 7000.0 {
            pen += PEN_JADEDNESS_MODERATE;
        }
        if self.leadership < HARD_INVALIDITY_LEADERSHIP {
            pen += PEN_LEADERSHIP_FLOOR;
        } else if self.leadership < MIN_LEADERSHIP_FOR_FALLBACK {
            pen += PEN_LEADERSHIP_LOW;
        }
        if self.controversy > 17.0 && self.professionalism < 10.0 {
            pen += PEN_VOLATILE;
        }
        // Age — fallback ranking only. The persistent captain / vice route
        // skips this block entirely via `valid_present`, so an officially
        // appointed young captain keeps his armband.
        let exceptional = self.young_exception();
        pen += match self.age {
            0..=18 => PEN_AGE_TEEN,
            19..=20 if exceptional => PEN_AGE_19_20_EXCEPTIONAL,
            19..=20 => PEN_AGE_19_20,
            21..=22 if exceptional => PEN_AGE_21_22_EXCEPTIONAL,
            21..=22 => PEN_AGE_21_22,
            _ => 0.0,
        };
        pen
    }

    /// A young player is allowed into the primary fallback pool *only* if
    /// every facet of dressing-room voice, professionalism, drive and either
    /// stature or international experience clears a high bar. Mirrors the
    /// monthly assigner's youth-captain exception so the same prodigy who
    /// can captain the club month-to-month is the only kind of young player
    /// the matchday fallback will reach for.
    fn young_exception(&self) -> bool {
        self.age >= 20
            && self.leadership >= 17.0
            && self.professionalism >= 15.0
            && self.determination >= 14.0
            && (self.reputation >= 7000.0 || self.experience >= 20.0)
    }

    /// Low discipline risk: high temperament & sportsmanship, low dirtiness,
    /// low controversy. 0..1. Mirrors the persistent captaincy model's
    /// discipline term so the matchday read agrees with the season-long one.
    fn discipline01(&self) -> f32 {
        Self::n20(self.temperament) * 0.35
            + Self::n20(self.sportsmanship) * 0.25
            + (1.0 - Self::n20(self.dirtiness)) * 0.25
            + (1.0 - Self::n20(self.controversy)) * 0.15
    }

    /// Condition as a 0..1 factor (clamped — engine values occasionally
    /// drift slightly above 100% after recovery ticks).
    fn condition01(&self) -> f32 {
        (self.condition_pct / 100.0).clamp(0.0, 1.0)
    }

    /// Inverse jadedness on 0..1 — 1.0 means fresh legs, 0.0 means flogged.
    fn low_jaded01(&self) -> f32 {
        1.0 - (self.jadedness / 10_000.0).clamp(0.0, 1.0)
    }

    /// Reputation lift, square-root scaled to avoid the top of the curve
    /// dominating the block (matches the persistent captaincy model).
    fn rep01(&self) -> f32 {
        (self.reputation.max(0.0) / 10_000.0).sqrt().clamp(0.0, 1.0)
    }

    /// International experience as a saturating 0..1 — 50 caps reads as
    /// full experience, anything beyond is rounding.
    fn exp01(&self) -> f32 {
        (self.experience / 50.0).clamp(0.0, 1.0)
    }

    /// Positional captaincy convention: spine roles see the whole pitch
    /// and naturally lead; wide and advanced roles do less so. Mirrors
    /// the persistent captaincy model so the same player wears the
    /// armband in either path.
    fn position_factor(&self) -> f32 {
        use PlayerPositionType::*;
        match self.position {
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

    /// Deterministic best-first ordering for `max_by`: `Greater` means
    /// `a` is the better captain candidate than `b`. Scores are quantised
    /// onto the [`SCORE_EPSILON`] grid so rounding noise doesn't flip the
    /// armband while the ordering stays transitive (and therefore
    /// independent of candidate order — see the constant's docs); grid
    /// ties fall through a fixed chain (leadership → mental-trio sum →
    /// reputation → position factor → lower id). Lower id last keeps the
    /// call stable across builds.
    fn cmp_score(a: &Self, b: &Self) -> Ordering {
        let qa = (a.score() / SCORE_EPSILON).round() as i64;
        let qb = (b.score() / SCORE_EPSILON).round() as i64;
        if qa != qb {
            return qa.cmp(&qb);
        }
        a.leadership
            .partial_cmp(&b.leadership)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                let ma = a.professionalism + a.teamwork + a.determination;
                let mb = b.professionalism + b.teamwork + b.determination;
                ma.partial_cmp(&mb).unwrap_or(Ordering::Equal)
            })
            .then_with(|| {
                a.reputation
                    .partial_cmp(&b.reputation)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| {
                a.position_factor()
                    .partial_cmp(&b.position_factor())
                    .unwrap_or(Ordering::Equal)
            })
            // Lower id wins under `max_by`, so we flip the comparison here.
            .then_with(|| b.id.cmp(&a.id))
    }

    /// Hard invalidity: a sitting captain whose leadership has collapsed
    /// below the floor loses the armband even though he started. Anything
    /// gentler stays a tie-break, not an override.
    fn is_hard_invalid(&self) -> bool {
        self.leadership < HARD_INVALIDITY_LEADERSHIP
    }
}

impl MatchdayLeadership {
    /// Resolve the armband over a set of on-field candidates, honouring the
    /// persistent club hierarchy where it overlaps the candidate pool. See the
    /// module docs for the full rule set. `persistent_captain` /
    /// `persistent_vice` are the club's monthly armband holders (pass `None`
    /// for national teams or any side without a persistent hierarchy).
    pub fn resolve(
        persistent_captain: Option<u32>,
        persistent_vice: Option<u32>,
        candidates: &[LeadershipCandidate],
    ) -> Self {
        if candidates.is_empty() {
            return MatchdayLeadership::default();
        }

        let captain = Self::valid_present(candidates, persistent_captain)
            .or_else(|| Self::valid_present(candidates, persistent_vice))
            .or_else(|| Self::best_leader(candidates, &[]));

        let vice = match captain {
            Some(cap) => Self::valid_present(candidates, persistent_vice)
                .filter(|v| *v != cap)
                .or_else(|| {
                    Self::valid_present(candidates, persistent_captain).filter(|c| *c != cap)
                })
                .or_else(|| Self::best_leader(candidates, &[cap])),
            None => None,
        };

        MatchdayLeadership {
            captain_id: captain,
            vice_captain_id: vice,
        }
    }

    /// Resolve the armband over a selected `MatchPlayer` XI and return the
    /// chosen captain / vice as clones drawn from that XI, so the stored ids
    /// always point at genuinely selected starters. `persistent_*` carry the
    /// club hierarchy (pass `None` for national teams).
    ///
    /// `date` drives the age read on each candidate — passed in rather than
    /// taken from `Utc::now()` so simulation paths remain deterministic and
    /// historical replays score against the original matchday's ages.
    pub fn from_match_squad_at(
        persistent_captain: Option<u32>,
        persistent_vice: Option<u32>,
        main_squad: &[MatchPlayer],
        date: NaiveDate,
    ) -> (Option<MatchPlayer>, Option<MatchPlayer>) {
        let candidates: Vec<LeadershipCandidate> = main_squad
            .iter()
            .map(|p| LeadershipCandidate::from_match_player_at(p, date))
            .collect();
        let resolved = Self::resolve(persistent_captain, persistent_vice, &candidates);
        let pick =
            |id: Option<u32>| id.and_then(|id| main_squad.iter().find(|p| p.id == id).cloned());
        (pick(resolved.captain_id), pick(resolved.vice_captain_id))
    }

    /// An id only counts as present if its holder is in the candidate pool
    /// *and* isn't hard-invalid (leadership has collapsed below the floor).
    /// The hard-invalidity gate is conservative — anything short of that
    /// keeps the official hierarchy intact, including the age penalties that
    /// only ever fire on the fallback ranking.
    fn valid_present(candidates: &[LeadershipCandidate], id: Option<u32>) -> Option<u32> {
        let wanted = id?;
        candidates
            .iter()
            .find(|c| c.id == wanted)
            .filter(|c| !c.is_hard_invalid())
            .map(|c| c.id)
    }

    /// Best leader in the candidate pool, excluding `exclude`. Three-tier
    /// fallback: prefer mature leadership-floor candidates (or the rare
    /// `young_exception` prodigy); if that pool is empty fall through to
    /// the full leadership-floor pool; if *that* is empty pick the best of
    /// the full XI, so a side without a single natural leader still gets
    /// an armband.
    fn best_leader(candidates: &[LeadershipCandidate], exclude: &[u32]) -> Option<u32> {
        let pick = |c: &&LeadershipCandidate| !exclude.contains(&c.id);

        let primary = candidates
            .iter()
            .filter(pick)
            .filter(|c| c.leadership >= MIN_LEADERSHIP_FOR_FALLBACK)
            .filter(|c| c.age >= MATURITY_AGE || c.young_exception())
            .max_by(|a, b| LeadershipCandidate::cmp_score(a, b))
            .map(|c| c.id);
        if primary.is_some() {
            return primary;
        }

        let any_leader = candidates
            .iter()
            .filter(pick)
            .filter(|c| c.leadership >= MIN_LEADERSHIP_FOR_FALLBACK)
            .max_by(|a, b| LeadershipCandidate::cmp_score(a, b))
            .map(|c| c.id);
        if any_leader.is_some() {
            return any_leader;
        }

        candidates
            .iter()
            .filter(pick)
            .max_by(|a, b| LeadershipCandidate::cmp_score(a, b))
            .map(|c| c.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builders for test candidates. Wrapped in a unit struct so the test
    /// fixtures live behind a name and stay out of the free-function space.
    struct Fixture;

    impl Fixture {
        /// Candidate with the given id and leadership; every other attribute
        /// held at a neutral level so leadership (and the explicit exclude
        /// rules) decide the ranking. Defaults are healthy / fresh / clean,
        /// and the age is parked at 28 — squarely in the peak band so the
        /// age curve and penalties don't muddy attribute-driven assertions.
        fn cand(id: u32, leadership: f32) -> LeadershipCandidate {
            LeadershipCandidate {
                id,
                age: 28,
                position: PlayerPositionType::MidfielderCenter,
                leadership,
                teamwork: 10.0,
                determination: 10.0,
                composure: 10.0,
                professionalism: 10.0,
                loyalty: 10.0,
                pressure: 10.0,
                consistency: 10.0,
                important_matches: 10.0,
                temperament: 12.0,
                sportsmanship: 12.0,
                dirtiness: 6.0,
                controversy: 6.0,
                reputation: 1000.0,
                experience: 0.0,
                condition_pct: 100.0,
                jadedness: 0.0,
            }
        }

        /// Famous, charismatic but low-leadership starter — high reputation
        /// and stature but the dressing room doesn't follow him.
        fn celebrity(id: u32, leadership: f32) -> LeadershipCandidate {
            let mut c = Self::cand(id, leadership);
            c.reputation = 9500.0;
            c.experience = 80.0;
            c.position = PlayerPositionType::ForwardCenter;
            c
        }

        /// High-leadership, low-fame professional — the dressing-room voice.
        fn professional(id: u32, leadership: f32) -> LeadershipCandidate {
            let mut c = Self::cand(id, leadership);
            c.professionalism = 17.0;
            c.consistency = 15.0;
            c.important_matches = 14.0;
            c
        }
    }

    #[test]
    fn matchday_captain_is_starting_club_captain_when_selected() {
        let xi = vec![
            Fixture::cand(1, 8.0),
            Fixture::cand(2, 18.0),
            Fixture::cand(3, 12.0),
        ];
        // Club captain 1 started — he keeps the armband even though 2 is the
        // stronger raw leader.
        let r = MatchdayLeadership::resolve(Some(1), Some(3), &xi);
        assert_eq!(r.captain_id, Some(1));
        assert_eq!(r.vice_captain_id, Some(3));
    }

    #[test]
    fn matchday_captain_uses_vice_when_club_captain_benched() {
        let xi = vec![Fixture::cand(2, 9.0), Fixture::cand(3, 12.0)];
        // Club captain 1 was rotated out; vice 3 started → vice wears it.
        let r = MatchdayLeadership::resolve(Some(1), Some(3), &xi);
        assert_eq!(r.captain_id, Some(3));
        // Vice slot falls to the next best leader on the pitch.
        assert_eq!(r.vice_captain_id, Some(2));
    }

    #[test]
    fn matchday_captain_falls_back_to_best_xi_leader() {
        let xi = vec![
            Fixture::cand(5, 11.0),
            Fixture::cand(6, 17.0),
            Fixture::cand(7, 9.0),
        ];
        // Neither persistent captain nor vice on the pitch.
        let r = MatchdayLeadership::resolve(Some(1), Some(2), &xi);
        assert_eq!(r.captain_id, Some(6)); // strongest leader
        assert_eq!(r.vice_captain_id, Some(5)); // second strongest
    }

    #[test]
    fn matchday_captain_is_never_unselected_player() {
        let xi = vec![Fixture::cand(5, 11.0), Fixture::cand(6, 17.0)];
        // Persistent hierarchy points at players who didn't make the XI.
        let r = MatchdayLeadership::resolve(Some(99), Some(98), &xi);
        assert!(xi.iter().any(|c| Some(c.id) == r.captain_id));
        assert!(xi.iter().any(|c| Some(c.id) == r.vice_captain_id));
    }

    #[test]
    fn matchday_vice_excludes_captain() {
        let xi = vec![Fixture::cand(1, 18.0), Fixture::cand(2, 16.0)];
        // Degenerate input: same id as captain and vice. Vice must still be a
        // different player.
        let r = MatchdayLeadership::resolve(Some(1), Some(1), &xi);
        assert_eq!(r.captain_id, Some(1));
        assert_ne!(r.vice_captain_id, r.captain_id);
        assert_eq!(r.vice_captain_id, Some(2));
    }

    #[test]
    fn subbed_off_captain_transfers_armband_to_active_vice() {
        // Captain 1 has left the pitch; the still-active set is re-resolved.
        // Vice 2 is on → the armband transfers to him.
        let active = vec![Fixture::cand(2, 12.0), Fixture::cand(3, 16.0)];
        let r = MatchdayLeadership::resolve(Some(1), Some(2), &active);
        assert_eq!(r.captain_id, Some(2));
    }

    #[test]
    fn sent_off_captain_transfers_armband_to_best_active_leader() {
        // Captain 1 sent off and vice 2 already subbed off — neither is in the
        // active set, so the best remaining leader takes the armband.
        let active = vec![
            Fixture::cand(3, 10.0),
            Fixture::cand(4, 15.0),
            Fixture::cand(5, 13.0),
        ];
        let r = MatchdayLeadership::resolve(Some(1), Some(2), &active);
        assert_eq!(r.captain_id, Some(4));
    }

    #[test]
    fn match_events_use_actual_matchday_captain_not_team_captain() {
        // Mirrors the post-match path: the persistent club captain (1) was
        // benched, so leadership events must attach to the player who actually
        // started and led — never the stale club captain.
        let started_xi = vec![Fixture::cand(2, 14.0), Fixture::cand(3, 11.0)];
        let r = MatchdayLeadership::resolve(Some(1), None, &started_xi);
        assert_ne!(r.captain_id, Some(1));
        assert_eq!(r.captain_id, Some(2));
        assert!(started_xi.iter().any(|c| Some(c.id) == r.captain_id));
    }

    #[test]
    fn national_team_assigns_captain_from_selected_xi() {
        // National teams carry no persistent hierarchy: best leader in the
        // selected XI wears the armband.
        let xi = vec![
            Fixture::cand(10, 13.0),
            Fixture::cand(11, 17.0),
            Fixture::cand(12, 9.0),
        ];
        let r = MatchdayLeadership::resolve(None, None, &xi);
        assert_eq!(r.captain_id, Some(11));
        assert_eq!(r.vice_captain_id, Some(10));
        assert!(xi.iter().any(|c| Some(c.id) == r.captain_id));
    }

    #[test]
    fn bench_player_does_not_count_as_captain_at_kickoff() {
        // A strong leader (id 9) exists at the club but isn't in the XI pool,
        // so he can never be picked.
        let xi = vec![Fixture::cand(1, 10.0), Fixture::cand(2, 11.0)];
        let r = MatchdayLeadership::resolve(None, None, &xi);
        assert_ne!(r.captain_id, Some(9));
        assert!(xi.iter().any(|c| Some(c.id) == r.captain_id));
    }

    #[test]
    fn empty_xi_yields_no_armband() {
        let r = MatchdayLeadership::resolve(Some(1), Some(2), &[]);
        assert_eq!(r.captain_id, None);
        assert_eq!(r.vice_captain_id, None);
    }

    // -- Coverage for the richer matchday model ------------------------------

    #[test]
    fn low_leadership_celebrity_loses_to_high_leadership_professional() {
        // A famous but quiet 7-leadership forward shouldn't outrank a 16-
        // leadership central midfielder. The persistent-style score has to
        // weight dressing-room voice over raw fame.
        let xi = vec![Fixture::celebrity(1, 7.0), Fixture::professional(2, 16.0)];
        let r = MatchdayLeadership::resolve(None, None, &xi);
        assert_eq!(r.captain_id, Some(2));
    }

    #[test]
    fn collapsed_leadership_official_captain_loses_armband() {
        // Persistent captain is on the pitch but his leadership has collapsed
        // below the hard-invalidity floor (a season-long collapse the monthly
        // assigner hasn't caught yet). The next eligible XI member wears it.
        let xi = vec![
            Fixture::cand(1, 4.0),
            Fixture::cand(2, 14.0),
            Fixture::cand(3, 12.0),
        ];
        let r = MatchdayLeadership::resolve(Some(1), None, &xi);
        assert_ne!(r.captain_id, Some(1));
        assert_eq!(r.captain_id, Some(2));
    }

    #[test]
    fn full_xi_fallback_when_every_starter_below_leadership_floor() {
        // Rotation XI with no one above the floor — best of a bad lot still
        // wears the armband rather than the side fielding no captain.
        let xi = vec![Fixture::cand(1, 5.0), Fixture::cand(2, 7.0)];
        let r = MatchdayLeadership::resolve(None, None, &xi);
        assert_eq!(r.captain_id, Some(2)); // higher leadership of the two
    }

    #[test]
    fn exact_tie_is_deterministic_by_leadership_then_id() {
        // Two candidates with identical scoring inputs — score tie breaks via
        // the fixed chain (leadership equal here, so id breaks it: lower id
        // wins).
        let xi = vec![Fixture::cand(7, 14.0), Fixture::cand(3, 14.0)];
        let r1 = MatchdayLeadership::resolve(None, None, &xi);
        let xi2 = vec![Fixture::cand(3, 14.0), Fixture::cand(7, 14.0)];
        let r2 = MatchdayLeadership::resolve(None, None, &xi2);
        assert_eq!(r1.captain_id, Some(3));
        assert_eq!(r2.captain_id, Some(3));
    }

    #[test]
    fn near_tie_armband_is_independent_of_candidate_order() {
        // Three candidates whose scores sit a fraction of a point apart —
        // close enough that neighbouring pairs land inside the tie-break
        // grid while the extremes do not. A non-transitive comparator
        // (the old |a-b| <= epsilon window) made `max_by` order-sensitive
        // here: the same XI listed in a different order could produce a
        // different captain. The quantised comparator must return one
        // captain/vice pair for every permutation.
        let mut a = Fixture::cand(1, 14.0);
        a.condition_pct = 100.0;
        let mut b = Fixture::cand(2, 14.0);
        b.condition_pct = 99.9;
        let mut c = Fixture::cand(3, 14.0);
        c.condition_pct = 99.8;

        let orders: Vec<Vec<LeadershipCandidate>> = vec![
            vec![a, b, c],
            vec![a, c, b],
            vec![b, a, c],
            vec![b, c, a],
            vec![c, a, b],
            vec![c, b, a],
        ];
        let first = MatchdayLeadership::resolve(None, None, &orders[0]);
        for xi in &orders {
            let r = MatchdayLeadership::resolve(None, None, xi);
            assert_eq!(
                (r.captain_id, r.vice_captain_id),
                (first.captain_id, first.vice_captain_id),
                "armband must not depend on candidate order"
            );
        }
    }

    #[test]
    fn wrecked_condition_demotes_starter_below_fresh_peer() {
        // Two equally capable leaders; one's condition is wrecked (< 45%),
        // triggering the matchday condition penalty. The fresh peer should
        // take the armband even though leadership is identical.
        let mut tired = Fixture::cand(1, 15.0);
        tired.condition_pct = 30.0;
        let fresh = Fixture::cand(2, 15.0);
        let r = MatchdayLeadership::resolve(None, None, &vec![tired, fresh]);
        assert_eq!(r.captain_id, Some(2));
    }

    #[test]
    fn heavy_jadedness_demotes_starter_below_fresh_peer() {
        // Same idea, deep tiredness — a flogged starter with full condition
        // still reads worse than a fresh-legged peer.
        let mut flogged = Fixture::cand(1, 15.0);
        flogged.jadedness = 9000.0;
        let fresh = Fixture::cand(2, 15.0);
        let r = MatchdayLeadership::resolve(None, None, &vec![flogged, fresh]);
        assert_eq!(r.captain_id, Some(2));
    }

    #[test]
    fn spine_role_breaks_tie_against_wide_attacker() {
        // Identical leadership, otherwise neutral. The spine role (centre
        // back) should win the armband over a wide forward via the position
        // factor in stature.
        let mut cb = Fixture::cand(1, 14.0);
        cb.position = PlayerPositionType::DefenderCenter;
        let mut wide = Fixture::cand(2, 14.0);
        wide.position = PlayerPositionType::ForwardLeft;
        let r = MatchdayLeadership::resolve(None, None, &vec![cb, wide]);
        assert_eq!(r.captain_id, Some(1));
    }

    // -- Age / seniority guardrails -----------------------------------------

    #[test]
    fn twenty_year_old_does_not_captain_over_mature_senior() {
        // A 20-year-old leadership-15 starter with a healthy reputation
        // shouldn't out-stand a 28-year-old leadership-14 senior. The
        // primary pool excludes him (age < 23, doesn't clear young_exception)
        // so the senior takes the armband.
        let mut young = Fixture::cand(1, 15.0);
        young.age = 20;
        young.reputation = 5000.0;
        let mature = Fixture::cand(2, 14.0);
        let r = MatchdayLeadership::resolve(None, None, &vec![young, mature]);
        assert_eq!(r.captain_id, Some(2));
    }

    #[test]
    fn exceptional_young_leader_can_captain_fallback() {
        // A 21-year-old clearing the young-exception gate (leadership 18,
        // professionalism 16, determination 15, rep 8000) wins against an
        // older but lesser leader. Realistic: the genuine prodigy captain
        // is rare but allowed.
        let mut prodigy = Fixture::cand(1, 18.0);
        prodigy.age = 21;
        prodigy.professionalism = 16.0;
        prodigy.determination = 15.0;
        prodigy.reputation = 8000.0;
        let older = Fixture::cand(2, 12.0);
        let r = MatchdayLeadership::resolve(None, None, &vec![prodigy, older]);
        assert_eq!(r.captain_id, Some(1));
    }

    #[test]
    fn official_young_captain_still_keeps_armband_when_selected() {
        // Persistent captain is 21 and not hard-invalid (leadership 8 >= 6).
        // The age penalty only ever bites the fallback ranking — an officially
        // appointed young captain keeps it regardless of his age band.
        let mut official_young = Fixture::cand(1, 8.0);
        official_young.age = 21;
        let mature_alt = Fixture::cand(2, 17.0);
        let r = MatchdayLeadership::resolve(Some(1), None, &vec![official_young, mature_alt]);
        assert_eq!(r.captain_id, Some(1));
    }

    #[test]
    fn national_team_prefers_senior_leader_when_scores_are_close() {
        // No persistent hierarchy. Two leadership-14 candidates with
        // identical attributes apart from age — the senior should win the
        // armband. The seniority block tilts the close comparison without
        // needing the hard age penalty.
        let mut youngish = Fixture::cand(1, 14.0);
        youngish.age = 23; // youngest still in primary pool, peak block barely engaged
        let mut senior = Fixture::cand(2, 14.0);
        senior.age = 28;
        let r = MatchdayLeadership::resolve(None, None, &vec![youngish, senior]);
        assert_eq!(r.captain_id, Some(2));
    }

    #[test]
    fn fallback_still_returns_someone_when_xi_is_all_young() {
        // No mature candidate, no exceptional youngster. Primary pool is
        // empty, secondary leadership-floor pool catches whoever has
        // leadership >= 8 and the matchday side still gets an armband.
        let mut a = Fixture::cand(1, 10.0);
        a.age = 19;
        let mut b = Fixture::cand(2, 12.0);
        b.age = 20;
        let mut c = Fixture::cand(3, 8.0);
        c.age = 18;
        let r = MatchdayLeadership::resolve(None, None, &vec![a, b, c]);
        assert!(matches!(r.captain_id, Some(1) | Some(2) | Some(3)));
        // Most-leadership young player wins, since they all carry similar
        // age penalties.
        assert_eq!(r.captain_id, Some(2));
    }
}
