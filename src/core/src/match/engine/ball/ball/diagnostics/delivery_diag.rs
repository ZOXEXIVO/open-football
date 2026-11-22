//! **The delivery census** — every cross followed from its strike to the
//! first player to play it and to how it ended, and where corners,
//! throw-ins and own goals come from.
//!
//! A cross is booked at the strike and completed by its first contact, so
//! the questions the wide game raises are all about that one flight: who
//! closed the crosser, who got to the ball first, and what became of it.
//! The open ledger rides on the ball from the strike to its end; the
//! per-match [`DeliveryTally`] goes back on the result, so the realism
//! report reads it like any other line. `match-logs` only.

use crate::r#match::engine::ball::ball::block_diag::CrossPriceCensus;
use crate::r#match::engine::ball::ball::contest::pass_block::{PassBlock, StrikeLine};
use crate::r#match::engine::ball::ball::{AerialReach, Ball, PassOriginRestart};
use crate::r#match::midfielders::states::shared::LaneAhead;
use crate::r#match::passing::{CrossModel, CrossType, FlankPlay};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, MatchField, MatchPlayer, PlayerSide};
use nalgebra::Vector3;
use serde::{Deserialize, Serialize};

/// What a touch was, as far as the censuses need to know. A touch no site
/// names is read off the touch itself: under control, above the boot, or
/// a foot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TouchKind {
    #[default]
    Other,
    Control,
    Feet,
    Head,
    Tackle,
    /// A pass charged down or lunged at.
    Block,
    ShotBlock,
    Clearance,
    Pass,
    Cross,
    Shot,
    PutOut,
}

impl TouchKind {
    pub fn of_contact(controlled: bool, height: f32) -> Self {
        if controlled {
            TouchKind::Control
        } else if height > AerialReach::VOLLEY {
            TouchKind::Head
        } else {
            TouchKind::Feet
        }
    }
}

const KINDS: usize = 5;
const TYPES: usize = 6;
const CLOSERS: usize = 5;
const FIRSTS: usize = 9;
const ENDS: usize = 8;
const LENGTHS: usize = 4;
const AIMS: usize = 3;
const CORNER_SOURCES: usize = 7;
const THROW_SOURCES: usize = 9;
const OWN_GOAL_ROUTES: usize = 6;

/// One match's deliveries and sources. Whole numbers only, so the result's
/// tally keeps its `Eq`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryTally {
    /// `[kind][type]` — see [`DeliveryCensus::KINDS`], [`DeliveryCensus::TYPES`].
    pub struck: [[u32; TYPES]; KINDS],
    /// Σ of the strike's depth from the goal line, in decimetres.
    pub depth_dm: [[u64; TYPES]; KINDS],
    pub inside_width: [[u32; TYPES]; KINDS],
    pub within_six: [[u32; TYPES]; KINDS],
    pub byline: [[u32; TYPES]; KINDS],
    pub closer: [[[u32; CLOSERS]; TYPES]; KINDS],
    pub first: [[[u32; FIRSTS]; TYPES]; KINDS],
    pub end: [[[u32; ENDS]; TYPES]; KINDS],
    /// Completions the pass-reception rule credited.
    pub booked_completed: [[u32; TYPES]; KINDS],
    pub headers_at_goal: [[u32; TYPES]; KINDS],
    /// Other open-play crosses: `[lofted][length][aim]`.
    pub other_shape: [[[u32; AIMS]; LENGTHS]; 2],
    pub corners: [u32; CORNER_SOURCES],
    /// `[source][in his own half, in the opponents' half]`.
    pub throw_ins: [[u32; 2]; THROW_SOURCES],
    /// `[route][own goal, credited to an attacker, refused at the line]`.
    pub own_goals: [[u32; 3]; OWN_GOAL_ROUTES],
    /// Own goals given although an attacker touched the ball last.
    pub own_goals_off_an_attacker: u32,
    /// Balls that crossed the line inside the frame off an attacker with
    /// no shot behind them.
    pub refused_no_shot: u32,
    /// Throw-ins that followed another, and the ticks between the two
    /// summed — the touchline's own clock, which no touch resets.
    pub throw_in_gaps: u32,
    pub throw_in_gap_ticks: u64,
}

