//! Wire DTOs for the worker protocol. Wraps `core::r#match::MatchSquad`
//! / `MatchPlayer` (which carry engine runtime state that is undefined
//! at squad-build time) in a flat, wire-friendly form. Everything
//! that already serde-derives in core is held inline — only the few
//! engine-runtime fields are stripped and re-initialised to defaults
//! on the worker side via the inverse `into_squad` / `into_player`
//! conversions.
//!
//! Result types (`MatchResultRaw`, `MatchResult`, `Score`, …) all
//! serde-derive directly in core, so they cross the wire without a
//! parallel DTO.

use core::club::player::mind::KickoffMind;
use core::club::player::traits::PlayerTrait;
use core::r#match::{
    CoachCard, FixtureContext, Match, MatchPlayer, MatchSquad, OmittedPlayer, PlayerSide,
};
use core::{
    FormationFamiliarity, PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills,
    RoleLevels, Tactics,
};
use serde::{Deserialize, Serialize};

/// Wire image of a `MatchSquad`. Captain / vice / penalty-taker /
/// free-kick-taker travel as the player ids the squad holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquadWire {
    pub team_id: u32,
    pub team_name: String,
    pub tactics: Tactics,
    pub main_squad: Vec<PlayerWire>,
    pub substitutes: Vec<PlayerWire>,
    pub captain_id: Option<u32>,
    pub vice_captain_id: Option<u32>,
    pub penalty_taker_id: Option<u32>,
    pub free_kick_taker_id: Option<u32>,
    pub selection_omissions: Vec<OmittedPlayer>,
    /// The head coach's card, so a remote match is managed by the same
    /// coach as a local one.
    pub coach_card: Option<CoachCard>,
    /// The side's familiarity with each formation, earned club-side from
    /// its record.
    pub familiarity: Option<FormationFamiliarity>,
}

/// Wire image of a `MatchPlayer`. Only fields that the engine reads at
/// squad-build time travel over the wire — runtime state (memory,
/// waypoints, statistics, condition accumulator, in-state timers, …)
/// is reinitialised to defaults on the worker via `into_player` so the
/// match starts from a clean engine state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerWire {
    pub id: u32,
    pub team_id: u32,
    pub position: [f32; 3],
    pub start_position: [f32; 3],
    pub attributes: PersonAttributes,
    pub player_attributes: PlayerAttributes,
    pub skills: PlayerSkills,
    pub tactical_position: PlayerPositionType,
    pub side: Option<PlayerSide>,
    pub traits: Vec<PlayerTrait>,
    pub birth_date: chrono::NaiveDate,
    pub is_force_match_selection: bool,
    pub starting_condition: i16,
    pub starting_recovery_debt: f32,
    /// Pre-match settledness stamp (transfer settling × rustiness) —
    /// derived club-side, so it must travel with the player.
    pub settledness: f32,
    /// Pre-match form draw — the day-to-day performance multiplier.
    /// Derived club-side (it needs the matchday calendar and the
    /// persisted `Player`), so it crosses the wire as a plain value.
    pub matchday_form: f32,
    /// Kickoff state of mind — assurance, belief, morale — read club-side
    /// from the persisted `Player`, so it crosses the wire as a value.
    pub kickoff_mind: KickoffMind,
    /// His level in every role, his belief in the coach's plan and the
    /// team-mates he has started alongside, all read club-side.
    pub role_levels: RoleLevels,
    pub buy_in: f32,
    pub started_with: Option<Vec<u32>>,
    pub use_extended_state_logging: bool,
}

/// Wire image of a `core::r#match::Match` (league fixture). Carries
/// the same identity + flags the call site supplied and a pair of
/// `SquadWire`s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeagueMatchWire {
    pub id: String,
    pub league_id: u32,
    pub league_slug: String,
    pub fixture: FixtureContext,
    pub home: SquadWire,
    pub away: SquadWire,
}

/// Wire image of a raw squad-vs-squad fixture (national / international).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquadFixtureWire {
    pub idx: usize,
    pub fixture: FixtureContext,
    pub home: SquadWire,
    pub away: SquadWire,
}

// Conversions ─────────────────────────────────────────────────────────

impl PlayerWire {
    pub fn from_player(p: &MatchPlayer) -> Self {
        PlayerWire {
            id: p.id,
            team_id: p.team_id,
            position: [p.position.x, p.position.y, p.position.z],
            start_position: [p.start_position.x, p.start_position.y, p.start_position.z],
            attributes: p.attributes,
            player_attributes: p.player_attributes,
            skills: p.skills,
            tactical_position: p.tactical_position.current_position,
            side: p.side,
            traits: p.traits.clone(),
            birth_date: p.birth_date,
            is_force_match_selection: p.is_force_match_selection,
            starting_condition: p.starting_condition,
            starting_recovery_debt: p.starting_recovery_debt,
            settledness: p.settledness,
            matchday_form: p.matchday_form,
            kickoff_mind: p.kickoff_mind,
            role_levels: p.role_levels,
            buy_in: p.buy_in,
            started_with: p.started_with.clone(),
            use_extended_state_logging: p.use_extended_state_logging,
        }
    }

