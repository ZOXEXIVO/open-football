//! **The stance**: how much a side should commit, read from what the next
//! goal is worth to it.
//!
//! The coach models the rest of the match as two Poisson processes, one
//! per side, at real football's rates: a base rate moved by the strength
//! gap, the venue and the men on the pitch. Committing raises the side's
//! own rate by a share and the opponent's by a larger one. From the score
//! and the time left he values the match for the occasion — league
//! points, a tie decided on the day, a friendly — and reads, per minute of
//! play, what committing gains through the goal it might bring against
//! what it costs through the goal it might concede. That is squashed to
//! −1 (hold on to what the side has) … +1 (throw everything forward) and
//! bent by his temperament.

use crate::r#match::engine::teamplay::coach::card::Temperament;
use crate::r#match::engine::environment::MatchEnvironment;
use crate::r#match::MatchContext;

/// What the result is worth on this occasion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Occasion {
    /// Three points for a win, one for a draw.
    League,
    /// Level at the whistle goes to extra time and penalties.
    Knockout,
    /// League stakes, scaled by how much the fixture matters.
    Friendly { importance: f32 },
}

/// Each side's chance of scoring, in goals a minute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoringRates {
    pub own: f32,
    pub opp: f32,
}

/// The coach's picture of the fixture, fixed at kickoff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outlook {
    /// Own strength over the opponent's, in levels.
    pub strength_gap: f32,
    /// +1 at home, −1 away, 0 on neutral ground.
    pub venue: f32,
    pub occasion: Occasion,
}

impl Outlook {
    /// A real side's goals in ninety minutes against an equal one.
    pub const BASE_PER_90: f32 = 1.38;
    /// Log-rate per level of strength, split evenly between the two sides:
    /// fitted to the ladder's real table (gap 0, 2, 4, 6, 9: favourite wins
    /// ~38/45/58/70/78%, draws 25/25/22/17/13%) averaged over venues.
    pub const STRENGTH_PER_LEVEL: f32 = 0.156;
    /// The home side's rate over the neutral one, and the away side's under
    /// it: a real home edge of ~0.35 goals a match, 45/25/30 between equals.
    pub const HOME_FACTOR: f32 = 1.13;
    /// Each man more than the opponent multiplies a side's rate by this.
    pub const MAN_FACTOR: f32 = 1.2;
    /// What full commitment adds to the side's own rate, and to the
    /// opponent's: committing costs more than it buys.
    pub const OWN_SHARE: f32 = 0.25;
    pub const OPP_SHARE: f32 = 0.29;
    /// One level of the generator's squads in composite units: 0.575 skill
    /// points on the 0..1 scale of a normalised attribute.
    pub const LEVEL_IN_COMPOSITE: f32 = 0.575 / 20.0;
    /// The crowd-times-advantage edge of an ordinary home fixture: the
    /// venue at which `HOME_FACTOR` was fitted.
    pub const REFERENCE_HOME_EDGE: f32 = 0.275;
    /// Extra time, in minutes.
    const EXTRA_TIME: f32 = 30.0;

    /// Each side's picture of a fixture, the home side's first: the two
    /// elevens' `strengths`, the home edge the `environment` gives (none
    /// with the home edge switched off) and what is at stake.
    pub fn for_fixture(
        strengths: [f32; 2],
        environment: &MatchEnvironment,
        knockout: bool,
        friendly: bool,
    ) -> [Self; 2] {
        let venue = if MatchContext::home_flat() {
            0.0
        } else {
            (environment.crowd_intensity * environment.home_advantage / Self::REFERENCE_HOME_EDGE)
                .clamp(0.0, 1.5)
        };
        let occasion = if knockout {
            Occasion::Knockout
        } else if friendly {
            Occasion::Friendly {
                importance: environment.match_importance.clamp(0.1, 1.0),
            }
        } else {
            Occasion::League
        };
        [
            Self::from_strengths(strengths[0], strengths[1], venue, occasion),
            Self::from_strengths(strengths[1], strengths[0], -venue, occasion),
        ]
    }

    /// The outlook from the two sides' strengths on the composite scale the
    /// match standard reads.
    pub fn from_strengths(own: f32, opp: f32, venue: f32, occasion: Occasion) -> Self {
        Outlook {
            strength_gap: (own - opp) / Self::LEVEL_IN_COMPOSITE,
            venue,
            occasion,
        }
    }

