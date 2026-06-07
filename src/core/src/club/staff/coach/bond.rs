//! Derived coach-player bond — the unified social read every downstream
//! system (selection, training receptiveness, tactical buy-in, conflict
//! risk) consults instead of pulling the four underlying stores
//! (staff relation, rapport, promises, coach memory) directly.
//!
//! Why a single struct, not separate accessors per consumer: every
//! consumer of "is this coach-player relationship healthy?" needs the
//! same blend of evidence — long-term staff relation, short-term
//! rapport, promise track-record, and the coach's own memory. Composing
//! it once amortises the input reads and keeps the weighting in a
//! single place that's easy to tune. The four outputs share inputs but
//! weight them differently:
//!
//! * `selection_trust` — "would I put this player in the XI?"
//! * `training_receptiveness` — "is this player listening on the training ground?"
//! * `tactical_buy_in` — "does he believe in the plan?"
//! * `conflict_risk` — "is this relationship about to explode?"
//!
//! All outputs sit on 0..1. The struct is pure data; construction reads
//! everything it needs out of `&Player` and `&Staff` without taking
//! mutable borrows or touching any global state — safe to call from any
//! selection / behaviour / chemistry path.

use crate::HappinessEventType;
use crate::club::player::interaction::InteractionOutcome;
use crate::club::player::player::Player;
use crate::club::relations::StaffRelation;
use crate::club::staff::Staff;
use chrono::NaiveDate;

/// Window used by the "recent broken promise" counter that drives
/// broken_promise_pressure / conflict_risk. Six months matches the
/// typical playing-time promise horizon and is long enough that a
/// player still smarts from a broken assurance from the previous window.
const BROKEN_PROMISE_WINDOW_DAYS: i64 = 180;

/// Window used to weight the player's most recent talk outcomes back
/// into selection_trust. A successful month-old talk still helps, a
/// row from a year ago no longer does.
const RECENT_TALK_WINDOW_DAYS: i64 = 120;

/// Maximum signed contribution any single sub-signal can make to a
/// derived axis. Keeps a stack of bad signals from saturating the
/// output (which would feel like a permanent coach-player feud once
/// any single thing slipped).
const SIGNAL_CONTRIBUTION_CAP: f32 = 0.5;

/// Canonical neutral baseline for the `personal_bond` axis on
/// [`StaffRelation`]. The relation type defaults `personal_bond` to
/// 25.0; centring on that value keeps "absent relation" and "freshly-
/// created neutral relation" producing identical bond reads.
const PERSONAL_BOND_NEUTRAL: f32 = 25.0;

/// Canonical neutral baseline for the `loyalty` axis. Matches the
/// `StaffRelation::new_neutral` default of 30.0.
const LOYALTY_NEUTRAL: f32 = 30.0;

/// Derived snapshot of the coach-player bond. All four axes are 0..1
/// with the cognitive midpoint at 0.5 — a freshly-generated pair with
/// no history reads as neutral on every axis.
#[derive(Debug, Clone, Copy)]
pub struct CoachPlayerBond {
    /// 0..1. Selection confidence — high values mean the coach is
    /// happy to put the player in the XI. Reads staff relation
    /// quality + coach memory trust + role fairness + promise
    /// credibility + rapport.
    pub selection_trust: f32,
    /// 0..1. How readily the player absorbs coaching from this coach.
    /// Stronger weight on rapport + receptiveness; promise / memory
    /// terms still matter (a player who feels lied to switches off).
    pub training_receptiveness: f32,
    /// 0..1. How much the player believes in the coach's tactical
    /// plan. Authority, trust in abilities and the coach's tactical
    /// memory weight heaviest.
    pub tactical_buy_in: f32,
    /// 0..1. Likelihood of an explicit incident in the near term.
    /// Reads unmet role expectations, broken promises, low authority,
    /// low rapport, and the player's controversy personality.
    pub conflict_risk: f32,
}

impl Default for CoachPlayerBond {
    fn default() -> Self {
        // Neutral defaults so a "no data" pair reads identically to a
        // pair that was deliberately judged neutral.
        Self {
            selection_trust: 0.5,
            training_receptiveness: 0.5,
            tactical_buy_in: 0.5,
            conflict_risk: 0.0,
        }
    }
}

