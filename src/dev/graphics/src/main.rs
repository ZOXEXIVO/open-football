use core::r#match::VectorExtensions;
use core::r#match::ball::Ball;
use core::r#match::player::MatchPlayer;
use core::r#match::FootballEngine;
use core::r#match::MatchContext;
use core::r#match::MatchField;
use macroquad::prelude::*;
use std::thread;
use std::time::Duration;
//tactics
use core::club::player::Player;
use core::club::player::PlayerPositionType;
use core::club::team::tactics::{MatchTacticType, Tactics};
use core::r#match::squad::TeamSquad;
use core::r#match::strategies::ball::MatchBallLogic;
use core::r#match::MatchPlayerCollection;
use core::r#match::ResultMatchPositionData;
use core::Vector3;
use env_logger::Env;
use std::time::Instant;

use core::r#match::PlayerSide;
use core::r#match::Score;
use core::r#match::GOAL_WIDTH;
use core::NaiveDate;
use core::PlayerGenerator;

const INNER_FIELD_WIDTH: f32 = 840.0;
const INNER_FIELD_HEIGHT: f32 = 545.0;

#[macroquad::main(window_conf)]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();

    let width = screen_width() - 30.0;
    let height = screen_height();

    let window_aspect_ratio = width / height;
    let field_aspect_ratio = INNER_FIELD_WIDTH / INNER_FIELD_HEIGHT;

    let (field_width, field_height, scale) = if window_aspect_ratio > field_aspect_ratio {
        let scale = height / INNER_FIELD_HEIGHT;
        (INNER_FIELD_WIDTH * scale, height, scale)
    } else {
        let scale = width / INNER_FIELD_WIDTH;
        (width, INNER_FIELD_HEIGHT * scale, scale)
    };

    let offset_x = (width - field_width) / 2.0 + 20.0;
    let offset_y = (height - field_height) / 2.0 + 10.0;

    let home_squad = get_home_squad();
    let away_squad = get_away_squad();

    let players = MatchPlayerCollection::from_squads(&home_squad, &away_squad);

    let mut field = MatchField::new(
        INNER_FIELD_WIDTH as usize,
        INNER_FIELD_HEIGHT as usize,
        home_squad,
        away_squad,
    );

    let field_size = field.size.clone();

    let score = Score::new(1, 2);

    let mut context = MatchContext::new(&field_size, players, score);

    let mut current_frame = 0u64;

    let mut match_data = ResultMatchPositionData::new();

    let mut left_mouse_pressed = false;

    loop {
        current_frame += 1;

        clear_background(Color::new(255.0, 238.0, 7.0, 65.0));

        let field_color = Color::from_rgba(132, 240, 207, 255);
        let border_color = Color::from_rgba(51, 184, 144, 255);
        let border_width = 5.0;

        draw_rectangle_ex(
            offset_x,
            offset_y,
            field_width,
            field_height,
            DrawRectangleParams {
                color: field_color,
                offset: Vec2 { x: 0.0, y: 0.0 },
                rotation: 0.0,
            },
        );

        draw_rectangle_lines_ex(
            offset_x - border_width / 2.0,
            offset_y - border_width / 2.0,
            field_width + border_width,
            field_height + border_width,
            border_width,
            DrawRectangleParams {
                color: border_color,
                offset: Vec2 { x: 0.0, y: 0.0 },
                rotation: 0.0,
            },
        );

        let start = Instant::now();

        FootballEngine::<840, 545>::game_tick(&mut field, &mut context, &mut match_data);

        let elapsed = start.elapsed();

        draw_goals(offset_x, offset_y, &context, field_width, scale);
        draw_players(offset_x, offset_y, &field, field.ball.current_owner, scale);

        draw_ball(offset_x, offset_y, &field.ball, scale);

        draw_player_list(
            offset_x + 20.0,
            offset_y + field_height + 10.0,
            field.players.iter().filter(|p| p.team_id == 2).collect(),
            field.ball.current_owner,
            scale,
        );
        draw_player_list(
            offset_x + 20.0,
            offset_y - 50.0,
            field.players.iter().filter(|p| p.team_id == 1).collect(),
            field.ball.current_owner,
            scale,
        );

        // FPS
        const AVERAGE_FPS_BUCKET_SIZE: usize = 50;

        let mut max_fps: u128 = 0;

        let mut fps_data = [0u128; AVERAGE_FPS_BUCKET_SIZE];

        let fps_data_current_idx = (current_frame % AVERAGE_FPS_BUCKET_SIZE as u64) as usize;

        let elapsed_mcs = elapsed.as_micros();

        fps_data[fps_data_current_idx] = elapsed.as_micros();

        if current_frame > 100 && elapsed_mcs > max_fps {
            max_fps = elapsed_mcs;
        }

        draw_fps(offset_x, offset_y, &fps_data, max_fps);

        left_mouse_pressed = is_mouse_button_down(MouseButton::Left);

        if left_mouse_pressed {
            thread::sleep(Duration::from_millis(500));
        }

        next_frame().await;
    }
}

