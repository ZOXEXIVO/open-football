//! Shared crossing model. Both the forwarder and midfielder crossing
//! states drive their delivery through [`CrossModel`], so the cross-type
//! / aim-point / aerial-duel model is consistent across roles.
//!
//! # Why a cross is not a pass
//!
//! A cross used to be emitted as a plain `PassTo` at a teammate's feet,
//! and the trajectory solver then chose its shape from *lane traffic*
//! (`select_trajectory_type_contextual`). Those two rules are mutually
//! exclusive: the crossing states only accepted a target with a clear
//! straight lane, and the solver only lifted the ball off the deck when
//! the lane was blocked — so by construction an open-play cross was a
//! ground pass rolled across the face of goal. Nobody could contest it
//! either, because a named pass target holds exclusive claim for the
//! whole flight.
//!
//! So a cross is modelled here as its own action:
//!
//! * a [`CrossType`] chosen from geometry and the target's profile,
//! * an **aim point** — a patch of the box, not a pair of feet — because
//!   that is what a crosser actually hits and what lets more than one
//!   player attack the delivery,
//! * a flight shape ([`CrossType::apex_metres`]) the pass solver honours
//!   directly instead of re-deriving from lane traffic,
//! * and, for lofted deliveries, a single aerial contest resolved by the
//!   engine (`resolve_cross_contest`) rather than by whichever player's
//!   state machine happened to run first.

use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::ball::ball::contest::pass_block::{PassBlock, StrikeLine};
use crate::r#match::engine::ball::ball::{AerialReach, Ball, GRAVITY_PER_TICK};
use crate::r#match::engine::corner_shape::CornerShape;
use crate::r#match::engine::environment::EnvModifiers;
use crate::r#match::engine::set_pieces::CornerRoutine;
use crate::r#match::engine::teamplay::standard::MatchStandard;
use crate::r#match::engine::zones::LateralLane;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{
    MatchContext, MatchPlayer, MatchPlayerLite, PlayerSide, StateProcessingContext,
};
use nalgebra::Vector3;

/// Half-width of the penalty area in game units (20.16 m at 0.125 m/u).
const BOX_HALF_WIDTH: f32 = 161.0;
/// Depth of the penalty area from the goal line (16.5 m).
const BOX_DEPTH: f32 = 132.0;
/// Contact radius of an aerial challenge, in game units (~4.3 m).
const AERIAL_MARKER_RADIUS: f32 = 34.0;

/// Cross delivery type. Drives flight shape, aim point, and whether the
/// delivery is resolved through the aerial contest or as a ground ball.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossType {
    /// High lofted ball hung up to the back post — attackers attacking
    /// the second six. The slowest delivery and the easiest to defend,
    /// but the one that beats a packed near post.
    FloatedFarPost,
    /// Hard low-driven cross a yard above the grass. Fast and difficult
    /// to defend even with a clean run; resolved on the deck, not in the
    /// air, so a poor header can still attack it.
    DrivenLowCross,
    /// Pulled back from the byline to a runner at the edge of the area.
    /// Ground ball, away from the keeper, into the highest-value shooting
    /// zone in football.
    Cutback,
    /// Whipped delivery across the six-yard line for a flick-on or a
    /// first-time finish. Fast and flat enough that the keeper has to
    /// commit, which is why it is the highest-value aerial ball.
    WhippedNearPost,
    /// Early ball clipped in behind a high line before the defence can
    /// set — aimed at space for a striker to run onto rather than at a
    /// stationary aerial target.
    EarlyCross,
}

impl CrossType {
    /// Whether the delivery arrives through the air (and therefore
    /// resolves through the aerial contest) or along the ground.
    pub fn is_lofted(self) -> bool {
        matches!(
            self,
            CrossType::FloatedFarPost | CrossType::WhippedNearPost | CrossType::EarlyCross
        )
    }

