//! Whole-career development simulations.
//!
//! Each career runs the production pipeline one day at a time — match
//! load, overnight recovery, the club's daily training sessions, the
//! off-season camp and the weekly development tick — under a seeded roll
//! stream, so a career is a pure function of its inputs. Balance is judged
//! against the world the game loads: where the shipped database's players
//! of each age stand relative to their potential, and the age curve the
//! world generator builds veterans on.

use super::ceilings::PositionalSkillCeilings;
use super::coaching::CoachingEffect;
use super::rolls::RollSource;
use super::skills_array::SkillKey;
use super::skills_array::{
    SK_GK_AERIAL_REACH, SK_GK_THROWING, SK_MATCH_READINESS, SKILL_COUNT, skills_to_array,
    write_array_into,
};

use crate::club::player::builder::PlayerBuilder;
use crate::club::player::maturation::{MaturationGroup, SkillMaturation};
use crate::club::player::player::Player;
use crate::club::team::TeamFixtureWindow;
use crate::shared::fullname::FullName;
use crate::utils::DateUtils;
use crate::{
    CoachingPhilosophy, IndividualTrainingPlan, PeriodizationPhase, PersonAttributes,
    PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
    PlayerStatistics, PlayerTraining, PositionWeights, RotationPreference, SkillType, Staff,
    StaffStub, TacticalFocus, TrainingFocus, TrainingIntensityPreference, TrainingSession,
    TrainingType, WeeklyTrainingPlan,
};
use chrono::{Datelike, Duration, NaiveDate, Weekday};

/// SplitMix64: every career owns one stream, so careers are reproducible
/// and independent of test order.
struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn normal(&mut self) -> f32 {
        let u1 = self.unit().max(1e-7);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
    }
}

impl RollSource for Stream {
    fn roll_unit(&mut self) -> f32 {
        self.unit()
    }
}

/// CA/PA percentiles of the shipped database's records with a stated
/// potential of 110 or more, by age at the 2026 world start (36,062
/// records, keepers included).
struct RealWorld;

impl RealWorld {
    /// `(age, p10, p25, median, p90)`.
    const PERCENTILES: [(u8, f32, f32, f32, f32); 10] = [
        (21, 0.705, 0.743, 0.794, 0.857),
        (22, 0.724, 0.763, 0.803, 0.875),
        (23, 0.764, 0.797, 0.839, 0.898),
        (24, 0.776, 0.828, 0.873, 0.924),
        (25, 0.793, 0.847, 0.897, 0.951),
        (26, 0.810, 0.864, 0.914, 0.967),
        (27, 0.817, 0.873, 0.919, 0.982),
        (28, 0.818, 0.879, 0.926, 0.984),
        (29, 0.828, 0.881, 0.926, 0.984),
        (30, 0.814, 0.873, 0.922, 0.983),
    ];

    fn at(age: u8) -> (f32, f32, f32, f32) {
        let row = Self::PERCENTILES
            .iter()
            .find(|row| row.0 == age)
            .expect("age outside the reference table");
        (row.1, row.2, row.3, row.4)
    }
}

/// The club a player spends his career at.
#[derive(Clone, Copy, Debug)]
struct Club {
    league_reputation: u16,
    club_reputation: f32,
    coaching: u8,
    youth_coaching: f32,
    facilities: f32,
}

impl Club {
    const ELITE: Club = Club {
        league_reputation: 8800,
        club_reputation: 0.90,
        coaching: 17,
        youth_coaching: 0.85,
        facilities: 0.85,
    };
    const AVERAGE: Club = Club {
        league_reputation: 5500,
        club_reputation: 0.50,
        coaching: 11,
        youth_coaching: 0.50,
        facilities: 0.50,
    };
    const MINNOW: Club = Club {
        league_reputation: 2500,
        club_reputation: 0.20,
        coaching: 6,
        youth_coaching: 0.20,
        facilities: 0.20,
    };

    fn coach(&self) -> Staff {
        let level = self.coaching;
        let mut staff = StaffStub::build();
        staff.id = 900;
        let c = &mut staff.staff_attributes.coaching;
        c.attacking = level;
        c.defending = level;
        c.fitness = level;
        c.mental = level;
        c.tactical = level;
        c.technical = level;
        c.working_with_youngsters = level;
        let g = &mut staff.staff_attributes.goalkeeping;
        g.distribution = level;
        g.handling = level;
        g.shot_stopping = level;
        staff.staff_attributes.mental.determination = level;
        staff
    }

    fn coaching_effect(&self) -> CoachingEffect {
        let c = self.coaching;
        CoachingEffect::from_scores(c, c, c, c, self.youth_coaching)
    }
}

