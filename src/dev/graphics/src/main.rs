//tactics
use core::club::player::Player;
use core::club::player::PlayerPositionType;
use core::club::team::tactics::{MatchTacticType, Tactics};
use core::r#match::ball::Ball;
use core::r#match::player::strategies::MatchBallLogic;
use core::r#match::player::MatchPlayer;
use core::r#match::FootballEngine;
use core::r#match::MatchContext;
use core::r#match::MatchField;
use core::r#match::MatchPlayerCollection;
use core::r#match::MatchSquad;
use core::r#match::ResultMatchPositionData;
use core::r#match::VectorExtensions;
use core::Vector3;
use env_logger::Env;
use macroquad::prelude::*;
use std::time::Duration;
use std::time::Instant;
use tokio::time::sleep;

use core::r#match::PlayerSide;
use core::r#match::Score;
use core::r#match::GOAL_WIDTH;
use core::staff_contract_mod::NaiveDate;
use core::PlayerGenerator;

/// Tracks pass target for visualization
#[derive(Debug, Clone)]
struct PassTargetInfo {
    target_player_id: u32,
    timestamp: u64,
}

const INNER_FIELD_WIDTH: f32 = 840.0;
const INNER_FIELD_HEIGHT: f32 = 545.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum MatchSpeed {
    Percent100 = 1,
    Percent50 = 2,
    Percent10 = 10,
    Percent1 = 100,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PlayMode {
    Live,   // Running simulation in real-time
    Replay, // Playing back recorded positions
}

impl MatchSpeed {
    fn label(&self) -> &'static str {
        match self {
            MatchSpeed::Percent100 => "100%",
            MatchSpeed::Percent50 => "50%",
            MatchSpeed::Percent10 => "10%",
            MatchSpeed::Percent1 => "1%",
        }
    }

    fn all() -> [MatchSpeed; 4] {
        [
            MatchSpeed::Percent100,
            MatchSpeed::Percent50,
            MatchSpeed::Percent10,
            MatchSpeed::Percent1,
        ]
    }
}

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

    let score = Score::new(1, 2);

    let mut context = MatchContext::new(&field, players, score);

    context.enable_logging();

    let mut current_frame = 0u64;
    let mut tick_frame = 0u64;

    // Use new_with_tracking() to enable pass event tracking for visualization
    let mut match_data = ResultMatchPositionData::new_with_tracking();

    let mut left_mouse_pressed;
    let mut selected_speed = MatchSpeed::Percent100;
    let mut play_mode = PlayMode::Live;
    let mut is_paused = false;
    let mut replay_time: u64 = 0;
    let mut max_live_time: u64 = 0;  // Track maximum time reached in Live mode
    let mut pass_target: Option<PassTargetInfo> = None;  // Track current pass target

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

        // Speed control UI at the top right corner
        draw_speed_control(offset_x, offset_y, field_width, &mut selected_speed);

        // Time slider and play/pause controls (at the bottom)
        let slider_y = screen_height() - 50.0;
        draw_time_slider(
            20.0,  // Start from left edge with padding
            slider_y,
            screen_width() - 40.0,  // Full width with padding
            &match_data,
            &mut play_mode,
            &mut is_paused,
            &mut replay_time,
            max_live_time,
        );

        let start = Instant::now();

        // Execute game tick or update replay based on mode
        match play_mode {
            PlayMode::Live if !is_paused => {
                // Only execute game tick based on selected speed
                let should_tick = tick_frame % (selected_speed as u64) == 0;
                tick_frame += 1;

                if should_tick {
                    // Increment time before game tick (like the engine does)
                    context.increment_time();
                    FootballEngine::<840, 545>::game_tick(&mut field, &mut context, &mut match_data);
                }

                // Update replay_time to current match time for slider display
                replay_time = context.time.time;
                // Track the maximum time reached in live mode
                max_live_time = max_live_time.max(replay_time);
            }
            PlayMode::Live if is_paused => {
                // Paused in live mode - keep current time
                replay_time = context.time.time;
                // Track the maximum time reached in live mode
                max_live_time = max_live_time.max(replay_time);
            }
            PlayMode::Replay if !is_paused => {
                // Auto-advance replay time using same speed as live mode
                let should_advance = tick_frame % (selected_speed as u64) == 0;
                tick_frame += 1;

                if should_advance {
                    // Increment by 10ms to match live mode speed (MATCH_TIME_INCREMENT_MS)
                    replay_time += 10;
                    // Use max_live_time as the limit (highest time reached in Live mode)
                    if replay_time > max_live_time {
                        replay_time = max_live_time;
                        is_paused = true;
                    }
                }

                // Update field positions from recorded data
                update_positions_from_replay(&mut field, &match_data, replay_time);
            }
            _ => {
                // Paused in replay mode
                if play_mode == PlayMode::Replay {
                    // Make sure positions are set to current replay time
                    update_positions_from_replay(&mut field, &match_data, replay_time);
                }
            }
        }

        let elapsed = start.elapsed();

        // Get recent pass event from match data (show for 2 seconds)
        if let Some(recent_pass) = match_data.get_recent_pass_at(context.time.time) {
            // Only show passes from last 2 seconds
            if context.time.time - recent_pass.timestamp <= 2000 {
                pass_target = Some(PassTargetInfo {
                    target_player_id: recent_pass.to_player_id,
                    timestamp: recent_pass.timestamp,
                });
            } else {
                pass_target = None;
            }
        } else {
            pass_target = None;
        }

        draw_goals(offset_x, offset_y, &context, field_width, scale);
        draw_waypoints(offset_x, offset_y, &field, scale);
        draw_players(offset_x, offset_y, &field, field.ball.current_owner, pass_target.as_ref(), scale);

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
            sleep(Duration::from_millis(500)).await;
        }

        next_frame().await;
    }
}

