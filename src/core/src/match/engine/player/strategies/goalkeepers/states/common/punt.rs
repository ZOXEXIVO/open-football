//! **The kick from the hands.**
//!
//! A goalkeeper with the ball in his gloves has an option no other player
//! on the pitch has: he can drop it and hit it, and the ball travels
//! further from his hands than any ball struck off the deck. It is the
//! single most common thing a keeper does with a possession, and the
//! engine could not do it.
//!
//! # What was there before
//!
//! Every release the keeper had — `Distributing`, `Throwing`, `Kicking` —
//! ended in `PlayerEvent::PassTo`, a pass at a NAMED team-mate. That is
//! the right shape for a roll to a centre-half and the wrong shape for a
//! punt, which is aimed at a patch of grass and contested by both sides
//! when it lands.
//!
//! Worse, `Kicking`'s target search scored bands in raw units while its
//! comments read them as metres (`distance > 300.0` was annotated
//! "300m+"), and its "extreme kick" tier was gated on a distribution
//! composite above 0.62 that the population sits at **0.34**. So the
//! branch that could pick a long target was unreachable for almost every
//! keeper in the game, and the best-scoring option left was a midfielder
//! 100-200u away. 100-200u is **12 to 25 metres**. The state called
//! `Kicking`, chosen ~10 times a keeper a match, produced a short pass to
//! a nearby midfielder — which is exactly the reported behaviour: *he
//! doesn't necessarily have to pass to nearby players, he can and should
//! kick it*.
//!
//! # What a punt actually is
//!
//! Three properties, none of which a `PassTo` can express:
//!
//! 1. **Nobody is the receiver.** The ball is aimed at a channel at the
//!    keeper's own kicking range and both sides run at the drop. It is
//!    launched as a loose ball ([`PlayerEvent::ClearBall`]) so no team-mate
//!    carries the in-flight claim privilege a pass confers.
//! 2. **The range is the keeper's leg, not the geometry.** How far it goes
//!    is decided before he looks up. Where he aims is chosen INSIDE that
//!    range; a man beyond it is not an option however free he is.
//! 3. **It is struck from 1.15 m up**, which is why it out-carries a goal
//!    kick, and it is the highest ball in football — around 20 m at the
//!    top of the arc, four seconds of hang time for the target man to
//!    attack.
//!
//! # Two shapes
//!
//! [`PuntShape::Full`] is the territorial punt: maximum height, maximum
//! carry, land it on the target man past the halfway line. [`PuntShape::
//! DropKick`] is the counter-attack ball — struck flatter and harder off
//! the half-volley, so it arrives sooner and lower, trading the aerial
//! contest for the seconds a broken defence has not yet used. A keeper
//! only reaches for it when a break is genuinely on and he can strike it.

use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::ball::ball::Ball;
use crate::r#match::goalkeepers::states::common::KeeperRelease;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchPlayerLite, StateProcessingContext};
use nalgebra::Vector3;

/// How the ball comes off his foot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PuntShape {
    /// The full punt: dropped, struck under, sent as high as it is long.
    Full,
    /// The drop-kick: taken on the half-volley, flatter and faster, so it
    /// arrives while the opposition is still turning.
    DropKick,
}

/// A solved kick from the hands.
#[derive(Debug, Clone, Copy)]
pub struct PuntPlan {
    pub shape: PuntShape,
    /// Where he is trying to drop it, after execution error.
    pub target: Vector3<f32>,
    /// Top of the arc, in metres.
    pub apex: f32,
    /// Launch vector to hand to [`PlayerEvent::ClearBall`].
    pub velocity: Vector3<f32>,
    /// The team-mate the punt is FOR, when he had one in mind. `None` is
    /// the honest "just get it up the pitch" punt.
    pub target_man: Option<u32>,
}

impl PuntPlan {
    /// Was this struck off the half-volley rather than dropped and
    /// launched?
    pub fn is_drop_kick(&self) -> bool {
        self.shape == PuntShape::DropKick
    }
}

pub struct KeeperPunt;

