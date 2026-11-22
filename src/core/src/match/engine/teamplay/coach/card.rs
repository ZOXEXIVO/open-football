//! **The coach card.** What a match needs to know about the man in the
//! dugout: his temperament, the football he prefers and how he coaches.
//! Small and serialisable, so it travels with the match squad and across the
//! worker wire.

use crate::Staff;
use crate::club::staff::{CoachProfile, CoachingStyle, StaffCoaching, StaffMental};
use crate::r#match::engine::teamplay::coach::plan::GamePlan;
use serde::{Deserialize, Serialize};

/// How the coach decides, each 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Temperament {
    /// How much risk he accepts to win.
    pub risk_tolerance: f32,
    /// How much he fears losing what he has.
    pub caution: f32,
    /// How firmly he holds to the plan he started with.
    pub stubbornness: f32,
    /// How readily he changes it when the match asks.
    pub adaptability: f32,
    /// How strongly he reacts to what has just happened.
    pub volatility: f32,
    /// How well he reads a match.
    pub read: f32,
    pub man_management: f32,
    pub motivating: f32,
}

impl Temperament {
    pub const fn neutral() -> Self {
        Temperament {
            risk_tolerance: 0.5,
            caution: 0.5,
            stubbornness: 0.5,
            adaptability: 0.5,
            volatility: 0.5,
            read: 0.5,
            man_management: 0.5,
            motivating: 0.5,
        }
    }

    /// How well he handles and lifts players.
    pub fn handling(&self) -> f32 {
        0.5 * self.man_management + 0.5 * self.motivating
    }

    /// How much a better plan has to offer before he changes his.
    pub fn resistance_to_change(&self) -> f32 {
        0.5 * self.stubbornness + 0.5 * (1.0 - self.adaptability)
    }
}

/// The parts of a coach the card is read from: his mind, what he coaches,
/// how well he knows the game, his style, and who he is.
#[derive(Debug, Clone)]
pub struct CoachMakeup {
    pub id: u32,
    pub style: CoachingStyle,
    pub mental: StaffMental,
    pub coaching: StaffCoaching,
    pub tactical_knowledge: u8,
}

impl CoachMakeup {
    pub fn of(staff: &Staff) -> Self {
        let attributes = &staff.staff_attributes;
        CoachMakeup {
            id: staff.id,
            style: staff.coaching_style,
            mental: attributes.mental.clone(),
            coaching: attributes.coaching.clone(),
            tactical_knowledge: attributes.knowledge.tactical_knowledge,
        }
    }

    /// A coach drawn from the population, the same for the same match and
    /// side. Each attribute is drawn as the staff generator draws a head
    /// coach's: a club standing, then each attribute uniform between
    /// `8 × standing` and `6 + 14 × standing`; every generated coach is
    /// `Democratic`.
    pub fn drawn(seed: u64, team_id: u32) -> Self {
        let mut state = seed
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((team_id as u64).wrapping_mul(0xD1B5_4A32_D192_ED03))
            .wrapping_add(0x7C0A_C4CA_7D00_0000);
        let standing = Self::unit(&mut state);
        let low = (standing * 8.0) as i32;
        let high = (6 + (standing * 14.0) as i32).min(20);
        let mut attribute = || {
            let span = (high - low + 1) as f32;
            (low + ((Self::unit(&mut state) * span) as i32).min(high - low)) as u8
        };
        let coaching = StaffCoaching {
            attacking: attribute(),
            defending: attribute(),
            fitness: attribute(),
            mental: attribute(),
            tactical: attribute(),
            technical: attribute(),
            working_with_youngsters: attribute(),
        };
        let mental = StaffMental {
            adaptability: attribute(),
            determination: attribute(),
            discipline: attribute(),
            man_management: attribute(),
            motivating: attribute(),
        };
        let tactical_knowledge = attribute();
        CoachMakeup {
            id: (Self::unit(&mut state) * u32::MAX as f32) as u32,
            style: CoachingStyle::Democratic,
            mental,
            coaching,
            tactical_knowledge,
        }
    }

    /// splitmix64 into `[0, 1)`.
    fn unit(state: &mut u64) -> f32 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoachCard {
    pub temperament: Temperament,
    /// The plan he would play with nothing to answer: his idea of football.
    pub preference: GamePlan,
    pub style: CoachingStyle,
}

impl CoachCard {
    const TEMPERAMENT_KEYS: [&'static str; 8] = [
        "risk", "caution", "stubborn", "adapt", "volatile", "read", "manage", "motivate",
    ];

