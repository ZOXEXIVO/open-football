use crate::Tactics;
use crate::club::staff::CoachMatchSnapshot;
use crate::r#match::ball::Ball;
use crate::r#match::{
    FieldSquad, MatchFieldSize, MatchPlayer, MatchSquad, POSITION_POSITIONING, PlayerSide,
    PositionType, TransitionSource,
};
use nalgebra::Vector3;

pub struct MatchField {
    pub size: MatchFieldSize,
    pub ball: Ball,
    pub players: Vec<MatchPlayer>,
    pub substitutes: Vec<MatchPlayer>,

    pub home_team_id: u32,
    pub away_team_id: u32,

    pub left_side_players: Option<FieldSquad>,
    pub left_team_tactics: Tactics,

    pub right_side_players: Option<FieldSquad>,
    pub right_team_tactics: Tactics,

    /// Live-match snapshot of each side's head coach. Carries the
    /// memory store, perception profile, and strategy needed by the
    /// substitution layer's coach-aware pair scorer. `None` when the
    /// squad was built outside the real club flow (tests / dev_match
    /// / wire-format reconstruction) — the substitution layer then
    /// falls back to the legacy memory-less scoring.
    ///
    /// Stored on `MatchField` rather than `MatchContext` because the
    /// snapshot is fundamentally tied to the squads' identities, and
    /// `MatchField::new` is the construction point that already
    /// consumes the `MatchSquad` values; `MatchContext::new` borrows
    /// the field afterwards and can read these without an extra
    /// parameter.
    pub home_coach_snapshot: Option<CoachMatchSnapshot>,
    pub away_coach_snapshot: Option<CoachMatchSnapshot>,
}

impl MatchField {
    pub fn new(
        width: usize,
        height: usize,
        left_team_squad: MatchSquad,
        right_team_squad: MatchSquad,
    ) -> Self {
        let home_team_id = left_team_squad.team_id;
        let away_team_id = right_team_squad.team_id;

        let left_squad = FieldSquad::from_team(&left_team_squad);
        let away_squad = FieldSquad::from_team(&right_team_squad);

        let left_tactics = left_team_squad.tactics.clone();
        let right_tactics = right_team_squad.tactics.clone();

        // Snapshots taken before the squads are consumed by the
        // player-setup pass. Cheap: the inner memory map is bounded
        // by the player roster the coach has observed.
        let home_coach_snapshot = left_team_squad.coach_snapshot.clone();
        let away_coach_snapshot = right_team_squad.coach_snapshot.clone();

        let (players_on_field, substitutes) =
            setup_player_on_field(left_team_squad, right_team_squad);

        let field = MatchField {
            size: MatchFieldSize::new(width, height),
            ball: Ball::with_coord(width as f32, height as f32),
            players: players_on_field,
            substitutes,
            home_team_id,
            away_team_id,
            left_side_players: Some(left_squad),
            left_team_tactics: left_tactics,
            right_side_players: Some(away_squad),
            right_team_tactics: right_tactics,
            home_coach_snapshot,
            away_coach_snapshot,
        };

        field
    }

    /// Borrow the coach snapshot for the given `team_id`, if any.
    /// Centralises the home/away mapping so the substitution layer
    /// reads by team rather than by side.
    pub fn coach_snapshot_for_team(&self, team_id: u32) -> Option<&CoachMatchSnapshot> {
        if team_id == self.home_team_id {
            self.home_coach_snapshot.as_ref()
        } else if team_id == self.away_team_id {
            self.away_coach_snapshot.as_ref()
        } else {
            None
        }
    }

    pub fn reset_players_positions(&mut self) {
        self.players.iter_mut().for_each(|p| {
            p.position = p.start_position;
            p.velocity = Vector3::zeros();

            // Formation rebuild on a restart — resets each player's state
            // timer along with the state (via `transition_to`).
            p.set_default_state(TransitionSource::Reset);
        });
    }

