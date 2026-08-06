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

pub struct SkillMaturation;

impl SkillMaturation {
    /// Fraction of his eventual (PA-derived) level a player of this age
    /// can hold in this skill family.
    ///
    /// The technical / mental / physical rows are the generator's own
    /// numbers, moved here verbatim so generation and development cannot
    /// drift apart again. The goalkeeping row is new — the generator had
    /// no separate one because it folds GK skills into its technical
    /// group — and follows the later peak both modules already document
    /// for keepers (28-33), sitting between technical and mental.
    pub fn ratio(age: u32, group: MaturationGroup) -> f32 {
        match group {
            MaturationGroup::Technical => match age {
                0..=17 => 0.75,
                18..=19 => 0.82,
                20..=22 => 0.90,
                23..=26 => 0.95,
                27..=29 => 1.00,
                30..=32 => 0.97,
                _ => 0.93,
            },
            MaturationGroup::Mental => match age {
                0..=17 => 0.55,
                18..=19 => 0.62,
                20..=22 => 0.72,
                23..=26 => 0.85,
                27..=29 => 0.95,
                _ => 1.00,
            },
            MaturationGroup::Physical => match age {
                0..=17 => 0.70,
                18..=19 => 0.78,
                20..=22 => 0.88,
                23..=26 => 0.95,
                27..=29 => 1.00,
                30..=32 => 0.93,
                _ => 0.82,
            },
            // Sprinters peak around 20-25 and are already close to it in
            // their late teens; the decline is later and gentler than
            // strength's, but it is the axis that visibly goes first.
            MaturationGroup::Explosive => match age {
                0..=15 => 0.78,
                16..=17 => 0.88,
                18..=19 => 0.94,
                20..=24 => 1.00,
                25..=28 => 0.97,
                29..=31 => 0.90,
                _ => 0.80,
            },
            MaturationGroup::Goalkeeping => match age {
                0..=17 => 0.62,
                18..=19 => 0.70,
                20..=22 => 0.80,
                23..=26 => 0.90,
                27..=29 => 0.97,
                30..=33 => 1.00,
                _ => 0.97,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the table: a mind is not finished when a body
    /// is. If these ever converge, a teenager can be developed into a
    /// complete senior professional again.
    #[test]
    fn mind_matures_later_than_body() {
        for age in 16..=22 {
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
            for age in 15..=28 {
                let r = SkillMaturation::ratio(age, group);
                assert!(r >= prev, "{group:?} dipped at {age}: {prev} -> {r}");
                prev = r;
            }
        }
    }

    /// Keepers mature latest — a 22-year-old outfielder is technically
    /// closer to finished than a 22-year-old keeper is.
    #[test]
    fn keepers_mature_latest() {
        assert!(
            SkillMaturation::ratio(22, MaturationGroup::Goalkeeping)
                < SkillMaturation::ratio(22, MaturationGroup::Technical)
        );
        assert_eq!(
            SkillMaturation::ratio(31, MaturationGroup::Goalkeeping),
            1.0
        );
    }

    /// A teenager can be the fastest man on the pitch and cannot be the
    /// strongest. Lumping speed in with strength capped every quick young
    /// winger — a PA 190 wide midfielder was held to 13.3 pace at 17.
    #[test]
    fn speed_arrives_years_before_strength() {
        for age in 15..=20 {
            let explosive = SkillMaturation::ratio(age, MaturationGroup::Explosive);
            let physical = SkillMaturation::ratio(age, MaturationGroup::Physical);
            assert!(
                explosive > physical,
                "speed must lead strength at {age} — explosive {explosive}, \
                 physical {physical}"
            );
        }
        // Close to finished in the late teens, unlike the rest of the body.
        assert!(SkillMaturation::ratio(17, MaturationGroup::Explosive) >= 0.85);
    }

    /// And it goes first. A 31-year-old has lost a yard while his
    /// decision-making is at its peak.
    #[test]
    fn speed_declines_before_the_mind_does() {
        let explosive = SkillMaturation::ratio(31, MaturationGroup::Explosive);
        let mental = SkillMaturation::ratio(31, MaturationGroup::Mental);
        assert!(
            explosive < mental,
            "a 31-year-old should be losing pace while his head peaks — \
             explosive {explosive}, mental {mental}"
        );
    }

    /// A teenager can hold barely half of the mind he will one day have.
    #[test]
    fn seventeen_year_old_is_mentally_unfinished() {
        assert_eq!(SkillMaturation::ratio(17, MaturationGroup::Mental), 0.55);
        assert_eq!(SkillMaturation::ratio(28, MaturationGroup::Mental), 0.95);
    }
}
