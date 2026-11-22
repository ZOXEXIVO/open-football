//! **One read of the game for every defender without the ball.** The same
//! situation, put to a defender in each state he can hold without it, gets
//! one answer; the rungs of the ladder (closing, containing, the challenge)
//! hand to one another on the spec's terms; the slot is one point.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::{
    DefenderRead, DefensiveLine, DefensiveRecovery, DefensiveSlot, LineStep,
};
use crate::r#match::engine::ball::ball::{AwaitedRestart, PassOriginRestart, RestartQuiet};
use crate::r#match::engine::result::Score;
use crate::r#match::events::Event;
use crate::r#match::passing::FlankPlay;
use crate::r#match::player::events::{FoulSource, PlayerEvent};
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::defenders::states::covering::DefenderCoveringState;
use crate::r#match::player::strategies::defenders::states::heading::DefenderHeadingState;
use crate::r#match::player::strategies::defenders::states::holding_line::DefenderHoldingLineState;
use crate::r#match::player::strategies::defenders::states::jockeying::DefenderJockeyingState;
use crate::r#match::player::strategies::defenders::states::marking::DefenderMarkingState;
use crate::r#match::player::strategies::defenders::states::pressing::DefenderPressingState;
use crate::r#match::player::strategies::defenders::states::pushingup::DefenderPushingUpState;
use crate::r#match::player::strategies::defenders::states::returning::DefenderReturningState;
use crate::r#match::player::strategies::defenders::states::running::DefenderRunningState;
use crate::r#match::player::strategies::defenders::states::standing::DefenderStandingState;
use crate::r#match::player::strategies::defenders::states::tackling::DefenderTacklingState;
use crate::r#match::player::strategies::defenders::states::tracking_back::DefenderTrackingBackState;
use crate::r#match::player::strategies::defenders::states::walking::DefenderWalkingState;
use crate::r#match::player::strategies::players::ops::clearance::{
    DefensiveRelease, TouchlineCall,
};
use crate::r#match::player::transition::TransitionSource;
use crate::r#match::shared_states::{
    BlockChallenge, ChallengeKind, ShapeTempo, TackleEngagement, TackleOutcome,
};
use crate::r#match::{
    ActivityIntensity, ConditionContext, DefenceRefreshInputs, DefensiveDuty, DefensivePlan,
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, MatchRng, MovementEffort,
    PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const HOME: u32 = 1;
const AWAY: u32 = 2;
const DL: u32 = 101;
const DCL: u32 = 102;
const DCR: u32 = 103;
const DR: u32 = 104;
const MCL: u32 = 106;
const AWAY_MR: u32 = 208;
const AWAY_FL: u32 = 209;
const AWAY_FR: u32 = 210;
const LINE_X: f32 = 140.0;

/// Every state a defender holds without the ball, below the loose-ball rung.
const OFF_BALL: [DefenderState; 10] = [
    DefenderState::Standing,
    DefenderState::Walking,
    DefenderState::HoldingLine,
    DefenderState::PushingUp,
    DefenderState::Returning,
    DefenderState::TrackingBack,
    DefenderState::Marking,
    DefenderState::Covering,
    DefenderState::Pressing,
    DefenderState::Jockeying,
];

/// Home on the left, defending x = 0, ten minutes in. The back four flat on
/// `LINE_X` at their formation widths, the keeper on his line, everyone else
/// parked well away.
struct Pitch {
    field: MatchField,
    context: MatchContext,
}

impl Pitch {
    fn new() -> Self {
        let home = squad(HOME, 100);
        let away = squad(AWAY, 200);
        let players = MatchPlayerCollection::from_squads(&home, &away);
        let mut field = MatchField::new(840, 545, home, away);
        assert_eq!(field.side_of(HOME), PlayerSide::Left);
        field.ball.kickoff_taker = None;
        for (index, p) in field.players.iter_mut().enumerate() {
            p.velocity = Vector3::zeros();
            let x = if p.team_id == HOME { 700.0 } else { 790.0 };
            p.position = Vector3::new(x, 20.0 + (index % 11) as f32 * 48.0, 0.0);
        }
        for id in [DL, DCL, DCR, DR] {
            let y = field.get_player(id).unwrap().start_position.y;
            field.get_player_mut(id).unwrap().position = Vector3::new(LINE_X, y, 0.0);
        }
        field.get_player_mut(100).unwrap().position = Vector3::new(20.0, 272.0, 0.0);
        field.get_player_mut(200).unwrap().position = Vector3::new(820.0, 272.0, 0.0);
        let mut context = MatchContext::new(&field, players, Score::new(HOME, AWAY), false, false);
        context.total_match_time = 600_000;
        Pitch { field, context }
    }

    fn stand(&mut self, id: u32, x: f32, y: f32) -> &mut Self {
        let p = self.field.get_player_mut(id).unwrap();
        p.position = Vector3::new(x, y, 0.0);
        p.velocity = Vector3::zeros();
        self
    }

    fn on_the_ball(&mut self, id: u32) -> &mut Self {
        let at = self.field.get_player(id).unwrap().position;
        self.field.ball.position = at;
        self.field.ball.velocity = Vector3::zeros();
        self.field.ball.current_owner = Some(id);
        self.field.ball.pass_target_player_id = None;
        self
    }