impl CoachPlayerBond {
    /// Build the derived bond from current state. Reads:
    ///   * `player.relations.get_staff(staff.id)` — long-term staff bond
    ///   * `player.rapport.score(staff.id)` — short-term rapport
    ///   * `player.happiness.factors.promise_trust` — promise track record
    ///   * `player.interactions.entries` — recent talk outcomes
    ///   * `staff.coach_memory.get(player.id)` — coach's own read
    ///
    /// `today` is used to age recent-talk and broken-promise signals.
    pub fn build(player: &Player, staff: &Staff, today: NaiveDate) -> Self {
        let inputs = BondInputs::collect(player, staff, today);
        Self {
            selection_trust: SelectionTrust::compose(&inputs),
            training_receptiveness: TrainingReceptiveness::compose(&inputs),
            tactical_buy_in: TacticalBuyIn::compose(&inputs),
            conflict_risk: ConflictRisk::compose(&inputs),
        }
    }

    /// Signed selection-slot adjustment around zero. Positive values
    /// nudge the coach toward the player; negative values nudge away.
    ///
    /// Asymmetric scaling per design: positive sentiment is dampened
    /// (×0.85) so the coach can never favouritism his way past a more
    /// objectively qualified rival, while negative sentiment is
    /// amplified (×1.20) — coach-player fallouts in real football
    /// keep a player out of the XI even when his form deserves
    /// selection. Caller still chooses the base magnitude via `scale`.
    #[inline]
    pub fn selection_adjustment(&self, scale: f32) -> f32 {
        let centered = self.selection_trust - 0.5;
        if centered >= 0.0 {
            centered * scale * 0.85
        } else {
            centered * scale * 1.20
        }
    }
}

/// Stateless namespace that owns the centred-axis math used by every
/// [`CoachPlayerBond`] read. Each method maps a raw axis value (-100..100
/// or 0..100 depending on the field) onto a 0..1 axis with 0.5 = neutral.
/// Constants for the per-axis neutral baselines live at module scope so
/// the relation's `new_neutral` defaults are the single source of truth.
struct Axis;

impl Axis {
    /// `level` lives on -100..100 with neutral 0. Centre on 0.5 so a
    /// freshly-created relation slots cleanly between "absent" and
    /// "explicitly hostile".
    #[inline]
    fn level(level: f32) -> f32 {
        (0.5 + level / 200.0).clamp(0.0, 1.0)
    }

    /// `authority_respect`, `trust_in_abilities`, `receptiveness` all
    /// sit on 0..100 with neutral 50.
    #[inline]
    fn authority(value: f32) -> f32 {
        (0.5 + (value - 50.0) / 100.0).clamp(0.0, 1.0)
    }

    #[inline]
    fn ability_trust(value: f32) -> f32 {
        (0.5 + (value - 50.0) / 100.0).clamp(0.0, 1.0)
    }

    #[inline]
    fn receptiveness(value: f32) -> f32 {
        (0.5 + (value - 50.0) / 100.0).clamp(0.0, 1.0)
    }

    /// `personal_bond` neutral default is 25 (see
    /// `StaffRelation::new_neutral`). Centring on 25 keeps a brand-new
    /// "we don't really know each other yet" pair at 0.5 rather than
    /// at 0.25 (which the pre-polish read used).
    #[inline]
    fn personal_bond(value: f32) -> f32 {
        (0.5 + (value - PERSONAL_BOND_NEUTRAL) / 100.0).clamp(0.0, 1.0)
    }

    /// `loyalty` neutral default is 30.
    #[inline]
    fn loyalty(value: f32) -> f32 {
        (0.5 + (value - LOYALTY_NEUTRAL) / 100.0).clamp(0.0, 1.0)
    }
}

/// Internal — the pre-aggregated input bundle every axis composer
/// shares. Built once per [`CoachPlayerBond::build`] so each formula
/// doesn't have to re-walk the player's interaction log.
#[derive(Debug, Clone, Copy)]
struct BondInputs {
    /// Quality of the staff relation, 0..1 (centred on 0.5 = neutral).
    staff_relation_quality: f32,
    /// Player's authority_respect axis remapped to 0..1.
    authority_respect: f32,
    /// Player's trust_in_abilities axis remapped to 0..1.
    trust_in_abilities: f32,
    /// Player's personal_bond axis remapped to 0..1.
    personal_bond: f32,
    /// Player's receptiveness axis remapped to 0..1.
    receptiveness: f32,
    /// Rapport with the coach mapped to 0..1 (50 = neutral, 100 = peak).
    rapport_norm: f32,
    /// Coach's tactical_trust EMA, 0..1.
    coach_memory_tactical_trust: f32,
    /// Coach's training_trust EMA, 0..1.
    coach_memory_training_trust: f32,
    /// Coach's big_match_trust EMA, 0..1.
    coach_memory_big_match_trust: f32,
    /// Promise credibility, 0..1: derived from promise_trust factor.
    promise_credibility: f32,
    /// Broken-promise pressure, 0..1: count of recent broken
    /// promise interactions ramped against the typical "this is a
    /// problem" threshold.
    broken_promise_pressure: f32,
    /// Role fairness, 0..1: how closely the player's actual minutes
    /// match what their squad status implies they should get.
    role_fairness: f32,
    /// Unmet role expectation, 0..1: the inverse of role_fairness
    /// pulled by the player's expected_start_share so a KeyPlayer
    /// being benched complains harder than a backup.
    unmet_role_expectation: f32,
    /// Net "recent talk outcomes" contribution, -0.2..+0.2.
    recent_talk_outcomes: f32,
    /// Player controversy attribute mapped to 0..1.
    controversy: f32,
}

