//! The S2 census.
//!
//! `docs/staff_mind.md` §9 gates the phase that re-homes a coach's
//! per-player state on a "before/after selection census on a fixed
//! corpus". This is that census, written as a regression test so it
//! keeps guarding after the move rather than being a number in a
//! changelog.
//!
//! **The plan's risk assessment for S2 pointed at the wrong store.** It
//! calls S2 "the highest-risk one because selection reads it". Selection
//! reads [`CoachMemoryStore`], via `CoachDecisionEngine::from_staff` —
//! and that store already lived on `Staff` and already travelled with
//! the man. The store that was homeless is `CoachDecisionState`, and
//! nothing in the selection path touches it: its readers are squad
//! composition (`SquadManager`), the recruitment budget
//! (`transfers::pipeline::evaluation`) and the manager's own situation.
//!
//! So there are two censuses here, and they guard different moves:
//!
//! | Census | Guards |
//! |---|---|
//! | [`selection`] | moving `CoachMemory` under this organ — a pure path change |
//! | [`composition`] | moving `CoachDecisionState` off `TeamCollection` and onto `Staff` |
//!
//! Both are pinned to exact values on a deterministic corpus. A value
//! that moves is a behaviour change, and a behaviour change here has to
//! be argued for rather than absorbed.
//!
//! [`selection`]: selection_census
//! [`composition`]: composition_census

#![cfg(test)]

use crate::club::player::builder::PlayerBuilder;
use crate::club::staff::coach::{CoachMatchObservation, CoachMemoryStore};
use crate::club::staff::perception::CoachProfile;
use crate::r#match::squad::selection::SquadSelector;
use crate::shared::fullname::FullName;
use crate::{
    MatchTacticType, PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerSkills, SelectionOmissionReason, Staff,
    StaffClubContract, StaffCollection, StaffPosition, StaffStatus, StaffStub, Tactics, Team,
    TeamBuilder, TeamReputation, TeamType, TrainingSchedule,
};
use chrono::{NaiveDate, NaiveTime};

/// A fixed corpus. Nothing here draws on the RNG, so the two censuses
/// below produce the same numbers on every machine and every run.
struct Corpus;