impl KeeperPunt {
    // ── Range ────────────────────────────────────────────────────────
    //
    // Carry of the punt itself, ball-to-ground, in metres. A senior
    // goalkeeper's punt from hands lands between the halfway line and the
    // far side of it; the poorest carry it barely past the centre circle.
    // Struck from the release point (~10.6 m off his line) these put the
    // ball down 55-79 m from his own goal, against a halfway line at 52.5.

    /// Carry of the weakest punt in the game.
    const CARRY_MIN_M: f32 = 44.0;
    /// Carry of the best.
    const CARRY_MAX_M: f32 = 68.0;
    /// A drop-kick trades height for arrival time and gives up some of
    /// the carry with it.
    const DROP_KICK_CARRY: f32 = 0.86;

    /// Game units to the metre (1u = 12.5 cm). A keeper's leg is
    /// described in metres and consumed in units.
    const UNITS_PER_METRE: f32 = 8.0;

    // ── Shape ────────────────────────────────────────────────────────

    /// Apex of the weakest full punt, metres. `MAX_APEX_METRES` in the
    /// ball physics is 40 and its own comment puts a keeper's kick from
    /// hand "near 30" — these sit inside that by construction.
    const APEX_MIN_M: f32 = 17.0;
    /// Apex of the best.
    const APEX_MAX_M: f32 = 25.0;
    /// A drop-kick is struck through the ball, not under it.
    const DROP_KICK_APEX: f32 = 0.55;

    /// Height the ball leaves from — [`crate::r#match::Ball::carry_height`],
    /// which is where a keeper holds it into his chest. This is not a
    /// detail: the drop is a fifth of a second of free fall the ball never
    /// has to buy back, and it is why a punt out-carries a goal kick.
    const RELEASE_HEIGHT_M: f32 = 1.15;

    // ── Aim ──────────────────────────────────────────────────────────

    /// Nobody nearer than this is a punt target. 25 m is past his own
    /// back four; a keeper who wants to find a man closer than that rolls
    /// it to him, and that is `Distributing`'s job.
    const MIN_TARGET_DISTANCE: f32 = 200.0;
    /// How far from the touchline the AIM point is kept. The keeper is
    /// not trying to put it out; execution error still can, and should.
    const TOUCHLINE_MARGIN: f32 = 45.0;
    /// …and from the goal line he is kicking towards. A punt that lands
    /// in the opposition six-yard box is a punt nobody wanted.
    const GOAL_LINE_MARGIN: f32 = 90.0;
    /// Radius the target man's cover is counted in (~5 m).
    const COVER_RADIUS: f32 = 40.0;

    // ── Execution ────────────────────────────────────────────────────

    /// Lateral scatter of the landing point as a fraction of the range,
    /// for a keeper with no distribution at all. An ordinary keeper
    /// (0.34) sails a 60 m punt about 6 m off his channel; an elite one
    /// (0.70) about 2.5 m.
    const LATERAL_ERROR: f32 = 0.155;
    /// …and how much of the range he loses or overhits, same scale.
    const RANGE_ERROR: f32 = 0.16;

    /// A ball struck off the FLOOR gives up the drop the punt gets, so the
    /// same leg does not send it as far.
    const OFF_THE_DECK: f32 = 0.88;

    /// Diagnostic switch: with `OF_KEEPER_PUNT=off` the keeper goes back to
    /// resolving every release as a pass at a named team-mate.
    ///
    /// The A/B control for this whole model. A keeper distributes 20-25
    /// times a match and the ball ends up sixty metres from where it used
    /// to, so the change reaches territory, corner supply and chance
    /// creation — none of which can be answered by reading the diff, and
    /// none of which may be answered by checking out an older revision
    /// either, because the working tree moves under you. Same pattern and
    /// purpose as `OF_KEEPER_SERVO` and `OF_STANDARD_OFF`; read once per
    /// process. Debug infrastructure — do not remove.
    ///
    /// Deliberately does NOT gate the drag-aware trajectory solver
    /// ([`Ball::launch_for_range`]) or the unit fixes in
    /// [`KeeperRelease`]: a clearance that lands a quarter short of its
    /// own aim point and a "throw range" of 13.75 m are not behaviours
    /// anyone chose, so both arms should have those. What this isolates is
    /// the judgement call — whether the keeper kicks it or passes it.
    pub fn armed() -> bool {
        use std::sync::OnceLock;
        static ARMED: OnceLock<bool> = OnceLock::new();
        *ARMED.get_or_init(|| std::env::var("OF_KEEPER_PUNT").as_deref() != Ok("off"))
    }