    /// Peak height of the delivery in metres. This is the knob a crosser
    /// actually turns, and it is the only one expressible on the ball's
    /// mixed axes (units horizontally, metres vertically) — see
    /// `PlayerEventDispatcher::target_apex`.
    ///
    /// Longer deliveries climb higher, as they must to carry; the caps
    /// keep every type inside the band its name claims.
    pub fn apex_metres(self, distance_units: f32) -> f32 {
        let metres = distance_units * 0.125;
        match self {
            // Hung up: peaks around head height plus the whole flight,
            // ~2.5 s of hang from a 30 m ball.
            CrossType::FloatedFarPost => (3.2 + metres * 0.13).clamp(3.2, 9.0),
            // Skims the grass — under the knee, so a defender has to get
            // down to it and the keeper cannot claim.
            CrossType::DrivenLowCross => (0.35 + metres * 0.012).clamp(0.35, 1.1),
            // Along the deck by definition.
            CrossType::Cutback => 0.03,
            // Flat and fast — clears the first defender's head and drops
            // onto the six-yard line, no more.
            CrossType::WhippedNearPost => (1.9 + metres * 0.055).clamp(1.9, 4.2),
            // Clipped over the line into space for a runner.
            CrossType::EarlyCross => (2.4 + metres * 0.075).clamp(2.4, 5.5),
        }
    }

    /// Extra weighting-error multiplier. Crossing is a low-percentage
    /// skill, and the hardest deliveries are the ones that have to beat a
    /// defender AND drop inside the six-yard box. Applied on top of the
    /// crossing-skill shortfall in `handle_pass_to_event`.
    pub fn difficulty(self) -> f32 {
        match self {
            CrossType::Cutback => 0.75,
            CrossType::DrivenLowCross => 1.0,
            CrossType::EarlyCross => 1.1,
            CrossType::WhippedNearPost => 1.25,
            CrossType::FloatedFarPost => 1.15,
        }
    }

    /// How much harder than a pass to feet a ball along the deck is
    /// struck. The danger of a driven cross is that nobody has time on it;
    /// a cutback is a firm pass to a man arriving.
    pub fn ground_pace_scale(self) -> f32 {
        match self {
            CrossType::DrivenLowCross => 1.40,
            CrossType::Cutback => 1.15,
            _ => 1.0,
        }
    }

    /// The ball the crosser expects to strike over `distance_units`: its
    /// pace across the grass (u/tick) and its lift (m/tick), before his
    /// weighting error. A lofted ball is solved drag-free to come down
    /// through head height — it prices the first metres of the flight,
    /// where the man in front of him stands and drag has not yet told.
    pub fn launch(self, distance_units: f32, conditions: &EnvModifiers) -> (f32, f32) {
        let lift = Ball::launch_speed_for_apex(self.apex_metres(distance_units));
        if !self.is_lofted() {
            return (
                Ball::pass_pace(distance_units, conditions) * self.ground_pace_scale(),
                lift,
            );
        }
        let falling = (lift * lift - 2.0 * GRAVITY_PER_TICK * AerialReach::ATTACKED)
            .max(0.0)
            .sqrt();
        let ticks = ((lift + falling) / GRAVITY_PER_TICK).max(1.0);
        (distance_units / ticks, lift)
    }

    /// Stable index for diagnostics bucketing. Kept next to the variants
    /// so a new delivery type can't silently land in someone else's bin.
    pub fn diag_index(self) -> usize {
        match self {
            CrossType::FloatedFarPost => 0,
            CrossType::DrivenLowCross => 1,
            CrossType::Cutback => 2,
            CrossType::WhippedNearPost => 3,
            CrossType::EarlyCross => 4,
        }
    }

    /// How much easier this delivery is for an ATTACKER to win in the
    /// air. A whipped or driven ball arrives before the defence can set;
    /// a floated one hangs long enough for them to.
    pub fn contest_edge(self) -> f32 {
        match self {
            CrossType::WhippedNearPost => 0.06,
            CrossType::DrivenLowCross => 0.05,
            CrossType::EarlyCross => 0.02,
            CrossType::FloatedFarPost => -0.02,
            CrossType::Cutback => 0.0,
        }
    }