impl BondInputs {
    fn collect(player: &Player, staff: &Staff, today: NaiveDate) -> Self {
        // Read all axes through centered helpers so a relation sitting
        // at every-axis neutral (authority 50, trust 50, bond 25, …)
        // produces the same 0.5 reads as an absent relation. The
        // pre-polish version mixed centered (authority) and raw
        // (personal_bond / loyalty) axes, so an explicit-neutral pair
        // and an absent pair disagreed by ~0.04 on selection_trust.
        let relation = player.relations.get_staff(staff.id);
        let staff_relation_quality = relation
            .map(BondInputs::staff_relation_quality_of)
            .unwrap_or(0.5);
        let authority_respect = relation
            .map(|r| Axis::authority(r.authority_respect))
            .unwrap_or(0.5);
        let trust_in_abilities = relation
            .map(|r| Axis::ability_trust(r.trust_in_abilities))
            .unwrap_or(0.5);
        let personal_bond = relation
            .map(|r| Axis::personal_bond(r.personal_bond))
            .unwrap_or(0.5);
        let receptiveness = relation
            .map(|r| Axis::receptiveness(r.receptiveness))
            .unwrap_or(0.5);

        let rapport_norm = BondInputs::rapport_norm_of(player, staff.id);
        let memory = staff.coach_memory.get(player.id);
        let coach_memory_tactical_trust = memory.map(|m| m.tactical_trust).unwrap_or(0.5);
        let coach_memory_training_trust = memory.map(|m| m.training_trust).unwrap_or(0.5);
        let coach_memory_big_match_trust = memory.map(|m| m.big_match_trust).unwrap_or(0.5);

        let promise_credibility = BondInputs::promise_credibility_of(player);
        let broken_promise_pressure = BondInputs::broken_promise_pressure_of(player, today);
        let role_fairness = BondInputs::role_fairness_of(player);
        // unmet_role_expectation only registers when role_fairness drops
        // BELOW neutral (0.5). A player getting fair playing time
        // produces zero unmet pressure regardless of squad status — only
        // an actual gap between expectation and reality fires the term.
        let unmet_role_expectation = ((0.5 - role_fairness) * 2.0).clamp(0.0, 1.0)
            * BondInputs::role_expectation_weight_of(player);
        let recent_talk_outcomes = BondInputs::recent_talk_outcomes_of(player, staff.id, today);
        let controversy = (player.attributes.controversy / 20.0).clamp(0.0, 1.0);

        Self {
            staff_relation_quality,
            authority_respect,
            trust_in_abilities,
            personal_bond,
            receptiveness,
            rapport_norm,
            coach_memory_tactical_trust,
            coach_memory_training_trust,
            coach_memory_big_match_trust,
            promise_credibility,
            broken_promise_pressure,
            role_fairness,
            unmet_role_expectation,
            recent_talk_outcomes,
            controversy,
        }
    }

    /// 0..1 read of the StaffRelation as a whole. Centred on 0.5 so a
    /// neutral StaffRelation (every axis at its `new_neutral` default —
    /// level 0, authority/trust/receptiveness 50, personal_bond 25,
    /// loyalty 30) produces exactly 0.5, matching the "no relation"
    /// fallback. Weights match the spec: 0.30/0.25/0.25/0.10/0.10.
    fn staff_relation_quality_of(r: &StaffRelation) -> f32 {
        let level = Axis::level(r.level);
        let authority = Axis::authority(r.authority_respect);
        let ability_trust = Axis::ability_trust(r.trust_in_abilities);
        let bond = Axis::personal_bond(r.personal_bond);
        let loyalty = Axis::loyalty(r.loyalty);
        (level * 0.30
            + authority * 0.25
            + ability_trust * 0.25
            + bond * 0.10
            + loyalty * 0.10)
            .clamp(0.0, 1.0)
    }