    pub fn rates(&self, own_men: u8, opp_men: u8, commitment: f32) -> ScoringRates {
        let base = Self::BASE_PER_90 / 90.0;
        let strength = (Self::STRENGTH_PER_LEVEL * self.strength_gap * 0.5).exp();
        let venue = Self::HOME_FACTOR.powf(self.venue);
        let men = Self::MAN_FACTOR.powi(own_men as i32 - opp_men as i32);
        let r = commitment.clamp(-1.0, 1.0);
        ScoringRates {
            own: base * strength * venue * men * (1.0 + Self::OWN_SHARE * r),
            opp: base / (strength * venue * men) * (1.0 + Self::OPP_SHARE * r),
        }
    }

    /// `[win, draw, loss]` from `difference` with `minutes` left at `rates`.
    pub fn outcome(difference: i32, minutes: f32, rates: ScoringRates) -> [f32; 3] {
        const MAX_GOALS: usize = 12;
        let poisson = |mean: f32| {
            let mut p = [0.0f32; MAX_GOALS];
            p[0] = (-mean).exp();
            for k in 1..MAX_GOALS {
                p[k] = p[k - 1] * mean / k as f32;
            }
            p
        };
        let minutes = minutes.max(0.0);
        let (own, opp) = (poisson(rates.own * minutes), poisson(rates.opp * minutes));
        let mut result = [0.0f32; 3];
        for (i, p_own) in own.iter().enumerate() {
            for (j, p_opp) in opp.iter().enumerate() {
                let end = difference + i as i32 - j as i32;
                let slot = match end.cmp(&0) {
                    std::cmp::Ordering::Greater => 0,
                    std::cmp::Ordering::Equal => 1,
                    std::cmp::Ordering::Less => 2,
                };
                result[slot] += p_own * p_opp;
            }
        }
        result
    }

    /// What the match is worth to the side from here.
    pub fn value(&self, difference: i32, minutes: f32, rates: ScoringRates) -> f32 {
        let [win, draw, _] = Self::outcome(difference, minutes, rates);
        match self.occasion {
            Occasion::League => 3.0 * win + draw,
            Occasion::Friendly { importance } => (3.0 * win + draw) * importance,
            Occasion::Knockout => win + self.knockout_share(rates) * draw,
        }
    }

    /// The chance of winning a tie that is level at the whistle: extra time
    /// at the same rates, then a coin for the penalties.
    fn knockout_share(&self, rates: ScoringRates) -> f32 {
        let [win, draw, _] = Self::outcome(0, Self::EXTRA_TIME, rates);
        win + 0.5 * draw
    }
}

/// What the stance reads: the score as the coach sees it, the time left and
/// the men on the pitch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Situation {
    pub difference: i32,
    pub minutes_left: f32,
    pub own_men: u8,
    pub opp_men: u8,
}

/// The coach's commitment, −1 … +1, as it moves through the match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stance {
    /// The reading the side is moving toward.
    target: f32,
    /// The reading in force.
    current: f32,
    /// A volatile coach's reaction to the last goal, decaying.
    impulse: f32,
    /// The match minute the stance was last moved.
    minute: f32,
}

impl Default for Outlook {
    fn default() -> Self {
        Outlook {
            strength_gap: 0.0,
            venue: 0.0,
            occasion: Occasion::League,
        }
    }
}

impl Default for Situation {
    fn default() -> Self {
        Situation {
            difference: 0,
            minutes_left: 90.0,
            own_men: 11,
            opp_men: 11,
        }
    }
}

impl Default for Stance {
    fn default() -> Self {
        Stance {
            target: 0.0,
            current: 0.0,
            impulse: 0.0,
            minute: 0.0,
        }
    }
}