const TRACKING_PLAYER_ID: u32 = 111;

pub fn get_home_squad() -> TeamSquad {
    let players = [
        get_player(101, PlayerPositionType::Goalkeeper),
        get_player(102, PlayerPositionType::DefenderLeft),
        get_player(103, PlayerPositionType::DefenderCenterLeft),
        get_player(104, PlayerPositionType::DefenderCenterRight),
        get_player(105, PlayerPositionType::DefenderRight),
        get_player(106, PlayerPositionType::MidfielderLeft),
        get_player(107, PlayerPositionType::MidfielderCenterLeft),
        get_player(108, PlayerPositionType::MidfielderCenterRight),
        get_player(109, PlayerPositionType::MidfielderRight),
        get_player(110, PlayerPositionType::ForwardLeft),
        get_player(111, PlayerPositionType::ForwardRight),
    ];

    let match_players: Vec<MatchPlayer> = players
        .iter()
        .map(|player| {
            MatchPlayer::from_player(
                1,
                player,
                player.position(),
                player.id == TRACKING_PLAYER_ID,
            )
        })
        .collect();

    let home_squad = TeamSquad {
        team_id: 1,
        team_name: String::from("123"),
        tactics: Tactics::new(MatchTacticType::T442),
        main_squad: match_players,
        substitutes: Vec::new(),
    };

    home_squad
}

pub fn get_away_squad() -> TeamSquad {
    let players = [
        get_player(113, PlayerPositionType::Goalkeeper),
        get_player(114, PlayerPositionType::DefenderLeft),
        get_player(115, PlayerPositionType::DefenderCenterLeft),
        get_player(116, PlayerPositionType::DefenderCenterRight),
        get_player(117, PlayerPositionType::DefenderRight),
        get_player(118, PlayerPositionType::MidfielderLeft),
        get_player(119, PlayerPositionType::MidfielderCenterLeft),
        get_player(120, PlayerPositionType::MidfielderCenterRight),
        get_player(121, PlayerPositionType::MidfielderRight),
        get_player(122, PlayerPositionType::ForwardLeft),
        get_player(123, PlayerPositionType::ForwardRight),
    ];

    let match_players: Vec<MatchPlayer> = players
        .iter()
        .map(|player| {
            MatchPlayer::from_player(
                2,
                player,
                player.position(),
                player.id == TRACKING_PLAYER_ID,
            )
        })
        .collect();

    let away_squad = TeamSquad {
        team_id: 2,
        team_name: String::from("321"),
        tactics: Tactics::new(MatchTacticType::T442),
        main_squad: match_players,
        substitutes: Vec::new(),
    };

    away_squad
}

fn get_player(id: u32, position: PlayerPositionType) -> Player {
    let mut player = PlayerGenerator::generate(
        1,
        NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
        position,
        20,
    );

    player.id = id;

    player
}

fn average(numbers: &[u128]) -> u128 {
    let sum: u128 = numbers.iter().sum();
    let count = numbers.len() as u128;
    sum / count
}

fn player_state(player: &MatchPlayer) -> String {
    let state = player.state.to_string();

    let cleaned_state = state.split(':').nth(1).unwrap_or(&state).trim();

    cleaned_state.to_string()
}

fn distance(a: &Vector3<f32>, b: &Vector3<f32>) -> usize {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt() as usize
}

pub fn is_towards_player(
    ball_position: &Vector3<f32>,
    ball_velocity: &Vector3<f32>,
    player_position: &Vector3<f32>,
) -> (bool, f32) {
    MatchBallLogic::is_heading_towards_player(ball_position, ball_velocity, player_position, 0.95)
}