    fn rapport_norm_of(player: &Player, coach_id: u32) -> f32 {
        // Rapport range is -50..100. Map onto 0..1 with 0 (neutral) at 0.5.
        let raw = player.rapport.score(coach_id) as f32;
        if raw >= 0.0 {
            (0.5 + (raw / 100.0) * 0.5).clamp(0.0, 1.0)
        } else {
            (0.5 + (raw / 50.0) * 0.5).clamp(0.0, 1.0)
        }
    }

    /// Map the player's promise_trust factor (-10..+6 in practice) onto
    /// a 0..1 credibility axis where 0.5 = neutral, 1.0 = full faith.
    fn promise_credibility_of(player: &Player) -> f32 {
        let raw = player.happiness.factors.promise_trust;
        // Asymmetric mapping mirrors the asymmetric factor band.
        let norm = if raw >= 0.0 {
            0.5 + (raw / 6.0) * 0.5
        } else {
            0.5 + (raw / 10.0) * 0.5
        };
        norm.clamp(0.0, 1.0)
    }

    /// Count broken-promise signals in the last
    /// [`BROKEN_PROMISE_WINDOW_DAYS`] days and ramp them into a 0..1
    /// pressure signal. Reads two complementary sources with separate
    /// weights so a row that fires in only one log doesn't undercount:
    ///
    ///   * `PromiseBroken` happiness events (weight 0.30 each) — the
    ///     authoritative log of promise verification failures, written
    ///     by the promise verifier when a deadline lapses.
    ///   * Talk-log entries with `promise_created` + `Negative` outcome
    ///     (weight 0.20 each) — captures the "manager broke off a chat
    ///     with a fudged promise" case before the verifier formally
    ///     invalidates it.
    ///
    /// The two channels are intentionally additive: the same row can
    /// appear in both stores during the small window between the talk
    /// failure and the verifier tick, but the talk weight is smaller
    /// so the overlap rounds to "this is a real broken promise" rather
    /// than double-counting it.
    fn broken_promise_pressure_of(player: &Player, today: NaiveDate) -> f32 {
        let window_days_u16 = BROKEN_PROMISE_WINDOW_DAYS as u16;
        let event_pressure = player
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                e.event_type == HappinessEventType::PromiseBroken && e.days_ago <= window_days_u16
            })
            .count() as f32
            * 0.30;

        let talk_pressure = player
            .interactions
            .entries
            .iter()
            .filter(|e| {
                e.outcome == InteractionOutcome::Negative
                    && e.promise_created
                    && (today - e.date).num_days() <= BROKEN_PROMISE_WINDOW_DAYS
            })
            .count() as f32
            * 0.20;

        (event_pressure + talk_pressure).clamp(0.0, 1.0)
    }

    /// 0..1 fairness signal: 0 = player is being completely overlooked
    /// vs their squad status; 1 = exceeding expectations. Reads
    /// happiness.factors.role_clarity (already incorporates squad
    /// status alignment with appearance share) and pulls toward the
    /// neutral 0.5 when no data is available.
    fn role_fairness_of(player: &Player) -> f32 {
        let role_clarity = player.happiness.factors.role_clarity; // -8..+5
        // Normalise around 0.5 — role_clarity 0 reads as neutral.
        let norm = 0.5 + (role_clarity / 16.0);
        norm.clamp(0.0, 1.0)
    }

    /// Weighting on the unmet-expectation term: a KeyPlayer who is
    /// being benched complains harder than a backup. Reads the player's
    /// expected start share to scale.
    fn role_expectation_weight_of(player: &Player) -> f32 {
        use crate::PlayerSquadStatus as S;
        let status = player.contract.as_ref().map(|c| &c.squad_status);
        match status {
            Some(S::KeyPlayer) => 1.2,
            Some(S::FirstTeamRegular) => 1.0,
            Some(S::FirstTeamSquadRotation) => 0.7,
            Some(S::MainBackupPlayer) => 0.5,
            Some(S::HotProspectForTheFuture) => 0.6,
            Some(S::DecentYoungster) => 0.4,
            Some(S::NotNeeded) => 0.2,
            _ => 0.6,
        }
    }

    /// Net contribution from recent talk outcomes (signed, -0.2..+0.2).
    /// Positive talks lift, negative talks drop; old talks decay
    /// linearly to zero across [`RECENT_TALK_WINDOW_DAYS`].
    fn recent_talk_outcomes_of(player: &Player, coach_id: u32, today: NaiveDate) -> f32 {
        let mut net: f32 = 0.0;
        for entry in player.interactions.entries.iter() {
            if entry.staff_id != coach_id {
                continue;
            }
            let age = (today - entry.date).num_days();
            if age < 0 || age > RECENT_TALK_WINDOW_DAYS {
                continue;
            }
            let decay = 1.0 - (age as f32 / RECENT_TALK_WINDOW_DAYS as f32);
            net += match entry.outcome {
                InteractionOutcome::Positive => 0.08 * decay,
                InteractionOutcome::PromiseMade => 0.04 * decay,
                InteractionOutcome::Neutral => 0.0,
                InteractionOutcome::Negative => -0.10 * decay,
            };
        }
        net.clamp(-0.2, 0.2)
    }
}