    /// The plan the side would refresh to, with `holder` on the ball.
    fn plan(&mut self, holder: u32) -> &mut Self {
        DefensivePlan::refresh(
            &mut self.context.defence_home,
            &mut self.context.defence_away,
            &DefenceRefreshInputs {
                field: &self.field,
                possession_holder: Some(holder),
                home_team_id: HOME,
                away_team_id: AWAY,
                home_keeper_voice: 0.5,
                away_keeper_voice: 0.5,
            },
        );
        self
    }

    fn put_in(&mut self, id: u32, state: DefenderState) {
        self.field
            .get_player_mut(id)
            .unwrap()
            .transition_to(PlayerState::Defender(state), TransitionSource::Reset);
    }

    fn with_ctx<R>(&self, id: u32, read: impl FnOnce(&StateProcessingContext) -> R) -> R {
        let tick = GameTickContext::new(&self.field, &self.context.players);
        let ctx = StateProcessingContext {
            in_state_time: 40,
            player: self.field.get_player(id).unwrap(),
            context: &self.context,
            tick_context: &tick,
        };
        read(&ctx)
    }

    fn process(&mut self, id: u32, state: DefenderState) -> Option<StateChangeResult> {
        self.put_in(id, state);
        self.with_ctx(id, |ctx| match state {
            DefenderState::Standing => DefenderStandingState::default().process(ctx),
            DefenderState::Walking => DefenderWalkingState::default().process(ctx),
            DefenderState::HoldingLine => DefenderHoldingLineState::default().process(ctx),
            DefenderState::PushingUp => DefenderPushingUpState::default().process(ctx),
            DefenderState::Returning => DefenderReturningState::default().process(ctx),
            DefenderState::TrackingBack => DefenderTrackingBackState::default().process(ctx),
            DefenderState::Marking => DefenderMarkingState::default().process(ctx),
            DefenderState::Covering => DefenderCoveringState::default().process(ctx),
            DefenderState::Pressing => DefenderPressingState::default().process(ctx),
            DefenderState::Jockeying => DefenderJockeyingState::default().process(ctx),
            other => panic!("no handler wired for {other:?}"),
        })
    }

    fn velocity(&mut self, id: u32, state: DefenderState) -> Vector3<f32> {
        self.put_in(id, state);
        self.with_ctx(id, |ctx| match state {
            DefenderState::Walking => DefenderWalkingState::default().velocity(ctx),
            DefenderState::HoldingLine => DefenderHoldingLineState::default().velocity(ctx),
            DefenderState::PushingUp => DefenderPushingUpState::default().velocity(ctx),
            DefenderState::Returning => DefenderReturningState::default().velocity(ctx),
            DefenderState::TrackingBack => DefenderTrackingBackState::default().velocity(ctx),
            DefenderState::Marking => DefenderMarkingState::default().velocity(ctx),
            other => panic!("no handler wired for {other:?}"),
        })
        .expect("a velocity")
    }

    /// Where he goes from `state`: the state handed to, or `state` itself.
    fn answer(&mut self, id: u32, state: DefenderState) -> DefenderState {
        match self.process(id, state).and_then(|change| change.state) {
            None => state,
            Some(PlayerState::Defender(next)) => next,
            Some(other) => panic!("{state:?} handed a defender to {other:?}"),
        }
    }

    /// The read's answer, and every off-ball state's, which must be it.
    fn one_answer(&mut self, id: u32) -> DefenderState {
        self.put_in(id, DefenderState::Standing);
        let read = self.with_ctx(id, DefenderRead::resolve);
        for state in OFF_BALL {
            assert_eq!(self.answer(id, state), read, "from {state:?}");
        }
        read
    }
}

// ── One answer from every state ─────────────────────────────────────────

#[test]
fn a_carrier_in_the_area_is_engaged_from_every_state() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 80.0, 272.0)
        .on_the_ball(AWAY_FL)
        .plan(AWAY);
    for id in [DCL, DCR] {
        assert_eq!(
            pitch.one_answer(id),
            DefenderState::Pressing,
            "defender {id}"
        );
    }
}

#[test]
fn a_recovering_defender_challenges() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 450.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(DCL, 400.0, 240.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(DCL));
    for state in [DefenderState::Returning, DefenderState::TrackingBack] {
        assert_eq!(
            pitch.answer(DCL, state),
            DefenderState::Pressing,
            "from {state:?}"
        );
    }
    assert_eq!(pitch.one_answer(DCL), DefenderState::Pressing);
}

#[test]
fn the_cover_man_covers_from_every_state() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 450.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(DCL, 400.0, 240.0)
        .stand(DCR, 360.0, 300.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(DCL));
    assert_eq!(
        pitch.context.defence_home.duty_of(DCR),
        DefensiveDuty::Cover
    );
    assert_eq!(pitch.one_answer(DCR), DefenderState::Covering);
}

#[test]
fn a_marked_man_is_marked_from_every_state() {
    let mut pitch = Pitch::new();
    let y = pitch.field.get_player(DCR).unwrap().position.y;
    pitch
        .stand(AWAY_MR, 450.0, 272.0)
        .on_the_ball(AWAY_MR)
        .stand(AWAY_FR, LINE_X + 30.0, y + 10.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.mark_of(DCR), Some(AWAY_FR));
    assert_eq!(pitch.one_answer(DCR), DefenderState::Marking);
}

#[test]
fn a_defender_out_of_the_line_with_no_job_tracks_back_from_every_state() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_MR, 600.0, 272.0)
        .on_the_ball(AWAY_MR)
        .plan(AWAY);
    let y = pitch.field.get_player(DR).unwrap().position.y;
    pitch.stand(DR, 420.0, y);
    assert_eq!(
        pitch.context.defence_home.duty_of(DR),
        DefensiveDuty::HoldZone
    );
    assert_eq!(pitch.one_answer(DR), DefenderState::TrackingBack);
}