impl Corpus {
    const CLUB: u32 = 1;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 11, 9).expect("valid census date")
    }

    /// The eleven slots a 4-4-2 wants, plus seven for the bench and
    /// three who will not make the squad. Ability descends with id, so
    /// the "right" XI is unambiguous and any reshuffle is visible.
    fn squad() -> Vec<Player> {
        let shape = [
            (PlayerPositionType::Goalkeeper, 2),
            (PlayerPositionType::DefenderLeft, 2),
            (PlayerPositionType::DefenderCenterLeft, 2),
            (PlayerPositionType::DefenderCenterRight, 2),
            (PlayerPositionType::DefenderRight, 2),
            (PlayerPositionType::MidfielderLeft, 2),
            (PlayerPositionType::MidfielderCenterLeft, 2),
            (PlayerPositionType::MidfielderCenterRight, 2),
            (PlayerPositionType::MidfielderRight, 2),
            (PlayerPositionType::Striker, 3),
        ];

        let mut players = Vec::new();
        let mut id = 1u32;
        for (position, count) in shape {
            for rank in 0..count {
                // 170 down to 120 in steps, so the first at each
                // position is clearly the best and the reserve is
                // clearly second.
                let ability = 170 - (rank as u8 * 20) - ((id as u8 % 5) * 2);
                players.push(Self::player(id, position, ability));
                id += 1;
            }
        }
        players
    }

    fn player(id: u32, position: PlayerPositionType, ability: u8) -> Player {
        let mut player = PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Census".into(), format!("Player{id}")))
            .birth_date(NaiveDate::from_ymd_opt(2003, 3, 3).expect("valid birth date"))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 19,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .contract(None)
            .build()
            .expect("census player builds");

        player.player_attributes.current_ability = ability;
        player.player_attributes.potential_ability = ability;
        player.player_attributes.condition = 9_500;
        player.player_attributes.fitness = 9_000;
        player.player_attributes.days_since_last_match = 7;
        Self::stamp_skills(&mut player, ability as f32 / 10.0);
        player
    }

    /// Selection perception reads observable skills, not the ability
    /// digit — without a stamp every fixture player looks identical to
    /// any skill-based term.
    fn stamp_skills(player: &mut Player, level: f32) {
        let v = level.clamp(1.0, 20.0);
        let readiness = player.skills.physical.match_readiness;
        player.skills = PlayerSkills::default();
        player.skills.physical.match_readiness = readiness;

        for slot in [
            &mut player.skills.technical.corners,
            &mut player.skills.technical.crossing,
            &mut player.skills.technical.dribbling,
            &mut player.skills.technical.finishing,
            &mut player.skills.technical.first_touch,
            &mut player.skills.technical.heading,
            &mut player.skills.technical.marking,
            &mut player.skills.technical.passing,
            &mut player.skills.technical.tackling,
            &mut player.skills.technical.technique,
        ] {
            *slot = v;
        }
        for slot in [
            &mut player.skills.mental.anticipation,
            &mut player.skills.mental.composure,
            &mut player.skills.mental.concentration,
            &mut player.skills.mental.decisions,
            &mut player.skills.mental.determination,
            &mut player.skills.mental.off_the_ball,
            &mut player.skills.mental.positioning,
            &mut player.skills.mental.teamwork,
            &mut player.skills.mental.vision,
            &mut player.skills.mental.work_rate,
        ] {
            *slot = v;
        }
        for slot in [
            &mut player.skills.physical.acceleration,
            &mut player.skills.physical.agility,
            &mut player.skills.physical.balance,
            &mut player.skills.physical.jumping,
            &mut player.skills.physical.natural_fitness,
            &mut player.skills.physical.pace,
            &mut player.skills.physical.stamina,
            &mut player.skills.physical.strength,
        ] {
            *slot = v;
        }
        if player.positions.is_goalkeeper() {
            for slot in [
                &mut player.skills.goalkeeping.aerial_reach,
                &mut player.skills.goalkeeping.command_of_area,
                &mut player.skills.goalkeeping.handling,
                &mut player.skills.goalkeeping.one_on_ones,
                &mut player.skills.goalkeeping.reflexes,
            ] {
                *slot = v;
            }
        }
    }

    fn team(players: Vec<Player>) -> Team {
        TeamBuilder::new()
            .id(10)
            .league_id(Some(1))
            .club_id(Self::CLUB)
            .name("Census FC".to_string())
            .slug("census-fc".to_string())
            .team_type(TeamType::Main)
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(10, 0, 0).expect("valid time"),
                NaiveTime::from_hms_opt(17, 0, 0).expect("valid time"),
            ))
            .reputation(TeamReputation::new(3_000, 3_000, 3_000))
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .tactics(Some(Tactics::new(MatchTacticType::T442)))
            .build()
            .expect("census team builds")
    }

    /// A head coach with a real body of observation behind him — the
    /// point of the census is that the memory is *loaded*, because an
    /// empty store would make the two sides of the move trivially agree.
    fn coach() -> Staff {
        let mut staff = StaffStub::default();
        staff.id = 900;
        staff.contract = Some(StaffClubContract::new(
            250_000,
            NaiveDate::from_ymd_opt(2034, 6, 30).expect("valid contract expiry"),
            StaffPosition::Manager,
            StaffStatus::Active,
        ));
        staff.staff_attributes.mental.man_management = 14;
        staff.staff_attributes.mental.determination = 12;
        staff.staff_attributes.mental.discipline = 11;
        staff.staff_attributes.mental.adaptability = 13;
        staff.staff_attributes.knowledge.judging_player_ability = 15;
        staff.staff_attributes.knowledge.judging_player_potential = 14;

        let profile = CoachProfile::from_staff(&staff);
        let mut memory = CoachMemoryStore::new();
        for player_id in 1..=21u32 {
            for week in 0..8u32 {
                // Ratings walk a fixed sawtooth keyed on the player id, so
                // the coach ends up with genuinely different views of
                // different players and none of it is random.
                let swing = ((player_id * 7 + week * 3) % 11) as f32 / 5.0;
                let rating = 4.8 + swing;
                let observed = Self::date() - chrono::Duration::days((8 - week) as i64 * 7);
                memory.observe(
                    &CoachMatchObservation {
                        player_id,
                        effective_rating: rating,
                        minutes_played: 90,
                        is_starter: true,
                        match_importance: 0.6,
                        is_cup: week % 4 == 0,
                        is_derby: false,
                        is_continental: false,
                        goals: 0,
                        assists: 0,
                        errors_leading_to_goal: u16::from(rating < 5.0),
                        yellow_cards: 0,
                        red_cards: 0,
                        team_won: rating > 6.0,
                        was_substituted_early: rating < 5.2,
                        role_fit: 1.0,
                        professionalism_signal: 0.6,
                        date: observed,
                    },
                    &profile,
                );
            }
        }
        staff.coach_memory = memory;
        staff
    }
}

