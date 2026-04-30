use crate::generators::player::SquadRole;
use crate::generators::{PlayerGenerator, PositionType};
use crate::{DatabaseEntity, ForeignPlayerEntry};
use core::{AcademyGenerationContext, AcademyIntakeState, PlayerGenerator as CorePlayerGenerator};
use core::PeopleNameGeneratorData;
use core::utils::IntegerUtils;
use core::{Player, TeamType};
use chrono::Local;

/// Per-position role allocation. Each position bucket (GK/DEF/MID/ST)
/// carries its own queue of `SquadRole` slots so role quality is spread
/// realistically across the squad spine — Stars and Starters can't all
/// land on goalkeepers by chance, and a Main squad always has a clear
/// #1 GK + a backup rather than two random Backups.
///
/// Quotas are tuned per `TeamType`:
///   - Main: stars/starters across DEF/MID/ST spine, GK is 1 Starter +
///     a #2 + depth, attack carries a Star and a prospect.
///   - B/Reserve/Second: senior depth, no Stars, mostly backups + prospects.
///   - U-teams: prospect-heavy with rotation/backup mix.
struct PositionRoleQueue {
    gk: Vec<SquadRole>,
    def: Vec<SquadRole>,
    mid: Vec<SquadRole>,
    st: Vec<SquadRole>,
}

impl PositionRoleQueue {
    fn for_team(team_type: TeamType) -> Self {
        let (gk, def, mid, st) = match team_type {
            TeamType::Main => (
                Self::shuffled(&[(SquadRole::Starter, 1), (SquadRole::Rotation, 1),
                                 (SquadRole::Backup, 2), (SquadRole::Prospect, 1)]),
                Self::shuffled(&[(SquadRole::Star, 1), (SquadRole::Starter, 4),
                                 (SquadRole::Rotation, 2), (SquadRole::Backup, 1),
                                 (SquadRole::Prospect, 1)]),
                Self::shuffled(&[(SquadRole::Star, 1), (SquadRole::Starter, 4),
                                 (SquadRole::Rotation, 2), (SquadRole::Backup, 1),
                                 (SquadRole::Prospect, 1)]),
                Self::shuffled(&[(SquadRole::Star, 1), (SquadRole::Starter, 3),
                                 (SquadRole::Rotation, 1), (SquadRole::Backup, 1),
                                 (SquadRole::Prospect, 1), (SquadRole::Fringe, 1)]),
            ),
            TeamType::Second | TeamType::B | TeamType::Reserve => (
                Self::shuffled(&[(SquadRole::Starter, 1), (SquadRole::Backup, 2),
                                 (SquadRole::Prospect, 2)]),
                Self::shuffled(&[(SquadRole::Starter, 1), (SquadRole::Rotation, 2),
                                 (SquadRole::Backup, 3), (SquadRole::Prospect, 2),
                                 (SquadRole::Fringe, 1)]),
                Self::shuffled(&[(SquadRole::Starter, 1), (SquadRole::Rotation, 2),
                                 (SquadRole::Backup, 3), (SquadRole::Prospect, 3),
                                 (SquadRole::Fringe, 1)]),
                Self::shuffled(&[(SquadRole::Rotation, 1), (SquadRole::Backup, 2),
                                 (SquadRole::Prospect, 2), (SquadRole::Fringe, 1)]),
            ),
            TeamType::U23 | TeamType::U21 | TeamType::U20 => (
                Self::shuffled(&[(SquadRole::Rotation, 1), (SquadRole::Backup, 1),
                                 (SquadRole::Prospect, 3)]),
                Self::shuffled(&[(SquadRole::Rotation, 1), (SquadRole::Backup, 2),
                                 (SquadRole::Prospect, 5), (SquadRole::Fringe, 1)]),
                Self::shuffled(&[(SquadRole::Rotation, 2), (SquadRole::Backup, 2),
                                 (SquadRole::Prospect, 5), (SquadRole::Fringe, 1)]),
                Self::shuffled(&[(SquadRole::Rotation, 1), (SquadRole::Backup, 1),
                                 (SquadRole::Prospect, 4), (SquadRole::Fringe, 1)]),
            ),
            // U18/U19 don't use this distribution (academy path), but cover
            // the case for completeness.
            TeamType::U18 | TeamType::U19 => (
                Self::shuffled(&[(SquadRole::Prospect, 4), (SquadRole::Fringe, 1)]),
                Self::shuffled(&[(SquadRole::Prospect, 7), (SquadRole::Fringe, 2)]),
                Self::shuffled(&[(SquadRole::Prospect, 8), (SquadRole::Fringe, 2)]),
                Self::shuffled(&[(SquadRole::Prospect, 5), (SquadRole::Fringe, 1)]),
            ),
        };
        PositionRoleQueue { gk, def, mid, st }
    }

