//! When each kind of footballing skill matures.
//!
//! A body finishes before a mind does. Pace and strength peak in the
//! early twenties; decisions, composure, positioning and vision keep
//! improving into the late twenties and hold into the thirties, because
//! they are built out of games watched, games played and mistakes made.
//! Goalkeeping craft matures latest of all — keepers peak in their early
//! thirties.
//!
//! The generator has always believed this: it builds a 17-year-old's
//! mental attributes at 0.55 of his eventual level and his technique at
//! 0.75. **Development did not.** The weekly tick's per-skill ceiling was
//! `PA/200 × 20 × position_weight` with no age term at all, and
//! `MaturityModel::biological_maturity_multiplier` — which does read age —
//! only slows the growth RATE, and reaches 1.0 at eighteen. So a player
//! who came through an academy grew toward his full adult ceiling and
//! arrived there around 18-19, before he had played a senior minute,
//! while a world-start 18-year-old was generated at 0.62 of the same
//! number. The two halves of one model disagreed about the same player.
//!
//! Live symptom: a seventeen-year-old with no career appearances, loaned
//! abroad, playing at a settled senior standard from his first match —
//! because he genuinely had a senior professional's concentration,
//! positioning and composure. The match rating was reporting him
//! correctly; he should not have had those attributes.
//!
//! This table is the single source of truth both halves now read.

use crate::club::player::development::{PositionalSkillCeilings, SkillKey};
use crate::{PlayerPositionType, PlayerSkills};

/// Skill families, grouped by when they mature rather than by what they
/// do. Deliberately its own enum: the generator and the development tick
/// index skills differently (37 vs 50 slots), so they share the CURVE
/// without having to share a layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MaturationGroup {
    Technical,
    Mental,
    /// Strength, stamina — the physical qualities that need a grown body
    /// and years of loading.
    Physical,
    /// Speed, acceleration, agility, balance, leap. Split off from
    /// [`MaturationGroup::Physical`] because they mature far earlier than
    /// strength does: a seventeen-year-old can be the fastest man on the
    /// pitch, and cannot be the strongest. Treating them alike made every
    /// quick teenage winger slower than he should be — a PA 190 wide
    /// midfielder capped at 13.3 pace at seventeen.
    ///
    /// This is the project's own existing belief, not a new one: the
    /// generator's `age_curve` already puts acceleration / pace / agility
    /// / jumping / balance / natural fitness in the earliest-peaking
    /// band (18-24) and leaves strength out of it.
    Explosive,
    Goalkeeping,
}

impl MaturationGroup {
    pub const COUNT: usize = 5;
    pub const ALL: [MaturationGroup; Self::COUNT] = [
        MaturationGroup::Technical,
        MaturationGroup::Mental,
        MaturationGroup::Physical,
        MaturationGroup::Explosive,
        MaturationGroup::Goalkeeping,
    ];
}

/// How an attribute ages: the classes `SkillMaturation::retention` reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ageing {
    Speed,
    Agility,
    Stamina,
    /// Strength, leap and balance.
    Frame,
    Fitness,
    Technique,
    Mind,
    Goalkeeping,
}

impl Ageing {
    fn of(idx: usize) -> Ageing {
        match idx {
            i if i == SkillKey::Pace.idx() || i == SkillKey::Acceleration.idx() => Ageing::Speed,
            i if i == SkillKey::Agility.idx() => Ageing::Agility,
            i if i == SkillKey::Stamina.idx() => Ageing::Stamina,
            i if i == SkillKey::Strength.idx()
                || i == SkillKey::Jumping.idx()
                || i == SkillKey::Balance.idx() =>
            {
                Ageing::Frame
            }
            i if i == SkillKey::NaturalFitness.idx() => Ageing::Fitness,
            _ => match PositionalSkillCeilings::maturation_group(idx) {
                MaturationGroup::Technical => Ageing::Technique,
                MaturationGroup::Goalkeeping => Ageing::Goalkeeping,
                _ => Ageing::Mind,
            },
        }
    }
}