/// The election gives this one to nobody he can beat to it, so it is the
/// other half of the loose-ball rung: a ball in flight coming to him.
#[test]
fn a_ball_coming_to_him_is_met_from_every_state() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_MR, 600.0, 272.0)
        .on_the_ball(AWAY_MR)
        .plan(AWAY);
    // An away pass to a man far away, running past him.
    let at = pitch.field.get_player(DCR).unwrap().position;
    let (start, end) = (
        at + Vector3::new(60.0, 0.0, 0.0),
        at - Vector3::new(50.0, 0.0, 0.0),
    );
    let ball = &mut pitch.field.ball;
    ball.position = start;
    ball.velocity = (end - start).normalize() * 2.0;
    ball.current_owner = None;
    ball.pass_target_player_id = Some(AWAY_MR);
    ball.cached_landing_position = start;
    ball.flags.in_flight_state = 10;
    ball.pending_pass_aim = Some(end);
    ball.current_tick_cached = 1_000;
    ball.last_release_tick = 0;
    assert_eq!(pitch.one_answer(DCR), DefenderState::Intercepting);
}

#[test]
fn nobody_marks_or_covers_once_his_side_has_the_ball() {
    let mut pitch = Pitch::new();
    let y = pitch.field.get_player(DCR).unwrap().position.y;
    pitch
        .stand(AWAY_FL, 450.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(DCL, 400.0, 240.0)
        .stand(DCR, 360.0, 300.0)
        .stand(AWAY_FR, LINE_X + 30.0, y + 10.0)
        .plan(AWAY);
    // Won, and the plan not yet refreshed: its duties are a tick stale.
    pitch.stand(MCL, 450.0, 272.0).on_the_ball(MCL);
    for id in [DCL, DCR, DL, DR] {
        for state in [DefenderState::Marking, DefenderState::Covering] {
            let next = pitch.answer(id, state);
            assert!(
                !matches!(next, DefenderState::Marking | DefenderState::Covering),
                "defender {id} from {state:?} went to {next:?}"
            );
        }
    }
}

// ── One assignment ──────────────────────────────────────────────────────

#[test]
fn a_midfielder_is_closing_and_the_defender_holds() {
    let mut pitch = Pitch::new();
    // 12 m outside the area, the midfielder on him, the nearest defender 8 m off.
    pitch
        .stand(AWAY_FL, 228.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(MCL, 248.0, 268.0)
        .stand(DCL, 164.0, 272.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(MCL));
    assert!(!pitch.with_ctx(DCL, DefenderRead::is_man_for_carrier));
    for state in OFF_BALL {
        let next = pitch.answer(DCL, state);
        assert!(
            !matches!(next, DefenderState::Pressing | DefenderState::Jockeying),
            "from {state:?} he went to close the carrier ({next:?})"
        );
    }
}

#[test]
fn a_man_given_to_another_defender_is_left_to_him() {
    let mut pitch = Pitch::new();
    let y = pitch.field.get_player(DCR).unwrap().position.y;
    pitch
        .stand(AWAY_MR, 600.0, 272.0)
        .on_the_ball(AWAY_MR)
        .stand(AWAY_FR, LINE_X + 30.0, y + 10.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.mark_of(DCR), Some(AWAY_FR));
    assert_eq!(pitch.context.defence_home.mark_of(DR), None);

    // Both men now in the right-back's zone, the marked one the more
    // dangerous: nearer the goal and nearer him.
    let slot_y = pitch.with_ctx(DR, DefensiveLine::compact_slot_y);
    pitch
        .stand(DR, LINE_X, slot_y)
        .stand(AWAY_FR, LINE_X + 20.0, slot_y + 10.0)
        .stand(AWAY_FL, LINE_X + 120.0, slot_y - 20.0);
    let man = pitch.with_ctx(DR, DefenderRead::man).map(|m| m.id);
    assert_eq!(man, Some(AWAY_FL));
}

// ── One marking model ───────────────────────────────────────────────────

#[test]
fn tight_near_goal_off_in_midfield() {
    let goal = Vector3::new(0.0, 272.0, 0.0);
    for tight in [7.0, 14.0] {
        let near = DefenderMarkingState::stand_off(goal, Vector3::new(160.0, 272.0, 0.0), tight);
        let far = DefenderMarkingState::stand_off(goal, Vector3::new(360.0, 272.0, 0.0), tight);
        assert!(near <= 14.0, "{near}u off a man 20 m out");
        assert!(far <= 20.0 && far >= near, "{far}u off a man 45 m out");
    }
}

// ── The slot ────────────────────────────────────────────────────────────

#[test]
fn every_shape_state_steers_at_the_one_point() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_MR, 600.0, 272.0)
        .on_the_ball(AWAY_MR)
        .plan(AWAY);
    let at = pitch.field.get_player(DCL).unwrap().position;
    pitch.stand(DCL, at.x + 60.0, at.y + 80.0);
    pitch.put_in(DCL, DefenderState::HoldingLine);
    let (point, position) =
        pitch.with_ctx(DCL, |ctx| (DefensiveSlot::point(ctx), ctx.player.position));
    let to_point = (point - position).xy().normalize();
    for state in [
        DefenderState::HoldingLine,
        DefenderState::Walking,
        DefenderState::PushingUp,
        DefenderState::Returning,
        DefenderState::TrackingBack,
    ] {
        let heading = pitch.velocity(DCL, state).xy().normalize();
        assert!(
            heading.dot(&to_point) > 0.999,
            "{state:?} steers elsewhere: {heading:?}"
        );
    }
}

