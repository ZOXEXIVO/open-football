//! Match-day form — how reliably a player turns his ability into a
//! performance *on this particular day*.
//!
//! # Why this exists at the engine layer and not at the rating layer
//!
//! "A young player is unpredictable" is a statement about football, not
//! about arithmetic. If it is implemented as a wobble added to the
//! finished rating, then the young player's *stat line* is identical to
//! the veteran's and only the printed number differs — his passes still
//! find their man, his shots still hit the target, and the number
//! disagrees with the match it claims to describe. Anyone reading the
//! match page sees it.
//!
//! So the wobble is applied where the performance is produced. The draw
//! below stamps a multiplier onto [`crate::MatchPlayer`] at kickoff and
//! [`effective_skill`](crate::r#match::engine) folds it into every
//! skill-mediated action — a duel, a first touch, a save, a finish. On a
//! bad day the teenager genuinely misplaces passes; on a good day he
//! genuinely doesn't. The rating then does what it always does: read the
//! stat line. It never learns who the player is, which is what keeps the
//! model outcome-based.
//!
//! # The draw has mean 1.0, always
//!
//! Volatility widens the distribution; it never shifts it. A streaky
//! player is not a worse player — over a season his form draws average
//! out to exactly the same 1.0 as a metronome's, and any difference in
//! his season rating comes from how the match engine and the rating
//! curve treat variance, not from a thumb on the scale. That is the
//! line between "this player handles a match day differently" (fair
//! game) and "this player is worse, so rate him lower" (never).
//!
//! # What widens it
//!
//! * **Age.** The dominant term. Teenagers are erratic in a way nobody
//!   disputes; steadiness arrives with the mid-twenties, holds through
//!   the peak years, and unwinds again late in a career as the body
//!   stops answering identically every week.
//! * **Consistency**, the attribute that exists for exactly this, plus
//!   **concentration** (does he stay switched on for ninety minutes),
//!   **professionalism** (does he prepare the same way every week) and
//!   **composure** (does the occasion get to him).
//!
//! A sixteen-year-old with consistency 3 swings about ±11% on the day;
//! a twenty-eight-year-old with consistency 18 and professionalism 17
//! swings about ±2%. Both average out to 1.0.

use crate::club::player::Player;
use crate::utils::DateUtils;
use chrono::{Datelike, NaiveDate};

/// Standard deviation of the form multiplier for a maximally erratic
/// player. ±11% of effective skill is a genuinely different afternoon —
/// about 1.5 points of a 14-rated skill — without being a different
/// player.
const MAX_VOLATILITY: f32 = 0.11;

/// Hard bounds on the multiplier. The draw is bell-shaped and bounded
/// at ±3 sd already; this is the belt-and-braces floor/ceiling so no
/// combination of inputs can produce a player who is unrecognisable.
const FORM_MIN: f32 = 0.75;
const FORM_MAX: f32 = 1.25;

/// Age at which steadiness has fully arrived, and the age past which it
/// starts to unwind again.
const STEADY_FROM: f32 = 26.0;
const DECLINE_FROM: f32 = 31.0;

pub struct MatchdayForm;

impl MatchdayForm {
    /// The multiplier for this player on this date. Deterministic: the
    /// same player on the same day always draws the same form, so a
    /// match can be replayed and a save file reloaded without the
    /// result moving.
    pub fn factor(player: &Player, now: Option<NaiveDate>) -> f32 {
        // No calendar means no match day to draw for — synthetic squads
        // and unit fixtures play at exactly their listed ability, which
        // is what makes them reproducible reference points.
        let Some(date) = now else {
            return 1.0;
        };
        let volatility = Self::volatility(player, Some(date));
        if volatility <= 0.0 {
            return 1.0;
        }
        let day = date.num_days_from_ce() as u64;
        (1.0 + volatility * Self::draw(player.id as u64, day)).clamp(FORM_MIN, FORM_MAX)
    }

    /// Standard deviation of this player's form distribution — how much
    /// day-to-day variation he carries. Never negative; zero only for a
    /// hypothetically perfect professional.
    pub fn volatility(player: &Player, now: Option<NaiveDate>) -> f32 {
        MAX_VOLATILITY * Self::mental_steadiness(player) * Self::age_factor(player, now)
    }