    /// Compact the remaining players of `team_id` after a red card.
    /// Squeezes each player's `start_position` ~15% toward the team's
    /// own-goal line and narrows them laterally by ~10%. This is a
    /// cheap proxy for dropping from 4-4-2 to 4-4-1 / 4-3-2: players
    /// hold a lower, tighter shape. New positions apply from the
    /// next reset/kickoff and feed the state machines' "return to
    /// starting line" heuristics.
    pub fn compact_after_dismissal(&mut self, team_id: u32) {
        let field_width = self.size.width as f32;
        let field_height = self.size.height as f32;
        let mid_y = field_height * 0.5;

        // Decide which goal line this team defends by averaging
        // non-sent-off teammates' X. The closer to 0, the left goal.
        let (sum_x, count) = self.players.iter().fold((0.0f32, 0u32), |acc, p| {
            if p.team_id == team_id && !p.is_sent_off {
                (acc.0 + p.start_position.x, acc.1 + 1)
            } else {
                acc
            }
        });
        if count == 0 {
            return;
        }
        let avg_x = sum_x / count as f32;
        let own_goal_x = if avg_x < field_width * 0.5 {
            0.0
        } else {
            field_width
        };

        for p in self.players.iter_mut() {
            if p.team_id != team_id || p.is_sent_off {
                continue;
            }
            // Move start_position 15% of the way toward own goal X,
            // and 10% toward the vertical center.
            let new_x = p.start_position.x + (own_goal_x - p.start_position.x) * 0.15;
            let new_y = p.start_position.y + (mid_y - p.start_position.y) * 0.10;
            p.start_position.x = new_x;
            p.start_position.y = new_y;
        }
    }

    pub fn swap_squads(&mut self) {
        std::mem::swap(&mut self.left_side_players, &mut self.right_side_players);
        std::mem::swap(&mut self.left_team_tactics, &mut self.right_team_tactics);

        self.players.iter_mut().for_each(|p| {
            if let Some(side) = &p.side {
                let new_side = match side {
                    PlayerSide::Left => PlayerSide::Right,
                    PlayerSide::Right => PlayerSide::Left,
                };
                p.side = Some(new_side);
                p.tactical_position.regenerate_waypoints(Some(new_side));
                p.rebuild_waypoint_cache();

                if let Some(new_pos) = get_player_position(p, new_side) {
                    p.start_position = new_pos;
                }
            }
        });

        // Bench players must swap sides too. They sit in the live
        // position store with their `side` field, and the side-based
        // roster scans (loose-ball force/yield) classify them by it —
        // leaving the bench on first-half sides made every bench player
        // register as the WRONG team's "teammate" for the entire second
        // half. A substitute coming on also inherits the outgoing
        // player's slot, so keeping their own side current is the
        // consistent invariant.
        self.substitutes.iter_mut().for_each(|p| {
            if let Some(side) = &p.side {
                let new_side = match side {
                    PlayerSide::Left => PlayerSide::Right,
                    PlayerSide::Right => PlayerSide::Left,
                };
                p.side = Some(new_side);
                p.tactical_position.regenerate_waypoints(Some(new_side));
                p.rebuild_waypoint_cache();
            }
        });
    }

    pub fn get_player(&mut self, id: u32) -> Option<&MatchPlayer> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn get_player_mut(&mut self, id: u32) -> Option<&mut MatchPlayer> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    /// Single-pass linear lookup for a player's index in
    /// `self.players`. Cheaper than two `iter().find()` calls when the
    /// caller wants both keeper+shooter (or any pair) — see
    /// `two_player_indices`.
    #[inline]
    pub fn player_index(&self, id: u32) -> Option<usize> {
        self.players.iter().position(|p| p.id == id)
    }

    /// One pass over the player list to resolve two ids to indices.
    /// Returns `Some((idx_a, idx_b))` only when both are present.
    /// Order in the returned tuple matches the order of the arguments,
    /// independent of pitch ordering.
    #[inline]
    pub fn two_player_indices(&self, a: u32, b: u32) -> Option<(usize, usize)> {
        let mut idx_a: Option<usize> = None;
        let mut idx_b: Option<usize> = None;
        for (i, p) in self.players.iter().enumerate() {
            if idx_a.is_none() && p.id == a {
                idx_a = Some(i);
                if idx_b.is_some() {
                    break;
                }
            } else if idx_b.is_none() && p.id == b {
                idx_b = Some(i);
                if idx_a.is_some() {
                    break;
                }
            }
        }
        match (idx_a, idx_b) {
            (Some(ia), Some(ib)) => Some((ia, ib)),
            _ => None,
        }
    }

    /// Two-player mutable borrow via `split_at_mut`, safe when the
    /// indices differ. Returns `None` if the ids resolve to the same
    /// player or either is missing. Order of the returned references
    /// matches the order of `a, b`.
    pub fn two_players_mut(
        &mut self,
        a: u32,
        b: u32,
    ) -> Option<(&mut MatchPlayer, &mut MatchPlayer)> {
        let (ia, ib) = self.two_player_indices(a, b)?;
        if ia == ib {
            return None;
        }
        let (lo, hi) = if ia < ib { (ia, ib) } else { (ib, ia) };
        let (left, right) = self.players.split_at_mut(hi);
        let lo_ref = &mut left[lo];
        let hi_ref = &mut right[0];
        if ia < ib {
            Some((lo_ref, hi_ref))
        } else {
            Some((hi_ref, lo_ref))
        }
    }