#[test]
fn a_centre_back_is_not_pushed_up_like_a_full_back() {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_MR, 450.0, 272.0)
        .on_the_ball(AWAY_MR)
        .plan(AWAY);
    // The line on its own depth, so neither bound on it is in play.
    for id in [DL, DCL, DCR, DR] {
        let start = pitch.field.get_player(id).unwrap().start_position;
        pitch.stand(id, start.x, start.y);
        pitch.context.tactical_home.defensive_line_x = start.x;
    }
    let flank_y = pitch.field.get_player(DL).unwrap().start_position.y;
    let depth = |pitch: &mut Pitch, id: u32, ball_y: f32| {
        pitch.stand(AWAY_MR, 450.0, ball_y).on_the_ball(AWAY_MR);
        pitch.with_ctx(id, |ctx| DefensiveSlot::point(ctx).x)
    };
    let full_back = depth(&mut pitch, DL, flank_y) - depth(&mut pitch, DL, 272.0);
    let centre_back = depth(&mut pitch, DCL, flank_y) - depth(&mut pitch, DCL, 272.0);
    assert!(
        full_back > 0.0,
        "the full-back is not pushed up: {full_back}"
    );
    assert!(
        centre_back.abs() < 1e-3,
        "the centre-back moved {centre_back}"
    );
}

// ── The duel ────────────────────────────────────────────────────────────

/// The carrier `gap` units in front of a goal-side centre-back who is the
/// plan's presser, both 30 m out.
fn duel(gap: f32) -> Pitch {
    let mut pitch = Pitch::new();
    pitch
        .stand(AWAY_FL, 240.0 + gap, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(DCL, 240.0, 272.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(DCL));
    pitch
}

#[test]
fn a_defender_who_declines_the_challenge_keeps_containing() {
    let mut pitch = duel(TackleEngagement::CONTACT - 2.0);
    pitch.field.get_player_mut(DCL).unwrap().tackle_cooldown = 100;
    for _ in 0..50 {
        let change = pitch.process(DCL, DefenderState::Jockeying);
        assert!(
            change.and_then(|c| c.state).is_none(),
            "he left his containment"
        );
    }
    let carrier = pitch.field.get_player(AWAY_FL).unwrap().position;
    let me = pitch.field.get_player(DCL).unwrap().position;
    assert!(me.x < carrier.x, "goal-side");
}

#[test]
fn a_carrier_beyond_disengage_is_closed_down_again() {
    let mut pitch = duel(TackleEngagement::DISENGAGE + 8.0);
    assert_eq!(
        pitch.answer(DCL, DefenderState::Jockeying),
        DefenderState::Pressing
    );
}

#[test]
fn a_carrier_who_releases_the_ball_hands_him_to_the_read() {
    let mut pitch = duel(TackleEngagement::CONTACT);
    pitch.field.ball.current_owner = None;
    pitch.field.ball.velocity = Vector3::new(2.0, 0.0, 0.0);
    pitch.put_in(DCL, DefenderState::Jockeying);
    let read = pitch.with_ctx(DCL, DefenderRead::resolve);
    assert_eq!(pitch.answer(DCL, DefenderState::Jockeying), read);
    assert_ne!(read, DefenderState::Jockeying);
}

#[test]
fn a_closing_defender_on_cooldown_contains() {
    let mut pitch = duel(TackleEngagement::COMMIT - 2.0);
    pitch.field.get_player_mut(DCL).unwrap().tackle_cooldown = 100;
    assert_eq!(
        pitch.answer(DCL, DefenderState::Pressing),
        DefenderState::Jockeying
    );
}

#[test]
fn a_beaten_defender_only_has_the_stretched_challenge() {
    let mut pitch = Pitch::new();
    // A tackler at the carrier's shoulder, the carrier a stride past him.
    pitch
        .stand(AWAY_FL, 150.0, 272.0)
        .on_the_ball(AWAY_FL)
        .stand(DCL, 162.0, 268.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.presser(), Some(DCL));
    let tackler = pitch.field.get_player_mut(DCL).unwrap();
    tackler.stretch_ticks = 10;
    tackler.skills.technical.tackling = 15.0;
    tackler.skills.mental.aggression = 14.0;
    tackler.skills.mental.decisions = 12.0;
    let mut challenged = 0;
    for _ in 0..3000 {
        let Some(change) = pitch.process(DCL, DefenderState::Pressing) else {
            continue;
        };
        assert_eq!(
            change.state,
            Some(PlayerState::Defender(DefenderState::Tackling)),
            "a beaten man does not contain"
        );
        assert_eq!(change.staged_challenge, Some(ChallengeKind::Slide));
        challenged += 1;
    }
    assert!(challenged > 0, "the stretched challenge never came");
}

// ── The challenge ───────────────────────────────────────────────────────

/// A block tackle staged on a defender in contact with the carrier, rolled
/// `rolls` times: every roll leaves `Tackling` in the tick it was made, as
/// one of the three outcomes.
fn challenges(rolls: usize) -> [usize; 3] {
    let mut pitch = duel(TackleEngagement::CONTACT - 2.0);
    let defender = pitch.field.get_player_mut(DCL).unwrap();
    defender.skills.technical.tackling = 14.0;
    defender.skills.mental.aggression = 12.0;
    defender.skills.mental.decisions = 12.0;
    defender.skills.mental.positioning = 12.0;
    let carrier = pitch.field.get_player_mut(AWAY_FL).unwrap();
    carrier.skills.technical.dribbling = 14.0;
    carrier.skills.technical.first_touch = 12.0;
    let mut seen = [0usize; 3];
    for _ in 0..rolls {
        pitch.put_in(DCL, DefenderState::Tackling);
        pitch.field.get_player_mut(DCL).unwrap().staged_challenge = Some(ChallengeKind::Block);
        let mut change = pitch
            .with_ctx(DCL, |ctx| DefenderTacklingState::default().process(ctx))
            .expect("a challenge is decided on entry");
        assert!(change.start_tackle_cooldown);
        let events: Vec<Event> = change.events.drain().collect();
        let won = change.state == Some(PlayerState::Defender(DefenderState::Running))
            && events
                .iter()
                .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::TacklingBall(DCL))));
        let fouled = events.iter().any(|e| {
            matches!(e, Event::PlayerEvent(PlayerEvent::CommitFoul(foul))
                if foul.fouled == AWAY_FL
                    && foul.source == FoulSource::Tackle
                    && foul.challenge == Some(ChallengeKind::Block))
        });
        let missed = change.missed_challenge == Some(ChallengeKind::Block)
            && change.state.is_none()
            && events.is_empty();
        assert_eq!(
            [won, fouled, missed].iter().filter(|o| **o).count(),
            1,
            "a challenge has exactly one outcome"
        );
        seen[[won, fouled, missed].iter().position(|o| *o).unwrap()] += 1;
    }
    seen
}