/// How much football a player gets.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    /// Starts every senior match.
    Starter,
    /// Starts every match for the club's youth side, whose league the game
    /// runs as friendlies.
    YouthRegular,
    /// Trains with the seniors and never plays.
    Unused,
}

impl Role {
    /// Minutes on a fixture date and whether they are a friendly's.
    fn appearance(self) -> Option<(f32, bool)> {
        match self {
            Role::Starter => Some((90.0, false)),
            Role::YouthRegular => Some((90.0, true)),
            Role::Unused => None,
        }
    }
}

/// A European season: Saturday league fixtures from mid-August to late
/// May, friendlies on July Saturdays.
struct Calendar;

impl Calendar {
    /// `Some(is_friendly)` on match days.
    fn fixture(date: NaiveDate) -> Option<bool> {
        if date.weekday() != Weekday::Sat {
            return None;
        }
        let (m, d) = (date.month(), date.day());
        let in_season = (m == 8 && d >= 16) || m >= 9 || m <= 4 || (m == 5 && d <= 25);
        if in_season {
            Some(false)
        } else if m == 7 && d >= 10 {
            Some(true)
        } else {
            None
        }
    }

    fn is_official(date: NaiveDate) -> bool {
        Self::fixture(date) == Some(false)
    }

    fn is_off_season(date: NaiveDate) -> bool {
        let (m, d) = (date.month(), date.day());
        (m == 5 && d > 25) || m == 6 || m == 7 || (m == 8 && d < 16)
    }

    /// The fixture window the country pipeline writes before the clubs
    /// train: the league has played, so today's match is already recent.
    /// The youth side is taken to train around the same Saturdays.
    fn window(today: NaiveDate) -> TeamFixtureWindow {
        let dates = |days: &mut dyn Iterator<Item = i64>| -> Vec<NaiveDate> {
            days.map(|off| today + Duration::days(off))
                .filter(|d| Self::is_official(*d))
                .take(4)
                .collect()
        };
        TeamFixtureWindow {
            refreshed: Some(today),
            upcoming: dates(&mut (1..=35)),
            recent: dates(&mut (0..=35).map(|back| -back)),
        }
    }

    /// `TeamTraining::determine_phase`.
    fn phase(date: NaiveDate) -> PeriodizationPhase {
        match date.month() {
            6 | 7 => PeriodizationPhase::PreSeason,
            8 | 9 => PeriodizationPhase::EarlySeason,
            3 | 4 => PeriodizationPhase::LateSeason,
            5 => PeriodizationPhase::OffSeason,
            _ => PeriodizationPhase::MidSeason,
        }
    }
}

/// Generator-shaped starting profiles: the generation weights give the
/// role's shape, `SkillMaturation` the age's share of each attribute, and
/// the whole profile is then fitted to the requested CA.
struct Prospect;

impl Prospect {
    fn skills(
        position: PlayerPositionType,
        age: u32,
        pa: u8,
        ca: u8,
        character: f32,
        stream: &mut Stream,
    ) -> PlayerSkills {
        let level = PlayerSkills::ability_skill_level(pa);
        let spread = (level * 0.5).max(2.5);
        let shape = PositionWeights::for_position(position);
        let mut arr = [0.0f32; SKILL_COUNT];
        for (i, value) in arr.iter_mut().enumerate().take(SK_MATCH_READINESS) {
            let maturity = SkillMaturation::ratio(
                age as f32 + 0.5,
                PositionalSkillCeilings::maturation_group(i),
            );
            let mean = level * maturity + (shape[i] - 1.0) * spread;
            *value = (mean + stream.normal() * 1.2).clamp(1.0, 20.0);
        }
        arr[SK_MATCH_READINESS] = 14.0;
        if position == PlayerPositionType::Goalkeeper {
            let maturity = SkillMaturation::ratio(age as f32 + 0.5, MaturationGroup::Goalkeeping);
            for value in &mut arr[SK_GK_AERIAL_REACH..=SK_GK_THROWING] {
                *value = (level * maturity + stream.normal() * 1.2).clamp(1.0, 20.0);
            }
        }
        let mut skills = PlayerSkills::default();
        write_array_into(&mut skills, &arr);
        skills.mental.determination = character;
        skills.mental.work_rate = character;
        skills.fit_to_ability(position, ca, 20.0, None);
        skills
    }