    /// How far a coach's own taste moves each dial of his preferred plan.
    const TASTE: f32 = 0.12;

    pub const fn neutral() -> Self {
        CoachCard {
            temperament: Temperament::neutral(),
            preference: GamePlan::neutral(),
            style: CoachingStyle::Democratic,
        }
    }

    /// The card of a side's head coach, if it has one.
    pub fn of_head_coach(head_coach: &Staff) -> Option<Self> {
        (head_coach.id != 0).then(|| Self::from_makeup(&CoachMakeup::of(head_coach)))
    }

    /// The coach of a side with no head coach: drawn from the population by
    /// the match seed and the side, never the population's mean.
    pub fn drawn(seed: u64, team_id: u32) -> Self {
        Self::from_makeup(&CoachMakeup::drawn(seed, team_id))
    }

    /// Who manages a side in a match: a pinned coach, else the head coach
    /// the squad brought, else one drawn for this match and side.
    pub fn in_charge(
        pinned: Option<CoachCard>,
        squad: Option<CoachCard>,
        seed: u64,
        team_id: u32,
    ) -> Self {
        pinned
            .or(squad)
            .unwrap_or_else(|| Self::drawn(seed, team_id))
    }

    pub fn from_makeup(makeup: &CoachMakeup) -> Self {
        let decision = CoachProfile::temperament_of(&makeup.mental, makeup.style);
        let norm = |v: u8| v as f32 / 20.0;
        let mental = &makeup.mental;
        let coaching = &makeup.coaching;
        let temperament = Temperament {
            risk_tolerance: decision.risk_tolerance,
            caution: decision.conservatism,
            stubbornness: decision.stubbornness,
            adaptability: norm(mental.adaptability),
            volatility: decision.emotional_volatility,
            read: ((makeup.tactical_knowledge as f32 + coaching.tactical as f32) / 40.0)
                .clamp(0.0, 1.0),
            man_management: norm(mental.man_management),
            motivating: norm(mental.motivating),
        };
        CoachCard {
            temperament,
            preference: Self::preferred_plan(makeup),
            style: makeup.style,
        }
    }

    /// The football he would play with nothing to answer: an attacking
    /// coach defends higher and commits more, a fitness coach presses, a
    /// technical one keeps the ball, a defensive one stays compact, his
    /// style leans each way, and his own taste moves every dial.
    fn preferred_plan(makeup: &CoachMakeup) -> GamePlan {
        let c = &makeup.coaching;
        let centred = |v: u8| v as f32 / 20.0 - 0.5;
        let attack_share = if c.attacking as u16 + c.defending as u16 == 0 {
            0.5
        } else {
            c.attacking as f32 / (c.attacking as f32 + c.defending as f32)
        };
        let lean = (attack_share - 0.5) * 2.0;
        let (fitness, technical, defending, tactical) = (
            centred(c.fitness),
            centred(c.technical),
            centred(c.defending),
            centred(c.tactical),
        );
        // block, press, compact, width, tempo, direct, commit
        let base = [
            0.18 * lean + 0.12 * fitness,
            0.12 * lean + 0.25 * fitness,
            -0.12 * lean + 0.15 * defending + 0.10 * tactical,
            0.12 * lean,
            0.10 * lean + 0.15 * fitness - 0.10 * technical,
            -0.30 * technical - 0.05 * lean,
            0.22 * lean,
        ];
        let style = match makeup.style {
            CoachingStyle::Tactical => [0.0, 0.0, 0.08, 0.0, -0.05, -0.05, 0.0],
            CoachingStyle::Authoritarian => [-0.05, 0.0, 0.08, 0.0, 0.0, 0.08, -0.05],
            CoachingStyle::Transformational => [0.06, 0.06, 0.0, 0.05, 0.0, 0.0, 0.06],
            CoachingStyle::Democratic => [0.0; 7],
            CoachingStyle::LaissezFaire => [0.0, 0.0, -0.08, 0.06, 0.04, 0.0, 0.0],
        };
        let mut dials = [0.5f32; 7];
        for (i, dial) in dials.iter_mut().enumerate() {
            let taste = Self::taste(makeup.id, i as u32);
            *dial = (0.5 + base[i] + style[i] + taste * Self::TASTE).clamp(0.05, 0.95);
        }
        GamePlan::from_dials(dials)
    }