    /// Multiplier on the keeper's claim probability. A ball fizzed across
    /// the six-yard box is one he cannot come for.
    pub fn keeper_claim_scale(self) -> f32 {
        match self {
            CrossType::WhippedNearPost | CrossType::DrivenLowCross => 0.55,
            _ => 1.0,
        }
    }

    /// Human-readable label for diagnostics output.
    pub fn label(self) -> &'static str {
        match self {
            CrossType::FloatedFarPost => "floated-far",
            CrossType::DrivenLowCross => "driven-low",
            CrossType::Cutback => "cutback",
            CrossType::WhippedNearPost => "whipped-near",
            CrossType::EarlyCross => "early",
        }
    }

    /// Every delivery type, in `diag_index` order.
    pub const ALL: [CrossType; 5] = [
        CrossType::FloatedFarPost,
        CrossType::DrivenLowCross,
        CrossType::Cutback,
        CrossType::WhippedNearPost,
        CrossType::EarlyCross,
    ];
}

/// Decision a crossing state has resolved this tick: which cross to play,
/// who it is for, and where it is actually aimed.
#[derive(Debug, Clone, Copy)]
pub struct CrossDecision {
    pub cross_type: CrossType,
    /// The runner the delivery is FOR. Still needed for assist / pass
    /// accounting, but the ball is not aimed at their feet.
    pub target_id: u32,
    /// Where the ball is actually struck — a patch of the box the runner
    /// is attacking. Aiming at a space rather than a player is what lets
    /// a second attacker and a defender contest the same delivery.
    pub aim_point: Vector3<f32>,
}

/// Crossing decision + aerial-duel calculators. Everything the crossing
/// and heading states need to agree on lives here so the two roles can't
/// drift apart.
pub struct CrossModel;

impl CrossModel {
    /// Whether a player is wide enough to cross. Used by the crossing
    /// states' entry guard.
    /// Is this player wide enough to be crossing?
    ///
    /// NB this is NOT what limits crossing volume, though it looks like
    /// it should be. The engine strikes **2-3 open-play crosses a match
    /// against a real ~30** (the ~14 lofted deliveries it does produce
    /// are almost all corner kicks — see `CrossDiag`), and the block is
    /// only 40 m wide on a 68 m pitch, so its widest man stands ~14 m
    /// infield of the touchline. Two fixes were tried on that reasoning
    /// and BOTH measured nothing: widening the block plan to 64 m (the
    /// occupied width stayed at 40.1 m — players no more occupy lateral
    /// anchors than depth ones), and making this test shape-relative
    /// rather than pitch-relative so the wide man in a narrow side still
    /// counts. Crossing volume is gated by whatever decides to ENTER the
    /// crossing states, not by this predicate.
    pub fn is_in_wide_position(ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let y = ctx.player.position.y;
        let wide_margin = field_height * 0.2;
        y < wide_margin || y > field_height - wide_margin
    }

    /// Is a ball from `from` to `to` a cross: struck from a wide channel
    /// in the attacking third and into the opponents' box, or within 10u
    /// of its edge — a wide delivery to the edge of the area is still a
    /// cross. The distance to the edge used to read zero for any target
    /// level with the box's width however far upfield it was, and every
    /// ball played inside from a wide channel was booked as a cross.
    pub fn is_cross(
        from: Vector3<f32>,
        to: Vector3<f32>,
        side: PlayerSide,
        context: &MatchContext,
    ) -> bool {
        let field_h = context.field_size.height as f32;
        let field_w = context.field_size.width as f32;
        if !LateralLane::classify(from.y, field_h).is_wide()
            || side.attacking_progress_x(from.x, field_w) < 2.0 / 3.0
        {
            return false;
        }
        let opp_box = match side {
            PlayerSide::Left => context.penalty_area(false),
            PlayerSide::Right => context.penalty_area(true),
        };
        let dx = (opp_box.min.x - to.x).max(to.x - opp_box.max.x).max(0.0);
        let dy = (opp_box.min.y - to.y).max(to.y - opp_box.max.y).max(0.0);
        dx.hypot(dy) <= 10.0
    }