pub struct SkillMaturation;

impl SkillMaturation {
    /// The outfield roles an age curve written for outfielders is set on.
    const OUTFIELD_ROLES: [PlayerPositionType; 9] = [
        PlayerPositionType::DefenderCenter,
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::WingbackLeft,
        PlayerPositionType::DefensiveMidfielder,
        PlayerPositionType::MidfielderCenter,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::AttackingMidfielderCenter,
        PlayerPositionType::AttackingMidfielderLeft,
        PlayerPositionType::Striker,
    ];

    /// Fraction of his eventual (PA-derived) level a player of `age`
    /// years — fractional, a player grows into his age a day at a time —
    /// can hold in this skill family.
    ///
    /// Straight lines between the [`Self::knots`]; before the first and
    /// after the last the curve holds level.
    pub fn ratio(age: f32, group: MaturationGroup) -> f32 {
        Self::read(Self::knots(group), age) * Self::family_retention(age, group)
    }

    /// How readily work becomes ability at `age`: 1.0 at the family's most
    /// plastic years, in the mid-teens to early twenties, falling through
    /// the twenties and never rising again after its peak.
    pub fn plasticity(age: f32, group: MaturationGroup) -> f32 {
        Self::read(Self::plasticity_knots(group), age)
    }

    /// The share of his level above the scale's zero a body still sustains
    /// in attribute `idx` (the development tick's skill index) at `age`:
    /// 1.0 up to the attribute's decline age, then falling. Pace and
    /// acceleration go first and furthest, agility and stamina after them;
    /// strength, leap and balance barely move, technique a little, and the
    /// mind holds into the late thirties.
    pub fn retention(age: f32, idx: usize) -> f32 {
        Self::read(Self::retention_knots(Ageing::of(idx)), age)
    }

    /// The development tick's slowing of ageing for a player of the
    /// database's median natural fitness (12) and professionalism (13).
    pub const MEDIAN_KEEP: f32 = 0.7;

    /// What ageing leaves of a family at `age`: its attributes' retention,
    /// as a player of median natural fitness and professionalism keeps it.
    fn family_retention(age: f32, group: MaturationGroup) -> f32 {
        if age <= Self::AGEING_FROM {
            return 1.0;
        }
        let (mut sum, mut n) = (0.0f32, 0.0f32);
        for idx in 0..50 {
            if PositionalSkillCeilings::maturation_group(idx) == group
                && !PositionalSkillCeilings::is_condition(idx)
            {
                sum += Self::retention(age, idx).powf(Self::MEDIAN_KEEP);
                n += 1.0;
            }
        }
        sum / n
    }

    /// No ageing class gives anything back before this age.
    const AGEING_FROM: f32 = 27.0;

    /// An attribute's own share of what its family's ageing leaves, for a
    /// player who keeps his body by `keep`: the factor that turns a family's
    /// held share into the attribute's.
    pub fn aged_attribute(age: f32, idx: usize, keep: f32) -> f32 {
        if age <= Self::AGEING_FROM {
            return 1.0;
        }
        let group = PositionalSkillCeilings::maturation_group(idx);
        Self::retention(age, idx).powf(keep) / Self::family_retention(age, group).max(1e-3)
    }

    /// The share of his peak ability a regular of `position` holds at `age`,
    /// 1.0 until his peak: each attribute's height above the scale's zero as
    /// its family matures and as a median player keeps it, weighted by what
    /// it is worth in the position's ability, against its highest so far.
    pub fn aged_share(age: f32, position: PlayerPositionType) -> f32 {
        let prices = PlayerSkills::ability_weights(position);
        let held = |a: f32| -> f32 {
            prices
                .iter()
                .enumerate()
                .map(|(idx, p)| {
                    let group = PositionalSkillCeilings::maturation_group(idx);
                    p * Self::read(Self::knots(group), a)
                        * Self::retention(a, idx).powf(Self::MEDIAN_KEEP)
                })
                .sum()
        };
        let now = held(age);
        let mut peak = now;
        let mut a = 20.0;
        while a < age {
            peak = peak.max(held(a));
            a += 0.25;
        }
        now / peak
    }

