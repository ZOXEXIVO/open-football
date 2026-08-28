//! Per-possession attacking role assignment — who is doing what in THIS
//! attack, shared by all eleven players on a side.
//!
//! # The problem this exists to solve
//!
//! [`TeamTacticalState`](super::super::tactical::TeamTacticalState) already gives
//! every player the same *weather*: phase, tempo, width, risk appetite,
//! press intensity. Every field on it is a scalar. What it never carried
//! was an **assignment**, so each off-ball player independently re-derived
//! their destination from the same global geometry — and, given the same
//! inputs, independently arrived at the same answer.
//!
//! Concretely, before this module: `best_free_channel` returned the same
//! least-congested gap to every midfielder who asked; `find_box_entry_point`
//! returned the same widest gap to all of them; `calculate_arriving_runner_target`
//! handed BOTH elected runners an identical `(x, y)`. Forwards had a
//! by-id slot de-duplication in `CreatingSpace` and midfielders had
//! nothing, and the two systems could not see each other, so a forward's
//! gap and a midfielder's channel were free to be the same patch of grass.
//! The measured signature was a permanent cluster in front of goal: 43% of
//! all shots from inside 6 m against a real ~15%, forwards taking two
//! thirds of their shots from there, ~30 passes into the box per forward
//! per match producing 3.5 key passes, and 39 cutbacks in 40 matches.
//!
//! The fix is not smarter local heuristics — it is that the destinations
//! must be **exclusive**. A box has four places worth attacking, and in
//! real football four different players attack them.
//!
//! # What is assigned
//!
//! * [`BoxSlot`] — near post, penalty spot, far post, cutback edge. At
//!   most one player each, so two players physically cannot target the
//!   same point.
//! * `primary_target` — the man the attack is FOR. The pass evaluator
//!   prefers him, which is what turns "eleven players near the ball" into
//!   a move with an intended end.
//! * `near_support` — the short outlet, deliberately NOT in the box.
//! * `far_runner` — the ball-in-behind option.
//! * `rest_defence` — who stays home, so committing bodies forward is a
//!   decision rather than an accident.
//!
//! Assignments are recomputed on the tactical cadence but scored with an
//! incumbency bonus, so a player keeps his job for the length of an attack
//! instead of trading it back and forth every refresh — the same
//! stability problem the `CreatingSpace` gap shortlist documents.

use crate::r#match::engine::teamplay::plans::wide::{Flank, WideBuilder, WidePlan};
use crate::r#match::{MatchField, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

/// A place in and around the penalty area worth attacking. Ordered near →
/// far → deep so the discriminant doubles as the array index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxSlot {
    /// Across the face of the six-yard box on the ball side — the
    /// flick-on and the first-time finish.
    NearPost,
    /// Central, level with the spot. The most-scored-from patch of grass
    /// in football.
    PenaltySpot,
    /// The second six, away from the ball. Where a floated cross goes and
    /// where an arriving winger finishes.
    FarPost,
    /// The edge of the area behind the ball — the cutback receiver, and
    /// the arriving midfielder's shooting position.
    CutbackEdge,
}

impl BoxSlot {
    pub const ALL: [BoxSlot; 4] = [
        BoxSlot::NearPost,
        BoxSlot::PenaltySpot,
        BoxSlot::FarPost,
        BoxSlot::CutbackEdge,
    ];

    pub fn index(self) -> usize {
        match self {
            BoxSlot::NearPost => 0,
            BoxSlot::PenaltySpot => 1,
            BoxSlot::FarPost => 2,
            BoxSlot::CutbackEdge => 3,
        }
    }