    pub fn into_player(self) -> MatchPlayer {
        let PlayerWire {
            id,
            team_id,
            position,
            start_position,
            attributes,
            player_attributes,
            skills,
            tactical_position,
            side,
            traits,
            birth_date,
            is_force_match_selection,
            starting_condition,
            starting_recovery_debt,
            settledness,
            matchday_form,
            kickoff_mind,
            role_levels,
            buy_in,
            started_with,
            use_extended_state_logging,
        } = self;
        let mut player = MatchPlayer::from_inputs(
            id,
            team_id,
            position,
            start_position,
            attributes,
            player_attributes,
            skills,
            tactical_position,
            side,
            traits,
            birth_date,
            is_force_match_selection,
            starting_condition,
            starting_recovery_debt,
            settledness,
            matchday_form,
            kickoff_mind,
            role_levels,
            buy_in,
            use_extended_state_logging,
        );
        player.started_with = started_with;
        player
    }
}

impl SquadWire {
    pub fn from_squad(s: &MatchSquad) -> Self {
        SquadWire {
            team_id: s.team_id,
            team_name: s.team_name.clone(),
            tactics: s.tactics.clone(),
            main_squad: s.main_squad.iter().map(PlayerWire::from_player).collect(),
            substitutes: s.substitutes.iter().map(PlayerWire::from_player).collect(),
            captain_id: s.captain_id,
            vice_captain_id: s.vice_captain_id,
            penalty_taker_id: s.penalty_taker_id,
            free_kick_taker_id: s.free_kick_taker_id,
            selection_omissions: s.selection_omissions.clone(),
            coach_card: s.coach_card,
            familiarity: s.familiarity,
        }
    }

    pub fn into_squad(self) -> MatchSquad {
        let SquadWire {
            team_id,
            team_name,
            tactics,
            main_squad,
            substitutes,
            captain_id,
            vice_captain_id,
            penalty_taker_id,
            free_kick_taker_id,
            selection_omissions,
            coach_card,
            familiarity,
        } = self;

        let main: Vec<MatchPlayer> = main_squad
            .into_iter()
            .map(PlayerWire::into_player)
            .collect();
        let subs: Vec<MatchPlayer> = substitutes
            .into_iter()
            .map(PlayerWire::into_player)
            .collect();
        MatchSquad {
            team_id,
            team_name,
            tactics,
            captain_id,
            vice_captain_id,
            penalty_taker_id,
            free_kick_taker_id,
            main_squad: main,
            substitutes: subs,
            selection_omissions,
            overlooked: Vec::new(),
            // The substitution memory store stays club-side: distributed
            // match runs score changes without it.
            coach_snapshot: None,
            coach_card,
            familiarity,
        }
    }
}

impl LeagueMatchWire {
    /// Borrow-and-clone constructor. The caller keeps ownership of
    /// `m`, which is essential for the dispatcher's local-fallback
    /// path — if the worker errors mid-batch we replay the original
    /// `Match` on the local rayon pool rather than synthesising a
    /// placeholder.
    pub fn from_match(m: &Match) -> Self {
        LeagueMatchWire {
            id: m.id().to_string(),
            league_id: m.league_id(),
            league_slug: m.league_slug().to_string(),
            fixture: m.fixture,
            home: SquadWire::from_squad(&m.home_squad),
            away: SquadWire::from_squad(&m.away_squad),
        }
    }

