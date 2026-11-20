use super::ceilings::PositionalSkillCeilings;
use super::skills_array::*;
use super::work::SessionWork;

use crate::club::player::builder::PlayerBuilder;
use crate::club::player::maturation::MaturationGroup;
use crate::club::player::player::Player;
use crate::club::player::position::{PlayerPosition, PlayerPositions};
use crate::shared::fullname::FullName;
use crate::{PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills};
use chrono::Duration;
use chrono::NaiveDate;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn person_pro(prof: f32, ambition: f32) -> PersonAttributes {
    PersonAttributes {
        professionalism: prof,
        ambition,
        ..PersonAttributes::default()
    }
}

fn baseline_skills() -> PlayerSkills {
    // Mid-range outfield baseline — meaningfully below the per-skill
    // ceiling of a high-PA player so growth has room to happen.
    let mut s = PlayerSkills::default();
    s.technical.passing = 10.0;
    s.technical.first_touch = 10.0;
    s.technical.dribbling = 10.0;
    s.technical.finishing = 10.0;
    s.technical.tackling = 10.0;
    s.technical.marking = 10.0;
    s.technical.heading = 10.0;
    s.technical.technique = 10.0;
    s.technical.crossing = 10.0;
    s.technical.long_shots = 10.0;
    s.technical.long_throws = 10.0;
    s.technical.corners = 10.0;
    s.technical.free_kicks = 10.0;
    s.technical.penalty_taking = 10.0;

    s.mental.work_rate = 14.0;
    s.mental.determination = 14.0;
    s.mental.composure = 10.0;
    s.mental.decisions = 10.0;
    s.mental.positioning = 10.0;
    s.mental.anticipation = 10.0;
    s.mental.vision = 10.0;
    s.mental.teamwork = 10.0;
    s.mental.concentration = 10.0;
    s.mental.bravery = 10.0;
    s.mental.aggression = 10.0;
    s.mental.flair = 10.0;
    s.mental.leadership = 10.0;
    s.mental.off_the_ball = 10.0;

    s.physical.pace = 12.0;
    s.physical.acceleration = 12.0;
    s.physical.agility = 12.0;
    s.physical.balance = 12.0;
    s.physical.stamina = 12.0;
    s.physical.strength = 12.0;
    s.physical.jumping = 12.0;
    s.physical.natural_fitness = 14.0;
    s.physical.match_readiness = 15.0;

    s
}

fn gk_skills() -> PlayerSkills {
    let mut s = baseline_skills();
    // Give GK a meaningful goalkeeping baseline.
    s.goalkeeping.handling = 10.0;
    s.goalkeeping.reflexes = 10.0;
    s.goalkeeping.aerial_reach = 10.0;
    s.goalkeeping.one_on_ones = 10.0;
    s.goalkeeping.command_of_area = 10.0;
    s.goalkeeping.communication = 10.0;
    s.goalkeeping.kicking = 10.0;
    s.goalkeeping.first_touch = 10.0;
    s.goalkeeping.passing = 10.0;
    s.goalkeeping.throwing = 10.0;
    s.goalkeeping.punching = 10.0;
    s.goalkeeping.rushing_out = 10.0;
    s.goalkeeping.eccentricity = 10.0;
    s
}

fn positions(p: PlayerPositionType) -> PlayerPositions {
    PlayerPositions {
        positions: vec![PlayerPosition {
            position: p,
            level: 20,
        }],
    }
}

fn make_player(
    birth: NaiveDate,
    pos: PlayerPositionType,
    skills: PlayerSkills,
    pa: u8,
    person: PersonAttributes,
) -> Player {
    let attrs = PlayerAttributes {
        potential_ability: pa,
        // Start CA below PA so per-skill growth has room.
        current_ability: (pa as f32 * 0.5) as u8,
        condition: 9500,
        jadedness: 1000,
        injury_proneness: 5,
        ..Default::default()
    };

    PlayerBuilder::new()
        .id(1)
        .full_name(FullName::new("Test".to_string(), "Player".to_string()))
        .birth_date(birth)
        .country_id(1)
        .attributes(person)
        .skills(skills)
        .positions(positions(pos))
        .player_attributes(attrs)
        .build()
        .unwrap()
}