impl DeliveryTally {
    pub fn absorb(&mut self, other: &DeliveryTally) {
        for k in 0..KINDS {
            Self::add(&mut self.struck[k], &other.struck[k]);
            Self::add(&mut self.inside_width[k], &other.inside_width[k]);
            Self::add(&mut self.within_six[k], &other.within_six[k]);
            Self::add(&mut self.byline[k], &other.byline[k]);
            Self::add(&mut self.booked_completed[k], &other.booked_completed[k]);
            Self::add(&mut self.headers_at_goal[k], &other.headers_at_goal[k]);
            for t in 0..TYPES {
                self.depth_dm[k][t] += other.depth_dm[k][t];
                Self::add(&mut self.closer[k][t], &other.closer[k][t]);
                Self::add(&mut self.first[k][t], &other.first[k][t]);
                Self::add(&mut self.end[k][t], &other.end[k][t]);
            }
        }
        for lofted in 0..2 {
            for length in 0..LENGTHS {
                Self::add(
                    &mut self.other_shape[lofted][length],
                    &other.other_shape[lofted][length],
                );
            }
        }
        Self::add(&mut self.corners, &other.corners);
        for s in 0..THROW_SOURCES {
            Self::add(&mut self.throw_ins[s], &other.throw_ins[s]);
        }
        for r in 0..OWN_GOAL_ROUTES {
            Self::add(&mut self.own_goals[r], &other.own_goals[r]);
        }
        self.own_goals_off_an_attacker += other.own_goals_off_an_attacker;
        self.refused_no_shot += other.refused_no_shot;
        self.throw_in_gaps += other.throw_in_gaps;
        self.throw_in_gap_ticks += other.throw_in_gap_ticks;
    }

    fn add<const N: usize>(mine: &mut [u32; N], theirs: &[u32; N]) {
        for (m, t) in mine.iter_mut().zip(theirs) {
            *m += t;
        }
    }

    /// `(struck, byline)` over the deliveries the crossing decision made in
    /// open play.
    pub fn open_play_byline(&self) -> (u32, u32) {
        let k = DeliveryCensus::OPEN_PLAY;
        (self.struck[k].iter().sum(), self.byline[k].iter().sum())
    }

    /// Open-play deliveries whose first contact was a defender's block.
    pub fn open_play_blocked(&self) -> u32 {
        self.first[DeliveryCensus::OPEN_PLAY]
            .iter()
            .map(|firsts| firsts[DeliveryCensus::DEFENDER_BLOCK])
            .sum()
    }

    /// Headers at goal off open-play crosses, modelled or not.
    pub fn open_play_headers_at_goal(&self) -> u32 {
        [DeliveryCensus::OPEN_PLAY, DeliveryCensus::OTHER_OPEN_PLAY]
            .iter()
            .map(|&k| self.headers_at_goal[k].iter().sum::<u32>())
            .sum()
    }

    /// One throw-in, `since_previous` ticks after the last one.
    pub fn note_throw_in_gap(&mut self, since_previous: Option<u64>) {
        if let Some(gap) = since_previous {
            self.throw_in_gaps += 1;
            self.throw_in_gap_ticks += gap;
        }
    }
}

/// Labels, slots and the strike-side reads of the census.
pub struct DeliveryCensus;

impl DeliveryCensus {
    pub const KINDS: [&'static str; KINDS] = [
        "open-play delivery",
        "other open-play cross",
        "other cross from a restart",
        "corner",
        "free-kick delivery",
    ];
    pub const OPEN_PLAY: usize = 0;
    pub const OTHER_OPEN_PLAY: usize = 1;
    pub const OTHER_RESTART: usize = 2;
    pub const CORNER: usize = 3;
    pub const FREE_KICK: usize = 4;