const TRACKING_PLAYER_ID: u32 = 0;

pub fn get_home_squad() -> MatchSquad {
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

    let home_squad = MatchSquad {
        team_id: 1,
        team_name: String::from("123"),
        tactics: Tactics::new(MatchTacticType::T442),
        main_squad: match_players,
        substitutes: Vec::new(),
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
    };

    home_squad
}

pub fn get_away_squad() -> MatchSquad {
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

    MatchSquad {
        team_id: 2,
        team_name: String::from("321"),
        tactics: Tactics::new(MatchTacticType::T442),
        main_squad: match_players,
        substitutes: Vec::new(),
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
    }
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
const WINDOW_HEIGHT: i32 = 900;

#[cfg(target_os = "linux")]
const WINDOW_WIDTH: i32 = 1948;
#[cfg(target_os = "linux")]
const WINDOW_HEIGHT: i32 = 1721;

#[cfg(target_os = "windows")]
const WINDOW_WIDTH: i32 = 1948;
#[cfg(target_os = "windows")]
const WINDOW_HEIGHT: i32 = 1521;

fn window_conf() -> Conf {
    Conf {
        window_title: "OpenFootball Match Development".to_owned(),
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

fn draw_speed_control(offset_x: f32, offset_y: f32, field_width: f32, selected_speed: &mut MatchSpeed) {
    // All sizes reduced by 50%, then widths reduced by additional 40%
    let label_width = 36.0;  // Was 60.0 (60% of 60.0)
    let button_width = 24.0; // Was 40.0 (60% of 40.0)
    let button_height = 15.0; // Was 30.0 (height unchanged)
    let button_spacing = 3.0; // Was 5.0 (60% of 5.0)
    let font_size = 10.0; // Was 20.0 (unchanged)

    let speeds = MatchSpeed::all();
    let num_buttons = speeds.len() as f32;

    // Calculate total width of the control
    let total_width = label_width + (button_width * num_buttons) + (button_spacing * (num_buttons - 1.0));

    // Position at top right corner
    let x = offset_x + field_width - total_width - 10.0; // 10.0 padding from right edge
    let y = offset_y - 25.0; // Position above the field

    let mut current_x = x + label_width;

    for speed in &speeds {
        let is_selected = *selected_speed == *speed;

        // Draw radio button background
        let bg_color = if is_selected {
            Color::from_rgba(100, 200, 100, 255) // Green when selected
        } else {
            Color::from_rgba(200, 200, 200, 255) // Gray when not selected
        };

        draw_rectangle(current_x, y, button_width, button_height, bg_color);

        // Draw button border
        let border_color = if is_selected {
            Color::from_rgba(50, 150, 50, 255)
        } else {
            Color::from_rgba(100, 100, 100, 255)
        };
        draw_rectangle_lines(current_x, y, button_width, button_height, 1.0, border_color);

        // Draw button text (centered)
        let text = speed.label();
        let text_width = 15.0; // Approximate width for smaller font
        let text_x = current_x + (button_width - text_width) / 2.0;
        let text_y = y + button_height / 2.0 + 3.0;
        draw_text(text, text_x, text_y, font_size, BLACK);

        // Check if button is clicked
        if is_mouse_button_pressed(MouseButton::Left) {
            let (mouse_x, mouse_y) = mouse_position();
            if mouse_x >= current_x
                && mouse_x <= current_x + button_width
                && mouse_y >= y
                && mouse_y <= y + button_height
            {
                *selected_speed = *speed;
            }
        }

        current_x += button_width + button_spacing;
    }
}

fn draw_waypoints(offset_x: f32, offset_y: f32, field: &MatchField, scale: f32) {
    field.players.iter().for_each(|player| {
        let waypoints = player.get_waypoints_as_vectors();

        if waypoints.is_empty() {
            return;
        }

        // Determine color based on team
        let waypoint_color = if player.side == Some(PlayerSide::Left) {
            Color::from_rgba(0, 184, 186, 100) // Semi-transparent cyan for left team
        } else {
            Color::from_rgba(208, 139, 255, 100) // Semi-transparent purple for right team
        };

        let line_color = if player.side == Some(PlayerSide::Left) {
            Color::from_rgba(0, 184, 186, 180) // More opaque line
        } else {
            Color::from_rgba(208, 139, 255, 180)
        };

        // Draw lines connecting waypoints
        for i in 0..waypoints.len() {
            let current = &waypoints[i];
            let current_x = offset_x + current.x * scale;
            let current_y = offset_y + current.y * scale;

            // Draw line to next waypoint
            if i < waypoints.len() - 1 {
                let next = &waypoints[i + 1];
                let next_x = offset_x + next.x * scale;
                let next_y = offset_y + next.y * scale;

                draw_line(
                    current_x,
                    current_y,
                    next_x,
                    next_y,
                    1.5 * scale,
                    line_color,
                );
            }

            // Draw waypoint circle
            let radius = 3.0 * scale;
            draw_circle(current_x, current_y, radius, waypoint_color);

            // Highlight current waypoint
            if i == player.waypoint_manager.current_index && !player.waypoint_manager.path_completed {
                draw_circle_lines(current_x, current_y, radius + 2.0 * scale, 2.0, RED);
            }
        }

        // Draw line from player to current waypoint
        if !player.waypoint_manager.path_completed {
            let current_waypoint_idx = player.waypoint_manager.current_index;
            if current_waypoint_idx < waypoints.len() {
                let waypoint = &waypoints[current_waypoint_idx];
                let waypoint_x = offset_x + waypoint.x * scale;
                let waypoint_y = offset_y + waypoint.y * scale;
                let player_x = offset_x + player.position.x * scale;
                let player_y = offset_y + player.position.y * scale;

                draw_line(
                    player_x,
                    player_y,
                    waypoint_x,
                    waypoint_y,
                    1.0 * scale,
                    Color::from_rgba(255, 100, 100, 150), // Red dashed-looking line
                );
            }
        }
    });
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
    pass_target: Option<&PassTargetInfo>,
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

        // Draw red circle around pass target player
        if let Some(target_info) = pass_target {
            if player.id == target_info.target_player_id {
                draw_circle_lines(
                    translated_x,
                    translated_y,
                    circle_radius + 8.0 * scale,
                    4.0,
                    RED,
                );
            }
        }

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
        let right_goal = Vector3::new(
            field.size.width as f32,
            (field.size.height / 2usize) as f32,
            0.0,
        );

        let target_goal = match player.side {
            Some(PlayerSide::Left) => Vector3::new(right_goal.x, right_goal.y, 0.0),
            Some(PlayerSide::Right) => Vector3::new(left_goal.x, left_goal.y, 0.0),
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

    // Calculate visual scale based on height (perspective effect)
    // The higher the ball, the larger it appears (simulating it being closer to camera)
    let height_scale = 1.0 + (ball.position.z / 15.0).min(0.4);
    let ball_radius = (7.0 / 1.5) * scale * height_scale;

    // Draw the ball at its elevated position
    // Adjust y-position to simulate 3D height (isometric-like projection)
    let visual_y_offset = ball.position.z * scale * 0.5; // Scale z-height for visual effect
    let ball_visual_y = translated_y - visual_y_offset;

    // Draw simple white ball with black border
    draw_circle(translated_x, ball_visual_y, ball_radius, WHITE);
    draw_circle_lines(translated_x, ball_visual_y, ball_radius, 2.0, BLACK);

    // Draw black filled circle in the center
    let center_circle_radius = ball_radius * 0.3;
    draw_circle(translated_x, ball_visual_y, center_circle_radius, BLACK);

    draw_text(
        &format!(
            "BALL POS: x:{:.1}, y:{:.1}, z:{:.1}, IS_OUTSIDE: {:?}, IS_STANDS_OUTSIDE: {:?}, NOTIFIED: {:?}",
            ball.position.x,
            ball.position.y,
            ball.position.z,
            ball.is_ball_outside(),
            ball.is_stands_outside(),
            ball.take_ball_notified_players
        ),
        20.0,
        15.0,
        15.0,
        BLACK,
    );

    draw_text(
        &format!("BALL VEL: x:{:.1}, y:{:.1}, z:{:.1}", ball.velocity.x, ball.velocity.y, ball.velocity.z),
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

/// Update field positions from replay data at a specific timestamp
fn update_positions_from_replay(field: &mut MatchField, match_data: &ResultMatchPositionData, timestamp: u64) {
    // Update ball position
    if let Some(ball_pos) = match_data.get_ball_position_at(timestamp) {
        field.ball.position = ball_pos;
    }

    // Update all player positions
    for player in field.players.iter_mut() {
        if let Some(player_pos) = match_data.get_player_position_at(player.id, timestamp) {
            player.position = player_pos;
        }
    }
}

/// Draw time slider and playback controls (simple design)
fn draw_time_slider(
    offset_x: f32,
    offset_y: f32,
    total_width: f32,
    match_data: &ResultMatchPositionData,
    play_mode: &mut PlayMode,
    is_paused: &mut bool,
    replay_time: &mut u64,
    max_live_time: u64,
) {
    // Use max_live_time as the maximum time for the slider
    // This represents the furthest point the simulation has reached
    let max_time = max_live_time;

    // Play/Pause button on the left
    let button_size = 30.0;
    let button_x = offset_x;
    let button_y = offset_y;

    // Get mouse position once
    let (mouse_x, mouse_y) = mouse_position();

    let button_color = Color::from_rgba(100, 200, 100, 255);

    // Draw button circle
    let button_center_x = button_x + button_size / 2.0;
    let button_center_y = button_y + button_size / 2.0;
    draw_circle(button_center_x, button_center_y, button_size / 2.0, button_color);
    draw_circle_lines(button_center_x, button_center_y, button_size / 2.0, 2.0, BLACK);

    // Draw play/pause icon
    if *is_paused {
        // Play triangle
        let size = 8.0;
        draw_triangle(
            Vec2::new(button_center_x - size / 2.0, button_center_y - size),
            Vec2::new(button_center_x - size / 2.0, button_center_y + size),
            Vec2::new(button_center_x + size, button_center_y),
            BLACK,
        );
    } else {
        // Pause bars
        let bar_width = 3.0;
        let bar_height = 10.0;
        let bar_spacing = 4.0;
        draw_rectangle(button_center_x - bar_spacing - bar_width, button_center_y - bar_height / 2.0, bar_width, bar_height, BLACK);
        draw_rectangle(button_center_x + bar_spacing, button_center_y - bar_height / 2.0, bar_width, bar_height, BLACK);
    }

    // Only check button click when mouse is actually pressed
    if is_mouse_button_pressed(MouseButton::Left) {
        let button_rect = Rect::new(button_x, button_y, button_size, button_size);
        if button_rect.contains(Vec2::new(mouse_x, mouse_y)) {
            *is_paused = !*is_paused;
        }
    }

    // Slider bar
    let slider_padding = 10.0;
    let slider_x = offset_x + button_size + slider_padding;
    let slider_width = total_width - button_size - slider_padding - 100.0; // Leave space for time text
    let slider_height = 4.0;
    let slider_y = offset_y + button_size / 2.0 - slider_height / 2.0;

    // Draw background bar (gray)
    draw_rectangle(slider_x, slider_y, slider_width, slider_height, GRAY);

    // Draw progress bar (blue)
    if max_time > 0 {
        let progress = (*replay_time as f32 / max_time as f32).clamp(0.0, 1.0);
        draw_rectangle(
            slider_x,
            slider_y,
            slider_width * progress,
            slider_height,
            BLUE,
        );

        // Draw circle handle
        let handle_x = slider_x + slider_width * progress;
        let handle_y = slider_y + slider_height / 2.0;
        let handle_radius = 8.0;

        draw_circle(handle_x, handle_y, handle_radius, WHITE);
        draw_circle_lines(handle_x, handle_y, handle_radius, 2.0, BLUE);
    }

    // Handle mouse interaction with slider (only when mouse is pressed)
    if max_time > 0 && is_mouse_button_down(MouseButton::Left) {
        let mouse_on_slider = mouse_x >= slider_x - 10.0
            && mouse_x <= slider_x + slider_width + 10.0
            && mouse_y >= slider_y - 15.0
            && mouse_y <= slider_y + slider_height + 15.0;

        if mouse_on_slider {
            // User is dragging the slider
            let relative_x = (mouse_x - slider_x).clamp(0.0, slider_width);
            let new_progress = relative_x / slider_width;
            *replay_time = (new_progress * max_time as f32) as u64;

            // Switch to replay mode when dragging
            if *play_mode == PlayMode::Live {
                *play_mode = PlayMode::Replay;
            }
        }
    }

    // Draw time text on the right
    let time_text = format!("{} / {}", format_time(*replay_time), format_time(max_time));
    let time_x = slider_x + slider_width + 15.0;
    let time_y = offset_y + button_size / 2.0 + 5.0;
    draw_text(&time_text, time_x, time_y, 16.0, BLACK);
}

/// Format timestamp (milliseconds) to mm:ss format
fn format_time(timestamp: u64) -> String {
    let total_seconds = timestamp / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{:02}:{:02}", minutes, seconds)
}