    pub fn substitute_player(&mut self, player_out_id: u32, player_in_id: u32) -> bool {
        // Find the outgoing player's position info
        let out_info = match self.players.iter().find(|p| p.id == player_out_id) {
            Some(p) => (
                p.side,
                p.tactical_position.current_position,
                p.start_position,
            ),
            None => return false,
        };

        let (side, position, start_pos) = out_info;

        // Find and remove the substitute from the bench
        let sub_idx = match self.substitutes.iter().position(|p| p.id == player_in_id) {
            Some(idx) => idx,
            None => return false,
        };

        let mut player_in = self.substitutes.remove(sub_idx);

        // Set up the substitute with the outgoing player's tactical role
        player_in.side = side;
        player_in.tactical_position.current_position = position;
        player_in.tactical_position.regenerate_waypoints(side);
        player_in.start_position = start_pos;
        player_in.position = start_pos;
        player_in.set_default_state(TransitionSource::Substitution);

        // Replace the outgoing player in the field
        if let Some(out_slot) = self.players.iter_mut().find(|p| p.id == player_out_id) {
            *out_slot = player_in;

            // Clear any ball references to the substituted-out player
            self.ball.clear_player_reference(player_out_id);

            true
        } else {
            false
        }
    }
}

fn setup_player_on_field(
    left_team_squad: MatchSquad,
    right_team_squad: MatchSquad,
) -> (Vec<MatchPlayer>, Vec<MatchPlayer>) {
    let setup_squad = |squad: MatchSquad, side: PlayerSide| {
        let mut players = Vec::with_capacity(squad.main_squad.len());
        let mut subs = Vec::with_capacity(squad.substitutes.len());

        for mut player in squad.main_squad {
            player.side = Some(side);
            player.tactical_position.regenerate_waypoints(Some(side));
            player.rebuild_waypoint_cache();
            if let Some(position) = get_player_position(&player, side) {
                player.position = position;
                player.start_position = position;
                players.push(player);
            }
        }

        for mut player in squad.substitutes {
            player.side = Some(side);
            player.tactical_position.regenerate_waypoints(Some(side));
            player.rebuild_waypoint_cache();
            // Bench players are stashed at the same off-pitch sentinel as
            // sent-off players — NOT at an on-pitch coordinate. They are
            // part of the live position store (`PlayerFieldData` chains
            // `field.substitutes`), so an on-pitch position here (the old
            // (1.0, 1.0) — the pitch corner) made every distance-based
            // roster scan see up to 14 phantom players stacked at that
            // corner: the loose-ball "am I closest" veto could pick a
            // bench player and nobody would chase a ball rolling there.
            // Far off-pitch, they can never win a distance comparison.
            player.position = Vector3::new(-500.0, -500.0, 0.0);
            subs.push(player);
        }

        (players, subs)
    };

    let left_main_len = left_team_squad.main_squad.len();
    let right_main_len = right_team_squad.main_squad.len();
    let left_sub_len = left_team_squad.substitutes.len();
    let right_sub_len = right_team_squad.substitutes.len();

    let (mut left_players, mut left_subs) = setup_squad(left_team_squad, PlayerSide::Left);
    let (right_players, right_subs) = setup_squad(right_team_squad, PlayerSide::Right);

    // Preallocate combined buffers and extend in place — the previous
    // `[a, b].concat()` round-tripped through a temporary array AND
    // allocated a fresh Vec for each output.
    let mut players = Vec::with_capacity(left_main_len + right_main_len);
    players.append(&mut left_players);
    players.extend(right_players);

    let mut substitutes = Vec::with_capacity(left_sub_len + right_sub_len);
    substitutes.append(&mut left_subs);
    substitutes.extend(right_subs);

    (players, substitutes)
}

fn get_player_position(player: &MatchPlayer, side: PlayerSide) -> Option<Vector3<f32>> {
    POSITION_POSITIONING
        .iter()
        .find(|(pos, _, _)| *pos == player.tactical_position.current_position)
        .and_then(|(_, home, away)| match side {
            PlayerSide::Left => {
                if let PositionType::Home(x, y) = home {
                    Some((*x as f32, *y as f32))
                } else {
                    None
                }
            }
            PlayerSide::Right => {
                if let PositionType::Away(x, y) = away {
                    Some((*x as f32, *y as f32))
                } else {
                    None
                }
            }
        })
        .map(|(x, y)| Vector3::new(x, y, 0.0))
}