    /// A player of `age` on `start`. `character` is his professionalism,
    /// ambition, determination and work rate alike.
    fn build(
        id: u32,
        position: PlayerPositionType,
        start: NaiveDate,
        age: u32,
        pa: u8,
        ca: u8,
        character: f32,
    ) -> Player {
        let mut stream = Stream(0x5EED ^ (id as u64) << 20 ^ (pa as u64) << 8 ^ age as u64);
        let skills = Self::skills(position, age, pa, ca, character, &mut stream);
        let birth = NaiveDate::from_ymd_opt(start.year() - age as i32, 1, 15).unwrap();
        let attributes = PlayerAttributes {
            potential_ability: pa,
            current_ability: skills.calculate_ability_for_position(position),
            condition: 9500,
            fitness: 8500,
            injury_proneness: 8,
            ..Default::default()
        };
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Career".to_string(), format!("{id}")))
            .birth_date(birth)
            .country_id(1)
            .attributes(PersonAttributes {
                professionalism: character,
                ambition: character,
                ..PersonAttributes::default()
            })
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 20,
                }],
            })
            .player_attributes(attributes)
            .build()
            .unwrap()
    }
}

/// CA/PA of a profile sitting exactly on every ceiling the player may
/// reach at `age` — the most development allows him to hold.
struct Envelope;

impl Envelope {
    fn at(player: &Player, age: f32) -> f32 {
        let ceilings = PositionalSkillCeilings::for_player(player, age);
        let mut arr = skills_to_array(player);
        for (i, v) in arr.iter_mut().enumerate() {
            if i != SK_MATCH_READINESS && *v > 0.0 {
                *v = ceilings.at(i);
            }
        }
        let mut skills = player.skills;
        write_array_into(&mut skills, &arr);
        skills.calculate_ability_for_position(player.position()) as f32
            / player.player_attributes.potential_ability as f32
    }
}

/// A player's state at the close of a season (1 July).
#[derive(Clone, Copy, Debug)]
struct SeasonMark {
    age: u8,
    ca: u8,
    skill_ca: u8,
    pa: u8,
    lowest: f32,
    highest: f32,
}

impl SeasonMark {
    fn ratio(&self) -> f32 {
        self.ca as f32 / self.pa as f32
    }
}

struct Career {
    player: Player,
    club: Club,
    role: Role,
    coach: Staff,
    rolls: Stream,
    date: NaiveDate,
    marks: Vec<SeasonMark>,
}

impl Career {
    const RATING: f32 = 6.9;

    fn new(player: Player, club: Club, role: Role, start: NaiveDate) -> Self {
        let seed = player.id as u64 * 7919;
        Career {
            player,
            club,
            role,
            coach: club.coach(),
            rolls: Stream(seed),
            date: start,
            marks: Vec::new(),
        }
    }

    fn run_seasons(mut self, seasons: i32) -> Self {
        let end = NaiveDate::from_ymd_opt(self.date.year() + seasons, 7, 2).unwrap();
        while self.date < end {
            self.day();
            self.date = self.date.succ_opt().unwrap();
        }
        self
    }

    fn mark_at(&self, age: u8) -> SeasonMark {
        *self
            .marks
            .iter()
            .find(|m| m.age == age)
            .expect("career did not reach that age")
    }

    fn peak(&self) -> SeasonMark {
        *self.marks.iter().max_by_key(|m| m.ca).unwrap()
    }

    /// One simulated day, in the order the world runs it: the player's
    /// daily processing and the weekly tick, the club's sessions, the
    /// match result's exertion, then the country's off-season camp.
    fn day(&mut self) {
        let today = self.date;
        if today.month() == 7 && today.day() == 1 {
            self.close_season();
        }

        let form = self.player.training.form_recovery_baseline();
        self.player.load.daily_decay_with_form_target(today, form);
        self.player.process_condition_recovery(today);
        self.player.process_match_readiness_decay(today);
        if today.weekday() == Weekday::Mon {
            self.player.process_development_with(
                today,
                self.club.league_reputation,
                &self.club.coaching_effect(),
                self.club.club_reputation,
                &mut self.rolls,
            );
        }

        self.train(today);

        let appearance = match Calendar::fixture(today) {
            Some(true) => Some((45.0, true)),
            Some(false) => self.role.appearance(),
            None => None,
        };
        if let Some((minutes, friendly)) = appearance {
            self.player
                .on_match_exertion_minutes_only(minutes, today, friendly);
            self.record_appearance(minutes, friendly);
            self.heal();
        }

        if Calendar::is_off_season(today) {
            self.player.on_offseason_camp_day(self.club.facilities);
        }
    }