    /// [`CrossType::diag_index`] order, then the passes nobody typed.
    pub const TYPES: [&'static str; TYPES] = [
        "floated far",
        "driven across",
        "pulled back",
        "whipped near",
        "early",
        "untyped",
    ];
    pub const UNTYPED: usize = 5;

    pub const FIRSTS: [&'static str; FIRSTS] = [
        "attacker's head",
        "attacker's feet",
        "defender's block",
        "defender's header",
        "defender's clearance",
        "defender's feet",
        "keeper's claim",
        "keeper's punch, parry or flap",
        "nobody",
    ];
    pub const ATTACKER_HEAD: usize = 0;
    pub const ATTACKER_FEET: usize = 1;
    pub const DEFENDER_BLOCK: usize = 2;
    pub const DEFENDER_HEADER: usize = 3;
    pub const DEFENDER_CLEARANCE: usize = 4;
    pub const DEFENDER_FEET: usize = 5;
    pub const KEEPER_CLAIM: usize = 6;
    pub const KEEPER_PUNCH: usize = 7;
    pub const NOBODY: usize = 8;

    /// "Loose" is a ball a defender knocked loose and somebody then won;
    /// "cleared", one the defence took or played away.
    pub const ENDS: [&'static str; ENDS] = [
        "completed",
        "shot or header at goal",
        "cleared",
        "behind for a corner",
        "behind for a goal kick",
        "into touch",
        "in the net, no shot",
        "loose",
    ];
    pub const COMPLETED: usize = 0;
    pub const SHOT: usize = 1;
    pub const CLEARED: usize = 2;
    pub const BEHIND_CORNER: usize = 3;
    pub const BEHIND_GOAL_KICK: usize = 4;
    pub const INTO_TOUCH: usize = 5;
    pub const IN_THE_NET: usize = 6;
    pub const LOOSE: usize = 7;