    /// The same share for the outfield roles taken together.
    pub fn outfield_aged_share(age: f32) -> f32 {
        Self::OUTFIELD_ROLES
            .iter()
            .map(|role| Self::aged_share(age, *role))
            .sum::<f32>()
            / Self::OUTFIELD_ROLES.len() as f32
    }

    /// Years by which an attribute runs behind (positive) or ahead
    /// (negative) of its family's curves: a leader and a keeper's command
    /// of his area arrive late, pace early.
    pub fn attribute_shift(idx: usize) -> f32 {
        const SHIFTS: [(SkillKey, f32); 23] = [
            (SkillKey::Pace, -1.0),
            (SkillKey::Acceleration, -1.0),
            (SkillKey::Agility, -1.0),
            (SkillKey::Balance, -1.0),
            (SkillKey::Strength, 1.0),
            (SkillKey::Jumping, 1.0),
            (SkillKey::NaturalFitness, 2.0),
            (SkillKey::Leadership, 3.0),
            (SkillKey::Composure, 3.0),
            (SkillKey::Decisions, 2.0),
            (SkillKey::Vision, 2.0),
            (SkillKey::Positioning, 2.0),
            (SkillKey::Anticipation, 1.0),
            (SkillKey::Flair, -1.0),
            (SkillKey::Dribbling, -1.0),
            (SkillKey::GkCommandOfArea, 3.0),
            (SkillKey::GkCommunication, 3.0),
            (SkillKey::GkOneOnOnes, 2.0),
            (SkillKey::GkRushingOut, 2.0),
            (SkillKey::GkHandling, 1.0),
            (SkillKey::GkPunching, 1.0),
            (SkillKey::GkReflexes, -1.0),
            (SkillKey::GkAerialReach, -1.0),
        ];
        SHIFTS
            .iter()
            .find(|(key, _)| key.idx() == idx)
            .map_or(0.0, |(_, shift)| *shift)
    }

    /// A smooth curve through `knots`: a monotone cubic whose slope is
    /// continuous everywhere and zero at the first and last knot, so no knot
    /// changes the pace of anything that reads it. Before the first knot
    /// and after the last the curve holds level.
    fn read(knots: &[(f32, f32)], age: f32) -> f32 {
        let after = knots.partition_point(|&(knot_age, _)| knot_age <= age);
        if after == 0 {
            return knots[0].1;
        }
        if after == knots.len() {
            return knots[after - 1].1;
        }
        let k = after - 1;
        let ((a0, v0), (a1, v1)) = (knots[k], knots[k + 1]);
        let h = a1 - a0;
        let t = (age - a0) / h;
        let (m0, m1) = (Self::tangent(knots, k), Self::tangent(knots, k + 1));
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * v0
            + (t3 - 2.0 * t2 + t) * h * m0
            + (-2.0 * t3 + 3.0 * t2) * v1
            + (t3 - t2) * h * m1
    }

    /// Slope of the curve at knot `i`: the weighted harmonic mean of the
    /// slopes either side (monotone, as PCHIP), flat at a turning point and
    /// at either end.
    fn tangent(knots: &[(f32, f32)], i: usize) -> f32 {
        if i == 0 || i + 1 == knots.len() {
            return 0.0;
        }
        let (h0, h1) = (knots[i].0 - knots[i - 1].0, knots[i + 1].0 - knots[i].0);
        let d0 = (knots[i].1 - knots[i - 1].1) / h0;
        let d1 = (knots[i + 1].1 - knots[i].1) / h1;
        if d0 * d1 <= 0.0 {
            return 0.0;
        }
        let (w0, w1) = (2.0 * h1 + h0, h1 + 2.0 * h0);
        (w0 + w1) / (w0 / d0 + w1 / d1)
    }