impl Stance {
    /// What a goal swings, in points, for the reading to read ~0.76 (tanh 1).
    const SCALE: f32 = 0.67;
    /// How far temperament moves the reading before it is squashed.
    const RISK_SHIFT: f32 = 0.5;
    const CAUTION_SHIFT: f32 = 0.5;
    /// The impulse after a goal: its size for the most volatile coach, and
    /// how fast it fades.
    const IMPULSE: f32 = 0.6;
    const IMPULSE_HALF_LIFE: f32 = 2.5;
    /// How long a side takes to adopt a new reading, in minutes, for a calm
    /// coach and the reduction for a volatile one.
    const ADOPTION: f32 = 1.2;
    const ADOPTION_VOLATILE: f32 = 0.8;

    /// The reading for `situation`: the value's derivative in commitment,
    /// per minute of play, squashed, and bent by the temperament.
    pub fn reading(outlook: &Outlook, temperament: &Temperament, situation: &Situation) -> f32 {
        let rates = outlook.rates(situation.own_men, situation.opp_men, 0.0);
        let left = situation.minutes_left.max(0.0);
        let d = situation.difference;
        let now = outlook.value(d, left, rates);
        let for_goal = outlook.value(d + 1, left, rates) - now;
        let against_goal = now - outlook.value(d - 1, left, rates);
        let gain = Outlook::OWN_SHARE * rates.own * for_goal
            - Outlook::OPP_SHARE * rates.opp * against_goal;
        let per_rate = Outlook::OWN_SHARE * (rates.own + rates.opp) * 0.5;
        let raw = gain / (per_rate * Self::SCALE);
        let lean = (temperament.risk_tolerance - 0.5) * Self::RISK_SHIFT
            - (temperament.caution - 0.5) * Self::CAUTION_SHIFT;
        (raw + lean).tanh()
    }

    /// Set the reading at kickoff, adopted at once.
    pub fn kick_off(&mut self, reading: f32) {
        *self = Stance {
            target: reading,
            current: reading,
            impulse: 0.0,
            minute: 0.0,
        };
    }

    /// Move toward `reading` as of `minute`.
    pub fn follow(&mut self, reading: f32, minute: f32, temperament: &Temperament) {
        let elapsed = (minute - self.minute).max(0.0);
        self.impulse *= 0.5f32.powf(elapsed / Self::IMPULSE_HALF_LIFE);
        self.target = reading;
        let tau = Self::ADOPTION - Self::ADOPTION_VOLATILE * temperament.volatility;
        let toward = (self.target + self.impulse).clamp(-1.0, 1.0);
        self.current += (toward - self.current) * (1.0 - (-elapsed / tau.max(0.05)).exp());
        self.minute = minute;
    }

    /// A goal moved the reading from `before` to `after`; a volatile coach
    /// overreacts in the same direction for a few minutes.
    pub fn on_goal(&mut self, before: f32, after: f32, temperament: &Temperament) {
        self.impulse = (after - before) * temperament.volatility * Self::IMPULSE;
    }

    pub fn current(&self) -> f32 {
        self.current
    }