#[cfg(target_os = "macos")]
const WINDOW_WIDTH: i32 = 1040;
#[cfg(target_os = "macos")]
const WINDOW_HEIGHT: i32 = 800;

#[cfg(target_os = "windows")]
const WINDOW_WIDTH: i32 = 1948;
#[cfg(target_os = "windows")]
const WINDOW_HEIGHT: i32 = 1521;

fn window_conf() -> Conf {
    Conf {
        window_title: "FootballSimulatorTesting".to_owned(),
        window_width: WINDOW_WIDTH,
        window_height: WINDOW_HEIGHT,
        window_resizable: false,
        fullscreen: false,
        high_dpi: true,
        ..Default::default()
    }
}

// draw

fn draw_fps(offset_x: f32, offset_y: f32, fps_data: &[u128], max_fps: u128) {
    draw_text(
        &format!("FPS AVG: {} mcs", average(&fps_data)),
        offset_x + 10.0,
        offset_y + 20.0,
        20.0,
        BLACK,
    );

    draw_text(
        &format!("FPS MAX: {} mcs", max_fps),
        offset_x + 10.0,
        offset_y + 40.0,
        20.0,
        BLACK,
    );
}

fn draw_goals(offset_x: f32, offset_y: f32, context: &MatchContext, field_width: f32, scale: f32) {
    let color = Color::from_rgba(0, 184, 186, 255);

    draw_line(
        offset_x,
        offset_y + context.goal_positions.left.y * scale - GOAL_WIDTH * scale,
        offset_x,
        offset_y + context.goal_positions.left.y * scale + GOAL_WIDTH * scale,
        15.0,
        color,
    );

    draw_line(
        offset_x + field_width,
        offset_y + context.goal_positions.right.y * scale - GOAL_WIDTH * scale,
        offset_x + field_width,
        offset_y + context.goal_positions.right.y * scale + GOAL_WIDTH * scale,
        15.0,
        color,
    );
}

fn draw_players(
    offset_x: f32,
    offset_y: f32,
    field: &MatchField,
    ball_owner_id: Option<u32>,
    scale: f32,
) {
    field.players.iter().for_each(|player| {
        let translated_x = offset_x + player.position.x * scale;
        let translated_y = offset_y + player.position.y * scale;

        let mut color = if player.side == Some(PlayerSide::Left) {
            Color::from_rgba(0, 184, 186, 255)
        } else {
            Color::from_rgba(208, 139, 255, 255)
        };

        if player.tactical_position.current_position == PlayerPositionType::Goalkeeper {
            color = YELLOW;
        }

        let circle_radius = 15.0 * scale;

        // Draw the player circle
        draw_circle(translated_x, translated_y, circle_radius, color);

        if Some(player.id) == ball_owner_id {
            draw_circle_lines(
                translated_x,
                translated_y,
                circle_radius + scale - 2.0,
                5.0,
                ORANGE,
            );
        }

        // Player position
        let position = &player.tactical_position.current_position.get_short_name();
        let position_font_size = 17.0 * scale;
        let position_text_dimensions = measure_text(position, None, position_font_size as u16, 1.0);
        draw_text(
            position,
            translated_x - position_text_dimensions.width / 2.0,
            translated_y + position_text_dimensions.height / 3.0,
            position_font_size,
            BLACK,
        );

        // Player ID
        let id_text = &player.id.to_string();
        let id_font_size = 9.0 * scale;
        let id_text_dimensions = measure_text(id_text, None, id_font_size as u16, 1.0);
        draw_text(
            id_text,
            translated_x - id_text_dimensions.width / 2.0,
            translated_y + position_text_dimensions.height + id_text_dimensions.height / 2.0,
            id_font_size,
            DARKGRAY,
        );

        // Player state and distance
        let distance = distance(&field.ball.position, &player.position);
        let state_distance_text = &format!("{} ({})", player_state(player), distance);
        let state_distance_font_size = 13.0 * scale;
        let state_distance_text_dimensions = measure_text(
            state_distance_text,
            None,
            state_distance_font_size as u16,
            1.0,
        );
        draw_text(
            state_distance_text,
            translated_x - state_distance_text_dimensions.width / 2.5,
            translated_y + circle_radius + state_distance_text_dimensions.height + 0.0,
            state_distance_font_size,
            DARKGRAY,
        );

        // ID

        let left_goal = Vector3::new(0.0, field.size.height as f32 / 2.0, 0.0);
        let right_goal = Vector3::new(field.size.width as f32, (field.size.height / 2usize) as f32, 0.0);
        
        let target_goal = match player.side {
            Some(PlayerSide::Left) => Vector3::new(
                right_goal.x,
                right_goal.y,
                0.0,
            ),
            Some(PlayerSide::Right) => Vector3::new(
                left_goal.x,
                left_goal.y,
                0.0,
            ),
            _ => Vector3::new(0.0, 0.0, 0.0),
        };
        
        let goal_distance = field.ball.position.distance_to(&target_goal);
        
        let distance_to_opponent_goal = &format!("g_d = {}", goal_distance);

        let distance_to_opponent_goal_font_size = 13.0 * scale;
        let distance_to_opponent_goal_text_dimensions = measure_text(
            distance_to_opponent_goal,
            None,
            distance_to_opponent_goal_font_size as u16,
            1.0,
        );

        draw_text(
            distance_to_opponent_goal,
            translated_x - distance_to_opponent_goal_text_dimensions.width / 2.5,
            translated_y + circle_radius + distance_to_opponent_goal_text_dimensions.height + 15.0,
            distance_to_opponent_goal_font_size,
            DARKGRAY,
        );
    });
}