    fn train(&mut self, today: NaiveDate) {
        let philosophy = CoachingPhilosophy {
            tactical_focus: TacticalFocus::Balanced,
            training_intensity: TrainingIntensityPreference::Medium,
            youth_focus: false,
            rotation_preference: RotationPreference::Moderate,
        };
        let window = Calendar::window(today);
        let plan = WeeklyTrainingPlan::generate_for_date(
            today,
            window.previous_before(today),
            window.next_after(today),
            window.fixtures_this_week(today),
            Calendar::phase(today),
            &philosophy,
        );
        let Some(sessions) = plan.sessions.get(&today.weekday()) else {
            return;
        };
        let at = today.and_hms_opt(0, 0, 0).unwrap();
        for session in sessions {
            if !Self::participates(&self.player, session) {
                continue;
            }
            PlayerTraining::train(&self.player, &self.coach, session, at, self.club.facilities)
                .apply_to_player(&mut self.player, today);
            self.heal();
        }
    }

    /// `TeamTraining::select_participants` for a fit squad member.
    fn participates(player: &Player, session: &TrainingSession) -> bool {
        if player.player_attributes.condition_percentage() <= 30 {
            return false;
        }
        if player.player_attributes.is_in_recovery()
            && !matches!(
                session.session_type,
                TrainingType::Recovery
                    | TrainingType::LightRecovery
                    | TrainingType::Rehabilitation
                    | TrainingType::RestDay
                    | TrainingType::VideoAnalysis
                    | TrainingType::Positioning
            )
        {
            return false;
        }
        session.focus_positions.is_empty()
            || session
                .focus_positions
                .iter()
                .any(|p| player.positions.has_position(*p))
    }

    /// Careers measure development, not medical luck: the match and
    /// training injury rolls use the thread RNG, so an injury heals on
    /// the spot.
    fn heal(&mut self) {
        let a = &mut self.player.player_attributes;
        a.is_injured = false;
        a.injury_days_remaining = 0;
        a.recovery_days_remaining = 0;
        a.injury_type = None;
    }

    fn record_appearance(&mut self, minutes: f32, friendly: bool) {
        let stats = if friendly {
            &mut self.player.friendly_statistics
        } else {
            &mut self.player.statistics
        };
        if minutes >= 45.0 {
            stats.played += 1;
        } else {
            stats.played_subs += 1;
        }
        let weight = (minutes / 90.0).clamp(0.2, 1.0);
        stats.rating_points += Self::RATING * weight;
        stats.rating_weight += weight;
        stats.average_rating = stats.rating_points / stats.rating_weight;
        stats.rating_sum += Self::RATING;
        stats.rating_matches += 1;
    }

    fn close_season(&mut self) {
        let skills = skills_to_array(&self.player);
        let attributes = skills
            .iter()
            .enumerate()
            .filter(|(i, v)| *i != SK_MATCH_READINESS && **v > 0.0)
            .map(|(_, v)| *v);
        self.marks.push(SeasonMark {
            age: DateUtils::age(self.player.birth_date, self.date),
            ca: self.player.player_attributes.current_ability,
            skill_ca: self
                .player
                .skills
                .calculate_ability_for_position(self.player.position()),
            pa: self.player.player_attributes.potential_ability,
            lowest: attributes.clone().fold(f32::MAX, f32::min),
            highest: attributes.fold(f32::MIN, f32::max),
        });
        self.player.statistics = PlayerStatistics::default();
        self.player.cup_statistics = PlayerStatistics::default();
        self.player.friendly_statistics = PlayerStatistics::default();
    }
}

/// A regular's 30-day window: a 90-minute start every week.
const REGULAR_MONTH: f32 = 380.0;

const OUTFIELD: [PlayerPositionType; 7] = [
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefensiveMidfielder,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::AttackingMidfielderRight,
    PlayerPositionType::AttackingMidfielderCenter,
    PlayerPositionType::Striker,
];

/// One career per role, all under the same conditions.
struct Cohort {
    careers: Vec<Career>,
}

impl Cohort {
    fn start() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()
    }

    fn every_role() -> Vec<PlayerPositionType> {
        let mut positions = vec![PlayerPositionType::Goalkeeper];
        positions.extend(OUTFIELD);
        positions
    }

    /// Seventeen-year-olds at the 0.47 of potential the database's
    /// seventeen-year-olds hold, developed for `seasons` seasons.
    fn from_seventeen(
        pa: u8,
        club: Club,
        role: Role,
        character: f32,
        positions: &[PlayerPositionType],
        seasons: i32,
    ) -> Self {
        let ca = (pa as f32 * 0.47).round() as u8;
        Self::from_age(17, pa, ca, club, role, character, positions, seasons)
    }

    fn from_age(
        age: u32,
        pa: u8,
        ca: u8,
        club: Club,
        role: Role,
        character: f32,
        positions: &[PlayerPositionType],
        seasons: i32,
    ) -> Self {
        let start = Self::start();
        let careers = positions
            .iter()
            .enumerate()
            .map(|(k, position)| {
                let id = 100 + k as u32 + (character as u32) * 10 + age * 1000;
                let player = Prospect::build(id, *position, start, age, pa, ca, character);
                Career::new(player, club, role, start).run_seasons(seasons)
            })
            .collect();
        Cohort { careers }
    }

    fn mean_ratio(&self, age: u8) -> f32 {
        self.careers
            .iter()
            .map(|c| c.mark_at(age).ratio())
            .sum::<f32>()
            / self.careers.len() as f32
    }
}