/// Selection-trust composer. Five 0..1 inputs blended per the spec
/// weights. Output is clamped to 0..1.
///
/// Why this lens, not the others: selection cares more about the
/// long-term staff relation, coach-memory trust and promise credibility
/// than it does about today's mood — putting a player in the XI is a
/// medium-term call, not a snap reaction to one good or bad day.
struct SelectionTrust;

impl SelectionTrust {
    fn compose(i: &BondInputs) -> f32 {
        let coach_memory_trust =
            i.coach_memory_tactical_trust * 0.7 + i.coach_memory_big_match_trust * 0.3;
        let raw = 0.30 * i.staff_relation_quality
            + 0.20 * coach_memory_trust
            + 0.20 * i.role_fairness
            + 0.15 * i.promise_credibility
            + 0.15 * i.rapport_norm;
        // Recent talk outcomes nudge the base read; capped so a single
        // talk can't blow up the trust on its own.
        (raw + i.recent_talk_outcomes.clamp(-SIGNAL_CONTRIBUTION_CAP, SIGNAL_CONTRIBUTION_CAP))
            .clamp(0.0, 1.0)
    }
}

/// Training receptiveness composer. Weighted toward the short-term
/// signals — rapport and receptiveness — and the coach's training
/// trust memory. A high authority but low personal bond coach still
/// gets a player in the gym; a low rapport doesn't.
struct TrainingReceptiveness;

impl TrainingReceptiveness {
    fn compose(i: &BondInputs) -> f32 {
        let raw = 0.30 * i.rapport_norm
            + 0.25 * i.receptiveness
            + 0.20 * i.coach_memory_training_trust
            + 0.15 * i.personal_bond
            + 0.10 * i.promise_credibility;
        (raw + i.recent_talk_outcomes.clamp(-SIGNAL_CONTRIBUTION_CAP, SIGNAL_CONTRIBUTION_CAP) * 0.5)
            .clamp(0.0, 1.0)
    }
}

/// Tactical buy-in composer. Authority + trust in coach's abilities +
/// the coach's tactical memory dominate; role fairness colours how
/// willingly the player subordinates ego to the system.
struct TacticalBuyIn;

impl TacticalBuyIn {
    fn compose(i: &BondInputs) -> f32 {
        let raw = 0.30 * i.authority_respect
            + 0.25 * i.trust_in_abilities
            + 0.20 * i.coach_memory_tactical_trust
            + 0.15 * i.role_fairness
            + 0.10 * i.promise_credibility;
        raw.clamp(0.0, 1.0)
    }
}

/// Conflict-risk composer. Pulled up by unmet role expectation +
/// broken promises + below-neutral authority + below-neutral rapport
/// + above-baseline controversy. Output clamped to 0..1.
///
/// Crucially, each axis only contributes when it's WORSE than neutral
/// — a neutral pair (no data, default 0.5 axes) produces ~0 risk,
/// matching real-life expectation that "no information" doesn't itself
/// imply conflict. The previous all-magnitude version booked
/// ~0.29 of baseline risk on every fresh pair, which is wrong: a
/// trial signing the coach has never met isn't on the verge of a row.
///
/// Why controversy is in here at all: the same broken promise from the
/// same coach lands differently on a quiet pro and a tabloid lightning
/// rod. The controversy contribution above the typical baseline (0.3)
/// keeps the risk read realistic without requiring a separate
/// "personality penalty" elsewhere.
struct ConflictRisk;

impl ConflictRisk {
    /// Controversy attribute (already 0..1) below this threshold
    /// counts as a "normal" personality and contributes nothing —
    /// only above-typical controversy lifts conflict_risk. Tuned so
    /// the default `controversy = 5` reading (≈0.25) is silent.
    const CONTROVERSY_BASELINE: f32 = 0.30;