    /// Share of his eventual ability a player of `age` holds in `position`:
    /// each family's maturity, weighted by what that family is worth in the
    /// position's CA. A keeper's ability is half goalkeeping craft, so his
    /// share arrives later than any outfielder's and goes later too.
    pub fn ability_share(age: f32, position: PlayerPositionType) -> f32 {
        Self::share_of(age, &Self::family_weights(position))
    }

    /// The same share for a player who has banked no football: what
    /// training and growing up alone give his position's families at `age`.
    /// The curve a recorded player's timing is read against.
    pub fn unplayed_ability_share(age: f32, position: PlayerPositionType) -> f32 {
        let weights = Self::family_weights(position);
        MaturationGroup::ALL
            .iter()
            .map(|&group| {
                weights[group as usize] * Self::ratio(age, group) * (1.0 - Self::match_share(group))
            })
            .sum()
    }

    /// The same share for the outfield roles taken together — the
    /// reference an age curve written for outfielders is set on.
    pub fn outfield_ability_share(age: f32) -> f32 {
        let roles = Self::OUTFIELD_ROLES;
        let mut weights = [0.0f32; MaturationGroup::COUNT];
        for role in roles {
            for (total, w) in weights.iter_mut().zip(Self::family_weights(role)) {
                *total += w / roles.len() as f32;
            }
        }
        Self::share_of(age, &weights)
    }

    /// What each family is worth in a position's CA, as shares of the whole.
    fn family_weights(position: PlayerPositionType) -> [f32; MaturationGroup::COUNT] {
        let mut families = [0.0f32; MaturationGroup::COUNT];
        for (idx, w) in PlayerSkills::ability_weights(position).iter().enumerate() {
            families[PositionalSkillCeilings::maturation_group(idx) as usize] += w;
        }
        let total: f32 = families.iter().sum();
        families.map(|f| f / total)
    }

    fn share_of(age: f32, weights: &[f32; MaturationGroup::COUNT]) -> f32 {
        MaturationGroup::ALL
            .iter()
            .map(|&group| weights[group as usize] * Self::ratio(age, group))
            .sum()
    }