// ── Invariants ─────────────────────────────────────────────────────────

#[test]
fn twenty_seasons_keep_ability_within_potential_and_attributes_on_the_scale() {
    let positions = Cohort::every_role();
    for (pa, club, role, character) in [
        (190u8, Club::ELITE, Role::Starter, 20.0f32),
        (120, Club::AVERAGE, Role::YouthRegular, 12.0),
        (80, Club::MINNOW, Role::Unused, 2.0),
    ] {
        let cohort = Cohort::from_seventeen(pa, club, role, character, &positions, 20);
        for career in &cohort.careers {
            let position = career.player.position();
            assert_eq!(career.marks.len(), 21, "{position:?}: one mark per season");
            for mark in &career.marks {
                assert_eq!(mark.pa, pa, "{position:?} at {}: potential moved", mark.age);
                assert!(
                    mark.ca <= mark.pa,
                    "{position:?} at {}: CA {} above PA {}",
                    mark.age,
                    mark.ca,
                    mark.pa
                );
                assert!(
                    mark.skill_ca <= mark.pa,
                    "{position:?} at {}: the attributes score {} past PA {}",
                    mark.age,
                    mark.skill_ca,
                    mark.pa
                );
                assert!(
                    mark.lowest >= 1.0 && mark.highest <= 20.0,
                    "{position:?} at {}: attributes left the 1..20 scale ({}..{})",
                    mark.age,
                    mark.lowest,
                    mark.highest
                );
            }
        }
    }
}

#[test]
fn a_career_is_a_pure_function_of_its_inputs() {
    let run = || {
        let player = Prospect::build(
            7,
            PlayerPositionType::MidfielderCenter,
            Cohort::start(),
            18,
            160,
            80,
            13.0,
        );
        Career::new(player, Club::AVERAGE, Role::Starter, Cohort::start()).run_seasons(4)
    };
    let (a, b) = (run(), run());
    assert_eq!(skills_to_array(&a.player), skills_to_array(&b.player));
    assert_eq!(
        a.player.player_attributes.current_ability,
        b.player.player_attributes.current_ability
    );
}

// ── The world the game loads ───────────────────────────────────────────

/// A regular starter of median character at a mid-table club ends every
/// season from 21 to 29 inside the real spread of players his age: above
/// the database's lower quartile, below its top decile.
#[test]
fn a_typical_career_tracks_the_real_potential_curve() {
    let cohort = Cohort::from_seventeen(
        150,
        Club::AVERAGE,
        Role::Starter,
        12.0,
        &Cohort::every_role(),
        13,
    );
    let mut previous = 0.0;
    for age in 21..=29u8 {
        let ratio = cohort.mean_ratio(age);
        let (_, p25, _, p90) = RealWorld::at(age);
        assert!(
            ratio >= p25 && ratio <= p90,
            "at {age} a typical career holds {ratio:.3} of its potential; \
             real players of that age sit {p25:.3}..{p90:.3} (p25..p90)"
        );
        assert!(
            ratio >= previous,
            "a typical career went backwards at {age}: {previous:.3} -> {ratio:.3}"
        );
        previous = ratio;
    }
}

/// Growth slows season by season: no season gains more than the one
/// before it, as it would when a ceiling holds a player flat for years
/// and then releases him all at once.
#[test]
fn growth_slows_without_stalling() {
    let cohort = Cohort::from_seventeen(
        150,
        Club::AVERAGE,
        Role::Starter,
        12.0,
        &Cohort::every_role(),
        12,
    );
    let gain = |age: u8| cohort.mean_ratio(age) - cohort.mean_ratio(age - 1);
    for age in 19..=28u8 {
        let (before, now) = (gain(age - 1), gain(age));
        assert!(
            now > 0.0 && now <= before + 0.005,
            "the season to {age} gained {now:.3} after {before:.3}"
        );
    }
}