fn draw_ball(offset_x: f32, offset_y: f32, ball: &Ball, scale: f32) {
    let translated_x = offset_x + ball.position.x * scale;
    let translated_y = offset_y + ball.position.y * scale;

    draw_circle(translated_x, translated_y, 7.0 * scale, ORANGE);

    if ball.flags.running_for_ball {
        draw_circle(translated_x, translated_y, 3.0, BLACK);
    }

    draw_text(
        &format!(
            "BALL POSITION, {:?}, IS_OUTSIDE: {:?}, IS_STANDS_OUTSIDE: {:?}, NOTIFIED_PLAYER: {:?}",
            ball.position,
            ball.is_ball_outside(),
            ball.is_stands_outside(),
            ball.take_ball_notified_player
        ),
        20.0,
        15.0,
        15.0,
        BLACK,
    );

    draw_text(
        &format!("BALL VELOCITY: {:?}", ball.velocity),
        20.0,
        30.0,
        15.0,
        BLACK,
    );
}

fn draw_player_list(
    offset_x: f32,
    offset_y: f32,
    players: Vec<&MatchPlayer>,
    ball_owner_id: Option<u32>,
    scale: f32,
) {
    let player_width = 25.0 * scale;
    let player_height = 25.0 * scale;
    let player_spacing = 40.0 * scale;

    players.iter().enumerate().for_each(|(index, player)| {
        let player_x = offset_x + index as f32 * (player_width + player_spacing);
        let player_y = offset_y;

        // Draw player circle
        let player_color: Color =
            if player.tactical_position.current_position == PlayerPositionType::Goalkeeper {
                YELLOW
            } else if player.team_id == 1 {
                Color::from_rgba(0, 184, 186, 255)
            } else {
                Color::from_rgba(208, 139, 255, 255)
            };

        let circle_radius = player_width / 2.0;

        draw_circle(
            player_x + circle_radius,
            player_y + circle_radius,
            circle_radius,
            player_color,
        );

        if Some(player.id) == ball_owner_id {
            draw_circle_lines(
                player_x + circle_radius,
                player_y + circle_radius,
                circle_radius + scale - 2.0,
                5.0,
                ORANGE,
            );
        }

        // Draw player number
        let player_number = player.id.to_string();
        let number_font_size = 14.0 * scale;
        let number_dimensions = measure_text(&player_number, None, number_font_size as u16, 1.0);
        draw_text(
            &player_number,
            player_x + circle_radius - number_dimensions.width / 2.0,
            player_y + circle_radius + number_dimensions.height / 4.0,
            number_font_size,
            BLACK,
        );

        // Draw player state
        let state_text = player_state(player);
        let state_font_size = 12.0 * scale;
        let state_dimensions = measure_text(&state_text, None, state_font_size as u16, 1.0);
        draw_text(
            &state_text,
            player_x + circle_radius - state_dimensions.width / 2.0,
            player_y + player_height + state_dimensions.height / 2.0 + 5.0,
            state_font_size,
            BLACK,
        );
    });
}