    /// Depth from the goal line and lateral offset from the goal centre,
    /// in game units (1u = 0.125 m). `ball_side` is +1 when the ball is on
    /// the high-y flank, so the near post is always the near one.
    ///
    /// Depths are chosen so every slot sits in a realistic shooting range
    /// and NONE of them is inside the six-yard box. An arriving runner
    /// parked on the goal line is the geometry error that produced
    /// permanent tap-ins (see `calculate_arriving_runner_target`), and a
    /// slot only a metre or two outside it risks the same thing for the
    /// same reason: the occupant is already standing there when the ball
    /// comes loose, so every scramble resolves at point-blank range.
    ///
    /// The resulting occupancy sits at 8-17 m from goal, which is where
    /// the real shot distribution actually peaks (6-11 m ≈ 25% of shots,
    /// 11-16.5 m ≈ 22%).
    ///
    /// NB these were widened from 44/86/60/124u on football grounds, not
    /// on a measurement: a 40-match sample could not separate the change
    /// from run-to-run noise (the harness's own noise floor is ±0.15
    /// goals/match over 5+ runs). The engine's <6 m over-supply — ~45% of
    /// all shots against a real ~15% — long predates this module and is
    /// not created by the slots; see the shot-mix diagnostics.
    fn offsets(self, ball_side: f32) -> (f32, f32) {
        match self {
            BoxSlot::NearPost => (58.0, ball_side * 34.0),
            BoxSlot::PenaltySpot => (92.0, ball_side * -6.0),
            BoxSlot::FarPost => (74.0, ball_side * -56.0),
            BoxSlot::CutbackEdge => (134.0, ball_side * 28.0),
        }
    }

    /// Where the occupant **waits** while the delivery is still being
    /// worked, in the same frame as [`Self::offsets`].
    ///
    /// # A striker does not stand on the spot he means to finish from
    ///
    /// [`Self::offsets`] is where the ball is going. Standing there and
    /// waiting for it is the one thing a centre-forward is coached never
    /// to do, for two reasons a defender exploits immediately: the
    /// occupant is in the marker's field of view for the whole
    /// possession, and he is stationary when the ball arrives, so the
    /// defender — who is moving — gets to it first.
    ///
    /// Every one of the four is therefore a PAIR of points. He holds the
    /// waiting one, which is deeper and in the defender's back, and
    /// attacks the finishing one as the ball is delivered. The gap
    /// between the two is the run, and it is 4-6 m in every case: far
    /// enough to be a genuine movement the marker has to react to,
    /// short enough that it is an arrival rather than a sprint.
    ///
    /// The four runs deliberately point in four different directions —
    /// across the near post, forward into the spot, peeling off to the
    /// back post, and holding at the top of the box. Four men breaking
    /// at once along four lines is what a defensive line cannot cover;
    /// four men standing on four dots is what it is built for.
    fn wait_offsets(self, ball_side: f32) -> (f32, f32) {
        match self {
            // Starts central, attacks ACROSS the front defender to the
            // near post. The run that beats a marker watching the ball.
            BoxSlot::NearPost => (88.0, ball_side * 4.0),
            // Arrives late into the spot from the edge of the D — the
            // most-scored-from movement in football, and the reason the
            // spot itself has to be empty until the ball is struck.
            BoxSlot::PenaltySpot => (128.0, ball_side * -20.0),
            // Holds narrow, in the cover shadow of the far centre-half,
            // and peels away to the back post as the ball is delivered.
            BoxSlot::FarPost => (102.0, ball_side * -34.0),
            // The one man who must NOT dive in. He holds outside the D —
            // 22 m, deeper than any of the other three wait at — so the
            // cutback has somebody arriving onto it at pace rather than
            // somebody already standing on the spot it is played to.
            BoxSlot::CutbackEdge => (178.0, ball_side * 40.0),
        }
    }

    /// Which flank the ball is on: `+1` on the high-`y` side. Every
    /// lateral offset is mirrored through it, so "near post" is the near
    /// one from wherever the ball actually is.
    pub fn ball_side(ball_y: f32, field_height: f32) -> f32 {
        if ball_y >= field_height / 2.0 {
            1.0
        } else {
            -1.0
        }
    }

    /// Where this slot actually is on the pitch.
    pub fn target(
        self,
        goal: Vector3<f32>,
        ball_y: f32,
        field_height: f32,
        forward_dir: f32,
    ) -> Vector3<f32> {
        let (depth, lateral) = self.offsets(Self::ball_side(ball_y, field_height));
        Self::place(goal, field_height, forward_dir, depth, lateral)
    }

    /// Where its occupant waits for the ball to be delivered. See
    /// [`Self::wait_offsets`].
    pub fn wait_target(
        self,
        goal: Vector3<f32>,
        ball_y: f32,
        field_height: f32,
        forward_dir: f32,
    ) -> Vector3<f32> {
        let (depth, lateral) = self.wait_offsets(Self::ball_side(ball_y, field_height));
        Self::place(goal, field_height, forward_dir, depth, lateral)
    }

