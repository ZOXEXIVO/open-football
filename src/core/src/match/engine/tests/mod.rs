//! Cross-module regression tests for the match engine. Single-module
//! unit tests live next to each module; these exercise interactions
//! between modules so future refactors don't drift combined behavior.

mod corner_setup_tests;
mod defender_behavior_tests;
mod effort_appetite_tests;
mod engagement_contract_tests;
mod fatigue_calibration_tests;
mod friendly_recording_tests;
mod goal_celebration_tests;
mod goal_clip_recording_tests;
mod goal_distance_tests;
mod goal_kick_tests;
mod counter_press_tests;
mod fixture_environment_tests;
mod injury_tests;
mod landing_prediction_tests;
mod offside_trap_tests;
mod dead_ball_kick_tests;
mod short_handed_tests;
mod state_exit_tests;
mod wet_pitch_tests;
mod trait_bias_tests;
mod keeper_release_tests;
mod intelligence_tests;
mod interception_tests;
mod international_recording_tests;
mod keeper_body_tests;
mod keeper_punt_tests;
mod keeper_save_contact_tests;
mod keeper_space_tests;
mod keeper_tip_over_tests;
mod kickoff_tests;
mod loose_ball_chase_tests;
mod marker_read_tests;
mod match_clock_tests;
mod match_realism_tests;
mod offside_restart_tests;
mod pass_pressure_tests;
mod restart_law_tests;
mod restart_shape_tests;
mod position_store_tests;
mod recording_globals;
mod run_out_tests;
mod solid_engine_tests;
mod sprint_ramp_tests;
mod state_reachability_tests;
#[cfg(feature = "match-stub")]
mod stub_minutes_tests;
mod substitution_break_tests;
mod throw_in_tests;
