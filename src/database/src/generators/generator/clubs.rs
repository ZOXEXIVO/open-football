use crate::DatabaseEntity;
use crate::generators::player::OdbPositionCode;
use crate::generators::{PlayerGenerator, StaffGenerator};
use crate::loaders::OdbPlayer;
use chrono::{Datelike, Utc};
use core::club::academy::ClubAcademy;
use core::context::NaiveTime;
use core::shared::Location;
use core::transfers::market::knowledge::ClubMarketLedger;
use core::transfers::pipeline::{ClubTransferPlan, TransferTrace};
use core::{
    Club, ClubAffairLog, ClubBoard, ClubColors, ClubFacilities, ClubFinances, ClubLevelAnchor,
    ClubPhilosophy, ClubStatus, CountryEconomicFactors, FacilityLevel, Player, PlayerCollection,
    PlayerFieldPositionGroup, ReputationLevel, SponsorPerformance, SponsorRenewalContext,
    StaffCollection, TacticsSelector, Team, TeamCollection, TeamReputation, TeamType,
    TrainingSchedule,
};
use rayon::prelude::*;
use std::collections::HashMap;
use std::str::FromStr;

use super::DatabaseGenerator;
use super::staffs::ScoutMarketSeed;

impl DatabaseGenerator {
    pub(super) fn generate_clubs(
        scout_seed: &ScoutMarketSeed<'_>,
        country_reputation: u16,
        data: &DatabaseEntity,
        player_generator: &PlayerGenerator,
        staff_generator: &StaffGenerator,
    ) -> Vec<Club> {
        let country_id = scout_seed.country_id;
        let continent_id = scout_seed.continent_id;
        let country_code = scout_seed.country_code;
        let odb = data.players_odb.as_ref();
        let now_year = Utc::now().date_naive().year();

        // Parallelise club construction: each club hydrates or generates 25-200
        // players across 1-5 teams plus 10-15 staff. Work is dominated by
        // player skill generation (CPU-bound, no I/O) and is fully independent
        // per club, so par_iter scales near-linearly with cores. The RNG is
        // thread-local (see core::utils::random::engine), and both generators
        // now take &self, so no further synchronisation is needed.
        data.clubs
            .par_iter()
            .filter(|c| c.country_id == country_id)
            .map(|club| {
                // Pre-distribute ODB players for this club into TeamType buckets.
                // If the club has any ODB players, fake generation is skipped for
                // every senior team (Main/Reserve/B/U20/U21/U23). Academy teams
                // (U18/U19) always go through the existing generator regardless,
                // because youth intake is owned by the academy system.
                let odb_for_club: Option<HashMap<TeamType, Vec<OdbPlayer>>> = odb
                    .and_then(|o| o.for_club(club.id))
                    .filter(|players| !players.is_empty())
                    .map(|players| {
                        let available_team_types: Vec<TeamType> = club
                            .teams
                            .iter()
                            .filter_map(|t| TeamType::from_str(&t.team_type).ok())
                            .collect();
                        // The club's own reputation stands in for its senior
                        // bar wherever the source squad is too thin at a
                        // position to state one — read from `club.json`,
                        // like every other placement input.
                        let club_reputation = club
                            .teams
                            .iter()
                            .find(|t| t.team_type.eq_ignore_ascii_case("main"))
                            .map(|t| {
                                TeamReputation::new(
                                    t.reputation.home,
                                    t.reputation.national,
                                    t.reputation.world,
                                )
                            })
                            .unwrap_or_else(|| TeamReputation::new(0, 0, 0));
                        OdbSquadPlacement::distribute(
                            players,
                            &available_team_types,
                            now_year,
                            club_reputation,
                        )
                    });

                // Determine philosophy from main team reputation
                let philosophy = if let Some(ref p) = club.philosophy {
                    match p.as_str() {
                        "SignToCompete" => ClubPhilosophy::SignToCompete,
                        "DevelopAndSell" => ClubPhilosophy::DevelopAndSell,
                        "LoanFocused" => ClubPhilosophy::LoanFocused,
                        _ => ClubPhilosophy::Balanced,
                    }
                } else {
                    let main_rep = club
                        .teams
                        .iter()
                        .find(|t| t.team_type.eq_ignore_ascii_case("main"))
                        .map(|t| t.reputation.world)
                        .unwrap_or(0);
                    match TeamReputation::new(0, 0, main_rep).level() {
                        ReputationLevel::Elite => ClubPhilosophy::SignToCompete,
                        ReputationLevel::Continental => ClubPhilosophy::Balanced,
                        ReputationLevel::National => ClubPhilosophy::Balanced,
                        _ => ClubPhilosophy::LoanFocused,
                    }
                };

                // Ground capacity: the data carries a typical gate, not a
                // capacity, so gross it up. Falls back to a reputation
                // estimate for clubs with no attendance record — never zero,
                // or the club earns no matchday revenue at all.
                let average_attendance = club.average_attendance.unwrap_or(0);
                let club_reputation_score = club
                    .teams
                    .iter()
                    .find(|t| t.team_type.eq_ignore_ascii_case("main"))
                    .map(|t| t.reputation.world as f32 / 10_000.0)
                    .unwrap_or(0.0);
                let stadium_capacity =
                    ClubFacilities::seed_capacity(average_attendance, club_reputation_score);

                let facilities = match &club.facilities {
                    Some(f) => ClubFacilities {
                        training: FacilityLevel::from_str(&f.training),
                        youth: FacilityLevel::from_str(&f.youth),
                        academy: FacilityLevel::from_str(&f.academy),
                        recruitment: FacilityLevel::from_str(&f.recruitment),
                        average_attendance,
                        stadium_capacity,
                    },
                    None => ClubFacilities {
                        average_attendance,
                        stadium_capacity,
                        ..ClubFacilities::default()
                    },
                };

                // Extract facility values for youth generation before facilities is moved
                let academy_rating = facilities.academy.to_rating();
                let youth_quality = facilities.youth.multiplier();
                let academy_quality = facilities.academy.multiplier();
                let recruitment_quality = facilities.recruitment.multiplier();

                let teams = TeamCollection::new(
                    club.teams
                        .iter()
                        .map(|t| {
                            let team_rep = t.reputation.world;
                            let team_type = TeamType::from_str(&t.team_type).unwrap();

                            // Main and the senior reserves (B, Second) carry
                            // their full canonical name in the data
                            // ("Spartak Moscow", "Spartak Moscow 2", "Real
                            // Sociedad B"). Other sub-types (Reserve, U18..U23)
                            // get their short type label appended at runtime.
                            let team_name = match &team_type {
                                TeamType::Main | TeamType::Second | TeamType::B => t.name.clone(),
                                _ => format!("{} {}", t.name, team_type),
                            };

                            let players = PlayerCollection::new(build_team_players(
                                player_generator,
                                country_id,
                                continent_id,
                                country_code,
                                team_rep,
                                country_reputation,
                                &team_type,
                                t.league_id,
                                data,
                                academy_rating,
                                youth_quality,
                                academy_quality,
                                recruitment_quality,
                                odb_for_club.as_ref(),
                            ));

                            let staffs = StaffCollection::new(Self::generate_staffs(
                                staff_generator,
                                scout_seed,
                                team_rep,
                                &team_type,
                            ));

                            let mut team = Team::builder()
                                .id(t.id)
                                .league_id(t.league_id)
                                .club_id(club.id)
                                .name(team_name)
                                .slug(t.slug.clone())
                                .team_type(team_type)
                                .training_schedule(TrainingSchedule::new(
                                    NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                                    NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                                ))
                                .reputation(TeamReputation::new(
                                    t.reputation.home,
                                    t.reputation.national,
                                    t.reputation.world,
                                ))
                                .players(players)
                                .staffs(staffs)
                                .build()
                                .expect("Failed to build Team");

                            // Pre-select the persistent team tactic at
                            // load time. Without this every team starts
                            // at `tactics: None` and the web view falls
                            // back to a hardcoded 4-4-2 until the season
                            // tick first runs — and once that tick fires
                            // the legacy selector locked the league at
                            // T442 anyway. Pre-selecting on a properly
                            // squad-aware scorer breaks that loop and
                            // makes the tactics screen truthful from
                            // the first request.
                            let tactic = TacticsSelector::select(&team, team.staffs.head_coach());
                            team.tactics = Some(tactic);
                            team
                        })
                        .collect(),
                );

                // Day-one sponsorship book. Real clubs never operate with
                // zero commercial deals; without seeding, sponsorship
                // income is structurally $0 for every club (the monthly
                // renewal pass only replaces deals that expire, and an
                // empty book has nothing to expire). Sized off the main
                // team's reputation tier and the country's sponsorship
                // market — the same inputs the runtime renewal uses.
                let main_rep_level = teams
                    .main()
                    .map(|t| t.reputation.level())
                    .unwrap_or(ReputationLevel::Amateur);
                let sponsor_market = CountryEconomicFactors::from_reputation(country_reputation)
                    .sponsorship_market_strength;
                let sponsorship_book = SponsorRenewalContext::new(
                    main_rep_level,
                    sponsor_market,
                    SponsorPerformance::MidTable,
                )
                .generate_initial_portfolio(Utc::now().date_naive());

                Club {
                    id: club.id,
                    name: club.name.clone(),
                    location: Location {
                        city_id: club.location.city_id,
                    },
                    board: ClubBoard::new(),
                    status: ClubStatus::Professional,
                    finance: ClubFinances::new(club.finance.balance as i64, sponsorship_book),
                    academy: ClubAcademy::new(academy_rating),
                    colors: ClubColors {
                        background: club.colors.background.clone(),
                        foreground: club.colors.foreground.clone(),
                    },
                    transfer_plan: ClubTransferPlan::new(),
                    philosophy,
                    facilities,
                    rivals: club.rivals.clone(),
                    teams,
                    // Bootstrapped from the squad's own foreign
                    // nationalities once the world is assembled — see
                    // `SimulatorData::bootstrap_market_ledgers`.
                    market_ledger: ClubMarketLedger::default(),
                    // A new world has no history behind it; the diary
                    // starts on the first thing that happens to the club.
                    affairs: ClubAffairLog::new(),
                }
            })
            .collect()
    }
}

