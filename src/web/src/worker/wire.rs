//! Wire DTOs for the worker protocol. Wraps `core::r#match::MatchSquad`
//! / `MatchPlayer` (which carry engine runtime state that is undefined
//! at squad-build time) in a flat, bincode-friendly form. Everything
//! that already serde-derives in core is held inline — only the few
//! engine-runtime fields are stripped and re-initialised to defaults
//! on the worker side via the inverse `into_squad` / `into_player`
//! conversions.
//!
//! Result types (`MatchResultRaw`, `MatchResult`, `Score`, …) all
//! serde-derive directly in core, so they cross the wire without a
//! parallel DTO.

use core::club::player::traits::PlayerTrait;
use core::r#match::{Match, MatchPlayer, MatchSquad, OmittedPlayer, PlayerSide};
use core::{PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills, Tactics};
use serde::{Deserialize, Serialize};

/// Wire image of a `MatchSquad`. Captain / vice / penalty-taker /
/// free-kick-taker are stored as player ids and resolved against the
/// rebuilt main_squad on the worker side — sending the full
/// `MatchPlayer` clones the engine actually uses would just duplicate
/// data already on the wire.
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
    pub is_friendly: bool,
    pub is_knockout: bool,
    pub home: SquadWire,
    pub away: SquadWire,
}

/// Wire image of a raw squad-vs-squad fixture (national / international).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquadFixtureWire {
    pub idx: usize,
    pub is_knockout: bool,
    pub home: SquadWire,
    pub away: SquadWire,
}

/// Conversions ─────────────────────────────────────────────────────────

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
            use_extended_state_logging,
        } = self;
        MatchPlayer::from_inputs(
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
            use_extended_state_logging,
        )
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
            captain_id: s.captain_id.as_ref().map(|p| p.id),
            vice_captain_id: s.vice_captain_id.as_ref().map(|p| p.id),
            penalty_taker_id: s.penalty_taker_id.as_ref().map(|p| p.id),
            free_kick_taker_id: s.free_kick_taker_id.as_ref().map(|p| p.id),
            selection_omissions: s.selection_omissions.clone(),
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
        } = self;

        let main: Vec<MatchPlayer> = main_squad.into_iter().map(PlayerWire::into_player).collect();
        let subs: Vec<MatchPlayer> = substitutes.into_iter().map(PlayerWire::into_player).collect();
        let lookup = |maybe_id: Option<u32>| -> Option<MatchPlayer> {
            maybe_id.and_then(|id| main.iter().find(|p| p.id == id).cloned())
        };

        MatchSquad {
            team_id,
            team_name,
            tactics,
            captain_id: lookup(captain_id),
            vice_captain_id: lookup(vice_captain_id),
            penalty_taker_id: lookup(penalty_taker_id),
            free_kick_taker_id: lookup(free_kick_taker_id),
            main_squad: main,
            substitutes: subs,
            selection_omissions,
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
            is_friendly: m.is_friendly,
            is_knockout: m.is_knockout,
            home: SquadWire::from_squad(&m.home_squad),
            away: SquadWire::from_squad(&m.away_squad),
        }
    }

    pub fn into_match(self) -> Match {
        let LeagueMatchWire {
            id,
            league_id,
            league_slug,
            is_friendly,
            is_knockout,
            home,
            away,
        } = self;
        let home = home.into_squad();
        let away = away.into_squad();
        if is_knockout {
            Match::make_knockout(id, league_id, &league_slug, home, away)
        } else {
            let mut m = Match::make(id, league_id, &league_slug, home, away, is_friendly);
            // `Match::make` always sets is_knockout = false; respect the
            // value we got over the wire even when is_friendly is true
            // (defensive — shouldn't happen, but keeps round-tripping
            // semantically exact).
            m.is_knockout = is_knockout;
            m
        }
    }
}