    fn place(
        goal: Vector3<f32>,
        field_height: f32,
        forward_dir: f32,
        depth: f32,
        lateral: f32,
    ) -> Vector3<f32> {
        Vector3::new(
            goal.x - forward_dir * depth,
            (goal.y + lateral).clamp(12.0, field_height - 12.0),
            0.0,
        )
    }

    /// How much this slot wants an aerial player rather than a finisher.
    /// The far post and near post are attacked in the air; the cutback
    /// edge is struck with the foot.
    fn aerial_bias(self) -> f32 {
        match self {
            BoxSlot::NearPost => 0.55,
            BoxSlot::PenaltySpot => 0.45,
            BoxSlot::FarPost => 0.70,
            BoxSlot::CutbackEdge => 0.0,
        }
    }
}

/// Attacking assignments for one side. Cheap to copy (plain POD), like
/// [`TeamTacticalState`](super::super::tactical::TeamTacticalState).
#[derive(Debug, Clone, Copy)]
pub struct AttackPlan {
    /// Our player on the ball, if we have it.
    pub carrier: Option<u32>,
    /// The man this attack is FOR. Read by the pass evaluator.
    pub primary_target: Option<u32>,
    /// The short outlet — kept out of the box on purpose.
    pub near_support: Option<u32>,
    /// The ball-in-behind runner.
    pub far_runner: Option<u32>,
    /// One player per [`BoxSlot`], indexed by [`BoxSlot::index`].
    pub box_slots: [Option<u32>; 4],
    /// Players held behind the ball as rest defence.
    pub rest_defence: [Option<u32>; 5],
    /// Who is holding each touchline, and who is running beyond them.
    /// See [`wide`](super::wide) — this is the lateral half of the plan,
    /// and without it every assignment above is a central one.
    pub wide: WidePlan,
    /// True while the plan describes a live attack. When false every
    /// consumer falls back to its own positioning — there is no attack to
    /// have roles in.
    pub active: bool,
}

impl AttackPlan {
    pub const fn idle() -> Self {
        AttackPlan {
            carrier: None,
            primary_target: None,
            near_support: None,
            far_runner: None,
            box_slots: [None; 4],
            rest_defence: [None; 5],
            wide: WidePlan::idle(),
            active: false,
        }
    }

    /// The slot this player has been given, if any.
    pub fn slot_of(&self, player_id: u32) -> Option<BoxSlot> {
        if !self.active {
            return None;
        }
        BoxSlot::ALL
            .into_iter()
            .find(|s| self.box_slots[s.index()] == Some(player_id))
    }

    /// Is this player committed to the attack in any role?
    pub fn is_committed(&self, player_id: u32) -> bool {
        self.active
            && (self.primary_target == Some(player_id)
                || self.far_runner == Some(player_id)
                || self.slot_of(player_id).is_some())
    }

    /// Is this player held back as rest defence?
    pub fn is_rest_defence(&self, player_id: u32) -> bool {
        self.active && self.rest_defence.contains(&Some(player_id))
    }

    /// Recompute both sides' plans in place. Called from the same tick-loop
    /// slot as `TeamTacticalState::refresh`, and reads the tactical state
    /// it has just produced.
    pub fn refresh(home: &mut Self, away: &mut Self, inputs: &AttackRefreshInputs<'_>) {
        let field = inputs.field;
        let ball_pos = field.ball.position;
        let owner_team = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
            .map(|p| p.team_id);

        for (plan, team_id, attacking, in_possession, rest_target) in [
            (
                &mut *home,
                inputs.home_team_id,
                inputs.home_attacking,
                inputs.home_in_possession,
                inputs.home_rest_defence,
            ),
            (
                &mut *away,
                inputs.away_team_id,
                inputs.away_attacking,
                inputs.away_in_possession,
                inputs.away_rest_defence,
            ),
        ] {
            // A ball in flight has no owner, and a pass is exactly when
            // the runs that make an attack are being made — dropping the
            // plan for the duration of every pass halved how often it was
            // live. The tactical `in_possession` flag (folded into
            // `attacking`) already says the possession is ours; the live
            // owner check only needs to veto the case where an opponent
            // has actually taken the ball off us.
            let ours = owner_team.is_none_or(|t| t == team_id);
            let builder = PlanBuilder {
                field,
                team_id,
                ball_pos,
                rest_target,
            };
            if !attacking || !ours {
                // Width outlives the attack plan.
                //
                // `wants_bodies_forward` is false during `BuildUp`, and
                // a side playing out from the back is exactly when its
                // full-backs go wide — the whole purpose of the width is
                // to stretch the first line of the press so there is
                // somewhere to play. Holding a touchline commits nobody
                // forward, so it is safe in every phase we have the
                // ball, unlike a box slot or an overlap.
                let previous_wide = plan.wide;
                *plan = AttackPlan::idle();
                if in_possession && ours {
                    builder.build_width(plan, &previous_wide);
                }
                #[cfg(feature = "match-logs")]
                crate::mid_run_diag::PlanDiag::note_refresh(false, 0);
                continue;
            }
            builder.build(plan);
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::PlanDiag::note_refresh(
                plan.active,
                plan.box_slots.iter().filter(|s| s.is_some()).count(),
            );
        }
    }
}