    /// `1.0` for a player with no mental steadiness at all, `0.25` for a
    /// maximally reliable one. Consistency carries most of the weight —
    /// it is the attribute that exists for this — with concentration,
    /// professionalism and composure filling in the rest of what
    /// "turns up the same every week" means.
    fn mental_steadiness(player: &Player) -> f32 {
        let norm = |v: f32| (v / 20.0).clamp(0.0, 1.0);
        let steadiness = 0.55 * norm(player.attributes.consistency)
            + 0.20 * norm(player.skills.mental.concentration)
            + 0.15 * norm(player.attributes.professionalism)
            + 0.10 * norm(player.skills.mental.composure);
        1.0 - 0.75 * steadiness
    }

    /// Age multiplier on the spread: erratic in the teens, settled from
    /// the mid-twenties, loosening again in the mid-thirties. Continuous
    /// — a player does not become reliable on a birthday.
    fn age_factor(player: &Player, now: Option<NaiveDate>) -> f32 {
        let Some(date) = now else {
            // No calendar (synthetic squads, unit fixtures): treat the
            // player as settled rather than inventing an age.
            return 1.0;
        };
        let age = DateUtils::age(player.birth_date, date) as f32;
        let young = ((STEADY_FROM - age) / 10.0).clamp(0.0, 1.0);
        let old = ((age - DECLINE_FROM) / 9.0).clamp(0.0, 1.0);
        1.0 + 0.60 * young.powf(1.4) + 0.25 * old
    }

    /// A bounded, bell-shaped, zero-mean unit draw for `(player, day)`.
    ///
    /// Three independent hashes summed (Irwin–Hall) rather than one
    /// uniform: a flat distribution would make "career-best" and
    /// "utterly ordinary" equally likely every week, which is not how
    /// form behaves. The sum is centred and scaled to unit variance, and
    /// is naturally bounded at ±3 sd — no clamping artefacts at the
    /// tails.
    fn draw(player_id: u64, day: u64) -> f32 {
        let u = |salt: u64| Self::unit_hash(player_id ^ (day << 1), salt);
        // Irwin–Hall(3): mean 1.5, sd 0.5, support [0, 3].
        let sum = u(0x9E37_79B9_7F4A_7C15) + u(0xBF58_476D_1CE4_E5B9) + u(0x94D0_49BB_1331_11EB);
        (sum - 1.5) / 0.5
    }

    /// SplitMix64 finaliser → uniform `[0, 1)`. Chosen over the cheaper
    /// golden-ratio `fract()` trick used elsewhere in the pipeline
    /// because that one leaves visible structure when two correlated
    /// inputs (a player id and a date, both dense integers) are mixed —
    /// neighbouring squad ids would draw neighbouring form.
    fn unit_hash(seed: u64, salt: u64) -> f32 {
        let mut z = seed.wrapping_add(salt);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // Top 24 bits → [0, 1). f32 has 24 mantissa bits, so this is
        // exactly representable and uniformly spaced.
        (z >> 40) as f32 / (1u64 << 24) as f32
    }
}