#[test]
fn a_challenge_is_won_fouled_or_missed_in_the_tick_it_is_made() {
    let [won, fouled, missed] = challenges(3000);
    assert!(won > 0, "never won");
    assert!(fouled > 0, "never a foul");
    assert!(missed > 0, "never missed");
}

#[test]
fn a_challenge_with_nothing_staged_is_the_reads() {
    let mut pitch = duel(TackleEngagement::CONTACT - 2.0);
    pitch.put_in(DCL, DefenderState::Tackling);
    let read = pitch.with_ctx(DCL, DefenderRead::resolve);
    let change = pitch
        .with_ctx(DCL, |ctx| DefenderTacklingState::default().process(ctx))
        .expect("he leaves");
    assert_eq!(change.state, Some(PlayerState::Defender(read)));
    assert!(change.missed_challenge.is_none() && !change.events.has_events());
}

// ── The line's step ─────────────────────────────────────────────────────

/// The offside trap's picture: the back four flat 37.5 m out, the carrier
/// 19 m in front of it and a runner on the shoulder of the line.
#[test]
fn the_runners_marker_steps_past_him_with_the_line() {
    const STEP_LINE: f32 = 300.0;
    let mut pitch = Pitch::new();
    for id in [DL, DCL, DCR, DR] {
        let y = pitch.field.get_player(id).unwrap().position.y;
        pitch.stand(id, STEP_LINE, y);
        pitch
            .field
            .get_player_mut(id)
            .unwrap()
            .skills
            .mental
            .anticipation = 18.0;
    }
    pitch
        .stand(AWAY_MR, STEP_LINE + 150.0, 272.0)
        .on_the_ball(AWAY_MR)
        .stand(AWAY_FL, STEP_LINE + 5.0, 250.0)
        .plan(AWAY);
    let tick = pitch.context.current_tick();
    pitch
        .context
        .defence_home
        .call_line_step(&pitch.field, HOME, 0.65, 0.56, tick);
    let marker = [DL, DCL, DCR, DR]
        .into_iter()
        .find(|id| pitch.context.defence_home.mark_of(*id) == Some(AWAY_FL))
        .expect("somebody marks the runner");
    pitch.context.total_match_time += 600;
    pitch.put_in(marker, DefenderState::Marking);
    let read = pitch.with_ctx(marker, LineStep::is_read);
    assert!(read, "the call is live and read");
    assert!(
        pitch
            .with_ctx(marker, DefensiveRecovery::depth_override)
            .is_none()
    );
    let runner_x = pitch.field.get_player(AWAY_FL).unwrap().position.x;
    let me_x = pitch.field.get_player(marker).unwrap().position.x;
    assert!(me_x < runner_x);
    assert!(
        pitch.velocity(marker, DefenderState::Marking).x > 0.0,
        "he holds goal-side of the runner instead of stepping"
    );
}

// ── Possession ──────────────────────────────────────────────────────────

#[test]
fn in_possession_each_defender_holds_his_own_slot() {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 620.0, 272.0).on_the_ball(MCL);
    let points: Vec<(u32, Vector3<f32>)> = [DL, DCL, DCR, DR]
        .into_iter()
        .map(|id| {
            pitch.put_in(id, DefenderState::PushingUp);
            (id, pitch.with_ctx(id, DefensiveSlot::point))
        })
        .collect();
    for (id, point) in &points {
        let me = pitch.field.get_player(*id).unwrap().position;
        let nearest = points
            .iter()
            .min_by(|a, b| (a.1 - me).xy().norm().total_cmp(&(b.1 - me).xy().norm()))
            .unwrap()
            .0;
        assert_eq!(
            nearest, *id,
            "defender {id} is nearer another's place ({point:?})"
        );
    }
}

#[test]
fn in_possession_the_deepest_man_moves_up_with_the_line() {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 620.0, 272.0).on_the_ball(MCL);
    let y = pitch.field.get_player(DCL).unwrap().position.y;
    pitch.stand(DCL, LINE_X - 30.0, y);
    assert!(pitch.velocity(DCL, DefenderState::PushingUp).x > 0.0);
}