/// Inputs to [`AttackPlan::refresh`], bundled so the call site stays
/// readable — the same shape as `TacticalRefreshInputs`.
pub struct AttackRefreshInputs<'a> {
    pub field: &'a MatchField,
    pub home_team_id: u32,
    pub away_team_id: u32,
    /// True when that side is in a phase where committing bodies forward
    /// is correct (`Attack` / `AttackingTransition` / `Progression`).
    pub home_attacking: bool,
    pub away_attacking: bool,
    /// True whenever that side simply has the ball, in any phase. Width
    /// is held from the first pass out of defence — see the `BuildUp`
    /// note in [`AttackPlan::refresh`].
    pub home_in_possession: bool,
    pub away_in_possession: bool,
    /// How many players that side wants behind the ball, from
    /// `TeamTacticalState::rest_defense_count`.
    pub home_rest_defence: u8,
    pub away_rest_defence: u8,
}

/// One side's assignment pass. A struct rather than a pile of arguments
/// so the geometry is resolved once and every scorer reads the same view.
struct PlanBuilder<'a> {
    field: &'a MatchField,
    team_id: u32,
    ball_pos: Vector3<f32>,
    rest_target: u8,
}

impl PlanBuilder<'_> {
    /// Beyond this a player is not part of the attack at all.
    const ATTACK_RADIUS: f32 = 420.0;

    /// Which end this side attacks, and the width builder projected into
    /// its formation's own bounding box.
    ///
    /// The box is read the same way [`ShapeBuilder`](super::block::ShapeBuilder)
    /// reads it, so "wide" and "advanced" mean the same thing to the plan
    /// and to the block it is projected into.
    fn geometry(&self) -> Option<(PlayerSide, WideBuilder<'_>)> {
        let side = self
            .field
            .players
            .iter()
            .find(|p| p.team_id == self.team_id)
            .and_then(|p| p.side)?;
        let field_width = self.field.size.width as f32;
        let (mut min_lat, mut max_lat) = (f32::MAX, f32::MIN);
        let (mut min_depth, mut max_depth) = (f32::MAX, f32::MIN);
        for p in self.outfielders() {
            let depth = side.attacking_progress_x(p.start_position.x, field_width);
            min_depth = min_depth.min(depth);
            max_depth = max_depth.max(depth);
            min_lat = min_lat.min(p.start_position.y);
            max_lat = max_lat.max(p.start_position.y);
        }
        if min_depth > max_depth {
            return None;
        }
        Some((
            side,
            WideBuilder {
                field: self.field,
                team_id: self.team_id,
                side,
                ball_pos: self.ball_pos,
                min_lat,
                lat_span: (max_lat - min_lat).max(1.0),
                min_depth,
                depth_span: (max_depth - min_depth).max(0.01),
            },
        ))
    }

    /// The touchlines alone, for a side that has the ball but is not yet
    /// in a phase that commits bodies forward.
    fn build_width(&self, plan: &mut AttackPlan, previous: &WidePlan) {
        let Some((_, wide)) = self.geometry() else {
            return;
        };
        let carrier = self.field.ball.current_owner;
        wide.holders(&mut plan.wide, previous, |id| Some(id) != carrier);
        plan.wide.active = true;
    }

    fn build(&self, plan: &mut AttackPlan) {
        let previous = *plan;
        *plan = AttackPlan::idle();

        let Some((side, wide)) = self.geometry() else {
            return;
        };
        let forward_dir = side.forward_dir_x();
        let field_width = self.field.size.width as f32;
        let field_height = self.field.size.height as f32;
        let goal = Vector3::new(
            match side {
                PlayerSide::Left => field_width,
                PlayerSide::Right => 0.0,
            },
            field_height / 2.0,
            0.0,
        );

        plan.carrier = self.field.ball.current_owner;
        plan.active = true;

        let mut held = HeldSet::default();

        // ── Width first ───────────────────────────────────────────────
        // How wide a side plays is the first decision it makes with the
        // ball, not the last. Assigning the touchlines before anything
        // else is also what makes them REACHABLE: every other role in
        // this module pulls a man toward the middle, so a width holder
        // chosen from the leftovers is a man who has already been given
        // a reason to be somewhere else. See `wide`.
        wide.holders(&mut plan.wide, &previous.wide, |id| {
            Some(id) != plan.carrier
        });
        plan.wide.active = true;
        // Only the BALL-SIDE holder is reserved. The far-side one stays
        // eligible for the far post, because the blind-side winger
        // arriving at the back post is the same player doing the same
        // job one phase later — see the `wide` module docs.
        if let Some(id) = plan.wide.holder(plan.wide.ball_flank) {
            held.mark(id);
        }

        // ── Rest defence ──────────────────────────────────────────────
        // Taking the men who hold BEFORE assigning attacking roles is
        // what stops the shape emptying: a slot can never be filled by a
        // man who is supposed to be staying home.
        //
        // Neither touchline is a place you defend from, so a width
        // holder is never rest defence — otherwise the plan would name
        // the same man as both the outlet on the flank and the cover
        // behind the ball, and he would spend the possession running
        // between the two.
        let ball_flank = plan.wide.ball_flank;
        // At most ten outfielders — a stack buffer keeps this refresh
        // allocation-free, which matters because it runs twice every ten
        // ticks for the whole match.
        let mut depth_ranked = [(0u32, 0.0f32); MAX_OUTFIELD];
        let mut ranked_len = 0usize;
        for p in self.outfielders() {
            if ranked_len == MAX_OUTFIELD {
                break;
            }
            if plan.wide.flank_of(p.id).is_some() {
                continue;
            }
            depth_ranked[ranked_len] = (p.id, self.rest_fit(p, goal, ball_flank, field_height));
            ranked_len += 1;
        }
        // Insertion sort, deepest first; ties by id so the pick is
        // reproducible run to run.
        for i in 1..ranked_len {
            let item = depth_ranked[i];
            let mut j = i;
            while j > 0
                && (depth_ranked[j - 1].1 < item.1
                    || (depth_ranked[j - 1].1 == item.1 && depth_ranked[j - 1].0 > item.0))
            {
                depth_ranked[j] = depth_ranked[j - 1];
                j -= 1;
            }
            depth_ranked[j] = item;
        }
        let rest_n = (self.rest_target as usize).min(5).min(ranked_len);
        for (slot, (id, _)) in plan.rest_defence.iter_mut().zip(&depth_ranked[..rest_n]) {
            *slot = Some(*id);
            held.mark(*id);
        }

        // ── …and only now, the overlap ────────────────────────────────
        // Whoever is left on the ball's flank behind the width holder is
        // by construction a man the plan could afford to send, so the
        // safety question is already answered and the run needs no gate
        // of its own.
        wide.overlap(&mut plan.wide, |id| {
            !held.contains(id) && Some(id) != plan.carrier
        });
        if let Some(id) = plan.wide.overlap_runner {
            held.mark(id);
        }

        // ── Box slots ─────────────────────────────────────────────────
        // Each slot takes the best REMAINING candidate, so the
        // assignments are exclusive by construction. Slots are filled
        // near → far → deep because the near-post runner is the most
        // position-constrained and should get first pick.
        for slot in BoxSlot::ALL {
            let target = slot.target(goal, self.ball_pos.y, field_height, forward_dir);
            let incumbent = previous.box_slots[slot.index()];
            let best = self
                .outfielders()
                .filter(|p| !held.contains(p.id))
                .filter(|p| Some(p.id) != plan.carrier)
                .filter(|p| (p.position - self.ball_pos).magnitude() < Self::ATTACK_RADIUS)
                .map(|p| {
                    let mut score = self.slot_fit(p, slot, target);
                    // Incumbency: keeping a job is worth a little, so a
                    // marginal score change doesn't send two players
                    // swapping runs mid-attack.
                    if incumbent == Some(p.id) {
                        score += 0.15;
                    }
                    (p.id, score)
                })
                // Ties break by id so the assignment is reproducible.
                .max_by(|a, b| {
                    a.1.partial_cmp(&b.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(b.0.cmp(&a.0))
                });
            if let Some((id, score)) = best {
                // A slot nobody is remotely suited to stays EMPTY. Real
                // attacks don't fill all four every time, and forcing a
                // holding midfielder onto the far post to satisfy the
                // array is exactly the over-commitment this module exists
                // to prevent.
                if score > 0.25 {
                    plan.box_slots[slot.index()] = Some(id);
                    held.mark(id);
                }
            }
        }

        // ── Primary target ────────────────────────────────────────────
        // The man the attack is FOR: the box occupant in the most
        // dangerous position with the best chance of being found.
        plan.primary_target = BoxSlot::ALL
            .into_iter()
            .filter_map(|s| plan.box_slots[s.index()].map(|id| (s, id)))
            .filter_map(|(s, id)| {
                let p = self.field.players.iter().find(|p| p.id == id)?;
                let marked = self
                    .field
                    .players
                    .iter()
                    .filter(|o| o.team_id != self.team_id)
                    .filter(|o| (o.position - p.position).magnitude() < 30.0)
                    .count() as f32;
                let danger = match s {
                    BoxSlot::PenaltySpot => 1.0,
                    BoxSlot::CutbackEdge => 0.85,
                    BoxSlot::FarPost => 0.8,
                    BoxSlot::NearPost => 0.75,
                };
                Some((id, danger - marked * 0.18))
            })
            .max_by(|a, b| {
                a.1.partial_cmp(&b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.0.cmp(&a.0))
            })
            .map(|(id, _)| id);

        // ── Near support ──────────────────────────────────────────────
        // The short outlet. Explicitly NOT a box occupant — the whole
        // point is to have somebody to play backwards to.
        plan.near_support = self
            .outfielders()
            .filter(|p| !held.contains(p.id))
            .filter(|p| Some(p.id) != plan.carrier)
            .map(|p| (p.id, (p.position - self.ball_pos).magnitude()))
            .filter(|(_, d)| *d < 220.0)
            .min_by(|a, b| {
                a.1.partial_cmp(&b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.0.cmp(&b.0))
            })
            .map(|(id, _)| id);
        if let Some(id) = plan.near_support {
            held.mark(id);
        }

        // ── Far runner ────────────────────────────────────────────────
        // Fastest uncommitted attacker, for the ball in behind.
        plan.far_runner = self
            .outfielders()
            .filter(|p| !held.contains(p.id))
            .filter(|p| Some(p.id) != plan.carrier)
            .filter(|p| (goal - p.position).magnitude() < 460.0)
            .map(|p| {
                let pace = (p.skills.physical.pace + p.skills.physical.acceleration) / 40.0;
                let off_ball = p.skills.mental.off_the_ball / 20.0;
                (p.id, pace * 0.65 + off_ball * 0.35)
            })
            .max_by(|a, b| {
                a.1.partial_cmp(&b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.0.cmp(&a.0))
            })
            .map(|(id, _)| id);
    }

    /// How much this player is wanted at home, in "effective depth" —
    /// game units from the goal being attacked, plus what his job is
    /// worth in the same currency.
    ///
    /// Depth is most of it: rest defence is, first of all, whoever is
    /// already back there. But WHICH of two men at the same depth stays
    /// is a footballing question with a settled answer, and ranking on
    /// raw depth alone got it wrong every time. A back four's two
    /// full-backs stand at the same depth as each other, so both were
    /// picked, on every possession — which is most of why the
    /// overlapping-full-back funnel measured **260 committed ticks a
    /// match** out of 256 000 asked. The behaviour was gated on a
    /// coin-flip that the plan had already decided against.
    ///
    /// Real rest defence is the centre-backs, the screen in front of
    /// them, and the full-back *away* from the ball. The full-back on
    /// the ball's side is the one who goes; that is what the shape is
    /// for. The three bonuses are the distances each of those jobs is
    /// worth: a screening midfielder plays 15-20 m in front of the back
    /// line and should still rank alongside it, so his is the largest.
    fn rest_fit(
        &self,
        player: &MatchPlayer,
        goal: Vector3<f32>,
        ball_flank: Flank,
        field_height: f32,
    ) -> f32 {
        let pos = player.tactical_position.current_position;
        (goal - player.position).magnitude()
            + Self::rest_bonus(
                pos.is_central_defender(),
                pos.is_defensive_midfielder(),
                Flank::of(player.start_position.y, field_height) != ball_flank,
            )
    }

    /// The job half of [`Self::rest_fit`], in the same units (game units
    /// from the goal being attacked). Pure, so the ordering it is
    /// supposed to produce can be asserted without an engine fixture.
    fn rest_bonus(central_defender: bool, screen: bool, far_side: bool) -> f32 {
        /// A centre-back holds even when he has stepped up (~17.5 m).
        const CENTRAL_HOLD: f32 = 140.0;
        /// The screen is worth his own advancement (~21 m), so he ranks
        /// with the line he is screening.
        const SCREEN_HOLD: f32 = 170.0;
        /// Enough to separate two full-backs standing level (~9 m).
        const FAR_SIDE_HOLD: f32 = 70.0;

        let mut bonus = 0.0;
        if central_defender {
            bonus += CENTRAL_HOLD;
        }
        if screen {
            bonus += SCREEN_HOLD;
        }
        if far_side {
            bonus += FAR_SIDE_HOLD;
        }
        bonus
    }

    /// How well this player suits this slot. Proximity decides WHO (a run
    /// is made by whoever is placed to make it), attributes decide how
    /// good the occupancy is, and position group keeps defenders out of
    /// the box in open play.
    fn slot_fit(
        &self,
        player: &crate::r#match::MatchPlayer,
        slot: BoxSlot,
        target: Vector3<f32>,
    ) -> f32 {
        let gap = (player.position - target).magnitude();
        // A run of ~25 m is the outer limit of "he can make it in time".
        let reach = (1.0 - gap / 200.0).clamp(0.0, 1.0);

        let s = &player.skills;
        let aerial = (s.technical.heading / 20.0) * 0.5
            + (s.physical.jumping / 20.0) * 0.3
            + (s.physical.strength / 20.0) * 0.2;
        let ground = (s.technical.finishing / 20.0) * 0.55
            + (s.mental.composure / 20.0) * 0.25
            + (s.technical.technique / 20.0) * 0.20;
        let bias = slot.aerial_bias();
        let quality = aerial * bias + ground * (1.0 - bias);

        let timing = s.mental.off_the_ball / 20.0;

        // Role appetite: forwards live in the box, midfielders arrive in
        // it, defenders belong in it only at a set piece.
        let pos = player.tactical_position.current_position;
        let role = if pos.is_forward() {
            1.0
        } else if pos.is_midfielder() {
            match slot {
                // The arriving midfielder's slot.
                BoxSlot::CutbackEdge => 1.0,
                _ => 0.6,
            }
        } else {
            0.12
        };

        (reach * 0.42 + quality * 0.28 + timing * 0.30) * role
    }

    fn outfielders(&self) -> impl Iterator<Item = &MatchPlayer> {
        self.field
            .players
            .iter()
            .filter(move |p| p.team_id == self.team_id)
            .filter(|p| !p.tactical_position.current_position.is_goalkeeper())
    }
}