    pub const LENGTHS: [&'static str; LENGTHS] = ["<10 m", "10-20 m", "20-30 m", "30 m+"];
    pub const AIMS: [&'static str; AIMS] = [
        "deeper in the box",
        "front strip of the box",
        "outside the box",
    ];

    pub const CORNER_SOURCES: [&'static str; CORNER_SOURCES] = [
        "shot blocked",
        "keeper's save",
        "cross blocked behind",
        "cross headed or cleared behind",
        "pull-back or driven ball turned behind",
        "pass blocked behind",
        "other",
    ];
    const CORNER_SHOT_BLOCKED: usize = 0;
    const CORNER_KEEPER: usize = 1;
    const CORNER_CROSS_BLOCKED: usize = 2;
    const CORNER_CROSS_CLEARED: usize = 3;
    const CORNER_GROUND_DELIVERY: usize = 4;
    const CORNER_PASS_BLOCKED: usize = 5;
    const CORNER_OTHER: usize = 6;

    /// The realism report's tracked line for each corner source.
    pub const CORNER_LINES: [&'static str; CORNER_SOURCES] = [
        "corners/match: shot blocked",
        "corners/match: keeper's save",
        "corners/match: cross blocked behind",
        "corners/match: cross headed or cleared",
        "corners/match: pull-back or driven ball",
        "corners/match: pass blocked behind",
        "corners/match: other",
    ];

    pub const THROW_SOURCES: [&'static str; THROW_SOURCES] = [
        "tackle",
        "block",
        "clearance",
        "header",
        "pass",
        "cross",
        "carry",
        "put out under pressure",
        "other",
    ];

    pub const THROW_LINES: [&'static str; THROW_SOURCES] = [
        "throw-ins/match: tackle",
        "throw-ins/match: block",
        "throw-ins/match: clearance",
        "throw-ins/match: header",
        "throw-ins/match: pass",
        "throw-ins/match: cross",
        "throw-ins/match: carry",
        "throw-ins/match: put out under pressure",
        "throw-ins/match: other",
    ];

    pub const OWN_GOAL_ROUTES: [&'static str; OWN_GOAL_ROUTES] = [
        "deflected cross or pull-back",
        "deflected shot heading wide",
        "deflected shot on frame",
        "clearance or header",
        "the keeper",
        "other",
    ];
    pub const OWN_GOAL_LINES: [&'static str; OWN_GOAL_ROUTES] = [
        "own goals/match: deflected cross",
        "own goals/match: deflected shot wide",
        "own goals/match: deflected shot on frame",
        "own goals/match: clearance or header",
        "own goals/match: the keeper",
        "own goals/match: other",
    ];
    const ROUTE_DELIVERY: usize = 0;
    const ROUTE_SHOT_WIDE: usize = 1;
    const ROUTE_SHOT_ON_FRAME: usize = 2;
    const ROUTE_CLEARANCE: usize = 3;
    const ROUTE_KEEPER: usize = 4;
    const ROUTE_OTHER: usize = 5;
    pub const OWN_GOAL_OUTCOMES: [&'static str; 3] =
        ["own goal", "credited to an attacker", "refused at the line"];
    pub const OWN_GOAL: usize = 0;
    pub const CREDITED_TO_ATTACKER: usize = 1;
    pub const REFUSED: usize = 2;

    /// A pass has just been struck by `passer_id`: the delivery it
    /// interrupts ends, the striker's touch is named, and a ledger opens
    /// when the pass is booked as a cross.
    pub fn on_strike(
        field: &mut MatchField,
        context: &MatchContext,
        passer_id: u32,
        cross_type: Option<CrossType>,
        was_cross: bool,
        lofted: bool,
        target: Vector3<f32>,
    ) {
        let Some(passer) = field.players.iter().find(|p| p.id == passer_id) else {
            return;
        };
        let passer_team = passer.team_id;
        let height = field.ball.position.z;
        if let Some(mut ledger) = field.ball.delivery_ledger.take() {
            let end = ledger
                .on_touch(passer_id, passer_team, TouchKind::Pass, false, height)
                .unwrap_or_else(|| ledger.end_on_strike(passer_team));
            ledger.close(end, &mut field.ball.delivery_tally);
        }
        if field.ball.last_touch_player_id == Some(passer_id) {
            field.ball.last_touch_kind = if was_cross {
                TouchKind::Cross
            } else {
                TouchKind::Pass
            };
        }
        field.ball.delivery_identity = None;
        let Some(side) = passer.side.filter(|_| was_cross) else {
            return;
        };
        let kind = match (cross_type, field.ball.pass_origin_restart) {
            (Some(_), PassOriginRestart::Corner) => Self::CORNER,
            (Some(_), PassOriginRestart::DirectFreeKick | PassOriginRestart::IndirectFreeKick) => {
                Self::FREE_KICK
            }
            (Some(_), _) => Self::OPEN_PLAY,
            (None, PassOriginRestart::OpenPlay) => Self::OTHER_OPEN_PLAY,
            (None, _) => Self::OTHER_RESTART,
        };
        let slot = cross_type.map_or(Self::UNTYPED, CrossType::diag_index);
        let field_width = field.size.width as f32;
        let goal_x = side.opposite().own_goal_x(field_width);
        let from = passer.position;
        let depth = (goal_x - from.x).abs();
        let opp_box = context.penalty_area(side == PlayerSide::Right);
        let inside_width = (opp_box.min.y..=opp_box.max.y).contains(&from.y);
        let within_six = depth <= CrossModel::BYLINE_STRIP;
        let closer = Self::closer_class(field, passer);
        let keeper_id = field
            .players
            .iter()
            .find(|p| {
                p.team_id != passer_team && p.tactical_position.current_position.is_goalkeeper()
            })
            .map(|p| p.id);

        let tally = &mut field.ball.delivery_tally;
        tally.struck[kind][slot] += 1;
        tally.depth_dm[kind][slot] += (depth * 0.125 * 10.0) as u64;
        tally.inside_width[kind][slot] += u32::from(inside_width);
        tally.within_six[kind][slot] += u32::from(within_six);
        tally.byline[kind][slot] += u32::from(inside_width || within_six);
        tally.closer[kind][slot][closer] += 1;
        if kind == Self::OTHER_OPEN_PLAY {
            let length_m = (target - from).xy().norm() * 0.125;
            let length = match length_m {
                l if l < 10.0 => 0,
                l if l < 20.0 => 1,
                l if l < 30.0 => 2,
                _ => 3,
            };
            let box_depth = opp_box.max.x - opp_box.min.x;
            let aim = if !opp_box.contains(&target) {
                2
            } else if (goal_x - target.x).abs() >= box_depth - CrossModel::FRONT_STRIP {
                1
            } else {
                0
            };
            tally.other_shape[usize::from(lofted)][length][aim] += 1;
        }
        field.ball.delivery_identity = Some((kind, slot));
        field.ball.delivery_ledger = Some(DeliveryLedger {
            kind,
            slot,
            ground: matches!(
                cross_type,
                Some(CrossType::DrivenLowCross | CrossType::Cutback)
            ),
            attacking_team: passer_team,
            keeper_id,
            first: None,
            defended: false,
            cleared: false,
            header_shot: false,
        });
    }

    /// Where the man nearest the crosser stood at the strike — the
    /// [`CrossPriceCensus::CLOSERS`] classes, read off the ball as struck.
    fn closer_class(field: &MatchField, passer: &MatchPlayer) -> usize {
        let from = passer.position;
        let (Some(side), Some(closer)) = (
            passer.side,
            field
                .players
                .iter()
                .filter(|p| {
                    p.team_id != passer.team_id
                        && !p.off_pitch
                        && !p.tactical_position.current_position.is_goalkeeper()
                })
                .map(|p| (p, (p.position - from).xy().norm()))
                .filter(|(_, gap)| *gap < CrossPriceCensus::CLOSER_RADIUS)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(p, _)| p),
        ) else {
            return CrossPriceCensus::NOBODY;
        };
        let field_width = field.size.width as f32;
        let field_height = field.size.height as f32;
        let velocity = field.ball.velocity;
        let across = Vector3::new(velocity.x, velocity.y, 0.0);
        let on_line = across.try_normalize(1.0e-4).is_some_and(|direction| {
            let minute = sc::minute_from_ticks(field.ball.current_tick_cached);
            let line = StrikeLine {
                from,
                direction,
                pace: across.norm(),
                lift: velocity.z.max(0.0),
                delivery: PassBlock::technique(passer, minute, true),
                defending_side: side.opposite(),
            };
            PassBlock::reaches(&line, closer.position, field_width)
        });
        if on_line {
            return CrossPriceCensus::ON_THE_LINE;
        }
        let goal = Vector3::new(
            side.opposite().own_goal_x(field_width),
            field_height * 0.5,
            0.0,
        );
        let byline = FlankPlay::byline_point_from(from, side, goal, field_width, field_height);
        let on_run = (byline - from)
            .try_normalize(f32::EPSILON)
            .is_some_and(|heading| LaneAhead::in_channel(closer.position - from, heading));
        if on_run {
            CrossPriceCensus::ON_THE_RUN
        } else if (goal - closer.position).xy().norm() < (goal - from).xy().norm() {
            CrossPriceCensus::GOAL_SIDE
        } else {
            CrossPriceCensus::BEATEN
        }
    }

    /// Where a corner came from, read at the award off the ball's last
    /// touch and the delivery it may still be carrying.
    pub fn corner_source(
        ledger: Option<&DeliveryLedger>,
        kind: TouchKind,
        by_the_keeper: bool,
    ) -> usize {
        match ledger.filter(|l| l.defended) {
            Some(ledger) if ledger.ground => Self::CORNER_GROUND_DELIVERY,
            Some(_) if kind == TouchKind::Block => Self::CORNER_CROSS_BLOCKED,
            Some(_) => Self::CORNER_CROSS_CLEARED,
            None if kind == TouchKind::ShotBlock => Self::CORNER_SHOT_BLOCKED,
            None if by_the_keeper => Self::CORNER_KEEPER,
            None if kind == TouchKind::Block => Self::CORNER_PASS_BLOCKED,
            None => Self::CORNER_OTHER,
        }
    }

    pub fn throw_in_source(kind: TouchKind) -> usize {
        match kind {
            TouchKind::Tackle => 0,
            TouchKind::Block | TouchKind::ShotBlock => 1,
            TouchKind::Clearance => 2,
            TouchKind::Head => 3,
            TouchKind::Pass => 4,
            TouchKind::Cross => 5,
            TouchKind::Control => 6,
            TouchKind::PutOut => 7,
            TouchKind::Feet | TouchKind::Shot | TouchKind::Other => 8,
        }
    }

    /// The route a ball took into the net off the side defending it.
    /// `shot_met` is whether the touch met an untouched shot, and if so
    /// whether that shot was on frame.
    pub fn own_goal_route(
        shot_met: Option<bool>,
        by_the_keeper: bool,
        delivery_live: bool,
        kind: TouchKind,
    ) -> usize {
        match shot_met {
            Some(false) => Self::ROUTE_SHOT_WIDE,
            Some(true) => Self::ROUTE_SHOT_ON_FRAME,
            None if by_the_keeper => Self::ROUTE_KEEPER,
            None if delivery_live => Self::ROUTE_DELIVERY,
            None if matches!(kind, TouchKind::Clearance | TouchKind::Head) => Self::ROUTE_CLEARANCE,
            None => Self::ROUTE_OTHER,
        }
    }
}

/// The open book on one delivery, from its strike to its end.
#[derive(Clone, Copy, Debug)]
pub struct DeliveryLedger {
    kind: usize,
    slot: usize,
    /// A pull-back or a ball driven across: played along the deck.
    ground: bool,
    attacking_team: u32,
    keeper_id: Option<u32>,
    first: Option<usize>,
    /// A defender has played it.
    defended: bool,
    /// The last touch was the defence clearing it: whatever touches it
    /// next, the delivery was cleared — unless it goes out of play first.
    cleared: bool,
    header_shot: bool,
}

impl DeliveryLedger {
    /// One touch on the delivery's ball. Returns the end when this touch
    /// ends it.
    pub fn on_touch(
        &mut self,
        player_id: u32,
        team_id: u32,
        kind: TouchKind,
        controlled: bool,
        height: f32,
    ) -> Option<usize> {
        if self.cleared {
            return Some(DeliveryCensus::CLEARED);
        }
        let attacking = team_id == self.attacking_team;
        let keeper = self.keeper_id == Some(player_id);
        let header =
            kind == TouchKind::Head || (kind != TouchKind::Control && height > AerialReach::VOLLEY);
        if self.first.is_none() {
            self.first = Some(if keeper && controlled {
                DeliveryCensus::KEEPER_CLAIM
            } else if keeper {
                DeliveryCensus::KEEPER_PUNCH
            } else if attacking && header {
                DeliveryCensus::ATTACKER_HEAD
            } else if attacking {
                DeliveryCensus::ATTACKER_FEET
            } else if matches!(kind, TouchKind::Block | TouchKind::ShotBlock) {
                DeliveryCensus::DEFENDER_BLOCK
            } else if kind == TouchKind::Clearance {
                DeliveryCensus::DEFENDER_CLEARANCE
            } else if header {
                DeliveryCensus::DEFENDER_HEADER
            } else {
                DeliveryCensus::DEFENDER_FEET
            });
        }
        if attacking {
            if kind == TouchKind::Shot {
                self.header_shot = height > AerialReach::VOLLEY;
                return Some(DeliveryCensus::SHOT);
            }
            return controlled.then_some(if self.defended {
                DeliveryCensus::LOOSE
            } else {
                DeliveryCensus::COMPLETED
            });
        }
        self.defended = true;
        if controlled {
            return Some(DeliveryCensus::CLEARED);
        }
        self.cleared = keeper || matches!(kind, TouchKind::Clearance | TouchKind::Head);
        None
    }