/// The best environment and character reach the top of the real spread;
/// the worst still develop like the database's weakest tenth, not below.
#[test]
fn environment_and_character_span_the_real_spread() {
    let positions = [
        PlayerPositionType::DefenderCenter,
        PlayerPositionType::MidfielderCenter,
        PlayerPositionType::Striker,
    ];
    let best = Cohort::from_seventeen(150, Club::ELITE, Role::Starter, 18.0, &positions, 10);
    let typical = Cohort::from_seventeen(150, Club::AVERAGE, Role::Starter, 12.0, &positions, 10);
    let worst = Cohort::from_seventeen(150, Club::MINNOW, Role::Starter, 6.0, &positions, 10);
    for age in [23u8, 25, 27] {
        let (p10, _, median, _) = RealWorld::at(age);
        let (b, t, w) = (
            best.mean_ratio(age),
            typical.mean_ratio(age),
            worst.mean_ratio(age),
        );
        assert!(
            b > t && t > w,
            "at {age}: best {b:.3}, typical {t:.3}, worst {w:.3}"
        );
        assert!(
            b >= median,
            "at {age} the best environment holds {b:.3}; the real median is {median:.3}"
        );
        assert!(
            w >= p10,
            "at {age} the worst environment holds {w:.3}, below the real p10 {p10:.3}"
        );
        assert!(
            b - w >= 0.06,
            "at {age} environment and character barely matter: {b:.3} vs {w:.3}"
        );
    }
}

/// Football builds what training alone cannot: from 21 a regular starter
/// holds clearly more of his potential than a squad player who only
/// trains, and a regular of the youth side sits between them.
#[test]
fn football_builds_what_training_alone_cannot() {
    let positions = [
        PlayerPositionType::DefenderCenter,
        PlayerPositionType::MidfielderCenter,
        PlayerPositionType::Striker,
    ];
    let cohort = |role| Cohort::from_seventeen(150, Club::AVERAGE, role, 12.0, &positions, 11);
    let (starter, youth, unused) = (
        cohort(Role::Starter),
        cohort(Role::YouthRegular),
        cohort(Role::Unused),
    );
    for age in [21u8, 24, 27] {
        let (s, y, u) = (
            starter.mean_ratio(age),
            youth.mean_ratio(age),
            unused.mean_ratio(age),
        );
        assert!(
            s > y && y > u,
            "at {age}: starter {s:.3}, youth regular {y:.3}, unused {u:.3}"
        );
        assert!(
            (0.04..=0.12).contains(&(s - u)),
            "at {age} minutes are worth {:.3} of potential (starter {s:.3}, unused {u:.3})",
            s - u
        );
    }
}

/// A regular who loses his place keeps what his football built: the
/// ceiling stops his growth and never takes points back.
#[test]
fn a_regular_who_loses_his_place_keeps_what_he_built() {
    let start = Cohort::start();
    let player = Prospect::build(
        77,
        PlayerPositionType::MidfielderCenter,
        start,
        17,
        150,
        70,
        12.0,
    );
    let mut career = Career::new(player, Club::AVERAGE, Role::Starter, start).run_seasons(8);
    let established = career.mark_at(25).ca;
    career.role = Role::Unused;
    let career = career.run_seasons(2);
    let benched = career.mark_at(27).ca;
    assert!(
        benched >= established,
        "a benched 25-year-old went from CA {established} to {benched}"
    );
}

/// Every outfield role develops toward its potential alike: the CA
/// weights price roles differently, and the ceilings are sized per role.
#[test]
fn every_outfield_role_reaches_its_potential_alike() {
    for pa in [120u8, 180] {
        let cohort = Cohort::from_seventeen(pa, Club::AVERAGE, Role::Starter, 12.0, &OUTFIELD, 11);
        let ratios: Vec<(PlayerPositionType, f32)> = cohort
            .careers
            .iter()
            .map(|c| (c.player.position(), c.mark_at(27).ratio()))
            .collect();
        let low = ratios.iter().map(|r| r.1).fold(f32::MAX, f32::min);
        let high = ratios.iter().map(|r| r.1).fold(f32::MIN, f32::max);
        assert!(
            low >= 0.88 && high - low <= 0.08,
            "PA {pa} roles at 27 spread {low:.3}..{high:.3}: {ratios:?}"
        );
    }
}

/// The maturation ceiling is the most a player of an age can hold. Read
/// as a share of ability it sits within a few hundredths of the real top
/// decile, so the database's best young players are not frozen above it
/// on day one.
#[test]
fn the_age_ceiling_holds_the_real_top_decile() {
    let start = Cohort::start();
    for pa in [120u8, 150, 180] {
        for position in Cohort::every_role() {
            let mut player = Prospect::build(1, position, start, 22, pa, 60, 12.0);
            player.load.minutes_last_30 = REGULAR_MONTH;
            // The database's players "of an age" are half a year into it.
            let envelope = |age: u8| Envelope::at(&player, age as f32 + 0.5);
            // Keepers mature latest: their ceiling trails an outfielder's
            // by the goalkeeping row of the maturation table.
            let slack = if position == PlayerPositionType::Goalkeeper {
                0.07
            } else {
                0.04
            };
            for age in [21u8, 24, 27] {
                let (_, _, median, p90) = RealWorld::at(age);
                let held = envelope(age);
                assert!(
                    held >= p90 - slack && held >= median,
                    "{position:?} PA {pa} may hold {held:.3} at {age}; the real p90 is {p90:.3}"
                );
            }
            let teenager = envelope(17);
            assert!(
                teenager <= 0.78,
                "{position:?} PA {pa}: a seventeen-year-old may already hold {teenager:.3}"
            );
        }
    }
}