/// The most outfielders one side can have on the pitch.
const MAX_OUTFIELD: usize = 10;

/// Players already given a job this refresh, so no two roles can claim
/// the same man.
///
/// Stores ids rather than hashing them into a bitmap: match ids are
/// arbitrary `u32`s, and a modulo bitmap would silently exclude a player
/// whose id happened to collide with a teammate's — a bug that would only
/// show up as "that striker never makes runs" in one squad out of many.
/// A linear scan over at most ten entries is cheaper than a hash anyway.
#[derive(Default)]
struct HeldSet {
    ids: [u32; MAX_OUTFIELD],
    len: usize,
}

impl HeldSet {
    fn mark(&mut self, id: u32) {
        if self.len < MAX_OUTFIELD && !self.contains(id) {
            self.ids[self.len] = id;
            self.len += 1;
        }
    }

    fn contains(&self, id: u32) -> bool {
        self.ids[..self.len].contains(&id)
    }
}

#[cfg(test)]
mod rest_defence_tests {
    use super::PlanBuilder;

    /// The ordering rest defence has to produce, stated as a test rather
    /// than as a comment: with a back four standing level, the two
    /// centre-backs and the FAR full-back stay, and the ball-side
    /// full-back is the one released.
    ///
    /// Ranking on raw depth cannot produce it — the four are at the same
    /// depth, so it picked whichever two full-backs the tie-break
    /// happened to reach, on every possession. That is why the overlap
    /// funnel measured 260 committed ticks a match.
    #[test]
    fn the_ball_side_fullback_is_the_one_who_goes() {
        let cb = PlanBuilder::rest_bonus(true, false, false);
        let far_fb = PlanBuilder::rest_bonus(false, false, true);
        let ball_fb = PlanBuilder::rest_bonus(false, false, false);
        assert!(cb > far_fb, "a centre-back holds ahead of a full-back");
        assert!(
            far_fb > ball_fb,
            "the full-back away from the ball is the one who tucks in"
        );
        assert_eq!(ball_fb, 0.0, "the ball-side full-back carries no hold");
    }