/// Every family's football banked: a regular whose ceilings move with his
/// age alone.
fn regular(player: &mut Player) {
    for family in MaturationGroup::ALL {
        player.development.football.set(family, 1.0);
    }
}

fn passing_work(reps: f32) -> SessionWork {
    let mut work = SessionWork::default();
    work.practise(SkillKey::Passing, reps);
    work
}

// ── The read ──────────────────────────────────────────────────────

/// A season of football far below him reads as football he has outgrown,
/// and a year without any as no fitting football; neither is a view until a
/// season he was followed through is behind it.
#[test]
fn the_read_tells_outgrown_football_from_none() {
    use super::football::MatchFootball;
    let born = d(2004, 3, 1);
    let mut player = make_player(
        born,
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        150,
        person_pro(14.0, 14.0),
    );
    player.player_attributes.current_ability = 120;
    let mut idle = player.clone();
    let start = d(2026, 7, 6);
    assert!(!player.development.ledger.read(start).known);
    for week in 0..56 {
        let date = start + Duration::days(week * 7);
        player.bank_football(
            &MatchFootball {
                minutes: 90.0,
                standard: 0.2,
                slot: PlayerPositionType::MidfielderCenter,
                season_rating: 0.0,
                season_games: 0,
            },
            date,
        );
        idle.process_development(date);
    }
    let today = start + Duration::days(56 * 7);
    let played = player.development.ledger.read(today);
    assert!(played.known && played.outgrown > 0.5, "{played:?}");
    assert!(played.short_of_fitting_football());
    let none = idle.development.ledger.read(today);
    assert!(none.known && none.fitting_minutes == 0.0, "{none:?}");
    assert!(none.short_of_fitting_football());
}

// ── Round-trip ────────────────────────────────────────────────────

#[test]
fn skill_array_round_trip_preserves_all_fields() {
    let mut p = make_player(
        d(2000, 1, 1),
        PlayerPositionType::Striker,
        baseline_skills(),
        150,
        PersonAttributes::default(),
    );
    // Stamp every skill with a unique value so a missing field shows up
    // as a stale default rather than a coincidence.
    let mut tagged = [0.0f32; SKILL_COUNT];
    for (i, v) in tagged.iter_mut().enumerate() {
        *v = 1.0 + (i as f32) * 0.1;
    }
    write_array_into(&mut p.skills, &tagged);

    let round_tripped = skills_to_array(&p);
    for i in 0..SKILL_COUNT {
        assert!(
            (round_tripped[i] - tagged[i]).abs() < 1e-6,
            "skill index {} did not round-trip: wrote {}, read {}",
            i,
            tagged[i],
            round_tripped[i]
        );
    }
}

#[test]
fn skill_count_matches_enum() {
    assert_eq!(SkillKey::GkThrowing as usize + 1, SKILL_COUNT);
}

// ── The work rule ─────────────────────────────────────────────────

/// The same sessions teach a professional, ambitious player more.
#[test]
fn a_professional_takes_in_more_of_the_same_work() {
    let today = d(2026, 9, 1);
    let gain = |character: f32| {
        let mut p = make_player(
            d(2004, 3, 1),
            PlayerPositionType::MidfielderCenter,
            baseline_skills(),
            160,
            person_pro(character, character),
        );
        regular(&mut p);
        p.draw_blueprint(today);
        let before = p.skills.technical.passing;
        for day in 0..10 {
            p.on_training_session(&passing_work(0.06), today + Duration::days(day));
        }
        p.skills.technical.passing - before
    };
    let (pro, lax) = (gain(18.0), gain(4.0));
    assert!(pro > lax * 1.3, "professional {pro:.3}, lax {lax:.3}");
}

/// A drained, jaded body takes in less of the same session.
#[test]
fn a_tired_player_takes_in_less() {
    let today = d(2026, 9, 1);
    let gain = |tired: bool| {
        let mut p = make_player(
            d(2004, 3, 1),
            PlayerPositionType::MidfielderCenter,
            baseline_skills(),
            160,
            person_pro(12.0, 12.0),
        );
        regular(&mut p);
        p.draw_blueprint(today);
        if tired {
            p.player_attributes.condition = 4500;
            p.player_attributes.jadedness = 7000;
        }
        let before = p.skills.technical.passing;
        p.on_training_session(&passing_work(0.06), today);
        p.skills.technical.passing - before
    };
    let (fresh, tired) = (gain(false), gain(true));
    assert!(fresh > tired * 1.3, "fresh {fresh:.4}, tired {tired:.4}");
}