/// The share of each age that only matches build: a player without
/// football may reach less, by the weight those families carry in CA —
/// most for a keeper, whose CA is nearly all goalkeeping and mental.
#[test]
fn football_opens_the_last_share_of_each_age() {
    let start = Cohort::start();
    for position in Cohort::every_role() {
        let mut player = Prospect::build(1, position, start, 22, 150, 60, 12.0);
        for age in [20.5f32, 24.5, 28.5] {
            player.load.minutes_last_30 = REGULAR_MONTH;
            let regular = Envelope::at(&player, age);
            player.load.minutes_last_30 = 0.0;
            let unplayed = Envelope::at(&player, age);
            assert!(
                (0.03..=0.12).contains(&(regular - unplayed)),
                "{position:?} at {age}: a regular may hold {regular:.3}, \
                 a player without football {unplayed:.3}"
            );
        }
    }
}

/// A young player already at the real top decile of his age keeps
/// developing: the ceilings gate each attribute, not the whole player.
#[test]
fn a_prospect_ahead_of_his_age_keeps_developing() {
    let positions = [
        PlayerPositionType::DefenderCenter,
        PlayerPositionType::MidfielderCenter,
        PlayerPositionType::AttackingMidfielderRight,
        PlayerPositionType::Striker,
    ];
    let (_, _, _, p90) = RealWorld::at(21);
    let ca = (160.0 * p90).round() as u8;
    let cohort = Cohort::from_age(
        21,
        160,
        ca,
        Club::AVERAGE,
        Role::Starter,
        12.0,
        &positions,
        3,
    );
    for career in &cohort.careers {
        let (start, after) = (career.mark_at(21).ratio(), career.mark_at(24).ratio());
        assert!(
            after - start >= 0.04,
            "{:?} began at {start:.3} of his potential and held {after:.3} three seasons later",
            career.player.position()
        );
    }
}

/// A specialty plan's extra reps polish the skill it names, on the
/// training ground and under the same ceiling as everything else.
#[test]
fn a_specialty_plan_polishes_its_skill_within_the_ceiling() {
    let start = Cohort::start();
    let season = |plan: bool| {
        let mut player = Prospect::build(
            55,
            PlayerPositionType::DefenderCenter,
            start,
            19,
            150,
            90,
            12.0,
        );
        player.skills.technical.crossing = 6.0;
        if plan {
            player.individual_training = Some(IndividualTrainingPlan {
                player_id: player.id,
                focus_areas: vec![TrainingFocus::SpecificSkill(SkillType::Crossing)],
                intensity_modifier: 1.0,
                special_instructions: Vec::new(),
                started: Some(start),
            });
        }
        Career::new(player, Club::AVERAGE, Role::Starter, start).run_seasons(1)
    };
    let (with, without) = (season(true), season(false));
    let (planned, plain) = (
        with.player.skills.technical.crossing,
        without.player.skills.technical.crossing,
    );
    assert!(
        planned > plain + 0.1,
        "a season of crossing work ended at {planned:.2}, without it {plain:.2}"
    );
    // The ceiling he trained under: a regular's, at the season's end. In
    // the summer his own reads lower, and ceilings never take back.
    let mut regular = with.player.clone();
    regular.load.minutes_last_30 = REGULAR_MONTH;
    let ceiling = PositionalSkillCeilings::for_player(
        &regular,
        DateUtils::age_in_years(regular.birth_date, with.date),
    )
    .get(SkillKey::Crossing);
    assert!(
        planned <= ceiling + 1e-3,
        "the plan took crossing to {planned:.2}, past its ceiling {ceiling:.2}"
    );
}

// ── Peak and decline ───────────────────────────────────────────────────

/// Outfielders peak in their late twenties, keepers later.
#[test]
fn careers_peak_in_the_late_twenties() {
    let cohort = Cohort::from_seventeen(
        150,
        Club::AVERAGE,
        Role::Starter,
        12.0,
        &Cohort::every_role(),
        19,
    );
    for career in &cohort.careers {
        let peak = career.peak().age;
        let window = if career.player.position() == PlayerPositionType::Goalkeeper {
            27..=34
        } else {
            26..=31
        };
        assert!(
            window.contains(&peak),
            "{:?} peaked at {peak}, outside {window:?}",
            career.player.position()
        );
    }
}

