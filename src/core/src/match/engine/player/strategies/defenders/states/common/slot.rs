//! **The one point a defender holding the shape stands on.** Every state
//! that holds the shape steers at it, so two defenders with the same job in
//! two different states stand on the same spot.

use crate::r#match::DefensiveDuty;
use crate::r#match::StateProcessingContext;
use crate::r#match::defenders::states::common::{DefensiveLine, LineStep};
use nalgebra::Vector3;

pub struct DefensiveSlot;

impl DefensiveSlot {
    /// How far off the line the covering defender drops (~4.5 m): the near
    /// end of the back line's diagonal.
    pub const COVER_DROP: f32 = 36.0;
    /// How far the far-side defender tucks back with the ball fully on the
    /// other flank (~7.5 m), scaled by how far across the ball is.
    pub const FAR_SIDE_DROP: f32 = 60.0;
    /// How far a full-back steps up with the ball on his touchline (~1.9 m),
    /// less the nearer the middle it is, so his slot does not jump as the
    /// ball crosses it. He is wide because of where he plays, not where he
    /// happens to stand.
    pub const FULL_BACK_PUSH: f32 = 15.0;
    /// How far the line drops off a carrier with time on the ball and the
    /// space behind it in his range, at most (6 m).
    const MAX_DROP: f32 = 48.0;
    /// A carrier is pressed with a defending player within 3 m of him, and
    /// has all the time he wants with nobody within 6 m.
    const PRESSED: f32 = 24.0;
    const UNPRESSED: f32 = 48.0;
    /// The longest pass that puts a ball in behind the line (45 m).
    const PASS_RANGE: f32 = 360.0;

    /// A/B control: with `OF_DEFENDER_LINE_DROP=off` the line holds the
    /// depth its block gives it whoever is on the ball. Read once per
    /// process — debug infrastructure, do not remove.
    pub fn line_drop_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_DEFENDER_LINE_DROP").is_ok_and(|v| v == "off"))
    }

    /// How far the line drops off the carrier, in units: growing with the
    /// time he has (nobody within 3 m, all of it with nobody within 6 m)
    /// and with how much of the space behind the line is in his range.
    pub fn drop(ctx: &StateProcessingContext) -> f32 {
        if Self::line_drop_off() || ctx.team().is_control_ball() {
            return 0.0;
        }
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            return 0.0;
        };
        let pressure = ctx
            .players()
            .teammates()
            .all()
            .chain(std::iter::once(ctx.player.into()))
            .map(|mate| (mate.position - carrier.position).xy().norm())
            .fold(f32::MAX, f32::min);
        let time = ((pressure - Self::PRESSED) / (Self::UNPRESSED - Self::PRESSED)).clamp(0.0, 1.0);
        let to_line = (carrier.position.x - DefensiveLine::position_x(ctx)).abs();
        let range = (1.0 - to_line / Self::PASS_RANGE).clamp(0.0, 1.0);
        Self::MAX_DROP * time * range
    }

    pub fn point(ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_centre_y = ctx.context.field_size.height as f32 / 2.0;
        let ball = ctx.tick_context.positions.ball.position;
        let forward = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());

        // Depth: where the block wants him, staggered by his duty.
        let depth = ctx
            .team()
            .shape()
            .anchor_of(ctx.player.id)
            .map_or(ctx.player.start_position.x, |anchor| anchor.x);
        let defending = !ctx.team().is_control_ball();
        let ball_across = (ball.y - field_centre_y) / field_centre_y.max(1.0);
        let my_across = (ctx.player.start_position.y - field_centre_y) / field_centre_y.max(1.0);
        let stagger = match ctx.team().my_duty() {
            DefensiveDuty::Cover if defending => Self::COVER_DROP,
            DefensiveDuty::HoldZone if defending => {
                (-ball_across * my_across).clamp(0.0, 1.0) * Self::FAR_SIDE_DROP
            }
            _ => 0.0,
        };
        let full_back = ctx.player.tactical_position.current_position.is_full_back();
        let push = if full_back {
            (ball_across * my_across.signum()).clamp(0.0, 1.0) * Self::FULL_BACK_PUSH
        } else {
            0.0
        };
        let x = depth + forward * (push - stagger - Self::drop(ctx));

        // Width: the squeezed line, shifted toward the ball.
        let compactness = ctx.team().compactness_target();
        let y = DefensiveLine::compact_slot_y(ctx)
            + (ball.y - field_centre_y) * (0.08 + compactness * 0.18);

        let shaped = DefensiveLine::hold_shape(ctx, Vector3::new(x, y, 0.0));
        match LineStep::depth(ctx, shaped.x) {
            Some(stepped) => Vector3::new(stepped, shaped.y, 0.0),
            None => shaped,
        }
    }
}