#[test]
fn losing_the_ball_sends_nobody_to_cover_by_default() {
    let mut pitch = Pitch::new();
    for id in [DL, DCL, DCR, DR] {
        let y = pitch.field.get_player(id).unwrap().position.y;
        pitch.stand(id, 420.0, y);
    }
    pitch
        .stand(AWAY_MR, 560.0, 272.0)
        .on_the_ball(AWAY_MR)
        .plan(AWAY);
    for id in [DL, DCL, DCR, DR] {
        pitch.put_in(id, DefenderState::PushingUp);
        let next = pitch.with_ctx(id, DefenderRead::resolve);
        let duty = pitch.context.defence_home.duty_of(id);
        assert!(
            next != DefenderState::Covering || duty == DefensiveDuty::Cover,
            "defender {id} sent to cover with duty {duty:?}"
        );
    }
}

// ── The pace the play asks for ──────────────────────────────────────────

/// u/tick to m/s: 0.125 m a unit, 100 ticks a second.
const MS: f32 = 12.5;

#[test]
fn far_from_the_play_he_walks() {
    let mut pitch = Pitch::new();
    pitch.stand(MCL, 620.0, 272.0).on_the_ball(MCL);
    pitch.put_in(DCR, DefenderState::Walking);
    let slot = pitch.with_ctx(DCR, DefensiveSlot::point);
    for off in [0.0, 80.0] {
        pitch.stand(DCR, slot.x - off, slot.y);
        let pace = pitch.with_ctx(DCR, |ctx| ShapeTempo::to(ctx, slot).pace());
        assert!(
            pace * MS <= 2.0,
            "{} m/s from {} m off his slot",
            pace * MS,
            off / 8.0
        );
    }
}

#[test]
fn with_the_ball_in_behind_him_he_runs_back_at_full_effort() {
    let mut pitch = Pitch::new();
    pitch
        .stand(DCR, 300.0, 300.0)
        .stand(AWAY_FL, 200.0, 280.0)
        .on_the_ball(AWAY_FL)
        .plan(AWAY);
    for state in [
        DefenderState::HoldingLine,
        DefenderState::Walking,
        DefenderState::Marking,
    ] {
        pitch.put_in(DCR, state);
        assert!(!pitch.with_ctx(DCR, ShapeTempo::applies), "from {state:?}");
        let answer = pitch.with_ctx(DCR, DefenderRead::resolve);
        assert!(
            matches!(
                answer,
                DefenderState::Covering | DefenderState::TrackingBack
            ),
            "from {state:?} the read answered {answer:?}"
        );
    }
}

#[test]
fn with_the_ball_dead_he_walks_into_his_place() {
    let mut pitch = Pitch::new();
    pitch.stand(AWAY_MR, 600.0, 545.0).on_the_ball(AWAY_MR);
    pitch.field.ball.current_owner = None;
    let now = pitch.context.current_tick();
    pitch.field.ball.awaiting_restart = Some(AwaitedRestart {
        taker_id: AWAY_MR,
        spot: Vector3::new(600.0, 545.0, 0.0),
        take_from: None,
        settled: true,
        carrying: false,
        origin: PassOriginRestart::ThrowIn,
        awarded_tick: now,
        patience_ticks: AwaitedRestart::PATIENCE_TICKS,
        settled_tick: None,
        hold: None,
        quiet: RestartQuiet::default(),
    });
    pitch.put_in(DCR, DefenderState::Returning);
    let slot = pitch.with_ctx(DCR, DefensiveSlot::point);
    pitch.stand(DCR, slot.x + 64.0, slot.y);
    let pace = pitch.with_ctx(DCR, |ctx| ShapeTempo::to(ctx, slot).pace());
    assert!(
        pace * MS <= 1.6,
        "{} m/s, 8 m from his place with the ball dead",
        pace * MS
    );
}

#[test]
fn the_marker_of_a_walking_man_walks() {
    let mut pitch = Pitch::new();
    let y = pitch.field.get_player(DCR).unwrap().position.y;
    pitch
        .stand(AWAY_MR, 450.0, 272.0)
        .on_the_ball(AWAY_MR)
        .stand(AWAY_FR, LINE_X + 30.0, y + 10.0)
        .plan(AWAY);
    assert_eq!(pitch.context.defence_home.mark_of(DCR), Some(AWAY_FR));
    // His man strolls across the line at 1.25 m/s; the marker is walked
    // along with him until he is in position.
    let stroll = Vector3::new(0.0, 0.1, 0.0);
    pitch.field.get_player_mut(AWAY_FR).unwrap().velocity = stroll;
    let mut speed = 0.0;
    for _ in 0..120 {
        let v = pitch.velocity(DCR, DefenderState::Marking);
        speed = v.xy().norm();
        let marker = pitch.field.get_player_mut(DCR).unwrap();
        marker.position += Vector3::new(v.x, v.y, 0.0);
        marker.velocity = v;
        pitch.field.get_player_mut(AWAY_FR).unwrap().position += stroll;
    }
    assert!(
        speed * MS <= 2.0,
        "{} m/s on a man walking at 1.25",
        speed * MS
    );
}

// ── How he arrives counts ───────────────────────────────────────────────