    fn end_on_strike(&self, passer_team: u32) -> usize {
        if passer_team != self.attacking_team || self.cleared {
            DeliveryCensus::CLEARED
        } else if self.defended {
            DeliveryCensus::LOOSE
        } else {
            DeliveryCensus::COMPLETED
        }
    }

    pub fn close(self, end: usize, tally: &mut DeliveryTally) {
        let first = self.first.unwrap_or(DeliveryCensus::NOBODY);
        tally.first[self.kind][self.slot][first] += 1;
        tally.end[self.kind][self.slot][end] += 1;
        if end == DeliveryCensus::SHOT && self.header_shot {
            tally.headers_at_goal[self.kind][self.slot] += 1;
        }
    }
}

impl Ball {
    /// A throw-in awarded: booked by what the last toucher did and in
    /// which half, and the delivery that went out with it ends there.
    pub(crate) fn note_throw_in_source(&mut self, players: &[MatchPlayer]) {
        let field_width = self.field_width;
        let at = self.last_touch_position.x;
        let in_opponents_half = self
            .last_touch_player_id
            .and_then(|id| players.iter().find(|p| p.id == id))
            .and_then(|p| p.side)
            .is_some_and(|side| side.attacking_progress_x(at, field_width) >= 0.5);
        let source = DeliveryCensus::throw_in_source(self.last_touch_kind);
        self.delivery_tally.throw_ins[source][usize::from(in_opponents_half)] += 1;
        if let Some(ledger) = self.delivery_ledger.take() {
            ledger.close(DeliveryCensus::INTO_TOUCH, &mut self.delivery_tally);
        }
    }