    /// `(age, share)` points of each family's curve. Each share is held
    /// at the middle of the years it applies to and a peak across all of
    /// them, so a ceiling rises with the player's age rather than in
    /// steps his growth would stall under and then lurch through.
    ///
    /// The technical / mental / physical shares are the generator's own
    /// numbers, moved here so generation and development cannot drift
    /// apart again. The goalkeeping row is fitted on the career grid
    /// (`career_table`, 2026-10-09) to the database's keepers: a PA 150
    /// regular starting keeper of median character at an average club ends
    /// each season from 22 to 30 between 0.02 and 0.06 above the keeper
    /// median, as an outfield regular sits above the outfield one, and
    /// reaches his peak in his early thirties.
    fn knots(group: MaturationGroup) -> &'static [(f32, f32)] {
        match group {
            MaturationGroup::Technical => &[
                (13.0, 0.62),
                (17.5, 0.75),
                (19.0, 0.82),
                (21.5, 0.90),
                (25.0, 0.95),
                (27.0, 1.00),
            ],
            MaturationGroup::Mental => &[
                (13.0, 0.42),
                (17.5, 0.55),
                (19.0, 0.62),
                (21.5, 0.72),
                (25.0, 0.85),
                (28.5, 0.95),
                (30.0, 1.00),
            ],
            MaturationGroup::Physical => &[
                (13.0, 0.55),
                (17.5, 0.70),
                (19.0, 0.78),
                (21.5, 0.88),
                (25.0, 0.95),
                (27.0, 1.00),
            ],
            // Sprinters are already close to their peak in their late
            // teens. Ageing takes it back through `retention`.
            MaturationGroup::Explosive => &[
                (13.0, 0.66),
                (15.5, 0.78),
                (17.0, 0.88),
                (19.0, 0.94),
                (20.0, 1.00),
            ],
            MaturationGroup::Goalkeeping => &[
                (13.0, 0.42),
                (17.5, 0.55),
                (19.0, 0.60),
                (21.5, 0.66),
                (25.0, 0.79),
                (28.5, 0.855),
                (31.5, 0.92),
                (33.5, 1.00),
            ],
        }
    }

    /// `(age, plasticity)` points of each family's learning curve. Set so a
    /// season of the old stepped development tables is kept at each band's
    /// middle, then smoothed: no quarter-year moves it by more than a few
    /// per cent.
    fn plasticity_knots(group: MaturationGroup) -> &'static [(f32, f32)] {
        match group {
            MaturationGroup::Technical => &[
                (14.0, 0.62),
                (16.0, 0.85),
                (17.5, 1.00),
                (19.0, 1.00),
                (21.5, 0.72),
                (25.0, 0.50),
                (29.0, 0.35),
                (32.0, 0.24),
                (35.0, 0.15),
                (38.0, 0.08),
            ],
            MaturationGroup::Mental => &[
                (14.0, 0.55),
                (16.0, 0.80),
                (18.0, 1.00),
                (20.0, 1.00),
                (22.0, 0.85),
                (25.0, 0.65),
                (28.0, 0.48),
                (31.0, 0.36),
                (34.0, 0.25),
                (37.0, 0.15),
            ],
            MaturationGroup::Physical => &[
                (14.0, 0.45),
                (16.0, 0.75),
                (17.5, 1.00),
                (19.0, 0.95),
                (21.0, 0.70),
                (25.0, 0.42),
                (29.0, 0.28),
                (32.0, 0.18),
                (35.0, 0.10),
                (38.0, 0.04),
            ],
            MaturationGroup::Explosive => &[
                (13.0, 0.45),
                (15.0, 0.75),
                (16.5, 1.00),
                (18.0, 0.95),
                (20.0, 0.70),
                (24.0, 0.42),
                (28.0, 0.28),
                (31.0, 0.18),
                (34.0, 0.10),
                (37.0, 0.04),
            ],
            MaturationGroup::Goalkeeping => &[
                (14.0, 0.50),
                (16.0, 0.78),
                (17.5, 1.00),
                (20.0, 0.95),
                (23.0, 0.75),
                (26.5, 0.50),
                (30.0, 0.30),
                (33.0, 0.17),
                (36.0, 0.05),
                (39.0, 0.00),
            ],
        }
    }

    /// `(age, share sustained)` points of each ageing class. Fitted to the
    /// shipped database's attributes by age (`database_attributes_by_age.txt`),
    /// with decline somewhat steeper than its survivors show: from 29 to 35 a
    /// regular of median natural fitness and professionalism loses 2-4 points
    /// of pace and acceleration, 1-3 of agility and stamina, under a point of
    /// strength, leap, balance and each technical attribute, and nothing of
    /// his mind.
    fn retention_knots(class: Ageing) -> &'static [(f32, f32)] {
        match class {
            Ageing::Speed => &[
                (27.0, 1.0),
                (29.0, 0.97),
                (31.0, 0.88),
                (33.0, 0.74),
                (35.0, 0.56),
                (38.0, 0.34),
            ],
            Ageing::Agility => &[
                (28.0, 1.0),
                (30.0, 0.97),
                (32.0, 0.88),
                (35.0, 0.62),
                (38.0, 0.45),
            ],
            Ageing::Stamina => &[
                (28.0, 1.0),
                (30.0, 0.95),
                (32.0, 0.85),
                (35.0, 0.58),
                (38.0, 0.42),
            ],
            Ageing::Frame => &[(30.0, 1.0), (33.0, 0.97), (35.0, 0.92), (38.0, 0.82)],
            Ageing::Fitness => &[(33.0, 1.0), (37.0, 0.95)],
            Ageing::Technique => &[(30.0, 1.0), (33.0, 0.97), (35.0, 0.91), (38.0, 0.80)],
            Ageing::Mind => &[(36.0, 1.0), (39.0, 0.95), (41.0, 0.90)],
            Ageing::Goalkeeping => &[(34.0, 1.0), (36.0, 0.97), (38.0, 0.90)],
        }
    }

    /// Share of a family's maturity that only matches build. Training can
    /// drill a pass and a sprint; decisions, composure and a keeper's
    /// command of his area are made in games, so a player short of
    /// football holds only the rest of his age's share.
    ///
    /// Goalkeeping's share is fitted on the career grid (2026-10-09,
    /// `a_keeper_needs_football_more_than_an_outfielder`): developed from 19
    /// to 23 without a match, a keeper falls 0.11 of his potential behind a
    /// starting keeper, where a central midfielder falls 0.06 behind his.
    pub fn match_share(group: MaturationGroup) -> f32 {
        match group {
            MaturationGroup::Mental => 0.25,
            MaturationGroup::Goalkeeping => 0.30,
            MaturationGroup::Technical => 0.18,
            MaturationGroup::Physical => 0.08,
            MaturationGroup::Explosive => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILIES: [MaturationGroup; MaturationGroup::COUNT] = [
        MaturationGroup::Technical,
        MaturationGroup::Mental,
        MaturationGroup::Physical,
        MaturationGroup::Explosive,
        MaturationGroup::Goalkeeping,
    ];

    /// The whole point of the table: a mind is not finished when a body
    /// is. If these ever converge, a teenager can be developed into a
    /// complete senior professional again.
    #[test]
    fn mind_matures_later_than_body() {
        for age in 16..=22 {
            let age = age as f32 + 0.5;
            let mental = SkillMaturation::ratio(age, MaturationGroup::Mental);
            let physical = SkillMaturation::ratio(age, MaturationGroup::Physical);
            assert!(
                mental < physical,
                "mental maturity must trail physical at {age} — mental {mental}, \
                 physical {physical}"
            );
        }
    }

    /// Every family rises monotonically to its peak — no age is a worse
    /// place to be than the age before it, on the way up.
    #[test]
    fn maturation_rises_monotonically_to_peak() {
        for group in [
            MaturationGroup::Technical,
            MaturationGroup::Mental,
            MaturationGroup::Physical,
            MaturationGroup::Goalkeeping,
        ] {
            let mut prev = 0.0;
            for quarter in 15 * 4..=28 * 4 {
                let age = quarter as f32 / 4.0;
                let r = SkillMaturation::ratio(age, group);
                assert!(r >= prev, "{group:?} dipped at {age}: {prev} -> {r}");
                prev = r;
            }
        }
    }

    /// A ceiling rises with a player's age, not in steps: no week moves
    /// a family's share by more than a sliver.
    #[test]
    fn maturity_moves_by_the_week() {
        let week = 1.0 / 52.0;
        for group in FAMILIES {
            for w in 14 * 52..37 * 52 {
                let age = w as f32 * week;
                let step =
                    SkillMaturation::ratio(age + week, group) - SkillMaturation::ratio(age, group);
                assert!(
                    step.abs() <= 0.002,
                    "{group:?} moved {step:.4} in the week after {age:.2}"
                );
            }
        }
    }

    /// Keepers mature latest — a 22-year-old outfielder is technically
    /// closer to finished than a 22-year-old keeper is, and no other family
    /// reaches its peak as late as goalkeeping does.
    #[test]
    fn keepers_mature_latest() {
        assert!(
            SkillMaturation::ratio(22.5, MaturationGroup::Goalkeeping)
                < SkillMaturation::ratio(22.5, MaturationGroup::Technical)
        );
        let peaks_at = |group| {
            (15 * 4..=40 * 4)
                .map(|q| q as f32 / 4.0)
                .find(|&age| SkillMaturation::ratio(age, group) >= 1.0)
                .unwrap()
        };
        let keeper = peaks_at(MaturationGroup::Goalkeeping);
        for group in FAMILIES {
            assert!(
                peaks_at(group) <= keeper,
                "{group:?} peaks at {} after goalkeeping's {keeper}",
                peaks_at(group)
            );
        }
        assert_eq!(
            SkillMaturation::ratio(33.5, MaturationGroup::Goalkeeping),
            1.0
        );
    }

    /// A teenager can be the fastest man on the pitch and cannot be the
    /// strongest. Lumping speed in with strength capped every quick young
    /// winger — a PA 190 wide midfielder was held to 13.3 pace at 17.
    #[test]
    fn speed_arrives_years_before_strength() {
        for age in 15..=20 {
            let age = age as f32 + 0.5;
            let explosive = SkillMaturation::ratio(age, MaturationGroup::Explosive);
            let physical = SkillMaturation::ratio(age, MaturationGroup::Physical);
            assert!(
                explosive > physical,
                "speed must lead strength at {age} — explosive {explosive}, \
                 physical {physical}"
            );
        }
        // Close to finished in the late teens, unlike the rest of the body.
        assert!(SkillMaturation::ratio(17.5, MaturationGroup::Explosive) >= 0.85);
    }

    /// And it goes first. A 31-year-old has lost a yard while his
    /// decision-making is at its peak.
    #[test]
    fn speed_declines_before_the_mind_does() {
        let explosive = SkillMaturation::ratio(31.5, MaturationGroup::Explosive);
        let mental = SkillMaturation::ratio(31.5, MaturationGroup::Mental);
        assert!(
            explosive < mental,
            "a 31-year-old should be losing pace while his head peaks — \
             explosive {explosive}, mental {mental}"
        );
    }

    /// A teenager can hold barely half of the mind he will one day have.
    #[test]
    fn seventeen_year_old_is_mentally_unfinished() {
        assert_eq!(SkillMaturation::ratio(17.5, MaturationGroup::Mental), 0.55);
        assert_eq!(SkillMaturation::ratio(28.5, MaturationGroup::Mental), 0.95);
    }

    /// No birthday changes the pace of development: neither learning nor
    /// ageing moves by more than a sliver in any week of a career.
    #[test]
    fn plasticity_and_retention_move_by_the_week() {
        let week = 1.0 / 52.0;
        for w in 14 * 52..40 * 52 {
            let age = w as f32 * week;
            for group in FAMILIES {
                let step = SkillMaturation::plasticity(age + week, group)
                    - SkillMaturation::plasticity(age, group);
                assert!(
                    step.abs() <= 0.005,
                    "{group:?} plasticity moved {step:.4} in the week after {age:.2}"
                );
            }
            for idx in 0..50 {
                let step = SkillMaturation::retention(age + week, idx)
                    - SkillMaturation::retention(age, idx);
                assert!(
                    step.abs() <= 0.005,
                    "attribute {idx} retention moved {step:.4} in the week after {age:.2}"
                );
            }
        }
    }

    /// Once a family is at its most plastic, practice only gets harder.
    #[test]
    fn plasticity_never_rises_after_its_peak() {
        for group in FAMILIES {
            let mut peaked = false;
            let mut prev = 0.0;
            for q in 12 * 4..=42 * 4 {
                let p = SkillMaturation::plasticity(q as f32 / 4.0, group);
                if peaked {
                    assert!(
                        p <= prev,
                        "{group:?} plasticity rose after its peak at {}",
                        q as f32 / 4.0
                    );
                } else if p >= 1.0 {
                    peaked = true;
                }
                prev = p;
            }
            assert!(peaked, "{group:?} never reaches its peak");
        }
    }

    /// A body holds everything it has until its decline age.
    #[test]
    fn retention_is_whole_through_the_prime() {
        for idx in 0..50 {
            assert_eq!(
                SkillMaturation::retention(25.0, idx),
                1.0,
                "attribute {idx}"
            );
        }
    }
}