impl Player {
    /// Pre-match form multiplier for this player on `now`. Stamped onto
    /// the match player at kickoff and consumed by `effective_skill`
    /// alongside the settledness stamp — see [`MatchdayForm`].
    pub fn matchday_form(&self, now: Option<NaiveDate>) -> f32 {
        MatchdayForm::factor(self, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };

    const MATCHDAY: Option<NaiveDate> = None;

    fn day(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(y, m, d)
    }

    fn player(id: u32, born: (i32, u32, u32), consistency: f32, concentration: f32) -> Player {
        let mut attributes = PersonAttributes::default();
        attributes.consistency = consistency;
        attributes.professionalism = consistency;
        let mut skills = PlayerSkills::default();
        skills.mental.concentration = concentration;
        skills.mental.composure = concentration;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(born.0, born.1, born.2).unwrap())
            .country_id(1)
            .attributes(attributes)
            .skills(skills)
            .player_attributes(PlayerAttributes::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 15,
                }],
            })
            .build()
            .unwrap()
    }

    /// The whole point: volatility widens the distribution, it never
    /// shifts it. A streaky player is not a worse player.
    #[test]
    fn form_averages_to_one_over_a_season() {
        let streaky = player(1, (2010, 3, 1), 4.0, 5.0);
        let mut sum = 0.0;
        let mut n = 0;
        for d in 1..=250 {
            let date = NaiveDate::from_ymd_opt(2029, 1, 1).unwrap() + chrono::Duration::days(d);
            sum += MatchdayForm::factor(&streaky, Some(date));
            n += 1;
        }
        let mean = sum / n as f32;
        assert!(
            (mean - 1.0).abs() < 0.02,
            "form mean over {n} match days was {mean:.4} — the draw must be \
             centred, or volatility would double as an ability penalty"
        );
    }

    /// Youth and low consistency both widen the spread; age and mental
    /// steadiness both narrow it.
    #[test]
    fn the_young_and_the_streaky_swing_hardest() {
        let on = day(2029, 8, 1);
        let teenager = player(1, (2012, 3, 1), 4.0, 5.0);
        let peak_pro = player(2, (2000, 3, 1), 18.0, 17.0);
        let peak_streaky = player(3, (2000, 3, 1), 4.0, 5.0);
        let teen_pro = player(4, (2012, 3, 1), 18.0, 17.0);

        let v = |p: &Player| MatchdayForm::volatility(p, on);
        assert!(
            v(&teenager) > v(&peak_streaky),
            "a teenager must be less reliable than the same temperament at 29"
        );
        assert!(
            v(&peak_streaky) > v(&peak_pro),
            "consistency must narrow the spread at a fixed age"
        );
        assert!(
            v(&teen_pro) > v(&peak_pro),
            "youth must widen the spread at a fixed temperament"
        );
        assert!(
            v(&teenager) > v(&peak_pro) * 3.0,
            "the extremes must be far apart: teenager {:.4} vs settled pro {:.4}",
            v(&teenager),
            v(&peak_pro)
        );
    }

    /// Careers loosen up again at the far end.
    #[test]
    fn veterans_become_less_reliable_again() {
        let on = day(2029, 8, 1);
        let peak = player(1, (2000, 3, 1), 12.0, 12.0);
        let veteran = player(2, (1992, 3, 1), 12.0, 12.0);
        assert!(MatchdayForm::volatility(&veteran, on) > MatchdayForm::volatility(&peak, on));
    }

    /// Same player, same day, same form — a match must be replayable and
    /// a save file reloadable without the result moving.
    #[test]
    fn the_draw_is_deterministic() {
        let p = player(7, (2008, 5, 5), 9.0, 9.0);
        let on = day(2029, 11, 3);
        assert_eq!(MatchdayForm::factor(&p, on), MatchdayForm::factor(&p, on));
        assert_ne!(
            MatchdayForm::factor(&p, on),
            MatchdayForm::factor(&p, day(2029, 11, 10))
        );
    }

    /// Neighbouring squad ids on the same day must not draw neighbouring
    /// form — that was the failure mode of the cheaper golden-ratio hash
    /// used elsewhere in the pipeline, and it would make a whole back
    /// four have a bad day together.
    #[test]
    fn adjacent_squad_ids_do_not_share_a_form_day() {
        let on = day(2029, 4, 12);
        let squad: Vec<f32> = (100..111)
            .map(|id| MatchdayForm::factor(&player(id, (2005, 1, 1), 6.0, 6.0), on))
            .collect();
        let above = squad.iter().filter(|f| **f > 1.0).count();
        assert!(
            (2..=9).contains(&above),
            "{above} of 11 team-mates drew above-par form on the same day — \
             the hash is correlating ids: {squad:?}"
        );
    }

    /// No calendar, no draw: synthetic squads and unit fixtures play at
    /// exactly their listed ability so they stay reproducible.
    #[test]
    fn no_calendar_means_no_draw() {
        let p = player(1, (2012, 1, 1), 3.0, 3.0);
        assert_eq!(MatchdayForm::factor(&p, MATCHDAY), 1.0);
    }

    /// Bounded even for the most erratic player on the unluckiest seed.
    #[test]
    fn form_stays_within_recognisable_bounds() {
        let p = player(1, (2013, 1, 1), 1.0, 1.0);
        for d in 0..400 {
            let date = NaiveDate::from_ymd_opt(2029, 1, 1).unwrap() + chrono::Duration::days(d);
            let f = MatchdayForm::factor(&p, Some(date));
            assert!(
                (FORM_MIN..=FORM_MAX).contains(&f),
                "form {f} escaped the bounds"
            );
        }
    }
}
