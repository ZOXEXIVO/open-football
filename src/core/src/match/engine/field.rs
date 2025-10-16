use crate::r#match::ball::Ball;
use crate::r#match::{FieldSquad, MatchFieldSize, MatchPlayer, MatchSquad, PlayerSide, PositionType, POSITION_POSITIONING};
use crate::Tactics;
use nalgebra::Vector3;

pub struct MatchField {
    pub size: MatchFieldSize,
    pub ball: Ball,
    pub players: Vec<MatchPlayer>,
    pub substitutes: Vec<MatchPlayer>,

    pub left_side_players: Option<FieldSquad>,
    pub left_team_tactics: Tactics,

    pub right_side_players: Option<FieldSquad>,
    pub right_team_tactics: Tactics,
}

impl MatchField {
    pub fn new(
        width: usize,
        height: usize,
        left_team_squad: MatchSquad,
        right_team_squad: MatchSquad,
    ) -> Self {
        let left_squad = FieldSquad::from_team(&left_team_squad);
        let away_squad = FieldSquad::from_team(&right_team_squad);

        let left_tactics = left_team_squad.tactics.clone();
        let right_tactics = right_team_squad.tactics.clone();

        let (players_on_field, substitutes) =
            setup_player_on_field(left_team_squad, right_team_squad);

        MatchField {
            size: MatchFieldSize::new(width, height),
            ball: Ball::with_coord(width as f32, height as f32),
            players: players_on_field,
            substitutes,
            left_side_players: Some(left_squad),
            left_team_tactics: left_tactics,
            right_side_players: Some(away_squad),
            right_team_tactics: right_tactics,
        }
    }

    pub fn reset_players_positions(&mut self) {
        self.players.iter_mut().for_each(|p| {
            p.position = p.start_position;
            p.velocity = Vector3::zeros();

            p.set_default_state();
        });
    }

    pub fn swap_squads(&mut self) {
        std::mem::swap(&mut self.left_side_players, &mut self.right_side_players);

        self.players.iter_mut().for_each(|p| {
            if let Some(side) = &p.side {
                let new_side = match side {
                    PlayerSide::Left => PlayerSide::Right,
                    PlayerSide::Right => PlayerSide::Left,
                };
                p.side = Some(new_side);
                p.tactical_position.regenerate_waypoints(Some(new_side));
            }
        });
    }

    pub fn get_player_mut(&mut self, id: u32) -> Option<&mut MatchPlayer> {
        self.players.iter_mut().find(|p| p.id == id)
    }
}

fn setup_player_on_field(
    left_team_squad: MatchSquad,
    right_team_squad: MatchSquad,
) -> (Vec<MatchPlayer>, Vec<MatchPlayer>) {
    let setup_squad = |squad: MatchSquad, side: PlayerSide| {
        let mut players = Vec::new();
        let mut subs = Vec::new();

        for mut player in squad.main_squad {
            player.side = Some(side);
            player.tactical_position.regenerate_waypoints(Some(side));
            if let Some(position) = get_player_position(&player, side) {
                player.position = position;
                player.start_position = position;
                players.push(player);
            }
        }

        for mut player in squad.substitutes {
            player.side = Some(side);
            player.tactical_position.regenerate_waypoints(Some(side));
            player.position = Vector3::new(1.0, 1.0, 0.0);
            subs.push(player);
        }

        (players, subs)
    };

    let (left_players, left_subs) = setup_squad(left_team_squad, PlayerSide::Left);
    let (right_players, right_subs) = setup_squad(right_team_squad, PlayerSide::Right);

    (
        [left_players, right_players].concat(),
        [left_subs, right_subs].concat(),
    )
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