    /// A corner awarded: booked by where it came from, and the delivery
    /// that went behind ends there.
    pub(crate) fn note_corner_source(&mut self, players: &[MatchPlayer]) {
        let by_the_keeper = self.last_touched_by_a_keeper(players);
        let source = DeliveryCensus::corner_source(
            self.delivery_ledger.as_ref(),
            self.last_touch_kind,
            by_the_keeper,
        );
        self.delivery_tally.corners[source] += 1;
        if let Some(ledger) = self.delivery_ledger.take() {
            ledger.close(DeliveryCensus::BEHIND_CORNER, &mut self.delivery_tally);
        }
    }

    pub(crate) fn note_goal_kick_end(&mut self) {
        if let Some(ledger) = self.delivery_ledger.take() {
            ledger.close(DeliveryCensus::BEHIND_GOAL_KICK, &mut self.delivery_tally);
        }
    }

    /// A ball across the goal line inside the frame, and what the crediting
    /// rule made of it (`outcome`, one of [`DeliveryCensus::OWN_GOAL_OUTCOMES`]).
    /// Booked by route when the side it went in against touched it last.
    pub(crate) fn note_goal_line_crossing(
        &mut self,
        conceding: PlayerSide,
        outcome: usize,
        context: &MatchContext,
    ) {
        if let Some(toucher) = self
            .last_touch_player_id
            .and_then(|id| context.players.by_id(id))
        {
            if toucher.side == Some(conceding) {
                let route = DeliveryCensus::own_goal_route(
                    self.shot_met.map(|met| met.on_frame),
                    toucher.tactical_position.current_position.is_goalkeeper(),
                    self.delivery_ledger.is_some(),
                    self.last_touch_kind,
                );
                self.delivery_tally.own_goals[route][outcome] += 1;
            } else if outcome == DeliveryCensus::REFUSED {
                self.delivery_tally.refused_no_shot += 1;
            } else if outcome == DeliveryCensus::OWN_GOAL {
                self.delivery_tally.own_goals_off_an_attacker += 1;
            }
        }
        if outcome != DeliveryCensus::REFUSED
            && let Some(ledger) = self.delivery_ledger.take()
        {
            ledger.close(DeliveryCensus::IN_THE_NET, &mut self.delivery_tally);
        }
    }

    fn last_touched_by_a_keeper(&self, players: &[MatchPlayer]) -> bool {
        self.last_touch_player_id
            .and_then(|id| players.iter().find(|p| p.id == id))
            .is_some_and(|p| p.tactical_position.current_position.is_goalkeeper())
    }
}