    /// Pick the best cross for the current context. `None` when the
    /// crosser has no viable target — the caller should fall back to a
    /// regular pass.
    pub fn pick(ctx: &StateProcessingContext<'_>) -> Option<CrossDecision> {
        Self::pick_rated(ctx).map(|(decision, _)| decision)
    }

    /// [`Self::pick`], with how good the best delivery is: the runner's
    /// fit for the ball, whether he can reach it, how contested its patch
    /// is, how deep it lands and how much of it is the keeper's.
    pub fn pick_rated(ctx: &StateProcessingContext<'_>) -> Option<(CrossDecision, f32)> {
        let goal_pos = ctx.player().opponent_goal_position();
        let crosser_pos = ctx.player.position;
        let crosser_dist_to_goal = (crosser_pos - goal_pos).magnitude();
        let field_height = ctx.context.field_size.height as f32;
        let forward_dir = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());

        let gk_pos = ctx
            .players()
            .opponents()
            .goalkeeper()
            .next()
            .map(|gk| gk.position);
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let delivery = PassBlock::technique(ctx.player, minute, true);

        let mut best: Option<(CrossDecision, f32)> = None;

        for teammate in ctx.players().teammates().all() {
            if teammate.id == ctx.player.id {
                continue;
            }
            if (teammate.position - goal_pos).magnitude() > 260.0 {
                continue;
            }

            // Resolve the runner's profile from the full player record.
            let Some(runner) = ctx.context.players.by_id(teammate.id) else {
                continue;
            };

            let off_the_ball = (runner.skills.mental.off_the_ball / 20.0).clamp(0.0, 1.0);
            let heading = (runner.skills.technical.heading / 20.0).clamp(0.0, 1.0);
            let jumping = (runner.skills.physical.jumping / 20.0).clamp(0.0, 1.0);
            let strength = (runner.skills.physical.strength / 20.0).clamp(0.0, 1.0);
            let anticipation = (runner.skills.mental.anticipation / 20.0).clamp(0.0, 1.0);
            let composure = (runner.skills.mental.composure / 20.0).clamp(0.0, 1.0);
            let finishing = (runner.skills.technical.finishing / 20.0).clamp(0.0, 1.0);

            let cross_type = Self::pick_type(
                ctx,
                crosser_pos,
                crosser_dist_to_goal,
                teammate.position,
                goal_pos,
                heading,
            );

            // Lane requirement, by delivery.
            //
            // A LOFTED ball is played over the traffic by definition —
            // demanding an unobstructed raycast for it is what made
            // open-play crosses impossible, since the trajectory solver
            // then read the clear lane as "keep it down".
            //
            // A CUTBACK is a pass to feet and genuinely needs the lane. A
            // DRIVEN LOW cross does not: it is deliberately fizzed through
            // the six-yard area precisely because bodies are in the way,
            // and it is aimed at a space rather than at the man. Requiring
            // a clean raycast to the runner's feet for it filtered nearly
            // every ground delivery out of the model (driven-low 1%,
            // cutback 0% of deliveries).
            let needs_lane = matches!(cross_type, CrossType::Cutback);
            if needs_lane && !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

            let aim_point = Self::aim_point_for(
                cross_type,
                goal_pos,
                crosser_pos,
                teammate.position,
                forward_dir,
                field_height,
            );

            // How far the runner has to travel to attack the delivery. A
            // ball hung to a spot nobody can reach is a bad cross however
            // good the runner is.
            let runner_gap = (teammate.position - aim_point).magnitude();
            let reachability = (1.0 - (runner_gap / 130.0)).clamp(0.0, 1.0);

            // Marker proximity around the AIM POINT, not around the runner
            // — that is the space actually being contested.
            let contesting = ctx
                .players()
                .opponents()
                .all()
                .filter(|o| (o.position - aim_point).magnitude() < 45.0)
                .count();
            let separation = match contesting {
                0 => 1.0,
                1 => 0.72,
                2 => 0.45,
                _ => 0.25,
            };

            // Goalkeeper claim risk: a ball dropped inside the keeper's
            // zone gets caught. Real crossers aim away from him, and the
            // whipped near-post ball is dangerous precisely because it is
            // too fast to claim.
            let gk_claim_risk = gk_pos
                .map(|gk| {
                    let gap = (aim_point - gk).magnitude();
                    let base = if gap < 90.0 { 1.0 - (gap / 90.0) } else { 0.0 };
                    base * cross_type.keeper_claim_scale()
                })
                .unwrap_or(0.0);

            // Depth bonus: deliveries into the prime zone are worth more.
            let aim_depth = (aim_point - goal_pos).magnitude();
            let depth_bonus = (1.0 - (aim_depth / 190.0)).clamp(0.0, 1.0);

            // A ground delivery is attacked with the feet, an aerial one
            // with the head — score the runner on the attribute the
            // delivery actually asks of them.
            let attack_ability = if cross_type.is_lofted() {
                heading * 0.42 + jumping * 0.32 + strength * 0.26
            } else {
                finishing * 0.45 + composure * 0.30 + anticipation * 0.25
            };

            let score = (attack_ability * 0.34
                + off_the_ball * 0.20
                + anticipation * 0.08
                + reachability * 0.16
                + separation * 0.14
                + depth_bonus * 0.08
                - gk_claim_risk * 0.22)
                .max(0.0)
                * Self::gets_past(ctx, cross_type, aim_point, delivery, minute);

            let candidate = CrossDecision {
                cross_type,
                target_id: teammate.id,
                aim_point,
            };

            if best.as_ref().is_none_or(|(_, bs)| score > *bs) {
                best = Some((candidate, score));
            }
        }