/// Two seasons of the richest work never take ability past potential:
/// not the stored digit, and not the attributes the match engine reads.
#[test]
fn work_never_takes_ability_past_potential() {
    let pa = 110u8;
    let mut p = make_player(
        d(2001, 1, 1),
        PlayerPositionType::Striker,
        baseline_skills(),
        pa,
        person_pro(18.0, 18.0),
    );
    let start = d(2025, 8, 1);
    p.draw_blueprint(start);
    regular(&mut p);
    let mut work = SessionWork::default();
    for key in [
        SkillKey::Finishing,
        SkillKey::Passing,
        SkillKey::Decisions,
        SkillKey::Composure,
        SkillKey::OffTheBall,
        SkillKey::Pace,
    ] {
        work.practise(key, 0.2);
    }
    for day in 0..730 {
        let today = start + Duration::days(day);
        p.on_training_session(&work, today);
        if day % 7 == 0 {
            p.process_development(today);
        }
    }
    assert_eq!(p.player_attributes.potential_ability, pa);
    assert!(p.player_attributes.current_ability <= pa);
    assert!(
        p.skills
            .calculate_ability_for_position(PlayerPositionType::Striker)
            <= pa,
        "the attributes score past PA {pa}"
    );
}

/// A player hydrated with CA above PA is clamped down, and PA stays.
#[test]
fn ca_above_pa_is_clamped_down_not_pa_raised() {
    let now = d(2025, 6, 1);
    let mut p = make_player(
        d(2007, 1, 1),
        PlayerPositionType::Striker,
        baseline_skills(),
        140,
        person_pro(15.0, 12.0),
    );
    p.player_attributes.current_ability = 160;
    p.process_development(now);
    assert_eq!(p.player_attributes.potential_ability, 140);
    assert!(p.player_attributes.current_ability <= 140);
}

#[test]
fn veteran_pace_declines_noticeably_over_a_season() {
    let mut p = make_player(
        d(1992, 1, 1),
        PlayerPositionType::Striker,
        baseline_skills(),
        160,
        person_pro(10.0, 10.0),
    );
    let pre_pace = p.skills.physical.pace;
    let start = d(2025, 8, 1);
    for week in 0..52 {
        p.process_development(start + Duration::weeks(week));
    }
    let loss = pre_pace - p.skills.physical.pace;
    assert!(
        loss > 0.3 && loss < 2.5,
        "a 33-year-old loses pace gradually over a season, lost {loss:.2}"
    );
}

/// Work stops at an attribute's ceiling and never takes back what he has.
#[test]
fn skills_above_their_ceiling_freeze_instead_of_being_cut() {
    let now = d(2025, 9, 1);
    let mut p = make_player(
        d(1998, 1, 1),
        PlayerPositionType::Striker,
        baseline_skills(),
        130,
        person_pro(12.0, 12.0),
    );
    p.draw_blueprint(now);
    p.skills.mental.concentration = 16.0;
    p.skills.technical.marking = 16.0;
    let ceilings = PositionalSkillCeilings::for_player(&p, 27.7);
    assert!(ceilings.get(SkillKey::Concentration) < 16.0);
    assert!(ceilings.get(SkillKey::Marking) < 16.0);
    let mut work = SessionWork::default();
    work.practise(SkillKey::Concentration, 0.3);
    work.practise(SkillKey::Marking, 0.3);
    p.on_training_session(&work, now);
    assert_eq!(p.skills.mental.concentration, 16.0);
    assert_eq!(p.skills.technical.marking, 16.0);
}

// ── Ceilings by age ───────────────────────────────────────────────