/// The same challenge on the same carrier, `rolls` times: (won, fouls).
fn challenges_made(arriving: Vector3<f32>, set_ticks: u16, rolls: u64) -> (usize, usize) {
    let mut pitch = duel(TackleEngagement::CONTACT - 2.0);
    let defender = pitch.field.get_player_mut(DCL).unwrap();
    defender.velocity = arriving;
    defender.contact_ticks = set_ticks;
    defender.skills.technical.tackling = 14.0;
    defender.skills.mental.aggression = 12.0;
    let (mut won, mut fouls) = (0, 0);
    for seed in 0..rolls {
        pitch.context.rng = MatchRng::from_seed(seed);
        let challenger = pitch.field.get_player(DCL).unwrap();
        let carrier = pitch.field.get_player(AWAY_FL).unwrap();
        match BlockChallenge::between(challenger, carrier, &pitch.context)
            .resolve(&pitch.context.rng, None)
        {
            TackleOutcome::Won => won += 1,
            TackleOutcome::Foul(_) => fouls += 1,
            TackleOutcome::Missed => {}
        }
    }
    (won, fouls)
}

#[test]
fn diving_in_wins_less_and_fouls_more_than_being_set() {
    let sprint = Vector3::new(0.6, 0.0, 0.0);
    let (rushed_won, rushed_fouls) = challenges_made(sprint, 0, 2_000);
    let (set_won, set_fouls) = challenges_made(Vector3::zeros(), 25, 2_000);
    assert!(
        set_won > rushed_won,
        "set won {set_won}, at a sprint {rushed_won}"
    );
    assert!(
        set_fouls < rushed_fouls,
        "set fouled {set_fouls}, at a sprint {rushed_fouls}"
    );
}

#[test]
fn pace_can_beat_a_defender_backing_off() {
    let mut pitch = duel(TackleEngagement::COMMIT - 2.0);
    let conditions = pitch.context.conditions.clone();
    let defender = pitch.field.get_player_mut(DCL).unwrap();
    DefenderJockeyingState::default().process_conditions(ConditionContext {
        in_state_time: 10,
        player: defender,
        conditions: &conditions,
        match_progress: 0.3,
        side_on_ball: false,
        ball_dead: false,
    });
    assert!(matches!(
        defender.last_activity_intensity,
        ActivityIntensity::High
    ));
    let carrier = pitch.field.get_player(AWAY_FL).unwrap();
    assert!(
        MovementEffort::speed_fraction(ActivityIntensity::High, 100)
            < MovementEffort::carrier_ceiling(carrier, 30, 100),
        "a man containing keeps up with a carrier at full stride"
    );
}

// ── The line drops off an unpressed ball ────────────────────────────────

#[test]
fn time_on_the_ball_drops_the_line_and_pressure_brings_it_back() {
    let mut pitch = Pitch::new();
    // A carrier 35 m from goal, a runner on the shoulder of the last man.
    pitch
        .stand(AWAY_MR, 280.0, 272.0)
        .on_the_ball(AWAY_MR)
        .stand(AWAY_FL, LINE_X + 4.0, 250.0)
        .plan(AWAY);
    let free = pitch.with_ctx(DCR, |ctx| {
        (DefensiveSlot::drop(ctx), DefensiveSlot::point(ctx).x)
    });
    // A team-mate 2 m off him.
    pitch.stand(MCL, 264.0, 272.0);
    let pressed = pitch.with_ctx(DCR, |ctx| {
        (DefensiveSlot::drop(ctx), DefensiveSlot::point(ctx).x)
    });
    assert!(free.0 > 0.0 && free.0 <= 48.0, "dropped {} u", free.0);
    assert_eq!(
        pressed.0, 0.0,
        "the line still drops with the carrier pressed"
    );
    assert!(free.1 < pressed.1, "time on the ball held no deeper a line");
}

/// **A defender plays the cross he won.** Up for a set piece or arriving at
/// the far post, the header the contest gave him is his to strike — at goal
/// from a chance, glanced away otherwise — and not an award that lapses
/// because he is further than eleven metres out.
#[test]
fn a_defender_who_wins_a_cross_heads_it() {
    for x in [770.0, 735.0] {
        let mut pitch = Pitch::new();
        pitch.stand(DCL, x, 272.0);
        pitch.field.get_player_mut(DCL).unwrap().on_aerial_award();
        pitch.field.ball.current_owner = None;
        pitch.field.ball.position = Vector3::new(x + 0.4, 272.0, 2.0);
        pitch.field.ball.velocity = Vector3::new(0.1, 0.0, -0.02);
        pitch.field.ball.aerial_contest_winner = Some(DCL);
        let change = pitch
            .with_ctx(DCL, |ctx| DefenderHeadingState::default().process(ctx))
            .expect("he plays the header");
        let mut events = change.events;
        let events: Vec<Event> = events.drain().collect();
        assert!(
            events.iter().any(|e| matches!(
                e,
                Event::PlayerEvent(
                    PlayerEvent::Shoot(_) | PlayerEvent::KnockDown(..) | PlayerEvent::ClearBall(..)
                )
            )),
            "a won header {:.0} m out is struck",
            (840.0 - x) * 0.125
        );
    }
}