    pub fn into_match(self) -> Match {
        let LeagueMatchWire {
            id,
            league_id,
            league_slug,
            fixture,
            home,
            away,
        } = self;
        Match::make(
            id,
            league_id,
            &league_slug,
            home.into_squad(),
            away.into_squad(),
            fixture,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::transport::Frame;
    use chrono::NaiveDate;
    use core::r#match::{FootballEngine, MatchEngineConfig, MatchResultRaw};
    use core::{
        AcademyGenerationContext, MatchTacticType, PeopleNameGeneratorData, PlayerGenerator,
        PlayerPositionType,
    };
    use std::fmt::Write as _;

    fn player(id: u32, team_id: u32, position: PlayerPositionType) -> MatchPlayer {
        let names = PeopleNameGeneratorData {
            first_names: Vec::new(),
            last_names: Vec::new(),
            nicknames: Vec::new(),
        };
        let mut player = PlayerGenerator::generate_with_context(
            1,
            "",
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            position,
            &names,
            &AcademyGenerationContext::average(),
            25,
            28,
            None,
        );
        player.id = id;
        MatchPlayer::from_player(team_id, &player, position, false, None)
    }

    fn squad(team_id: u32, coach_card: Option<CoachCard>) -> MatchSquad {
        const BENCH: [PlayerPositionType; 5] = [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ];
        let tactics = Tactics::new(MatchTacticType::T442);
        let main_squad = tactics
            .positions()
            .iter()
            .enumerate()
            .map(|(i, &pos)| player(team_id * 100 + i as u32, team_id, pos))
            .collect();
        let substitutes = BENCH
            .iter()
            .enumerate()
            .map(|(i, &pos)| player(team_id * 100 + 20 + i as u32, team_id, pos))
            .collect();
        MatchSquad {
            team_id,
            team_name: format!("side {team_id}"),
            tactics,
            main_squad,
            substitutes,
            captain_id: None,
            vice_captain_id: None,
            penalty_taker_id: None,
            free_kick_taker_id: None,
            selection_omissions: Vec::new(),
            overlooked: Vec::new(),
            coach_snapshot: None,
            coach_card,
            familiarity: None,
        }
    }

    fn across_the_wire(squad: &MatchSquad) -> MatchSquad {
        let bytes = Frame::encode(&SquadWire::from_squad(squad)).unwrap();
        Frame::decode::<SquadWire>(&bytes).unwrap().into_squad()
    }

    /// Score, every player's line in player order, the changes made and how
    /// each side was managed.
    fn outcome(raw: &MatchResultRaw) -> String {
        let mut out = format!("{:?}|{:?}|{:?}", raw.score, raw.coach_home, raw.coach_away);
        let mut stats: Vec<_> = raw.player_stats.iter().collect();
        stats.sort_by_key(|(id, _)| **id);
        for (id, line) in stats {
            let _ = write!(out, "|{id}:{line:?}");
        }
        for change in &raw.substitutions {
            let _ = write!(
                out,
                "|sub {} {}->{} at {}",
                change.team_id, change.player_out_id, change.player_in_id, change.match_time_ms
            );
        }
        out
    }

    #[test]
    fn a_squad_crosses_the_wire_with_its_coach_card() {
        let card = CoachCard::from_pin("risk=0.83/caution=0.12/press=0.71/block=0.66").unwrap();
        let back = across_the_wire(&squad(1, Some(card))).coach_card.unwrap();
        let bits = |c: &CoachCard| {
            let t = &c.temperament;
            let mut v = vec![
                t.risk_tolerance,
                t.caution,
                t.stubbornness,
                t.adaptability,
                t.volatility,
                t.read,
                t.man_management,
                t.motivating,
            ];
            v.extend(c.preference.dials());
            v.into_iter().map(f32::to_bits).collect::<Vec<_>>()
        };
        assert_eq!(bits(&back), bits(&card));
        assert_eq!(back.style, card.style);
        assert_eq!(across_the_wire(&squad(2, None)).coach_card, None);
    }

    #[test]
    fn a_squad_crosses_the_wire_with_its_familiarity_and_its_players_belief() {
        let mut home = squad(1, None);
        home.familiarity = Some(FormationFamiliarity::settled(MatchTacticType::T433));
        home.main_squad[3].buy_in = 0.83;
        home.main_squad[4].started_with = Some(vec![7, 9]);
        let back = across_the_wire(&home);
        assert_eq!(back.familiarity, home.familiarity);
        for (sent, came) in home
            .main_squad
            .iter()
            .chain(&home.substitutes)
            .zip(back.main_squad.iter().chain(&back.substitutes))
        {
            assert_eq!(came.role_levels, sent.role_levels);
            assert_eq!(came.buy_in.to_bits(), sent.buy_in.to_bits());
            assert_eq!(came.started_with, sent.started_with);
        }
    }

    #[test]
    fn a_fixture_plays_the_same_locally_and_through_the_wire() {
        // A sharp, adaptable man in the home dugout, so the plan changes in
        // play as well as the talks.
        let mut card = CoachCard::from_pin("risktaker").unwrap();
        card.temperament.read = 1.0;
        card.temperament.adaptability = 1.0;
        card.temperament.stubbornness = 0.0;
        let (home, away) = (squad(1, Some(card)), squad(2, None));
        let (remote_home, remote_away) = (across_the_wire(&home), across_the_wire(&away));
        let local = FootballEngine::<840, 545>::play_with_config(
            home,
            away,
            MatchEngineConfig::seeded(0x7EA4_9A81),
        );
        let remote = FootballEngine::<840, 545>::play_with_config(
            remote_home,
            remote_away,
            MatchEngineConfig::seeded(0x7EA4_9A81),
        );
        assert!(
            local.coach_home.half_time_talk.is_some() && local.coach_away.half_time_talk.is_some(),
            "both coaches gave their talk"
        );
        assert!(
            !local.coach_home.changes.is_empty(),
            "a plan change is in play"
        );
        assert_eq!(outcome(&local), outcome(&remote));
    }
}