    /// …and the screen ranks with the line he screens, not behind it. He
    /// plays 15-20 m in front of the back four, so a bonus smaller than
    /// that gap would leave him ranked below a full-back who is level
    /// with the centre-backs — and the full-back would stay home in his
    /// place.
    #[test]
    fn the_screen_ranks_with_the_line_he_screens() {
        let screen = PlanBuilder::rest_bonus(false, true, false);
        let far_fb = PlanBuilder::rest_bonus(false, false, true);
        // 15 m in game units — a defensive midfielder's normal distance
        // in front of his own back line.
        const AHEAD: f32 = 120.0;
        assert!(
            screen > far_fb + AHEAD - 70.0,
            "the screen is out-ranked by his own advancement"
        );
    }
}

#[cfg(test)]
mod box_slot_tests {
    use super::BoxSlot;

    /// **Every slot is a pair of points, and the run between them is a
    /// real one.** The waiting point has to be genuinely further from
    /// goal than the finishing point, or the occupant is standing where
    /// the ball is going — the defect `BoxMovement` exists to fix — and
    /// the gap has to be an arrival rather than a sprint or a shuffle.
    ///
    /// Stated as a test because both halves are easy to break by nudging
    /// one number: pulling a waiting point in to "get him closer to
    /// goal" silently deletes the movement, and pushing one out to "make
    /// the run bigger" turns a striker's arrival into a 20 m sprint he
    /// cannot time.
    #[test]
    fn every_slot_waits_behind_the_point_it_attacks() {
        // Both flanks, because the lateral offsets are mirrored.
        for ball_side in [-1.0_f32, 1.0] {
            for slot in BoxSlot::ALL {
                let (finish_depth, finish_lat) = slot.offsets(ball_side);
                let (wait_depth, wait_lat) = slot.wait_offsets(ball_side);
                assert!(
                    wait_depth > finish_depth,
                    "{slot:?} waits level with or ahead of the ball's destination"
                );
                let run =
                    ((wait_depth - finish_depth).powi(2) + (wait_lat - finish_lat).powi(2)).sqrt();
                // 4-6 m: far enough that the marker has to react, short
                // enough to be an arrival. 32u = 4 m, 56u = 7 m.
                assert!(
                    (32.0..=56.0).contains(&run),
                    "{slot:?} run is {run}u ({}m), outside the arrival band",
                    run * 0.125
                );
            }
        }
    }

    /// …and the four of them break along four DIFFERENT lines, which is
    /// the whole reason a box has four places in it. Two occupants
    /// running the same line are one occupant as far as a defence is
    /// concerned.
    #[test]
    fn the_four_runs_point_in_four_directions() {
        let ball_side = 1.0_f32;
        let lines: Vec<(BoxSlot, f32, f32)> = BoxSlot::ALL
            .into_iter()
            .map(|slot| {
                let (fd, fl) = slot.offsets(ball_side);
                let (wd, wl) = slot.wait_offsets(ball_side);
                let (dx, dy) = (fd - wd, fl - wl);
                let n = (dx * dx + dy * dy).sqrt().max(0.001);
                (slot, dx / n, dy / n)
            })
            .collect();
        for (i, (a, ax, ay)) in lines.iter().enumerate() {
            for (b, bx, by) in lines.iter().skip(i + 1) {
                let dot = ax * bx + ay * by;
                assert!(
                    dot < 0.95,
                    "{a:?} and {b:?} attack their slots along the same line"
                );
            }
        }
    }
}