/// **Selection census.** Guards moving `CoachMemory` under this organ.
///
/// The starting XI, the bench and the omission set on a fixed corpus,
/// with a loaded coach memory behind them. A path change must not move
/// any of it; if it does, the move was not a path change.
#[test]
fn selection_census() {
    let team = Corpus::team(Corpus::squad());
    let coach = Corpus::coach();

    let result = SquadSelector::select(&team, &coach);

    let starters: Vec<u32> = result.main_squad.iter().map(|p| p.id).collect();
    let bench: Vec<u32> = result.substitutes.iter().map(|p| p.id).collect();

    assert_eq!(starters.len(), 11, "a starting XI is eleven players");

    let mut everyone = starters.clone();
    everyone.extend(&bench);
    let unique = {
        let mut sorted = everyone.clone();
        sorted.sort_unstable();
        sorted.dedup();
        sorted.len()
    };
    assert_eq!(everyone.len(), unique, "nobody is picked twice");

    // The pinned corpus. These are not aspirational numbers — they are
    // what the selector does today, recorded so a refactor that claims
    // to change nothing can be held to it.
    assert_eq!(
        starters,
        vec![1, 3, 6, 7, 9, 12, 13, 15, 17, 19, 20],
        "starting XI moved"
    );
    assert_eq!(bench, vec![2, 5, 16, 11, 4, 21, 10], "bench moved");
    assert_eq!(result.omissions.len(), 10, "omission set moved");

    // The reason distribution §10 asks for, as an ordered list of
    // (player, reason) pairs. Counting omissions alone would let a
    // refactor swap *why* a man was left out without moving the total.
    let reasons: Vec<(u32, SelectionOmissionReason)> = result
        .omissions
        .iter()
        .map(|omitted| (omitted.player_id, omitted.context.reason))
        .collect();
    assert_eq!(
        reasons,
        vec![
            (2, SelectionOmissionReason::TeammatePreferredOnAbility),
            (4, SelectionOmissionReason::TeammatePreferredOnAbility),
            (5, SelectionOmissionReason::TeammatePreferredOnFitness),
            (8, SelectionOmissionReason::TeammatePreferredOnAbility),
            (10, SelectionOmissionReason::TeammatePreferredOnAbility),
            (11, SelectionOmissionReason::TeammatePreferredOnFitness),
            (14, SelectionOmissionReason::TeammatePreferredOnAbility),
            (16, SelectionOmissionReason::TeammatePreferredOnAbility),
            (18, SelectionOmissionReason::TeammatePreferredOnAbility),
            (21, SelectionOmissionReason::NoNaturalRoleInFormation),
        ],
        "omission reason distribution moved"
    );
}

/// **Composition census.** Guards moving `CoachDecisionState` off
/// `TeamCollection` and onto `Staff`.
///
/// Runs the weekly coach-state cycle over a fixed corpus and pins the
/// three accumulators the move is actually about, plus the squad
/// satisfaction the recruitment budget reads.
#[test]
fn composition_census() {
    use crate::TeamCollection;

    let mut collection = TeamCollection::new(vec![Corpus::team(Corpus::squad())]);
    if let Some(main) = collection.main_mut() {
        main.staffs = StaffCollection::new(vec![Corpus::coach()]);
    }

    let start = Corpus::date();
    for week in 0..12i64 {
        let date = start + chrono::Duration::days(week * 7);
        collection.ensure_coach_state(date);
        collection.update_all_impressions(date);
    }

    let state = collection
        .head_coach_decision_state()
        .expect("a club with a head coach has a decision state");

    assert_eq!(state.coach_id, 900, "the state is bound to the coach");
    assert_eq!(
        state.impressions.len(),
        21,
        "one impression per player in the squad"
    );
    assert_eq!(
        (state.squad_satisfaction * 1_000.0).round() as i64,
        777,
        "squad satisfaction moved — the recruitment budget reads this"
    );
    assert_eq!(
        (state.emotional_heat * 1_000.0).round() as i64,
        0,
        "emotional heat moved"
    );
    assert_eq!(
        (state.trigger_pressure * 1_000.0).round() as i64,
        0,
        "trigger pressure moved"
    );

    // The perceived-quality spread is what `SquadSatisfaction` reads out
    // of the impressions, so pinning it pins the impression pass itself
    // rather than just its container.
    let spread: i64 = state
        .impressions
        .values()
        .map(|imp| (imp.perceived_quality * 100.0).round() as i64)
        .sum();
    assert_eq!(spread, 28_045, "the impression pass moved");
}