    pub fn impulse(&self) -> f32 {
        self.impulse
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn equal(occasion: Occasion) -> Outlook {
        Outlook {
            strength_gap: 0.0,
            venue: 0.0,
            occasion,
        }
    }

    fn at(difference: i32, minute: f32) -> Situation {
        Situation {
            difference,
            minutes_left: 90.0 - minute,
            own_men: 11,
            opp_men: 11,
        }
    }

    fn read(outlook: &Outlook, situation: Situation) -> f32 {
        Stance::reading(outlook, &Temperament::neutral(), &situation)
    }

    #[test]
    fn the_rates_reproduce_the_real_ladder_from_kickoff() {
        let reference = [
            (0.0, [0.375, 0.25, 0.375]),
            (2.0, [0.45, 0.25, 0.30]),
            (4.0, [0.58, 0.22, 0.20]),
            (6.0, [0.70, 0.17, 0.13]),
            (9.0, [0.78, 0.13, 0.09]),
        ];
        for (gap, expected) in reference {
            let mut shares = [0.0f32; 3];
            for venue in [1.0, -1.0] {
                let outlook = Outlook {
                    strength_gap: gap,
                    venue,
                    occasion: Occasion::League,
                };
                let o = Outlook::outcome(0, 90.0, outlook.rates(11, 11, 0.0));
                for (s, p) in shares.iter_mut().zip(o) {
                    *s += p * 0.5;
                }
            }
            for (got, want) in shares.iter().zip(expected) {
                assert!((got - want).abs() <= 0.03, "gap {gap}: {shares:?} against {expected:?}");
            }
        }
        let home = Outlook {
            strength_gap: 0.0,
            venue: 1.0,
            occasion: Occasion::League,
        }
        .rates(11, 11, 0.0);
        let edge = (home.own - home.opp) * 90.0;
        assert!((0.25..=0.45).contains(&edge), "home edge {edge}");
    }

    #[test]
    fn behind_commits_more_than_level_and_level_more_than_leading() {
        let league = equal(Occasion::League);
        for minute in [20.0, 50.0, 80.0] {
            let (behind, level, ahead) = (
                read(&league, at(-1, minute)),
                read(&league, at(0, minute)),
                read(&league, at(1, minute)),
            );
            assert!(behind > level && level > ahead, "{minute}': {behind} {level} {ahead}");
        }
    }

    #[test]
    fn a_late_lead_is_protected_harder_than_an_early_one() {
        let league = equal(Occasion::League);
        let early = read(&league, at(1, 20.0));
        let late = read(&league, at(1, 80.0));
        let level = read(&league, at(0, 20.0));
        assert!(late < early && early < level, "{late} {early} {level}");
    }

    #[test]
    fn the_favourite_pushes_and_the_underdog_takes_the_point() {
        let favourite = Outlook {
            strength_gap: 4.0,
            venue: 0.0,
            occasion: Occasion::League,
        };
        let underdog = Outlook {
            strength_gap: -4.0,
            ..favourite
        };
        assert!(read(&favourite, at(0, 80.0)) > read(&underdog, at(0, 80.0)));
        assert!(read(&favourite, at(0, 0.0)) > 0.0 && read(&underdog, at(0, 0.0)) < 0.0);
    }

    #[test]
    fn a_tie_decided_on_the_day_is_cagier_than_a_league_match() {
        let league = read(&equal(Occasion::League), at(0, 85.0));
        let knockout = read(&equal(Occasion::Knockout), at(0, 85.0));
        assert!(knockout < league, "{knockout} {league}");
    }

    #[test]
    fn equal_sides_kick_off_near_neutral() {
        let level = read(&equal(Occasion::League), at(0, 0.0));
        assert!(level.abs() < 0.1, "{level}");
    }

    #[test]
    fn a_man_down_protects_harder() {
        let league = equal(Occasion::League);
        let eleven = read(&league, at(1, 40.0));
        let ten = Stance::reading(
            &league,
            &Temperament::neutral(),
            &Situation {
                own_men: 10,
                ..at(1, 40.0)
            },
        );
        assert!(ten < eleven, "{ten} {eleven}");
    }

    #[test]
    fn a_risk_taker_commits_more_than_a_cautious_coach_when_behind() {
        let league = equal(Occasion::League);
        let mut bold = Temperament::neutral();
        bold.risk_tolerance = 0.9;
        bold.caution = 0.15;
        let mut careful = Temperament::neutral();
        careful.risk_tolerance = 0.15;
        careful.caution = 0.9;
        let behind = at(-1, 70.0);
        assert!(
            Stance::reading(&league, &bold, &behind) > Stance::reading(&league, &careful, &behind)
        );
    }

    #[test]
    fn a_volatile_coach_s_impulse_fades_within_ten_minutes() {
        let mut volatile = Temperament::neutral();
        volatile.volatility = 0.9;
        let mut stance = Stance::default();
        stance.kick_off(0.0);
        stance.on_goal(0.0, 0.6, &volatile);
        let start = stance.impulse();
        assert!(start > 0.0);
        stance.follow(0.6, 10.0, &volatile);
        assert!(stance.impulse() < 0.1 * start, "{} of {}", stance.impulse(), start);
    }

    #[test]
    fn a_side_adopts_a_new_reading_over_about_a_minute() {
        let calm = Temperament::neutral();
        let mut stance = Stance::default();
        stance.kick_off(0.0);
        stance.follow(1.0, 0.25, &calm);
        let early = stance.current();
        stance.follow(1.0, 5.0, &calm);
        assert!(early > 0.0 && early < 0.5, "{early}");
        assert!(stance.current() > 0.95, "{}", stance.current());
    }
}