/// **The full-back who overlapped does the same.** Wide and high with the
/// ball at the byline and the goal line open, he carries on into the box
/// exactly as a winger would, and his running carry steers there.
#[test]
fn an_overlapping_full_back_carries_into_the_box_like_a_winger() {
    let mut pitch = Pitch::new();
    for id in 201..=210 {
        let y = pitch.field.get_player(id).unwrap().position.y;
        pitch.stand(id, 300.0, y);
    }
    pitch.stand(DL, 840.0 - 44.0, 272.0 + 165.0).on_the_ball(DL);
    let aim = pitch
        .with_ctx(DL, |ctx| FlankPlay::carry_aim(ctx, 0.0))
        .expect("a carry");
    assert!(
        (73.0..100.0).contains(&(aim.y - 272.0).abs()),
        "the six-yard box's corner"
    );
    let velocity = pitch
        .with_ctx(DL, |ctx| DefenderRunningState::default().velocity(ctx))
        .expect("he carries it");
    assert!(
        velocity.y < 0.0 && velocity.x > 0.0,
        "along the line toward the near post: {velocity:?}"
    );
}

/// A full-back on the ball against his own touchline `x` from his own goal
/// line: an opponent `presser_gap` off him, another up the line ahead, and
/// his centre-back across the pitch, out of a pass.
fn pinned_at_the_touchline(pitch: &mut Pitch, x: f32, presser_gap: f32) {
    pitch.stand(DCL, x - 10.0, 272.0);
    pitch.stand(DL, x, 10.0).on_the_ball(DL);
    pitch.stand(AWAY_FL, x - presser_gap, 14.0);
    pitch.stand(AWAY_FR, x + 40.0, 12.0);
}

/// **Pinned with no out, he puts it out** — over his own touchline, up
/// the line.
#[test]
fn a_pinned_full_back_with_no_out_puts_it_out() {
    let mut pitch = Pitch::new();
    pinned_at_the_touchline(&mut pitch, 150.0, 4.0);
    assert_eq!(
        pitch.with_ctx(DL, TouchlineCall::decide),
        Some(DefensiveRelease::PutOut)
    );
    let change = pitch
        .with_ctx(DL, |ctx| DefenderRunningState::default().process(ctx))
        .expect("he plays it");
    let mut events = change.events;
    let put_out = events
        .drain()
        .find_map(|e| match e {
            Event::PlayerEvent(PlayerEvent::PutOut(id, velocity)) if id == DL => Some(velocity),
            _ => None,
        })
        .expect("he puts it out");
    assert!(put_out.y < 0.0, "over his own touchline");
    assert!(put_out.x > 0.0, "up the line, away from his goal");
}

/// …and with a safe pass on, he plays it instead.
#[test]
fn with_a_safe_pass_the_pinned_full_back_plays_it() {
    let mut pitch = Pitch::new();
    pinned_at_the_touchline(&mut pitch, 150.0, 4.0);
    pitch.stand(DCL, 210.0, 70.0);
    assert_eq!(pitch.with_ctx(DL, TouchlineCall::decide), None);
    let change = pitch
        .with_ctx(DL, |ctx| DefenderRunningState::default().process(ctx))
        .expect("he plays it");
    let mut events = change.events;
    let events: Vec<Event> = events.drain().collect();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::PutOut(..)))),
        "he does not put it out"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::PassTo(_))))
            || change.state == Some(PlayerState::Defender(DefenderState::Passing)),
        "he plays the pass"
    );
}

/// Under the same pressure with the same outs, nearer halfway than his own
/// goal, the limited player puts it out and the composed one rides it.
#[test]
fn the_limited_player_puts_it_out_more_readily() {
    let decide = |skill: f32| {
        let mut pitch = Pitch::new();
        pinned_at_the_touchline(&mut pitch, 330.0, 12.0);
        let s = &mut pitch.field.get_player_mut(DL).unwrap().skills;
        for v in [
            &mut s.technical.passing,
            &mut s.technical.technique,
            &mut s.technical.first_touch,
            &mut s.mental.composure,
            &mut s.mental.decisions,
            &mut s.mental.vision,
            &mut s.mental.concentration,
            &mut s.mental.teamwork,
        ] {
            *v = skill;
        }
        pitch.with_ctx(DL, TouchlineCall::decide)
    };
    assert_eq!(
        decide(2.0),
        Some(DefensiveRelease::PutOut),
        "the limited player"
    );
    assert_eq!(decide(20.0), None, "the composed player");
}

/// **A choice between clear lanes takes the cleaner.** A safe pass either
/// side of the man on the ball, the two the same in everything but one
/// opponent near one lane: near enough to price it, not near enough to
/// close it. The safe-pass search takes the other, whichever side he
/// stands.
#[test]
fn a_safe_pass_search_takes_the_cleaner_of_two_clear_lanes() {
    for (contested, clean) in [(DL, DR), (DR, DL)] {
        let mut pitch = Pitch::new();
        pitch.stand(DCR, 140.0, 520.0);
        pitch.stand(DCL, 300.0, 272.0).on_the_ball(DCL);
        pitch.stand(DL, 300.0, 200.0);
        pitch.stand(DR, 300.0, 344.0);
        let lane_y = (pitch.field.get_player(contested).unwrap().position.y + 272.0) / 2.0;
        let risk = (1..=40)
            .rev()
            .map(|off| {
                pitch.stand(AWAY_FL, 300.0 + off as f32, lane_y);
                pitch.with_ctx(DCL, |ctx| ctx.player().lane_risk(contested))
            })
            .find(|risk| (0.02..0.25).contains(risk))
            .expect("a man near enough to price the lane and not to close it");
        assert_eq!(
            pitch.with_ctx(DCL, |ctx| ctx.player().lane_risk(clean)),
            0.0
        );
        let pick = pitch.with_ctx(DCL, |ctx| {
            ctx.player().passing().find_safe_pass_option().map(|t| t.id)
        });
        assert_eq!(
            pick,
            Some(clean),
            "a lane priced at {risk} against a clean one"
        );
    }
}