    /// Is a punt physically available? Only out of the gloves — a ball on
    /// the floor is a goal kick or a foot pass, and both are struck
    /// differently and belong to other paths.
    pub fn from_hands(ctx: &StateProcessingContext) -> bool {
        Self::armed() && ctx.tick_context.ball.held_in_hands
    }

    /// How far up the pitch this keeper can put a long GOAL KICK, in
    /// units.
    ///
    /// Shared with the punt's own range so the two descriptions of one
    /// leg cannot disagree. The goal-kick target search reads it to score
    /// candidates against what this keeper can actually hit, rather than
    /// against fixed distance bands that meant one thing for a Premier
    /// League goalkeeper and something else entirely for a youth-team one.
    pub fn goal_kick_reach(ctx: &StateProcessingContext, prof: &GoalkeeperSkillProfile) -> f32 {
        Self::carry_units(ctx, prof, PuntShape::Full) * Self::OFF_THE_DECK
    }

    /// How far this keeper's punt from the hands carries, in units.
    pub fn punt_reach(ctx: &StateProcessingContext, prof: &GoalkeeperSkillProfile) -> f32 {
        Self::carry_units(ctx, prof, PuntShape::Full)
    }

    /// Solve the kick.
    ///
    /// `None` only when there is no direction to launch along, which
    /// needs the keeper to be standing exactly on his own aim point.
    pub fn plan(ctx: &StateProcessingContext) -> Option<PuntPlan> {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let shape = Self::pick_shape(ctx, &prof);

        let carry = Self::carry_units(ctx, &prof, shape);
        let (target_man, aim) = Self::aim(ctx, carry);

        // Execution. Both errors are drawn against the SAME distribution
        // composite the choice of target was made with, so a keeper who
        // picks well also strikes well and the two never disagree about
        // how good he is.
        let sloppiness = (1.0 - prof.distribution).clamp(0.0, 1.0);
        let rng = &ctx.context.rng;
        let range_scale = rng.jitter(1.0, sloppiness * Self::RANGE_ERROR);
        let lateral = rng.jitter(0.0, sloppiness * Self::LATERAL_ERROR * carry);

        let origin = ctx.player.position;
        let to_aim = Vector3::new(aim.x - origin.x, aim.y - origin.y, 0.0);
        let direction = to_aim.try_normalize(1.0e-4)?;
        // Across the line of the kick, so the scatter is a slice off the
        // channel he chose rather than a fixed drift in pitch coordinates.
        let across = Vector3::new(-direction.y, direction.x, 0.0);
        let struck = origin + direction * (to_aim.norm() * range_scale) + across * lateral;

        let apex = Self::apex(ctx, &prof, shape);
        let velocity = Ball::ballistic_launch(origin, struck, apex, Self::RELEASE_HEIGHT_M)?;

        Some(PuntPlan {
            shape,
            target: struck,
            apex,
            velocity,
            target_man,
        })
    }

    /// Full punt or drop-kick.
    ///
    /// The drop-kick is the counter ball, so it is asked for by the same
    /// picture that asks for a fast throw — team-mates ahead of the ball
    /// against opponents who have not got back — and it is a technique,
    /// so a keeper who cannot strike one does not try.
    fn pick_shape(ctx: &StateProcessingContext, prof: &GoalkeeperSkillProfile) -> PuntShape {
        // Both terms have to be there. An exposed back line with a keeper
        // who cannot strike a half-volley is still a full punt, and a
        // keeper who can is not going to waste it on a set defence.
        let counter = KeeperRelease::counter_opportunity(ctx);
        if counter > 0.45 && prof.distribution > 0.40 {
            PuntShape::DropKick
        } else {
            PuntShape::Full
        }
    }

