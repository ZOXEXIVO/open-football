//! When each kind of footballing skill matures.
//!
//! A body finishes before a mind does. Pace and strength peak in the
//! early twenties; decisions, composure, positioning and vision keep
//! improving into the late twenties and hold into the thirties, because
//! they are built out of games watched, games played and mistakes made.
//! Goalkeeping craft matures latest of all — keepers peak at 28-33.
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
}

pub struct SkillMaturation;

impl SkillMaturation {
    /// Fraction of his eventual (PA-derived) level a player of `age`
    /// years — fractional, a player grows into his age a day at a time —
    /// can hold in this skill family.
    ///
    /// Straight lines between the [`Self::knots`]; before the first and
    /// after the last the curve holds level.
    pub fn ratio(age: f32, group: MaturationGroup) -> f32 {
        let knots = Self::knots(group);
        let after = knots.partition_point(|&(knot_age, _)| knot_age <= age);
        if after == 0 {
            return knots[0].1;
        }
        if after == knots.len() {
            return knots[after - 1].1;
        }
        let ((a0, r0), (a1, r1)) = (knots[after - 1], knots[after]);
        r0 + (r1 - r0) * (age - a0) / (a1 - a0)
    }

    /// `(age, share)` points of each family's curve. Each share is held
    /// at the middle of the years it applies to and a peak across all of
    /// them, so a ceiling rises with the player's age rather than in
    /// steps his growth would stall under and then lurch through.
    ///
    /// The technical / mental / physical shares are the generator's own
    /// numbers, moved here so generation and development cannot drift
    /// apart again. The goalkeeping row follows the later peak both
    /// modules document for keepers (28-33), between technical and mental.
    fn knots(group: MaturationGroup) -> &'static [(f32, f32)] {
        match group {
            MaturationGroup::Technical => &[
                (17.5, 0.75),
                (19.0, 0.82),
                (21.5, 0.90),
                (25.0, 0.95),
                (27.0, 1.00),
                (30.0, 1.00),
                (31.5, 0.97),
                (34.0, 0.93),
            ],
            MaturationGroup::Mental => &[
                (17.5, 0.55),
                (19.0, 0.62),
                (21.5, 0.72),
                (25.0, 0.85),
                (28.5, 0.95),
                (30.0, 1.00),
            ],
            MaturationGroup::Physical => &[
                (17.5, 0.70),
                (19.0, 0.78),
                (21.5, 0.88),
                (25.0, 0.95),
                (27.0, 1.00),
                (30.0, 1.00),
                (31.5, 0.93),
                (34.0, 0.82),
            ],
            // Sprinters peak around 20-25 and are already close to it in
            // their late teens; the decline is later and gentler than
            // strength's, but it is the axis that visibly goes first.
            MaturationGroup::Explosive => &[
                (15.5, 0.78),
                (17.0, 0.88),
                (19.0, 0.94),
                (20.0, 1.00),
                (25.0, 1.00),
                (27.0, 0.97),
                (30.5, 0.90),
                (33.0, 0.80),
            ],
            MaturationGroup::Goalkeeping => &[
                (17.5, 0.62),
                (19.0, 0.70),
                (21.5, 0.80),
                (25.0, 0.90),
                (28.5, 0.97),
                (30.0, 1.00),
                (34.0, 1.00),
                (35.0, 0.97),
            ],
        }
    }

    /// Share of a family's maturity that only matches build. Training can
    /// drill a pass and a sprint; decisions, composure and a keeper's
    /// command of his area are made in games, so a player short of
    /// football holds only the rest of his age's share.
    pub fn match_share(group: MaturationGroup) -> f32 {
        match group {
            MaturationGroup::Mental => 0.15,
            MaturationGroup::Goalkeeping => 0.12,
            MaturationGroup::Technical => 0.08,
            MaturationGroup::Physical => 0.03,
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
    /// closer to finished than a 22-year-old keeper is.
    #[test]
    fn keepers_mature_latest() {
        assert!(
            SkillMaturation::ratio(22.5, MaturationGroup::Goalkeeping)
                < SkillMaturation::ratio(22.5, MaturationGroup::Technical)
        );
        assert_eq!(
            SkillMaturation::ratio(31.5, MaturationGroup::Goalkeeping),
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
}