/// Choose ODB-backed players if available for a senior team, otherwise fall
/// back to the procedural generator. U18/U19 squads always go through the
/// academy path — those players are owned by the youth/intake system.
fn build_team_players(
    player_generator: &PlayerGenerator,
    country_id: u32,
    continent_id: u32,
    country_code: &str,
    team_reputation: u16,
    country_reputation: u16,
    team_type: &TeamType,
    league_id: Option<u32>,
    data: &DatabaseEntity,
    academy_level: u8,
    youth_quality: f32,
    academy_quality: f32,
    recruitment_quality: f32,
    odb_for_club: Option<&HashMap<TeamType, Vec<OdbPlayer>>>,
) -> Vec<Player> {
    // If the club has any ODB players, it is fully ODB-backed: hydrate every
    // team (including U18/U19) exclusively from the file and skip synthetic
    // generation entirely. Buckets without ODB players for this team type
    // return an empty squad — we do not mix loaded and generated players.
    if let Some(buckets) = odb_for_club {
        return buckets
            .get(team_type)
            .map(|records| {
                // ODB hydration is per-record skill generation — the same
                // CPU-bound pipeline as procedural players. Parallelise the
                // per-record mapping so large squads (main teams carry 25+
                // records) don't serialise one whole club's hydration on a
                // single thread.
                records
                    .par_iter()
                    .map(|r| {
                        PlayerGenerator::generate_from_odb(r, continent_id, country_code, data)
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    // Academy teams for clubs without ODB data fall back to the academy
    // generator — youth intake is owned by the academy system.
    if matches!(team_type, TeamType::U18 | TeamType::U19) {
        return DatabaseGenerator::generate_players(
            player_generator,
            country_id,
            team_reputation,
            country_reputation,
            team_type,
            league_id,
            data,
            academy_level,
            youth_quality,
            academy_quality,
            recruitment_quality,
        );
    }

    // No ODB data for this club — original synthetic path.
    DatabaseGenerator::generate_players(
        player_generator,
        country_id,
        team_reputation,
        country_reputation,
        team_type,
        league_id,
        data,
        academy_level,
        youth_quality,
        academy_quality,
        recruitment_quality,
    )
}

/// Where each ODB record starts its life — which of his club's squads he
/// is registered in on day 0.
///
/// The age ladder alone (≤18 → U18, ≤19 → U19, … else Main) is a
/// calendar, not a squad decision, and it read nothing else in the record:
/// not ability, not value, not the wage the club is actually paying him. A
/// nineteen-year-old who is his club's joint-best forward, on a
/// nine-figure valuation and a first-team contract, went into the U20 with
/// three academy boys — and every downstream reading followed the
/// registration: he was labelled a prospect, classified as a development
/// asset, read as unimportant by the market, and lent to a
/// second-division club for nothing.
///
/// So the ladder stays as the default and a **senior-ready override**
/// runs on top of it: a record whose current ability clears what his own
/// club expects of a senior at his position is registered with the first
/// team whatever his birth year. Everything it reads is in the source
/// record or the club's own `club.json` reputation; nothing is invented.
struct OdbSquadPlacement;

impl OdbSquadPlacement {
    /// Youngest age at which raw ability alone can put a record on the
    /// first team. Below it the age ladder owns the placement however good
    /// the boy is — a fifteen-year-old belongs in the academy whatever his
    /// numbers say, and the promotion engine will pull him up when he is
    /// ready.
    const SENIOR_READY_AGE_MIN: i32 = 17;

    /// Age from which a squad member counts as a SENIOR when the club's
    /// own bar is being read off its roster. Below it the record is part
    /// of the question, not part of the answer.
    const SENIOR_SAMPLE_AGE_MIN: i32 = 21;

    /// Bucket ODB players into the senior team types the club actually
    /// has. The compiler hint still wins outright; then the senior-ready
    /// override; then the age ladder, exactly as before.
    fn distribute(
        players: &[OdbPlayer],
        available: &[TeamType],
        now_year: i32,
        club_reputation: TeamReputation,
    ) -> HashMap<TeamType, Vec<OdbPlayer>> {
        let has = |tt: TeamType| available.iter().any(|t| *t == tt);
        let anchor = ClubLevelAnchor::for_reputation(club_reputation.overall_score());
        // Position group per record and the club's senior bar per group,
        // both resolved once: the bar is a property of the squad, not of
        // the record being placed, and parsing position codes per pair
        // would make placement quadratic in squad size.
        let groups: Vec<PlayerFieldPositionGroup> = players
            .iter()
            .map(|p| OdbPositionCode::group(&p.positions))
            .collect();
        let floors = Self::senior_floors(players, &groups, now_year, &anchor);
        let mut out: HashMap<TeamType, Vec<OdbPlayer>> = HashMap::new();

        for (idx, p) in players.iter().enumerate() {
            // Compiler-set hint pins a player to a specific bucket (e.g. squad
            // folded in from a satellite "B-team" directory). Honour it whenever
            // the parent club actually has that bucket; otherwise fall through
            // to age-based placement.
            let hinted = p
                .team_type_hint
                .as_deref()
                .and_then(|s| TeamType::from_str(s).ok())
                .filter(|tt| has(*tt));
            let target = if let Some(tt) = hinted {
                tt
            } else {
                let age = now_year - p.birth_date.year();
                let senior_ready = age >= Self::SENIOR_READY_AGE_MIN
                    && p.current_ability >= Self::floor_for(&floors, groups[idx]);
                if senior_ready {
                    Self::senior_bucket(&has, available)
                } else if age <= 18 && has(TeamType::U18) {
                    TeamType::U18
                } else if age <= 19 && has(TeamType::U19) {
                    TeamType::U19
                } else if age <= 20 && has(TeamType::U20) {
                    TeamType::U20
                } else if age <= 21 && has(TeamType::U21) {
                    TeamType::U21
                } else if age <= 23 && has(TeamType::U23) {
                    TeamType::U23
                } else {
                    Self::senior_bucket(&has, available)
                }
            };
            if TransferTrace::is(p.id) {
                TransferTrace::line(
                    p.id,
                    "squad",
                    format!(
                        "placement squad={target:?} age={} ca={} senior_floor={} hint={:?} \
                         group={:?}",
                        now_year - p.birth_date.year(),
                        p.current_ability,
                        Self::floor_for(&floors, groups[idx]),
                        p.team_type_hint,
                        groups[idx],
                    ),
                );
            }
            out.entry(target).or_default().push(p.clone());
        }

        out
    }

    /// The senior squad this club actually has, in preference order.
    fn senior_bucket(has: &impl Fn(TeamType) -> bool, available: &[TeamType]) -> TeamType {
        if has(TeamType::Main) {
            TeamType::Main
        } else if has(TeamType::B) {
            TeamType::B
        } else if has(TeamType::Second) {
            TeamType::Second
        } else if has(TeamType::Reserve) {
            TeamType::Reserve
        } else {
            // No senior team at all — drop into the first available bucket so
            // the player isn't silently lost.
            *available
                .iter()
                .find(|t| !matches!(t, TeamType::U18 | TeamType::U19))
                .unwrap_or(&TeamType::Main)
        }
    }

    /// Current ability at/above which this club reads a player as a
    /// senior at his position.
    ///
    /// Two readings, and the bar is the LOWER of them.
    ///
    /// The club's own record states one: the ability of its
    /// `main_depth_cap`-th best established (21+) player in that group,
    /// which is the last man a first-team squad carries there. It needs no
    /// reputation curve — but it is measured at a DEPTH, and at a giant
    /// the twelfth-best defender is still an international. Taken alone it
    /// puts the bar above the division's own key-player floor, and a boy
    /// who would walk into most first teams in the country reads as not
    /// senior-ready at his own.
    ///
    /// So the divisional rotation band caps it: below that band a player
    /// is not first-team quality for a club of this reputation at all, and
    /// at or above it he is — whoever else happens to be on the roster.
    /// A club with fewer established players than the cap has not stated a
    /// bar of its own, and the band stands alone.
    ///
    /// Ties on current ability break on the source `value` field: it is
    /// the source database's own view of the player, not hidden potential.
    fn senior_floors(
        players: &[OdbPlayer],
        groups: &[PlayerFieldPositionGroup],
        now_year: i32,
        anchor: &ClubLevelAnchor,
    ) -> Vec<(PlayerFieldPositionGroup, u8)> {
        PlayerFieldPositionGroup::ALL
            .iter()
            .map(|&group| {
                let depth = group.main_depth_cap();
                let band = anchor.rotation_floor(group).clamp(1, 200) as u8;
                let mut established: Vec<(u8, u32)> = players
                    .iter()
                    .zip(groups.iter())
                    .filter(|(p, g)| {
                        **g == group
                            && now_year - p.birth_date.year() >= Self::SENIOR_SAMPLE_AGE_MIN
                    })
                    .map(|(p, _)| (p.current_ability, p.value.unwrap_or(0)))
                    .collect();
                if established.len() < depth {
                    return (group, band);
                }
                established.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
                (group, established[depth - 1].0.min(band))
            })
            .collect()
    }

    fn floor_for(floors: &[(PlayerFieldPositionGroup, u8)], group: PlayerFieldPositionGroup) -> u8 {
        floors
            .iter()
            .find(|(g, _)| *g == group)
            .map(|(_, floor)| *floor)
            .unwrap_or(u8::MAX)
    }
}

#[cfg(test)]
mod placement_tests {
    use super::{OdbSquadPlacement, TeamType};
    use crate::loaders::{OdbPlayer, OdbPosition};
    use chrono::NaiveDate;
    use core::TeamReputation;

    /// A club with the squads Barcelona actually registers in the source
    /// database, and a reputation of that order.
    struct Fx;

    impl Fx {
        const NOW: i32 = 2026;
        const SQUADS: [TeamType; 3] = [TeamType::Main, TeamType::U20, TeamType::U18];

        fn giant() -> TeamReputation {
            TeamReputation::new(9_300, 9_300, 9_300)
        }

        /// A forward record: `age` years old on the reference year, with
        /// `ca` current ability.
        fn forward(id: u32, ca: u8, age: i32) -> OdbPlayer {
            OdbPlayer {
                id,
                first_name: "T".into(),
                last_name: format!("P{id}"),
                middle_name: None,
                nickname: None,
                birth_date: NaiveDate::from_ymd_opt(Self::NOW - age, 7, 13).unwrap(),
                country_id: 1,
                club_id: 1,
                positions: vec![OdbPosition {
                    code: "AMR".into(),
                    level: 20,
                }],
                preferred_foot: None,
                foots: None,
                height: None,
                weight: None,
                current_ability: ca,
                potential_ability: 0,
                value: Some(1_000_000),
                reputation: None,
                contract: None,
                loan: None,
                history: Vec::new(),
                team_type_hint: None,
            }
        }

        fn bucket_of(players: &[OdbPlayer], id: u32) -> TeamType {
            let placed =
                OdbSquadPlacement::distribute(players, &Self::SQUADS, Self::NOW, Self::giant());
            for (team_type, squad) in placed {
                if squad.iter().any(|p| p.id == id) {
                    return team_type;
                }
            }
            panic!("player {id} was dropped by the placement pass");
        }
    }

    /// The Yamal case. A nineteen-year-old who is the joint-best forward
    /// at his club, on a first-team contract, was filed in the U20 with
    /// three academy boys purely because of his birth year — and every
    /// downstream reading followed the registration.
    #[test]
    fn a_first_team_calibre_teenager_is_registered_with_the_first_team() {
        let squad = vec![
            Fx::forward(1, 176, 19),
            Fx::forward(2, 176, 28),
            Fx::forward(3, 168, 26),
            Fx::forward(4, 90, 17),
        ];
        assert_eq!(Fx::bucket_of(&squad, 1), TeamType::Main);
    }

    /// …and the ladder still owns everybody else: an ordinary academy
    /// forward of the same age goes where he always did.
    #[test]
    fn an_ordinary_teenager_still_goes_to_the_youth_squad() {
        let squad = vec![
            Fx::forward(1, 176, 19),
            Fx::forward(2, 176, 28),
            Fx::forward(3, 110, 19),
        ];
        assert_eq!(Fx::bucket_of(&squad, 3), TeamType::U20);
    }

    /// Below the senior-ready age the ladder wins whatever the record
    /// says — a boy of fifteen is in the academy, not the first team.
    #[test]
    fn the_age_floor_holds_against_any_ability() {
        let squad = vec![Fx::forward(1, 176, 15), Fx::forward(2, 176, 28)];
        assert_eq!(Fx::bucket_of(&squad, 1), TeamType::U18);
    }

    /// A compiler hint still pins the record outright — the satellite
    /// B-team fold-in must not be re-routed by ability.
    #[test]
    fn the_compiler_hint_still_wins() {
        let mut hinted = Fx::forward(1, 176, 19);
        hinted.team_type_hint = Some("U18".into());
        let squad = vec![hinted, Fx::forward(2, 176, 28)];
        assert_eq!(Fx::bucket_of(&squad, 1), TeamType::U18);
    }

    /// When the club's own record IS deep enough at the position, the bar
    /// is measured rather than derived: the sixth-best established forward
    /// sets it, so a teenager above him is senior and one below is not.
    #[test]
    fn a_deep_group_measures_its_own_floor() {
        let mut squad: Vec<OdbPlayer> = (0..6)
            .map(|i| Fx::forward(10 + i as u32, 150 - i * 5, 25))
            .collect();
        squad.push(Fx::forward(1, 130, 19));
        squad.push(Fx::forward(2, 120, 19));
        // Sixth-best established forward is 125, so 130 clears and 120 does not.
        assert_eq!(Fx::bucket_of(&squad, 1), TeamType::Main);
        assert_eq!(Fx::bucket_of(&squad, 2), TeamType::U20);
    }
}