    /// How far this keeper's punt carries, in units.
    ///
    /// # Why `kicking` is read ABSOLUTELY here
    ///
    /// [`crate::r#match::engine::teamplay::standard::MatchStandard`] prices
    /// every skill read that decides a CONTEST against the standard of
    /// football in the match, because `tackling 14` is an ordinary
    /// centre-half in one division and an outstanding one three tiers
    /// down. Distance is not a contest. A fourth-tier goalkeeper really
    /// does punt it sixty metres, exactly as he really does tire like a
    /// fourth-tier player — the same reason the fitness reads are left
    /// absolute. Standard-pricing this would have lower divisions playing
    /// a physically shorter game, which is precisely the artefact the
    /// primitive exists to remove.
    ///
    /// The DECISION and the ACCURACY below both read `prof.distribution`,
    /// which is standard-priced, so the family stays consistent where it
    /// is describing quality rather than physics.
    fn carry_units(
        ctx: &StateProcessingContext,
        prof: &GoalkeeperSkillProfile,
        shape: PuntShape,
    ) -> f32 {
        let s = &ctx.player.skills;
        let leg = ((s.goalkeeping.kicking - 1.0) / 19.0).clamp(0.0, 1.0);
        let strength = ((s.physical.strength - 1.0) / 19.0).clamp(0.0, 1.0);
        // Three quarters technique, one quarter raw power — a keeper who
        // strikes a ball well out-kicks a stronger one who does not.
        let power = (leg * 0.75 + strength * 0.25).clamp(0.0, 1.0);
        // Tired legs lose distance, and only distance: `condition_mult`
        // bottoms out at 0.45, which would halve a punt, so it is folded
        // in gently.
        let fatigue = (0.85 + prof.condition_mult * 0.15).clamp(0.85, 1.0);
        let metres = (Self::CARRY_MIN_M + power * (Self::CARRY_MAX_M - Self::CARRY_MIN_M))
            * fatigue
            * match shape {
                PuntShape::Full => 1.0,
                PuntShape::DropKick => Self::DROP_KICK_CARRY,
            };
        metres * Self::UNITS_PER_METRE
    }

    /// Top of the arc, metres. A better striker of the ball gets it
    /// higher as well as further — the two come off the same contact.
    fn apex(ctx: &StateProcessingContext, prof: &GoalkeeperSkillProfile, shape: PuntShape) -> f32 {
        let leg = ((ctx.player.skills.goalkeeping.kicking - 1.0) / 19.0).clamp(0.0, 1.0);
        let base = Self::APEX_MIN_M + leg * (Self::APEX_MAX_M - Self::APEX_MIN_M);
        // Not every punt off the same boot is the same punt. The spread
        // is the keeper's own consistency.
        let spread = (1.0 - prof.distribution).clamp(0.0, 1.0) * 0.12;
        let struck = ctx.context.rng.jitter(base, base * spread);
        match shape {
            PuntShape::Full => struck,
            PuntShape::DropKick => struck * Self::DROP_KICK_APEX,
        }
    }

    /// Where to drop it, and who it is for.
    ///
    /// The keeper picks a CHANNEL, not a man: the ball goes his own
    /// kicking distance up the pitch, and the only choice left is which
    /// line of the pitch it comes down on. That is why a man beyond his
    /// range is no use however free he is, and why a man inside it is
    /// still worth aiming at — he runs onto the drop through four seconds
    /// of hang time, which at a forward's pace is twenty metres of ground.
    fn aim(ctx: &StateProcessingContext, carry: f32) -> (Option<u32>, Vector3<f32>) {
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let middle = field_height * 0.5;
        let origin = ctx.player.position;
        let forward = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());

        let man = Self::best_target(ctx, carry);