/// Veterans decline along the age curve the world generator builds its
/// veterans on (CA as a share of peak: 0.97 at 31, 0.90 at 33, 0.80 at
/// 35); a steeper or flatter decline moves every season's world further
/// from the one it started as.
#[test]
fn veterans_decline_along_the_world_generators_age_curve() {
    let cohort = Cohort::from_seventeen(150, Club::AVERAGE, Role::Starter, 12.0, &OUTFIELD, 19);
    let share_of_peak = |age: u8| {
        cohort
            .careers
            .iter()
            .map(|c| c.mark_at(age).ca as f32 / c.peak().ca as f32)
            .sum::<f32>()
            / cohort.careers.len() as f32
    };
    for (age, generated, tolerance) in
        [(31u8, 0.97f32, 0.04f32), (33, 0.90, 0.05), (35, 0.80, 0.06)]
    {
        let held = share_of_peak(age);
        assert!(
            (held - generated).abs() <= tolerance,
            "at {age} veterans hold {held:.3} of their peak; the generator builds them at {generated}"
        );
    }
}

/// Keepers' careers run longest.
#[test]
fn keepers_outlast_outfielders() {
    let keeper = Cohort::from_seventeen(
        150,
        Club::AVERAGE,
        Role::Starter,
        12.0,
        &[PlayerPositionType::Goalkeeper],
        19,
    );
    let outfield = Cohort::from_seventeen(150, Club::AVERAGE, Role::Starter, 12.0, &OUTFIELD, 19);
    let (k, o) = (keeper.mean_ratio(35), outfield.mean_ratio(35));
    assert!(
        k > o + 0.05,
        "at 35 the keeper holds {k:.3}, outfielders {o:.3}"
    );
}

/// Professionalism is what keeps a veteran going.
#[test]
fn professionals_age_better() {
    let positions = [
        PlayerPositionType::DefenderCenter,
        PlayerPositionType::Striker,
    ];
    let pro = Cohort::from_age(
        29,
        150,
        140,
        Club::AVERAGE,
        Role::Starter,
        18.0,
        &positions,
        6,
    );
    let lax = Cohort::from_age(
        29,
        150,
        140,
        Club::AVERAGE,
        Role::Starter,
        6.0,
        &positions,
        6,
    );
    for age in [32u8, 35] {
        let (p, l) = (pro.mean_ratio(age), lax.mean_ratio(age));
        assert!(
            p > l + 0.02,
            "at {age} the professional holds {p:.3}, the lax veteran {l:.3}"
        );
    }
}

// ── Off-season ─────────────────────────────────────────────────────────

/// A summer of camps sharpens a player and grows nothing: attributes are
/// the development tick's and the training ground's business.
#[test]
fn an_offseason_camp_sharpens_without_growing_attributes() {
    let mut player = Prospect::build(
        9,
        PlayerPositionType::MidfielderCenter,
        Cohort::start(),
        24,
        150,
        120,
        12.0,
    );
    player.skills.physical.match_readiness = 5.0;
    let before = skills_to_array(&player);
    for _ in 0..80 {
        player.on_offseason_camp_day(0.8);
    }
    let after = skills_to_array(&player);
    for i in 0..SKILL_COUNT {
        if i == SK_MATCH_READINESS {
            assert!(after[i] > before[i], "the camp rebuilt no sharpness");
        } else {
            assert_eq!(after[i], before[i], "a camp moved attribute {i}");
        }
    }
}

/// Diagnostic: CA/PA by age across clubs, minutes and character.
///
/// `cargo test -p core --lib career_table -- --ignored --nocapture`
#[test]
#[ignore]
fn career_table() {
    let positions = Cohort::every_role();
    let ages: Vec<u8> = (18..=36).step_by(2).collect();
    println!(
        "{:>8} {:>12} {:>4} {}",
        "club",
        "role",
        "chr",
        ages.iter().map(|a| format!("{a:>6}")).collect::<String>()
    );
    for (label, club) in [
        ("elite", Club::ELITE),
        ("average", Club::AVERAGE),
        ("minnow", Club::MINNOW),
    ] {
        for role in [Role::Starter, Role::YouthRegular, Role::Unused] {
            for character in [6.0f32, 12.0, 18.0] {
                let cohort = Cohort::from_seventeen(150, club, role, character, &positions, 19);
                let row: String = ages
                    .iter()
                    .map(|a| format!("{:>6.2}", cohort.mean_ratio(*a)))
                    .collect();
                println!(
                    "{label:>8} {:>12} {character:>4} {row}",
                    format!("{role:?}")
                );
            }
        }
    }
}