    /// A coach's own lean on one dial, in `[-1, 1]`, fixed by who he is.
    fn taste(id: u32, dial: u32) -> f32 {
        let mut z = (id as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((dial as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03));
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }

    /// A pinned coach, as `OF_COACH` states one: a preset (`pressing`,
    /// `lowblock`, `cautious`, `risktaker`, `stubborn`, `adaptable`), or
    /// temperament keys and plan dials with their values separated by `/`,
    /// starting from a neutral card — `risk=0.9/caution=0.1/press=0.8`.
    pub fn from_pin(spec: &str) -> Result<Self, String> {
        let spec = spec.trim();
        let mut card = Self::neutral();
        let t = &mut card.temperament;
        match spec.to_ascii_lowercase().as_str() {
            "pressing" => {
                card.preference = GamePlan::from_pin("block=0.85/press=0.9/tempo=0.7/commit=0.65")?;
                return Ok(card);
            }
            "lowblock" => {
                card.preference =
                    GamePlan::from_pin("block=0.15/press=0.2/compact=0.85/direct=0.7/commit=0.3")?;
                return Ok(card);
            }
            "cautious" => {
                t.risk_tolerance = 0.15;
                t.caution = 0.9;
                return Ok(card);
            }
            "risktaker" => {
                t.risk_tolerance = 0.9;
                t.caution = 0.15;
                return Ok(card);
            }
            "stubborn" => {
                t.stubbornness = 0.9;
                t.adaptability = 0.15;
                return Ok(card);
            }
            "adaptable" => {
                t.stubbornness = 0.15;
                t.adaptability = 0.9;
                return Ok(card);
            }
            _ => {}
        }
        let mut dials = card.preference.dials();
        for part in spec.split('/').filter(|p| !p.trim().is_empty()) {
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| format!("coach pin `{part}` is not a preset or `name=value`"))?;
            let key = key.trim().to_ascii_lowercase();
            let value: f32 = value
                .trim()
                .parse()
                .map_err(|_| format!("coach pin `{key}` needs a number 0..1"))?;
            if !(0.0..=1.0).contains(&value) {
                return Err(format!("coach pin `{key}` must be within 0..1"));
            }
            if let Some(i) = GamePlan::DIALS.iter().position(|d| *d == key) {
                dials[i] = value;
                continue;
            }
            let t = &mut card.temperament;
            let slot = match key.as_str() {
                "risk" => &mut t.risk_tolerance,
                "caution" => &mut t.caution,
                "stubborn" => &mut t.stubbornness,
                "adapt" => &mut t.adaptability,
                "volatile" => &mut t.volatility,
                "read" => &mut t.read,
                "manage" => &mut t.man_management,
                "motivate" => &mut t.motivating,
                _ => {
                    return Err(format!(
                        "unknown coach key `{key}`; keys are {} and the plan dials {}",
                        Self::TEMPERAMENT_KEYS.join(", "),
                        GamePlan::DIALS.join(", ")
                    ));
                }
            };
            *slot = value;
        }
        card.preference = GamePlan::from_dials(dials);
        Ok(card)
    }