        // The channel: the line of the pitch he wants it to come down on.
        // Straight up the middle when nobody has made himself available —
        // the punt a keeper hits with nothing on — and otherwise the target
        // man's line, hedged a third of the way back to the middle, because
        // a ball put down on the touchline has nowhere to go even when it
        // is won.
        let channel_y = match man {
            Some(m) => m.position.y * 0.68 + middle * 0.32,
            None => middle,
        }
        .clamp(
            Self::TOUCHLINE_MARGIN,
            field_height - Self::TOUCHLINE_MARGIN,
        );

        // Placed ON the arc of his own range, not at the range STRAIGHT UP
        // the pitch with a lateral offset bolted on — the second is a
        // longer kick than the first, and the whole model rests on the
        // carry being what his leg can do.
        let bearing = Vector3::new(forward * carry, channel_y - origin.y, 0.0)
            .try_normalize(1.0e-4)
            .unwrap_or_else(|| Vector3::new(forward, 0.0, 0.0));
        let landing = origin + bearing * carry;

        // Never over the far goal line: a punt that lands in the
        // opposition six-yard box is a punt nobody wanted.
        let x = if forward > 0.0 {
            landing.x.min(field_width - Self::GOAL_LINE_MARGIN)
        } else {
            landing.x.max(Self::GOAL_LINE_MARGIN)
        };
        (
            man.map(|m| m.id),
            Vector3::new(
                x,
                landing.y.clamp(
                    Self::TOUCHLINE_MARGIN,
                    field_height - Self::TOUCHLINE_MARGIN,
                ),
                0.0,
            ),
        )
    }

    /// The man the punt is for, or `None` if nobody has made himself one.
    fn best_target<'a>(ctx: &'a StateProcessingContext<'a>, carry: f32) -> Option<MatchPlayerLite> {
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let side = ctx.player.side;
        let origin = ctx.player.position;

        let mut best: Option<MatchPlayerLite> = None;
        let mut best_score = 0.0f32;

        for mate in ctx.players().teammates().all() {
            if mate.tactical_positions.position_group() == PlayerFieldPositionGroup::Goalkeeper {
                continue;
            }
            // Upfield only, and far enough that a punt is the right tool.
            let forward_progress = side.map_or(0.0, |s| s.forward_delta(origin.x, mate.position.x));
            if forward_progress < Self::MIN_TARGET_DISTANCE {
                continue;
            }

            // How well he sits inside the punt's range. A man at the drop
            // scores 1; one a long way off it scores near nothing, whether
            // he is short of it or beyond it. Beyond costs more — he has
            // to turn and come back, and the ball is behind him.
            let distance = (mate.position - origin).norm();
            let gap = distance - carry;
            let reach_scale = if gap > 0.0 {
                carry * 0.45
            } else {
                carry * 0.70
            };
            let reach = (1.0 - (gap.abs() / reach_scale.max(1.0))).clamp(0.05, 1.0);

            // Who wins a dropping ball. This is the whole reason a punt
            // has a target man rather than just a direction.
            let aerial = ctx
                .context
                .players
                .by_id(mate.id)
                .map(|p| sc::aerial_outfield_attacker(p, minute))
                .unwrap_or(0.45);

            // …and whether he will be allowed to attack it.
            let cover = ctx
                .tick_context
                .grid
                .opponents(mate.id, Self::COVER_RADIUS)
                .count() as f32;
            let space = 1.0 / (1.0 + cover * 0.55);

            // A punt is a forward's ball. A midfielder can be the target
            // when the shape has put him highest; a defender almost never
            // is, and when he is it is because the punt has gone wrong.
            let role = match mate.tactical_positions.position_group() {
                PlayerFieldPositionGroup::Forward => 1.0,
                PlayerFieldPositionGroup::Midfielder => 0.72,
                PlayerFieldPositionGroup::Defender => 0.30,
                PlayerFieldPositionGroup::Goalkeeper => 0.0,
            };

            let score = reach * (0.35 + aerial * 0.65) * space * role;
            if score > best_score {
                best_score = score;
                best = Some(mate);
            }
        }
        best
    }
}