    fn compose(i: &BondInputs) -> f32 {
        // Each "low-axis" term is the *gap below neutral*, scaled so a
        // fully-broken axis contributes 1.0 and a neutral axis
        // contributes 0.0.
        let low_authority = ((0.5 - i.authority_respect) * 2.0).clamp(0.0, 1.0);
        let low_rapport = ((0.5 - i.rapport_norm) * 2.0).clamp(0.0, 1.0);
        let controversy_excess =
            (i.controversy - Self::CONTROVERSY_BASELINE).clamp(0.0, 1.0);

        // unmet_role_expectation is already computed as a "below neutral"
        // value upstream — see BondInputs::collect.
        let raw = 0.30 * i.unmet_role_expectation
            + 0.25 * i.broken_promise_pressure
            + 0.20 * low_authority
            + 0.15 * low_rapport
            + 0.10 * controversy_excess;
        raw.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    //! Direct unit tests for `CoachPlayerBond` — these prove the
    //! derived bond reflects upstream changes (broken promises, good
    //! coaching, distinct memory vs relation channels) without
    //! double-counting evidence.
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::staff::CoachProfile;
    use crate::club::staff::StaffStub;
    use crate::club::staff::coach::memory::CoachMatchObservation;
    use crate::shared::fullname::FullName;
    use crate::{
        ChangeType, HappinessEvent, HappinessEventType, PersonAttributes, PlayerAttributes,
        PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills, RelationshipChange,
        Staff,
    };

    struct Fixture;

    impl Fixture {
        fn today() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        }

        fn pro_person() -> PersonAttributes {
            PersonAttributes {
                adaptability: 12.0,
                ambition: 12.0,
                controversy: 5.0,
                loyalty: 14.0,
                pressure: 12.0,
                professionalism: 14.0,
                sportsmanship: 12.0,
                temperament: 12.0,
                consistency: 12.0,
                important_matches: 12.0,
                dirtiness: 5.0,
            }
        }

        fn player(id: u32) -> Player {
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Bond".into(), id.to_string()))
                .birth_date(NaiveDate::from_ymd_opt(1998, 1, 1).unwrap())
                .country_id(1)
                .attributes(Self::pro_person())
                .skills(PlayerSkills::default())
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

        fn staff(id: u32) -> Staff {
            let mut s = StaffStub::default();
            s.id = id;
            s
        }

        /// Push a synthetic PromiseBroken event onto the player's
        /// happiness log. The bond reads `days_ago`, not the absolute
        /// date, so a freshly added event with days_ago = 0 sits in
        /// the broken-promise window.
        fn push_broken_promise(player: &mut Player) {
            player.happiness.recent_events.push(HappinessEvent {
                event_type: HappinessEventType::PromiseBroken,
                magnitude: -10.0,
                days_ago: 5,
                partner_player_id: None,
                context: None,
            });
            // Promise-trust factor drops in the daily recalc; mirror it
            // here so the test sees the combined effect.
            player.happiness.factors.promise_trust = -7.0;
        }
    }

    #[test]
    fn neutral_pair_reads_as_neutral_across_axes() {
        let player = Fixture::player(1);
        let staff = Fixture::staff(7);
        let bond = CoachPlayerBond::build(&player, &staff, Fixture::today());
        // All four axes should sit close to 0.5 (neutral). Conflict_risk
        // can edge above zero because the controversy attribute is
        // non-zero in a real player profile.
        assert!(
            (bond.selection_trust - 0.5).abs() < 0.08,
            "neutral selection_trust={}",
            bond.selection_trust
        );
        assert!(
            (bond.tactical_buy_in - 0.5).abs() < 0.08,
            "neutral tactical_buy_in={}",
            bond.tactical_buy_in
        );
        assert!(
            bond.conflict_risk < 0.20,
            "neutral conflict_risk should be low: {}",
            bond.conflict_risk
        );
    }

    #[test]
    fn broken_promise_drops_selection_trust_and_lifts_conflict_risk() {
        // Spec test: "Broken promise lowers manager trust and
        // increases conflict risk."
        let mut player = Fixture::player(1);
        let staff = Fixture::staff(7);
        let baseline = CoachPlayerBond::build(&player, &staff, Fixture::today());

        Fixture::push_broken_promise(&mut player);
        let after = CoachPlayerBond::build(&player, &staff, Fixture::today());

        assert!(
            after.selection_trust < baseline.selection_trust,
            "broken promise should lower selection_trust ({} → {})",
            baseline.selection_trust,
            after.selection_trust
        );
        assert!(
            after.conflict_risk > baseline.conflict_risk,
            "broken promise should raise conflict_risk ({} → {})",
            baseline.conflict_risk,
            after.conflict_risk
        );
    }

    #[test]
    fn coach_memory_trust_and_staff_relation_compose_without_double_counting() {
        // Spec test: "Coach memory and staff relation both contribute
        // without double-counting form." We apply the same effective
        // signal twice — once via coach_memory observations, once via
        // a positive staff relation update — and assert that:
        //   * each individual channel lifts trust over the neutral baseline;
        //   * the combined effect is bounded — adding the second channel
        //     does not produce a 2× lift from a single piece of evidence.
        let player_base = Fixture::player(1);
        let mut staff_base = Fixture::staff(7);

        // Variant A: memory-only (12 strong observations).
        let mut staff_a = staff_base.clone();
        let profile = CoachProfile::from_staff(&staff_a);
        for i in 0..12 {
            staff_a.coach_memory.observe(
                &CoachMatchObservation {
                    player_id: 1,
                    effective_rating: 7.6,
                    minutes_played: 90,
                    is_starter: true,
                    match_importance: 0.7,
                    is_cup: false,
                    is_derby: false,
                    is_continental: false,
                    goals: 1,
                    assists: 0,
                    errors_leading_to_goal: 0,
                    yellow_cards: 0,
                    red_cards: 0,
                    team_won: true,
                    was_substituted_early: false,
                    role_fit: 1.0,
                    professionalism_signal: 0.8,
                    date: NaiveDate::from_ymd_opt(2026, 4, 1 + i).unwrap(),
                },
                &profile,
            );
        }

        // Variant B: relation-only (a run of successful talks).
        // The bond dampens single updates so we apply a realistic
        // sequence — five positive sessions over a quarter — to mimic
        // the same magnitude of "evidence" the memory channel carries
        // through its 12 observations.
        let mut player_b = player_base.clone();
        for _ in 0..5 {
            player_b.relations.update_staff_relationship(
                staff_base.id,
                RelationshipChange::positive(ChangeType::CoachingSuccess, 5.0),
                Fixture::today(),
            );
        }

        // Variant C: both channels.
        let mut player_c = player_b.clone();
        let mut staff_c = staff_a.clone();
        // (player_c already has the relation; staff_c already has memory.)
        let _ = (&mut staff_base, &mut staff_c, &mut player_c);

        let baseline = CoachPlayerBond::build(&player_base, &staff_base, Fixture::today());
        let memory_only = CoachPlayerBond::build(&player_base, &staff_a, Fixture::today());
        let relation_only = CoachPlayerBond::build(&player_b, &staff_base, Fixture::today());
        let both = CoachPlayerBond::build(&player_c, &staff_c, Fixture::today());

        // Each channel on its own lifts trust over the baseline. The
        // memory channel contributes only 20% of selection_trust per
        // the spec weights, so 12 observations at 7.6 produces a
        // smaller (but real) lift than a single big positive talk.
        assert!(
            memory_only.selection_trust > baseline.selection_trust + 0.005,
            "memory-only lift baseline={} memory={}",
            baseline.selection_trust,
            memory_only.selection_trust
        );
        assert!(
            relation_only.selection_trust > baseline.selection_trust + 0.005,
            "relation-only lift baseline={} relation={}",
            baseline.selection_trust,
            relation_only.selection_trust
        );

        // Combined effect must be larger than either alone. And —
        // critically for "no double counting" — the combined lift
        // must equal the sum of the per-channel lifts within a tight
        // tolerance. Equality is the orthogonal-channels signature: if
        // memory and staff_relation were reading the same evidence
        // twice, the combined lift would EXCEED the sum (super-
        // addition); if they were redundant, combined would equal
        // max(A, B). Linear orthogonality lands precisely at the sum.
        let max_alone = memory_only.selection_trust.max(relation_only.selection_trust);
        let combined_lift = both.selection_trust - baseline.selection_trust;
        let sum_lifts = (memory_only.selection_trust - baseline.selection_trust)
            + (relation_only.selection_trust - baseline.selection_trust);
        assert!(
            both.selection_trust >= max_alone,
            "combined ({}) must be at least as strong as the strongest channel alone ({})",
            both.selection_trust,
            max_alone
        );
        assert!(
            (combined_lift - sum_lifts).abs() < 0.005,
            "combined lift ({:.4}) must match the sum of per-channel lifts ({:.4}) — \
             channels are orthogonal, no double counting",
            combined_lift,
            sum_lifts
        );
    }

    // ── Polish task #11 integration tests ──────────────────────────

    #[test]
    fn absent_relation_matches_explicit_neutral_within_design_tolerance() {
        // Polish task #1: an explicit-but-neutral StaffRelation must
        // produce the same bond result as an absent relation, to within
        // 0.02 on selection_trust. Pre-polish the personal_bond / loyalty
        // axes were read as raw 0..1 ratios (so neutral 25 → 0.25, not
        // 0.5), which made the two reads disagree by ~0.04.
        let player_absent = Fixture::player(1);
        let mut player_explicit = Fixture::player(1);
        let staff = Fixture::staff(7);
        // Touch the staff relation so it exists at every-axis neutral —
        // a no-op positive update with a 0.0 magnitude leaves the new
        // relation at its `new_neutral` default but ensures the lookup
        // returns `Some`.
        player_explicit.relations.update_staff_relationship(
            staff.id,
            RelationshipChange::positive(ChangeType::NaturalProgression, 0.0),
            Fixture::today(),
        );
        assert!(
            player_explicit.relations.get_staff(staff.id).is_some(),
            "test precondition: the explicit-neutral path must produce a relation row"
        );

        let absent_bond = CoachPlayerBond::build(&player_absent, &staff, Fixture::today());
        let explicit_bond = CoachPlayerBond::build(&player_explicit, &staff, Fixture::today());

        assert!(
            (absent_bond.selection_trust - explicit_bond.selection_trust).abs() <= 0.02,
            "absent vs explicit-neutral selection_trust gap exceeds 0.02 (absent={} explicit={})",
            absent_bond.selection_trust,
            explicit_bond.selection_trust
        );
    }

    #[test]
    fn broken_promise_interaction_lifts_conflict_risk_and_drops_trust() {
        // Polish task #2: broken-promise pressure must read both the
        // happiness log AND the manager-interaction log. A broken
        // promise that fires only on the interaction side (the talk
        // failed and was recorded as a PromiseMade-then-Negative outcome
        // before the verifier landed the PromiseBroken event) must still
        // move the bond.
        use crate::club::player::interaction::{
            InteractionOutcome, InteractionTone, InteractionTopic, ManagerInteraction,
        };
        let mut player = Fixture::player(1);
        let staff = Fixture::staff(7);
        let baseline = CoachPlayerBond::build(&player, &staff, Fixture::today());

        // Push an interaction-log broken-promise row dated yesterday.
        player.interactions.push(ManagerInteraction {
            date: Fixture::today() - chrono::Duration::days(1),
            staff_id: staff.id,
            topic: InteractionTopic::PlayingTime,
            tone: InteractionTone::Evasive,
            player_mood_before: 50.0,
            outcome: InteractionOutcome::Negative,
            promise_created: true,
            relationship_delta: -3.0,
            morale_delta: -8.0,
            cooldown_until: Fixture::today() + chrono::Duration::days(30),
        });
        let after = CoachPlayerBond::build(&player, &staff, Fixture::today());

        assert!(
            after.conflict_risk > baseline.conflict_risk,
            "interaction-log broken promise must lift conflict_risk ({} → {})",
            baseline.conflict_risk,
            after.conflict_risk
        );
        assert!(
            after.selection_trust < baseline.selection_trust,
            "interaction-log broken promise must drop selection_trust ({} → {})",
            baseline.selection_trust,
            after.selection_trust
        );
    }

    #[test]
    fn selection_adjustment_asymmetric_scale_per_polish_spec() {
        // Polish task #3: selection_adjustment scales positive trust by
        // 0.85 and negative trust by 1.20. A delta of ±0.2 must produce
        // magnitudes 0.20*0.85*scale vs 0.20*1.20*scale = 1.41× ratio.
        let mut high = CoachPlayerBond::default();
        let mut low = CoachPlayerBond::default();
        high.selection_trust = 0.7; // +0.2 above neutral
        low.selection_trust = 0.3; // -0.2 below neutral
        let pos = high.selection_adjustment(1.4);
        let neg = low.selection_adjustment(1.4);
        assert!(pos > 0.0 && neg < 0.0);
        let ratio = neg.abs() / pos.abs();
        assert!(
            (ratio - (1.20 / 0.85)).abs() < 0.01,
            "asymmetric ratio={:.3} expected {:.3}",
            ratio,
            1.20 / 0.85
        );
    }
}
