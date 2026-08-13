//! Per-possession attacking role assignment — who is doing what in THIS
//! attack, shared by all eleven players on a side.
//!
//! # The problem this exists to solve
//!
//! [`TeamTacticalState`](super::tactical::TeamTacticalState) already gives
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

    /// Where this slot actually is on the pitch.
    pub fn target(
        self,
        goal: Vector3<f32>,
        ball_y: f32,
        field_height: f32,
        forward_dir: f32,
    ) -> Vector3<f32> {
        let ball_side = if ball_y >= field_height / 2.0 {
            1.0
        } else {
            -1.0
        };
        let (depth, lateral) = self.offsets(ball_side);
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
/// [`TeamTacticalState`](super::tactical::TeamTacticalState).
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

        for (plan, team_id, attacking, rest_target) in [
            (
                &mut *home,
                inputs.home_team_id,
                inputs.home_attacking,
                inputs.home_rest_defence,
            ),
            (
                &mut *away,
                inputs.away_team_id,
                inputs.away_attacking,
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
            if !attacking || !ours {
                *plan = AttackPlan::idle();
                #[cfg(feature = "match-logs")]
                crate::mid_run_diag::PlanDiag::note_refresh(false, 0);
                continue;
            }
            PlanBuilder {
                field,
                team_id,
                ball_pos,
                rest_target,
            }
            .build(plan);
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

    fn build(&self, plan: &mut AttackPlan) {
        let previous = *plan;
        *plan = AttackPlan::idle();

        let Some(side) = self
            .field
            .players
            .iter()
            .find(|p| p.team_id == self.team_id)
            .and_then(|p| p.side)
        else {
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

        // ── Rest defence first ────────────────────────────────────────
        // Taking the deepest N off the table BEFORE assigning attacking
        // roles is what stops the shape emptying: a slot can never be
        // filled by a man who is supposed to be holding.
        let mut held = HeldSet::default();
        // At most ten outfielders — a stack buffer keeps this refresh
        // allocation-free, which matters because it runs twice every ten
        // ticks for the whole match.
        let mut depth_ranked = [(0u32, 0.0f32); MAX_OUTFIELD];
        let mut ranked_len = 0usize;
        for p in self.outfielders() {
            if ranked_len == MAX_OUTFIELD {
                break;
            }
            depth_ranked[ranked_len] = (p.id, (goal - p.position).magnitude());
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