#[test]
fn teenage_ceiling_holds_the_mind_back_but_not_the_body() {
    // A central midfielder whose decision-making is among his strengths.
    let mut skills = baseline_skills();
    skills.mental.decisions *= 1.2;
    let mut boy = make_player(
        d(2009, 1, 1),
        PlayerPositionType::MidfielderCenter,
        skills,
        180,
        PersonAttributes::default(),
    );
    regular(&mut boy);
    let young = PositionalSkillCeilings::for_player(&boy, 17.5);
    let peak = PositionalSkillCeilings::for_player(&boy, 28.5);
    let (mind_young, mind_peak) = (
        young.get(SkillKey::Decisions),
        peak.get(SkillKey::Decisions),
    );
    let (body_young, body_peak) = (young.get(SkillKey::Pace), peak.get(SkillKey::Pace));
    assert!(
        mind_peak - mind_young >= 4.0,
        "a 17-year-old's decision-making ceiling must sit far below his peak — {mind_young} vs {mind_peak}"
    );
    assert!(
        body_young / body_peak > mind_young / mind_peak,
        "legs mature before the head"
    );
}

/// An existing teenager already above his age's ceiling keeps every point.
#[test]
fn age_ceiling_never_cuts_what_a_player_already_has() {
    let mut prodigy = make_player(
        d(2009, 1, 1),
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        180,
        PersonAttributes::default(),
    );
    prodigy.draw_blueprint(d(2026, 7, 1));
    prodigy.skills.mental.decisions = 17.0;
    let ceiling = PositionalSkillCeilings::for_player(&prodigy, 17.5).get(SkillKey::Decisions);
    assert!(
        ceiling < 17.0,
        "the age ceiling should sit below his current 17.0"
    );
    assert_eq!(ceiling.max(17.0), 17.0);
}

/// A keeper's craft matures latest of all.
#[test]
fn teenage_keeper_cannot_reach_a_senior_handling_ceiling() {
    let mut boy = make_player(
        d(2009, 1, 1),
        PlayerPositionType::Goalkeeper,
        gk_skills(),
        120,
        PersonAttributes::default(),
    );
    regular(&mut boy);
    let young = PositionalSkillCeilings::for_player(&boy, 17.5).get(SkillKey::GkHandling);
    let peak = PositionalSkillCeilings::for_player(&boy, 33.5).get(SkillKey::GkHandling);
    assert!(
        peak - young >= 2.5,
        "a 17-year-old keeper's handling ceiling must trail his peak — {young} vs {peak}"
    );
}

// ── Football that fits ────────────────────────────────────────────

/// A season of thirty full matches at `lift` above his own standard, read
/// before each match.
fn a_season_at(mut boy: Player, lift: f32) -> Player {
    use super::football::{FootballFit, MatchFootball};
    let start = d(2026, 8, 3);
    for week in 0..30 {
        let own = FootballFit::own_standard(boy.player_attributes.current_ability);
        boy.bank_football(
            &MatchFootball {
                minutes: 90.0,
                standard: own + lift,
                slot: PlayerPositionType::MidfielderCenter,
                season_rating: 0.0,
                season_games: 0,
            },
            start + Duration::days(week * 7),
        );
    }
    boy
}

fn sixteen_year_old() -> Player {
    make_player(
        d(2010, 3, 1),
        PlayerPositionType::MidfielderCenter,
        baseline_skills(),
        150,
        person_pro(12.0, 12.0),
    )
}

#[test]
fn youth_football_suits_a_boy() {
    let boy = a_season_at(sixteen_year_old(), 0.0);
    let fit = boy.development.ledger.this_season().mean_fit();
    assert!(
        fit > 0.999,
        "football at his own standard fitted him {fit:.3}"
    );
}

#[test]
fn overmatched() {
    let (own, above) = (
        a_season_at(sixteen_year_old(), 0.0),
        a_season_at(sixteen_year_old(), 0.25),
    );
    let (a, b) = (
        own.development.football.share(),
        above.development.football.share(),
    );
    assert!(
        b < a,
        "senior football far above him banked {b:.4}, his own standard {a:.4}"
    );
}

#[test]
fn football_at_an_older_age_banks_less() {
    let boy = a_season_at(sixteen_year_old(), 0.0);
    let mut man = sixteen_year_old();
    man.birth_date = d(1998, 3, 1);
    let man = a_season_at(man, 0.0);
    let (young, older) = (
        boy.development.football.share(),
        man.development.football.share(),
    );
    assert!(
        older < young,
        "a season at 28 banked {older:.4}, at 16 {young:.4}"
    );
}
