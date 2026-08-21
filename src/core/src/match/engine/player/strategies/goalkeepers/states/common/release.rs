//! What a goalkeeper reads before he puts the ball back into play.
//!
//! The four questions are shared: `HoldingBall` weighs them against each
//! other to choose between rolling it out, throwing it and kicking it,
//! and [`KeeperPunt`](super::KeeperPunt) reads the counter to decide
//! whether the kick is a full punt or a flat drop-kick. They lived
//! inside `HoldingBall` as private helpers, which is why the punt could
//! not see them.

use crate::PlayerFieldPositionGroup;
use crate::r#match::StateProcessingContext;
use crate::r#match::teamplay::coach::CoachInstruction;

pub struct KeeperRelease;

impl KeeperRelease {
    /// Furthest a keeper can realistically throw — 37.5 m. Beyond this the
    /// ball has to be kicked.
    ///
    /// It was **110u**, and 110u is 13.75 metres. Every range in this
    /// model is in game units (1u = 12.5 cm) and this one was written as
    /// though it were metres, so "have I got a throw on?" was asked about
    /// the six yards around the keeper and "have I got a short outlet?"
    /// about a radius of six metres — inside which there is never anybody
    /// but him. Both terms therefore read **zero for the whole match**,
    /// which is a silent way of deleting two of a goalkeeper's three
    /// options: measured before the fix, keepers threw the ball 1.3 times
    /// a match at a mean of 9.5 m and punted 81% of everything they
    /// caught.
    pub const THROW_RANGE: f32 = 300.0;

    /// How far out the SHORT outlet is looked for — 17.5 m, a centre-half
    /// splitting to the edge of the box.
    pub const SHORT_RANGE: f32 = 140.0;

    /// Radius used to judge whether an outlet is actually free (~5 m).
    const MARKING_RADIUS: f32 = 40.0;

    /// How far up the pitch we look for an opponent when deciding whether
    /// our own third is being pressed (~17.5 m).
    const PRESS_RADIUS: f32 = 140.0;

    /// Fraction (0..1) of nearby team-mates inside `range` who are not
    /// tightly marked. This is "have I got someone to give it to?", the
    /// question that decides whether playing out is on at all.
    pub fn free_outlets(ctx: &StateProcessingContext, range: f32) -> f32 {
        let mut total = 0u32;
        let mut free = 0u32;
        for teammate in ctx.players().teammates().nearby(range) {
            total += 1;
            if ctx
                .tick_context
                .grid
                .opponents(teammate.id, Self::MARKING_RADIUS)
                .next()
                .is_none()
            {
                free += 1;
            }
        }
        if total == 0 {
            return 0.0;
        }
        // Three free outlets is a comfortable picture; more adds nothing.
        (free as f32 / 3.0).min(1.0)
    }

    /// How aggressively the opposition is squeezing our own third (0..1).
    /// Counts opponents who have committed into the area the keeper would
    /// have to play through.
    pub fn press_pressure(ctx: &StateProcessingContext) -> f32 {
        let committed = ctx.players().opponents().nearby(Self::PRESS_RADIUS).count();
        // Two opponents pressing high is normal; four-plus is a full press.
        ((committed as f32 - 1.0) / 3.0).clamp(0.0, 1.0)
    }

    /// Is a counter on (0..1)?
    ///
    /// # The old measure was inverted
    ///
    /// It compared "team-mates ahead of the ball" against opponents whose
    /// attacking progress was **below 0.55** and called those "recovering
    /// opponents". Progress is measured in OUR attacking direction, so an
    /// opponent below 0.55 is one who has come into our half — a
    /// **committed** attacker, the opposite of a recovering defender. The
    /// term therefore fell as the opposition committed, so the fast release
    /// was suppressed by exactly the picture that calls for it, and needed
    /// our runners to outnumber their whole team by two to reach 1.0. It
    /// read 0.0 for essentially every keeper possession in the game.
    ///
    /// What a keeper actually looks for is grass, not a headcount: how
    /// high the opposition's last outfield man is standing, and whether we
    /// have anybody to run in behind him. A back line that has pushed past
    /// the halfway line is a counter whether or not we have more bodies up
    /// there than they do.
    pub fn counter_opportunity(ctx: &StateProcessingContext) -> f32 {
        let Some(side) = ctx.player.side else {
            return 0.0;
        };
        let field_width = ctx.context.field_size.width as f32;

        // Their last man, in our attacking progress: 1.0 is on their own
        // goal line, 0.5 the halfway line. Their keeper is excluded — he
        // is always the deepest and would pin this at 1.0 forever.
        let last_man = ctx
            .players()
            .opponents()
            .all()
            .filter(|o| {
                o.tactical_positions.position_group() != PlayerFieldPositionGroup::Goalkeeper
            })
            .map(|o| side.attacking_progress_x(o.position.x, field_width))
            .fold(f32::MIN, f32::max);
        if last_man == f32::MIN {
            return 0.0;
        }

        // Nothing on with a back line still on its own 18-yard box; fully
        // exposed once the deepest of them is over the halfway line.
        let exposure = ((0.85 - last_man) / 0.30).clamp(0.0, 1.0);

        // …and it is only a counter if somebody can go. One runner up the
        // pitch is enough — that is what the ball is for.
        let runners = ctx
            .players()
            .teammates()
            .all()
            .filter(|t| {
                !matches!(
                    t.tactical_positions.position_group(),
                    PlayerFieldPositionGroup::Goalkeeper
                ) && side.attacking_progress_x(t.position.x, field_width) > 0.40
            })
            .count() as f32;
        let outlet = (runners / 2.0).clamp(0.0, 1.0);

        exposure * outlet
    }

    /// How direct the manager wants the side to be (0..1). A side told to
    /// push forward or chase the game bypasses midfield; one told to slow
    /// down or keep the ball plays out.
    pub fn directness(ctx: &StateProcessingContext) -> f32 {
        let from_instruction = match ctx.team().coach_instruction() {
            CoachInstruction::AllOutAttack => 0.90,
            CoachInstruction::PushForward => 0.60,
            CoachInstruction::ParkTheBus => 0.55,
            CoachInstruction::Normal => 0.30,
            CoachInstruction::SlowDown => 0.10,
            CoachInstruction::WasteTime => 0.0,
        };
        // Patient build-up sides go long less often regardless of the
        // in-game instruction.
        let patience = ctx.team().build_up_patience().clamp(0.0, 1.0);
        (from_instruction * (1.0 - patience * 0.5)).clamp(0.0, 1.0)
    }
}