    fn shuffled(slots: &[(SquadRole, usize)]) -> Vec<SquadRole> {
        let total: usize = slots.iter().map(|(_, n)| n).sum();
        let mut v: Vec<SquadRole> = Vec::with_capacity(total);
        for (role, n) in slots { for _ in 0..*n { v.push(*role); } }
        for i in (1..v.len()).rev() {
            let j = IntegerUtils::random(0, i as i32) as usize;
            v.swap(i, j);
        }
        v
    }

    /// Pop the next role for this bucket. When a bucket runs out (squad
    /// generation produced more players than the quota provided), fall back
    /// to a sensible filler tier so we don't accidentally promote leftovers.
    fn next(&mut self, bucket: PositionType) -> SquadRole {
        let q = match bucket {
            PositionType::Goalkeeper => &mut self.gk,
            PositionType::Defender => &mut self.def,
            PositionType::Midfielder => &mut self.mid,
            PositionType::Striker => &mut self.st,
        };
        q.pop().unwrap_or(SquadRole::Backup)
    }
}

use super::DatabaseGenerator;

impl DatabaseGenerator {
    pub(super) fn generate_players(
        player_generator: &PlayerGenerator,
        country_id: u32,
        team_reputation: u16,
        country_reputation: u16,
        team_type: &TeamType,
        league_id: Option<u32>,
        data: &DatabaseEntity,
        academy_level: u8,
        youth_quality: f32,
        academy_quality: f32,
        recruitment_quality: f32,
    ) -> Vec<Player> {
        let mut players = Vec::with_capacity(100);

        // Youth teams (U18/U19) use the academy generator — same logic as in-game academy intake
        // but with age range matching the team type. This ensures consistent quality between
        // initial squad generation and ongoing academy production.
        if matches!(team_type, TeamType::U18 | TeamType::U19) {
            let people_names = data.names_by_country.iter()
                .find(|n| n.country_id == country_id)
                .map(|n| PeopleNameGeneratorData {
                    first_names: n.first_names.clone(),
                    last_names: n.last_names.clone(),
                    nicknames: n.nicknames.clone(),
                })
                .unwrap_or_else(|| PeopleNameGeneratorData {
                    first_names: Vec::new(),
                    last_names: Vec::new(),
                    nicknames: Vec::new(),
                });

            let now = Local::now().date_naive();

            let (min_age, max_age) = match team_type {
                TeamType::U18 => (14, 18),
                _ => (14, 19),
            };

            // World-init U18/U19 squads also flow through the reputation-
            // aware academy context — the team_reputation we already have
            // at this point used to be ignored, which is why a small club
            // with strong facility data could mint elite prospects as
            // freely as a top one. League-level prestige isn't surfaced
            // here, so we use the team's own reputation as the league
            // proxy (same correlation argument used elsewhere in the
            // codebase). Pathway reputation seeds at 50 — a fresh academy.
            let league_proxy = team_reputation;
            let pathway_seed = 50;
            let staff_youth_quality_proxy = academy_quality; // best-known proxy at world-init time
            let gen_ctx = AcademyGenerationContext::from_components(
                academy_level,
                youth_quality,
                academy_quality,
                recruitment_quality,
                staff_youth_quality_proxy,
                team_reputation,
                league_proxy,
                country_reputation,
                pathway_seed,
            );
            let mut intake_state = AcademyIntakeState::new();

            let (gk_range, def_range, mid_range, st_range) = ((3, 5), (4, 9), (6, 10), (3, 6));

            let emit = |pos: core::PlayerPositionType,
                        players: &mut Vec<Player>,
                        state: &mut AcademyIntakeState| {
                players.push(CorePlayerGenerator::generate_with_context(
                    country_id,
                    now,
                    pos,
                    &people_names,
                    &gen_ctx,
                    min_age,
                    max_age,
                    Some(state),
                ));
            };

            for _ in 0..IntegerUtils::random(gk_range.0, gk_range.1) {
                emit(core::PlayerPositionType::Goalkeeper, &mut players, &mut intake_state);
            }
            for _ in 0..IntegerUtils::random(def_range.0, def_range.1) {
                let pos = match IntegerUtils::random(0, 4) {
                    0 => core::PlayerPositionType::DefenderLeft,
                    1 => core::PlayerPositionType::DefenderRight,
                    _ => core::PlayerPositionType::DefenderCenter,
                };
                emit(pos, &mut players, &mut intake_state);
            }
            for _ in 0..IntegerUtils::random(mid_range.0, mid_range.1) {
                let pos = match IntegerUtils::random(0, 3) {
                    0 => core::PlayerPositionType::DefensiveMidfielder,
                    1 => core::PlayerPositionType::MidfielderLeft,
                    2 => core::PlayerPositionType::MidfielderRight,
                    _ => core::PlayerPositionType::MidfielderCenter,
                };
                emit(pos, &mut players, &mut intake_state);
            }
            for _ in 0..IntegerUtils::random(st_range.0, st_range.1) {
                let pos = match IntegerUtils::random(0, 2) {
                    0 => core::PlayerPositionType::Striker,
                    1 => core::PlayerPositionType::ForwardLeft,
                    _ => core::PlayerPositionType::ForwardRight,
                };
                emit(pos, &mut players, &mut intake_state);
            }

            return players;
        }

        // Age range based on team type
        let (min_age, max_age) = match team_type {
            TeamType::U20 => (16, 20),
            TeamType::U21 => (16, 21),
            TeamType::U23 => (17, 23),
            TeamType::B | TeamType::Second | TeamType::Reserve => (17, 28),
            TeamType::Main => (17, 35),
            _ => (15, 18),
        };

        // League reputation drives the new senior generator. Falls back to
        // team reputation when the team has no league (rare; covers cases
        // like satellite squads without a league_id assigned).
        let league_reputation = league_id
            .and_then(|lid| data.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(team_reputation);

        let foreign_players: &[ForeignPlayerEntry] = league_id
            .and_then(|lid| data.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.foreign_players.as_slice())
            .unwrap_or(&[]);

        let total_foreign_weight: i32 = foreign_players.iter().map(|fp| fp.weight as i32).sum();

        let domestic_continent_id = data.countries.iter()
            .find(|c| c.id == country_id)
            .map(|c| c.continent_id)
            .unwrap_or(1);

        let mut roles = PositionRoleQueue::for_team(*team_type);

        let mut generate_one = |pos: PositionType| -> Player {
            let role = roles.next(pos);

            if total_foreign_weight > 0 {
                let roll = IntegerUtils::random(0, 100);
                if roll < total_foreign_weight {
                    let mut acc = 0i32;
                    for fp in foreign_players {
                        acc += fp.weight as i32;
                        if roll < acc {
                            let names = data.names_by_country.iter().find(|n| n.country_id == fp.country_id);
                            let people_names = match names {
                                Some(n) => PeopleNameGeneratorData {
                                    first_names: n.first_names.clone(),
                                    last_names: n.last_names.clone(),
                                    nicknames: n.nicknames.clone(),
                                },
                                None => PeopleNameGeneratorData {
                                    first_names: Vec::new(),
                                    last_names: Vec::new(),
                                    nicknames: Vec::new(),
                                },
                            };
                            let foreign_gen = PlayerGenerator::with_people_names(&people_names);
                            let foreign_country = data.countries.iter()
                                .find(|c| c.id == fp.country_id);
                            let foreign_country_rep = foreign_country.map(|c| c.reputation).unwrap_or(3000);
                            let foreign_continent_id = foreign_country.map(|c| c.continent_id).unwrap_or(1);
                            return foreign_gen.generate(
                                fp.country_id,
                                foreign_continent_id,
                                pos,
                                team_reputation,
                                league_reputation,
                                foreign_country_rep,
                                *team_type,
                                role,
                                min_age,
                                max_age,
                            );
                        }
                    }
                }
            }
            player_generator.generate(
                country_id,
                domestic_continent_id,
                pos,
                team_reputation,
                league_reputation,
                country_reputation,
                *team_type,
                role,
                min_age,
                max_age,
            )
        };

        // Main teams need larger squads to avoid fielding fewer than 11 after
        // injuries, bans, international duty, and condition drops.
        let (gk_range, def_range, mid_range, st_range) = match team_type {
            TeamType::Main => ((3, 5), (6, 9), (7, 10), (5, 8)),
            _ => ((3, 5), (4, 9), (6, 10), (3, 6)),
        };

        for _ in 0..IntegerUtils::random(gk_range.0, gk_range.1) {
            players.push(generate_one(PositionType::Goalkeeper));
        }
        for _ in 0..IntegerUtils::random(def_range.0, def_range.1) {
            players.push(generate_one(PositionType::Defender));
        }
        for _ in 0..IntegerUtils::random(mid_range.0, mid_range.1) {
            players.push(generate_one(PositionType::Midfielder));
        }
        for _ in 0..IntegerUtils::random(st_range.0, st_range.1) {
            players.push(generate_one(PositionType::Striker));
        }

        // Ensure main teams always have at least 25 players
        if *team_type == TeamType::Main {
            let positions = [PositionType::Defender, PositionType::Midfielder, PositionType::Striker];
            let mut pos_idx = 0;
            while players.len() < 25 {
                players.push(generate_one(positions[pos_idx % positions.len()]));
                pos_idx += 1;
            }
        }

        players
    }
}