    pub fn describe(&self) -> String {
        let t = &self.temperament;
        format!(
            "risk {:.2} caution {:.2} stubborn {:.2} adapt {:.2} volatile {:.2} read {:.2} handling {:.2} {:?}; prefers {}",
            t.risk_tolerance,
            t.caution,
            t.stubbornness,
            t.adaptability,
            t.volatility,
            t.read,
            t.handling(),
            self.style,
            self.preference.describe()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn makeup(id: u32) -> CoachMakeup {
        CoachMakeup {
            id,
            style: CoachingStyle::Democratic,
            mental: StaffMental {
                adaptability: 10,
                determination: 10,
                discipline: 10,
                man_management: 10,
                motivating: 10,
            },
            coaching: StaffCoaching {
                attacking: 10,
                defending: 10,
                fitness: 10,
                mental: 10,
                tactical: 10,
                technical: 10,
                working_with_youngsters: 10,
            },
            tactical_knowledge: 10,
        }
    }

    #[test]
    fn a_more_conservative_coach_is_more_cautious() {
        let loose = CoachCard::from_makeup(&makeup(1));
        let mut strict = makeup(1);
        strict.mental.discipline = 18;
        let strict = CoachCard::from_makeup(&strict);
        assert!(strict.temperament.caution > loose.temperament.caution);
    }

    #[test]
    fn a_more_adaptable_coach_resists_change_less() {
        let rigid = CoachCard::from_makeup(&makeup(1));
        let mut open = makeup(1);
        open.mental.adaptability = 18;
        let open = CoachCard::from_makeup(&open);
        assert!(open.temperament.resistance_to_change() < rigid.temperament.resistance_to_change());
    }

    #[test]
    fn equal_coaches_differ_in_taste() {
        let a = CoachCard::from_makeup(&makeup(1));
        let b = CoachCard::from_makeup(&makeup(2));
        assert_eq!(a.temperament, b.temperament);
        assert_ne!(a.preference, b.preference);
    }

    #[test]
    fn an_attacking_coach_defends_higher_and_commits_more() {
        let mut attacking = makeup(7);
        attacking.coaching.attacking = 18;
        attacking.coaching.defending = 6;
        let mut defensive = makeup(7);
        defensive.coaching.attacking = 6;
        defensive.coaching.defending = 18;
        let (a, d) = (
            CoachCard::from_makeup(&attacking).preference,
            CoachCard::from_makeup(&defensive).preference,
        );
        assert!(a.block_height > d.block_height);
        assert!(a.commitment > d.commitment);
        assert!(a.compactness < d.compactness);
    }

    #[test]
    fn a_side_without_a_head_coach_is_managed_by_a_drawn_one() {
        let head = CoachCard::from_makeup(&makeup(9));
        assert_eq!(CoachCard::in_charge(None, Some(head), 5, 1), head);
        let drawn = CoachCard::in_charge(None, None, 5, 1);
        assert_eq!(drawn, CoachCard::drawn(5, 1));
        assert_ne!(drawn, CoachCard::neutral());
        let pinned = CoachCard::from_pin("cautious").unwrap();
        assert_eq!(CoachCard::in_charge(Some(pinned), Some(head), 5, 1), pinned);
    }

    #[test]
    fn the_same_match_and_side_draw_the_same_coach() {
        assert_eq!(CoachCard::drawn(42, 1), CoachCard::drawn(42, 1));
        assert_ne!(CoachCard::drawn(42, 1), CoachCard::drawn(43, 1));
        assert_ne!(CoachCard::drawn(42, 1), CoachCard::drawn(42, 2));
    }

    #[test]
    fn drawn_coaches_sit_on_the_generated_population() {
        // The staff generator's head coach: a standing uniform in [0, 1),
        // then each attribute uniform in [8s, 6 + 14s] — sampled here by a
        // plain LCG, independent of the draw's own hash.
        let mut lcg = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            lcg = lcg
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (lcg >> 40) as f32 / (1u64 << 24) as f32
        };
        let mut generated = [0.0f32; 4];
        let mut drawn = [0.0f32; 4];
        let n = 1_000;
        for i in 0..n {
            let standing = next();
            let (low, high) = (
                (standing * 8.0) as i32,
                (6 + (standing * 14.0) as i32).min(20),
            );
            let mut attribute =
                || (low + ((next() * (high - low + 1) as f32) as i32).min(high - low)) as u8;
            let mut m = makeup(i);
            m.mental = StaffMental {
                adaptability: attribute(),
                determination: attribute(),
                discipline: attribute(),
                man_management: attribute(),
                motivating: attribute(),
            };
            m.coaching.tactical = attribute();
            m.tactical_knowledge = attribute();
            let g = CoachCard::from_makeup(&m).temperament;
            let d = CoachCard::drawn(i as u64 * 7919, 1).temperament;
            for (sum, v) in
                generated
                    .iter_mut()
                    .zip([g.risk_tolerance, g.caution, g.volatility, g.read])
            {
                *sum += v / n as f32;
            }
            for (sum, v) in
                drawn
                    .iter_mut()
                    .zip([d.risk_tolerance, d.caution, d.volatility, d.read])
            {
                *sum += v / n as f32;
            }
        }
        for (g, d) in generated.iter().zip(drawn.iter()) {
            assert!(
                (g - d).abs() < 0.03,
                "generated {generated:?} drawn {drawn:?}"
            );
        }
    }
}