        best
    }

    /// The chance a delivery gets past the men on its line: for each of
    /// them, the block contest's own chance ([`PassBlock::priced`]) on the
    /// ball this delivery will be. A floated ball is over the full-back's
    /// reach and a whipped one goes through him, which is how the crosser
    /// picks the ball that beats the man in front instead of refusing to
    /// cross past him.
    pub(crate) fn gets_past(
        ctx: &StateProcessingContext<'_>,
        cross_type: CrossType,
        aim_point: Vector3<f32>,
        delivery: f32,
        minute: u32,
    ) -> f32 {
        if MatchContext::charge_down_off() {
            return 1.0;
        }
        let Some(side) = ctx.player.side else {
            return 1.0;
        };
        let from = ctx.player.position;
        let across = Vector3::new(aim_point.x - from.x, aim_point.y - from.y, 0.0);
        let Some(direction) = across.try_normalize(1.0e-4) else {
            return 1.0;
        };
        let (pace, lift) = cross_type.launch(across.norm(), &ctx.context.conditions);
        let line = StrikeLine {
            from,
            direction,
            pace,
            lift,
            delivery,
            defending_side: side.opposite(),
        };
        let field_width = ctx.context.field_size.width as f32;
        ctx.players()
            .opponents()
            .all()
            .filter_map(|opponent| {
                ctx.context
                    .players
                    .by_id(opponent.id)
                    .map(|record| PassBlock::priced(&line, &opponent, record, minute, field_width))
            })
            .fold(1.0, |past, blocked| past * (1.0 - blocked))
    }

    /// A corner delivered as the routine called it: aimed at the runner the
    /// corner shape stationed for that routine, with the routine's ball.
    /// Who wins it in the air is the aerial contest's question. `None` for
    /// a short corner, which is a pass rather than a delivery.
    pub fn corner(
        ctx: &StateProcessingContext<'_>,
        routine: CornerRoutine,
    ) -> Option<CrossDecision> {
        let goal = ctx.player().opponent_goal_position();
        let forward = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
        let near = if ctx.player.position.y >= ctx.context.field_size.height as f32 * 0.5 {
            1.0
        } else {
            -1.0
        };
        let (cross_type, (depth, offset)) = match routine {
            CornerRoutine::NearPost => (CrossType::WhippedNearPost, CornerShape::NEAR_POST_RUN),
            CornerRoutine::PenaltySpot => {
                (CrossType::WhippedNearPost, CornerShape::PENALTY_SPOT_RUN)
            }
            CornerRoutine::FarPost => (CrossType::FloatedFarPost, CornerShape::BACK_POST_RUN),
            CornerRoutine::EdgeCutback => (CrossType::Cutback, CornerShape::EDGE_RUN),
            CornerRoutine::Short => return None,
        };
        let aim_point = Vector3::new(goal.x - forward * depth, goal.y + near * offset, 0.0);
        let target = ctx
            .players()
            .teammates()
            .all()
            .filter(|t| t.id != ctx.player.id && !t.tactical_positions.is_goalkeeper())
            .min_by(|a, b| {
                (a.position - aim_point)
                    .norm()
                    .total_cmp(&(b.position - aim_point).norm())
            })?;
        Some(CrossDecision {
            cross_type,
            target_id: target.id,
            aim_point,
        })
    }

    /// Where the ball is actually struck for a given cross type. Zones are
    /// expressed as (depth from the goal line, lateral offset from the
    /// goal centre) so they read the way a coach describes them, and are
    /// mirrored onto the crosser's flank — a cross from the left has its
    /// near post on the left.
    fn aim_point_for(
        cross_type: CrossType,
        goal_pos: Vector3<f32>,
        crosser_pos: Vector3<f32>,
        target_pos: Vector3<f32>,
        forward_dir: f32,
        field_height: f32,
    ) -> Vector3<f32> {
        let centre_y = field_height / 2.0;
        // +1 when the crosser is on the high-y flank, -1 on the low-y one.
        let near_side = if crosser_pos.y >= centre_y { 1.0 } else { -1.0 };

        let (depth, lateral): (f32, f32) = match cross_type {
            // Back post: deeper than the six-yard line, on the far flank.
            CrossType::FloatedFarPost => (58.0, -near_side * 52.0),
            // Across the face of the six-yard box on the near side.
            CrossType::WhippedNearPost => (42.0, near_side * 30.0),
            // Hard and low through the corridor of uncertainty — between
            // the keeper and his back line, level with the penalty spot.
            CrossType::DrivenLowCross => (70.0, -near_side * 14.0),
            // Pulled back to the edge of the area on the crosser's side,
            // the classic arriving-runner ball.
            CrossType::Cutback => (118.0, near_side * 34.0),
            // Clipped in behind for a runner: still wide of the keeper,
            // but deep enough that the ball beats the line, not the man.
            CrossType::EarlyCross => (96.0, -near_side * 40.0),
        };

        // Bias a little toward where the runner actually is, so the aim
        // point tracks the attack instead of being a fixed rosette on the
        // pitch. Bounded so it can never drag the ball out of the zone.
        let nominal_y =
            (goal_pos.y + lateral).clamp(centre_y - BOX_HALF_WIDTH, centre_y + BOX_HALF_WIDTH);
        let pull = (target_pos.y - nominal_y).clamp(-26.0, 26.0);

        Vector3::new(
            goal_pos.x - forward_dir * depth.min(BOX_DEPTH + 40.0),
            nominal_y + pull,
            0.0,
        )
    }

    fn pick_type(
        ctx: &StateProcessingContext,
        crosser_pos: Vector3<f32>,
        crosser_dist_to_goal: f32,
        target_pos: Vector3<f32>,
        goal_pos: Vector3<f32>,
        target_heading_skill: f32,
    ) -> CrossType {
        // ⚠ THE BYLINE IS A DEPTH, NOT A RADIUS.
        //
        // This was `crosser_dist_to_goal < 90.0` — the distance to the
        // goal's CENTRE. A player standing on the byline at the corner of
        // the penalty area is 11 m from the goal LINE and 22 m from the
        // goal centre, so the radial test read 176u and the pull-back
        // branch below could only ever fire from inside the six-yard box,
        // where nobody crosses from. Measured consequence: **19 cutbacks
        // in 200 matches**, against a real ~10% of all crosses, for a
        // ball that is the single highest-value pass in football.
        //
        // It is the same class of error as the keeper's `goal_line_y`:
        // a quantity whose name says one axis, measured on two. Depth
        // from the goal line is what "at the byline" means, and it is
        // what the second branch below needs too.
        let byline_depth = (goal_pos.x - crosser_pos.x).abs();
        let near_byline = byline_depth < 110.0;
        let target_inside_box = (target_pos - goal_pos).norm_squared() < BOX_DEPTH * BOX_DEPTH;

        // `target_heading_skill` is already normalised (raw/20). Compute
        // the sigmoid probability of "poor header" so the cutback /
        // driven-low choices scale smoothly with the target's actual
        // heading, instead of cliff-gating everyone below a threshold into
        // the same bucket.
        //
        // …and it is a POOR HEADER OF THE BALL RELATIVE TO THE PEOPLE HE
        // IS PLAYING AGAINST. The 10-11/20 pivots sit dead in the middle
        // of the generator, so read absolutely they flip the entire
        // crossing model over as the pyramid rises: measured
        // (`OPEN-PLAY CROSSING`, `dev_match stats 16 L L`) the mix runs
        // **57% along the ground at level 6 against 5% at level 18**,
        // with `FloatedFarPost` going 24% → 68% — and the aerial route
        // is a measured 4-5% conversion dead end, so the top of the
        // pyramid loses the whole channel. A ball on the deck into the
        // box is what produces box shots; whether it is the right ball
        // depends on the target against the men marking him, not against
        // a number from another league. See `MatchStandard`.
        let raw_heading =
            (target_heading_skill - MatchStandard::shift(ctx.context)).clamp(0.0, 1.0) * 20.0;
        let p_poor_header_byline = 1.0 - SkillCurve::new(raw_heading, 11.0, 0.6).probability();
        let p_poor_header_wide = 1.0 - SkillCurve::new(raw_heading, 10.0, 0.6).probability();

        if near_byline && target_inside_box {
            // At the byline the pull-back is on. Whether it is taken
            // depends on whether the runner is trailing the play (a
            // cutback needs somebody arriving behind the ball) and on
            // their aerial profile.
            // Behind the ball means further from the goal LINE: measured
            // to the goal centre, nobody in the box was ever behind a
            // crosser standing wide on the byline.
            let trailing = (goal_pos.x - target_pos.x).abs() > byline_depth;
            if trailing && ctx.context.rng.unit_f32() < p_poor_header_byline {
                return CrossType::Cutback;
            }
            return CrossType::WhippedNearPost;
        }

        if crosser_dist_to_goal > 340.0 {
            // Deep in the middle third (>42 m) — the early ball in behind,
            // before the line can drop.
            return CrossType::EarlyCross;
        }

        // The delivery is chosen from HOW DEEP the crosser is and WHO he
        // is crossing to — not from the raw crosser→target distance.
        //
        // That distance was the selector, at a 210u (26 m) threshold, and
        // it collapsed the whole model into one branch: a wide player and
        // a box runner are routinely 200-270u apart on a 545u-tall pitch
        // through the lateral axis ALONE, so almost every delivery cleared
        // the bar and 98% of crosses came out `FloatedFarPost`. Distance
        // is a property of the pitch's geometry here, not of the crosser's
        // decision.
        let poor_header = ctx.context.rng.unit_f32() < p_poor_header_wide;
        if poor_header {
            // Foot-runner profile — keep it out of the air. Deep enough
            // in and the pull-back is the better ball. Depth again, not
            // radius: 190u is 24 m from the goal line, roughly the edge
            // of the penalty area, which is as far out as a ball played
            // backwards into the box is still a cutback rather than a
            // cross.
            return if byline_depth < 190.0 && target_inside_box {
                CrossType::Cutback
            } else {
                CrossType::DrivenLowCross
            };
        }

        // Aerial target. WHICH aerial ball is decided by where the runner
        // is relative to the crosser's flank, not by how deep the crosser
        // is: you hang it to the back post because that is where your man
        // is, and you whip it across the near post because that is where
        // yours is. Keying this off depth alone left 93% of deliveries
        // floated, since most crossers sit beyond any fixed depth bar.
        let centre_y = ctx.context.field_size.height as f32 / 2.0;
        let crosser_high = crosser_pos.y >= centre_y;
        let target_high = target_pos.y >= centre_y;
        if crosser_high != target_high {
            // Runner attacking the far flank — the ball has to carry.
            CrossType::FloatedFarPost
        } else {
            CrossType::WhippedNearPost
        }
    }

    /// Resolve an aerial duel between an attacker and the closest
    /// defender. Returns true if the attacker wins the header.
    ///
    /// `minute` lets the duel feed through the engine's fatigue model: a
    /// tired CB late in the game genuinely loses more aerials. Routes both
    /// sides through the existing aerial composites
    /// (`aerial_outfield_attacker` weights `off_the_ball`,
    /// `aerial_outfield_defender` weights `positioning`) so the duel reads
    /// consistent with every other aerial composite read.
    pub fn resolve_aerial_duel(
        ctx: &StateProcessingContext,
        attacker: &MatchPlayer,
        defender: Option<&MatchPlayer>,
        minute: u32,
    ) -> bool {
        let attacker_score = sc::aerial_outfield_attacker(attacker, minute);
        let defender_score = defender
            .map(|d| sc::aerial_outfield_defender(d, minute))
            // Empty box → easier for the attacker, but not a free win.
            .unwrap_or(0.40);

        let win_prob = Self::sigmoid((attacker_score - defender_score) * 2.2).clamp(0.18, 0.82);
        ctx.context.rng.unit_f32() < win_prob
    }

    /// Pick the closest opposing outfielder to a delivery, for the aerial
    /// duel. Goalkeepers handle their own claim / punch model.
    pub fn pick_aerial_marker(
        ctx: &StateProcessingContext<'_>,
        target_pos: Vector3<f32>,
        radius: f32,
    ) -> Option<MatchPlayerLite> {
        let mut best: Option<(MatchPlayerLite, f32)> = None;
        for opp in ctx.players().opponents().all() {
            if let Some(full) = ctx.context.players.by_id(opp.id)
                && full.tactical_position.current_position.position_group()
                    == PlayerFieldPositionGroup::Goalkeeper
            {
                continue;
            }
            let dist = (opp.position - target_pos).magnitude();
            if dist > radius {
                continue;
            }
            match best {
                None => best = Some((opp, dist)),
                Some((_, d)) if dist < d => best = Some((opp, dist)),
                _ => {}
            }
        }
        best.map(|(p, _)| p)
    }

    /// The marker contesting a delivery at standard aerial-challenge
    /// range. Convenience over [`pick_aerial_marker`](Self::pick_aerial_marker)
    /// so callers don't each invent their own radius.
    pub fn nearest_marker(
        ctx: &StateProcessingContext<'_>,
        target_pos: Vector3<f32>,
    ) -> Option<MatchPlayerLite> {
        Self::pick_aerial_marker(ctx, target_pos, AERIAL_MARKER_RADIUS)
    }

    fn sigmoid(x: f32) -> f32 {
        1.0 / (1.0 + (-x).exp())
    }
}
