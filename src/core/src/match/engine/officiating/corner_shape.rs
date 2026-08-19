//! Where the other twenty players stand while a corner is taken.
//!
//! # Why this exists
//!
//! A real corner is preceded by thirty to sixty seconds of dead time, and
//! both sides spend all of it walking into a shape: the defending side
//! brings ten men back and posts them on the posts, across the six-yard
//! line, on the penalty-spot band and on the edge of the area; the
//! attacking side loads five or six into the box and leaves a short
//! option by the flag.
//!
//! The simulation has no such stoppage. The corner is awarded, the taker
//! is teleported onto the ball, and [`MidfielderCrossingState`] delivers
//! it five ticks (50 ms) later. Nobody can walk anywhere in 50 ms, so
//! until this module existed the "corner shape" was simply whatever
//! open-play positions the twenty-two happened to be standing in when the
//! ball went behind — which, after a counter-attack that ended with a
//! defender hooking it out, routinely meant **the defending side had
//! nobody but the goalkeeper in its own box**. Measured off a recorded
//! match: 3–6 outfielders inside the penalty area at the moment of the
//! cross, against a real 7–9, and a low of none.
//!
//! The engine already accepted the premise for one half of the problem —
//! `Ball::check_wide_of_goal` teleported the attacking side's two best
//! centre-backs into the box for exactly this reason. This module is that
//! idea carried through to all twenty players.
//!
//! # What it does NOT do
//!
//! It places people; it does not decide anything. No RNG, no mutation, no
//! reading of the match clock — the same twenty-two in the same positions
//! always produce the same plan, which is what keeps replays reproducible.
//! Holding a player on his station afterwards belongs to
//! `CornerHold` in the state dispatcher, and attacking the delivery
//! belongs to the states themselves.

use crate::r#match::{MatchPlayer, PlayerSide};
use nalgebra::Vector3;
use std::cmp::Ordering;

/// One player's job at this corner.
///
/// Carried alongside the position because the engine treats two of them
/// specially — [`CornerRole::BoxAttacker`] is forced into
/// `DefenderState::AttackingCorner` on arrival, and the roles are what
/// the box-occupancy census counts — and because a bare list of
/// coordinates is unreadable in a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CornerRole {
    // ── attacking side ────────────────────────────────────────────────
    /// Centre-back pushed up to attack the delivery. The one role that
    /// predates this module; kept on its original spot so the hold in
    /// `DefenderAttackingCornerState::box_attack_target` still agrees
    /// with where he was put.
    BoxAttacker,
    /// Anyone else attacking the delivery — near-post flick, penalty
    /// spot, back post.
    BoxRunner,
    /// The short-corner option, a few metres from the taker.
    ShortOption,
    /// Top of the area, for the cut-back and the second ball.
    EdgeRunner,

    // ── defending side ────────────────────────────────────────────────
    /// On a post, inside the goal frame.
    PostGuard,
    /// The zonal band across the front of the six-yard box.
    ZonalScreen,
    /// Picking a runner up on the penalty-spot band.
    Marker,
    /// Top of the area — first to any headed clearance.
    EdgeClearer,
    /// Sent out to the flag to deny the short corner.
    ShortCover,
    /// The one man left up the pitch to run onto a clearance.
    Outlet,
}

impl CornerRole {
    /// Is this a station on the side that is defending the corner?
    pub fn is_defensive(self) -> bool {
        matches!(
            self,
            CornerRole::PostGuard
                | CornerRole::ZonalScreen
                | CornerRole::Marker
                | CornerRole::EdgeClearer
                | CornerRole::ShortCover
                | CornerRole::Outlet
        )
    }
}

/// A player, where he is being put, and why.
#[derive(Debug, Clone, Copy)]
pub struct CornerStation {
    pub player_id: u32,
    pub position: Vector3<f32>,
    pub role: CornerRole,
}

/// A corner shape that is currently pinned on the players.
///
/// Held on the [`Ball`](crate::r#match::engine::ball::Ball) because the
/// ball is what the corner belongs to, and because the two facts needed to
/// know when to let go both concern it: when the shape went up, and who
/// took the kick.
#[derive(Debug, Clone, Copy)]
pub struct CornerShapeHold {
    /// Engine tick the stations were armed on.
    pub armed_tick: u64,
    /// The taker. He is the only man allowed to touch the ball without
    /// ending the set piece — the award stamps him as last toucher and so
    /// does his own delivery, so "anybody else has touched it" is the
    /// clean test for first contact.
    pub taker_id: u32,
}

/// The corner set-up planner.
pub struct CornerShape;

/// Half the goal width, in field units (1 u = 0.125 m). The real goal is
/// 7.32 m; `GOAL_WIDTH` in `flow::goal` carries the same number.
const GOAL_HALF_WIDTH: f32 = 29.0;
/// Depth of the six-yard box from the goal line: 5.5 m.
const SIX_YARD_DEPTH: f32 = 44.0;
/// Depth of the penalty spot: 11 m.
const PENALTY_SPOT_DEPTH: f32 = 88.0;
/// Depth of the penalty area: 16.5 m.
const PENALTY_AREA_DEPTH: f32 = 132.0;
/// Half-width of the penalty area: 20.16 m.
const PENALTY_AREA_HALF_WIDTH: f32 = 161.0;
/// The ten yards (9.15 m) an opponent must retreat from the corner arc.
/// The short-corner cover stands exactly on it, because that is where the
/// referee makes him stand.
const CORNER_RETREAT: f32 = 73.0;
/// Furthest the man left up the pitch may stand from his own goal line:
/// the halfway line of a 105 m pitch. He holds the last line, but he does
/// not cross into the opposition half while his side defends a corner.
const OUTLET_MAX_DEPTH: f32 = 420.0;

impl CornerShape {
    /// Plan the set-up for a corner about to be taken from `flag`.
    ///
    /// `goal_x` is the *defended* goal line (0.0 or the pitch width) and
    /// `taker_id` is excluded from both pools — he is already on the ball.
    /// Stations are emitted in descending priority, so a side reduced to
    /// nine men simply loses its lowest-value posts rather than leaving a
    /// gap in the middle of its box.
    pub fn plan(
        players: &[MatchPlayer],
        attacking_side: PlayerSide,
        taker_id: u32,
        goal_x: f32,
        flag: Vector3<f32>,
        field_width: f32,
        field_height: f32,
    ) -> Vec<CornerStation> {
        let geometry = CornerGeometry::new(goal_x, flag, field_width, field_height);
        let mut stations = Vec::with_capacity(18);

        let mut defenders = Self::pool(players, attacking_side, taker_id, false);
        let mut attackers = Self::pool(players, attacking_side, taker_id, true);

        // Attack first, deliberately: it leaves `attackers` holding exactly
        // the men the attacking side did NOT send forward, and those —
        // with their goalkeeper — are the last line the outlet has to stay
        // onside of.
        Self::plan_attack(&mut attackers, &geometry, &mut stations);
        let outlet_depth = Self::outlet_depth(players, &attackers, attacking_side, &geometry);
        Self::plan_defence(&mut defenders, &geometry, outlet_depth, &mut stations);

        stations
    }

    /// How far up the man left forward stands, as a depth from his own
    /// goal line.
    ///
    /// He is a striker holding the last line, and the last line is the
    /// second-rearmost opponent — the deeper of the two the attacking
    /// side leaves back, since their goalkeeper is the rearmost. Standing
    /// a stride goal-side of him is what a real outlet does, and the
    /// reason he does it is the same reason it matters here: everything
    /// beyond that line is offside the moment the clearance is played.
    /// Parking him on the halfway line regardless cost about one extra
    /// offside per match.
    ///
    /// Capped at halfway: the man left up does not wander into the
    /// opposition half while his side defends a corner, whatever the
    /// opposition's cover is doing.
    fn outlet_depth(
        players: &[MatchPlayer],
        cover: &[&MatchPlayer],
        attacking_side: PlayerSide,
        g: &CornerGeometry,
    ) -> f32 {
        // Depth here is measured from the goal being DEFENDED at this
        // corner, so the attacking side's own goalkeeper has the largest
        // depth of anybody and their cover men come next. Sorted
        // descending, the second entry is the second-rearmost opponent —
        // the offside line.
        let mut line: Vec<f32> = cover
            .iter()
            .map(|p| (p.position.x - g.goal_x).abs())
            .collect();
        line.extend(
            players
                .iter()
                .filter(|p| {
                    p.side == Some(attacking_side)
                        && !p.is_sent_off
                        && p.tactical_position.current_position.is_goalkeeper()
                })
                .map(|p| (p.position.x - g.goal_x).abs()),
        );
        line.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
        let last_line = line.get(1).copied().unwrap_or(OUTLET_MAX_DEPTH);
        // A stride onside of him, never past halfway, and never so deep
        // that the outlet is standing in his own box instead of forward.
        (last_line - 12.0).clamp(160.0, OUTLET_MAX_DEPTH)
    }

    /// Outfield players of one side who are still on the pitch, in roster
    /// order (which is stable across ticks, so every tie broken by it is
    /// reproducible).
    fn pool<'a>(
        players: &'a [MatchPlayer],
        attacking_side: PlayerSide,
        taker_id: u32,
        want_attacking: bool,
    ) -> Vec<&'a MatchPlayer> {
        players
            .iter()
            .filter(|p| {
                let is_attacking = p.side == Some(attacking_side);
                is_attacking == want_attacking
                    && p.side.is_some()
                    && !p.is_sent_off
                    && p.id != taker_id
                    && !p.tactical_position.current_position.is_goalkeeper()
            })
            .collect()
    }

    // ── defending side ────────────────────────────────────────────────

    /// Ten stations, assigned worst-in-the-air outward: the men who will
    /// not win a header guard the posts and chase the short corner, and
    /// the big men get the zone the ball is actually delivered into.
    ///
    /// The order of the `take` calls IS the priority order — see
    /// [`CornerShape::plan`].
    fn plan_defence(
        pool: &mut Vec<&MatchPlayer>,
        g: &CornerGeometry,
        outlet_depth: f32,
        out: &mut Vec<CornerStation>,
    ) {
        // One man stays up. Every side leaves someone forward so a cleared
        // ball has somewhere to go; without him a corner is a free
        // ten-versus-ten siege with no downside for the attacking side.
        // The quickest forward is the one who gets left, and he holds the
        // last line rather than the halfway line — see `outlet_depth`.
        if let Some(p) = Self::take_best(pool, |p| Self::outlet_score(p)) {
            out.push(CornerStation {
                player_id: p.id,
                position: g.at(outlet_depth, 0.0),
                role: CornerRole::Outlet,
            });
        }

        // The short-corner cover, on the ten-yard retreat line. Whoever is
        // already nearest the flag goes — it is a job about being close,
        // not about being good at anything in particular.
        if let Some(p) = Self::take_best(pool, |p| -(p.position - g.flag).magnitude()) {
            out.push(CornerStation {
                player_id: p.id,
                position: g.short_corner_cover(),
                role: CornerRole::ShortCover,
            });
        }

        // Posts. Deliberately the two WORST headers left: a man on the
        // post is a goal-line insurance policy, not a duellist, and
        // spending an aerial specialist on the job is how sides concede
        // the free header in the middle.
        for y in [GOAL_HALF_WIDTH - 4.0, -(GOAL_HALF_WIDTH - 4.0)] {
            if let Some(p) = Self::take_best(pool, |p| -Self::aerial_score(p)) {
                out.push(CornerStation {
                    player_id: p.id,
                    position: g.at(7.0, y),
                    role: CornerRole::PostGuard,
                });
            }
        }

        // The zonal screen across the front of the six-yard box — the
        // three positions the delivery has to beat — and then the
        // penalty-spot band behind it. Best heads first, near post
        // outwards, because the near-post flick is the ball that actually
        // gets attacked.
        let screen = [
            (SIX_YARD_DEPTH - 2.0, 60.0),
            (SIX_YARD_DEPTH - 2.0, 22.0),
            (SIX_YARD_DEPTH - 2.0, -16.0),
            (PENALTY_SPOT_DEPTH - 8.0, 44.0),
            (PENALTY_SPOT_DEPTH - 8.0, -20.0),
        ];
        for (i, (depth, y)) in screen.iter().enumerate() {
            let Some(p) = Self::take_best(pool, |p| Self::aerial_score(p)) else {
                break;
            };
            out.push(CornerStation {
                player_id: p.id,
                position: g.at(*depth, *y),
                role: if i < 3 {
                    CornerRole::ZonalScreen
                } else {
                    CornerRole::Marker
                },
            });
        }

        // Whoever is left holds the edge of the area for the clearance
        // that drops there. More than one man over is a waste on a
        // twenty-two-player pitch, so the loop stops at the first.
        if !pool.is_empty() {
            let p = pool.remove(0);
            out.push(CornerStation {
                player_id: p.id,
                position: g.at(PENALTY_AREA_DEPTH + 13.0, 6.0),
                role: CornerRole::EdgeClearer,
            });
        }
    }

    // ── attacking side ────────────────────────────────────────────────

    /// Two centre-backs into the box (unchanged from the original
    /// push-up), a short option by the flag, three runners on the near
    /// post / spot / back post, and one on the edge. Everyone else — in
    /// practice the full-backs and the holding midfielder — is left where
    /// he is, which is back, covering the counter.
    fn plan_attack(pool: &mut Vec<&MatchPlayer>, g: &CornerGeometry, out: &mut Vec<CornerStation>) {
        // The centre-back push-up, kept exactly as it was: the two best
        // headers of the back line, split either side of the goal line at
        // penalty-spot depth. Their spots are ABSOLUTE (not mirrored on
        // the corner being taken) because `AttackingCorner`'s own hold
        // target is absolute too, and the two have to agree or the CB
        // walks off the spot he was put on.
        let mut centre_backs: Vec<&MatchPlayer> = pool
            .iter()
            .copied()
            .filter(|p| p.tactical_position.current_position.is_central_defender())
            .collect();
        centre_backs.sort_by(|a, b| {
            b.skills
                .technical
                .heading
                .partial_cmp(&a.skills.technical.heading)
                .unwrap_or(Ordering::Equal)
                .then(a.id.cmp(&b.id))
        });
        for (i, cb) in centre_backs.iter().take(2).enumerate() {
            let y = if i == 0 {
                g.centre_y - g.field_height * 0.085
            } else {
                g.centre_y + g.field_height * 0.085
            };
            out.push(CornerStation {
                player_id: cb.id,
                position: Vector3::new(g.at(52.0, 0.0).x, y, 0.0),
                role: CornerRole::BoxAttacker,
            });
            pool.retain(|p| p.id != cb.id);
        }

        // Short option. `pick_corner_routine` can choose `Short` or
        // `EdgeCutback`, and both of them used to be played into an empty
        // quadrant — there was no one near the flag and no one on the edge
        // for the taker to find.
        if let Some(p) = Self::take_best(pool, |p| -(p.position - g.flag).magnitude()) {
            out.push(CornerStation {
                player_id: p.id,
                position: g.short_option(),
                role: CornerRole::ShortOption,
            });
        }

        // Near-post flick, penalty spot, back post — best heads first.
        for (depth, y) in [(30.0, 34.0), (86.0, 4.0), (46.0, -52.0)] {
            let Some(p) = Self::take_best(pool, |p| Self::aerial_score(p)) else {
                break;
            };
            out.push(CornerStation {
                player_id: p.id,
                position: g.at(depth, y),
                role: CornerRole::BoxRunner,
            });
        }

        // One on the edge for the cut-back and the second ball. The best
        // striker of a ball from range, since that is the shot he will get.
        if let Some(p) = Self::take_best(pool, |p| Self::edge_score(p)) {
            out.push(CornerStation {
                player_id: p.id,
                position: g.at(PENALTY_AREA_DEPTH + 20.0, -6.0),
                role: CornerRole::EdgeRunner,
            });
        }
    }

    // ── selection ─────────────────────────────────────────────────────

    /// Remove and return the pool member with the highest `score`. Ties
    /// break on roster order, so the plan is a pure function of the input
    /// slice.
    fn take_best<'a, F>(pool: &mut Vec<&'a MatchPlayer>, score: F) -> Option<&'a MatchPlayer>
    where
        F: Fn(&MatchPlayer) -> f32,
    {
        let mut best: Option<(usize, f32)> = None;
        for (i, p) in pool.iter().enumerate() {
            let s = score(p);
            if best.map_or(true, |(_, bs)| s > bs) {
                best = Some((i, s));
            }
        }
        best.map(|(i, _)| pool.remove(i))
    }

    /// Aerial suitability, on the 0–20 attribute scale. Deliberately the
    /// raw attributes rather than `skill_composites::aerial_outfield_*`:
    /// this only has to rank a squad against itself, and the composite
    /// applies a fatigue band that would make the same eleven produce a
    /// different shape in the 80th minute than in the 10th for no reason
    /// anyone watching could name.
    fn aerial_score(p: &MatchPlayer) -> f32 {
        let s = &p.skills;
        s.technical.heading * 0.45
            + s.physical.jumping * 0.30
            + s.physical.strength * 0.15
            + s.mental.bravery * 0.10
    }

    /// Who gets left up the pitch: a forward who can run onto a clearance
    /// and hold it. Non-forwards are pushed to the back of the queue so a
    /// side with a striker on the pitch always leaves the striker.
    fn outlet_score(p: &MatchPlayer) -> f32 {
        let s = &p.skills;
        let forward_bonus = if p.tactical_position.current_position.is_forward() {
            40.0
        } else {
            0.0
        };
        forward_bonus
            + s.physical.pace * 0.5
            + s.physical.acceleration * 0.3
            + s.physical.strength * 0.2
    }

    /// Who takes the edge of the box on the attacking side — the shot
    /// that comes back out is a half-volley from twenty metres.
    fn edge_score(p: &MatchPlayer) -> f32 {
        let s = &p.skills;
        s.technical.long_shots * 0.5 + s.technical.technique * 0.3 + s.mental.composure * 0.2
    }

    /// Is `position` inside the penalty area of the goal on `goal_x`?
    ///
    /// Lives here so the box-occupancy census, the tests and the shape
    /// itself all agree on what "in the box" means — the census number is
    /// only comparable with the real one (7-9 defending outfielders) if it
    /// counts the same rectangle a television camera does.
    pub fn is_in_penalty_area(position: Vector3<f32>, goal_x: f32, field_height: f32) -> bool {
        (position.x - goal_x).abs() <= PENALTY_AREA_DEPTH
            && (position.y - field_height * 0.5).abs() <= PENALTY_AREA_HALF_WIDTH
    }
}

/// Corner geometry in the frame of the goal being attacked.
///
/// Everything here is expressed as *depth* (units out from the defended
/// goal line, always positive) and *near-side offset* (units from the
/// centre of the goal toward the flag the kick is taken from, so a
/// positive offset is always the near post whichever of the four corners
/// this is). Doing the mirroring once, here, is what keeps the station
/// tables above readable as football rather than as sign arithmetic.
struct CornerGeometry {
    goal_x: f32,
    /// +1 when the defended goal is at x = 0, so "into the pitch" is +x.
    into: f32,
    centre_y: f32,
    /// +1 when the flag is on the high-y touchline.
    near: f32,
    field_height: f32,
    flag: Vector3<f32>,
}

impl CornerGeometry {
    fn new(goal_x: f32, flag: Vector3<f32>, field_width: f32, field_height: f32) -> Self {
        let centre_y = field_height * 0.5;
        CornerGeometry {
            goal_x,
            into: if goal_x <= field_width * 0.5 {
                1.0
            } else {
                -1.0
            },
            centre_y,
            near: if flag.y < centre_y { -1.0 } else { 1.0 },
            field_height,
            flag,
        }
    }

    /// A point `depth` units out from the goal line and `near_offset`
    /// units toward the near post.
    fn at(&self, depth: f32, near_offset: f32) -> Vector3<f32> {
        Vector3::new(
            self.goal_x + self.into * depth,
            (self.centre_y + self.near * near_offset).clamp(6.0, self.field_height - 6.0),
            0.0,
        )
    }

    /// Ten yards from the corner arc, on the line the taker would play
    /// the ball down — which is where the laws put the nearest defender
    /// and, not coincidentally, where he blocks the short corner from.
    fn short_corner_cover(&self) -> Vector3<f32> {
        let spot = self.at(PENALTY_SPOT_DEPTH, 0.0);
        let dir = spot - self.flag;
        let dir = if dir.magnitude() > 0.01 {
            dir.normalize()
        } else {
            Vector3::new(self.into, 0.0, 0.0)
        };
        self.flag + dir * CORNER_RETREAT
    }

    /// The attacking short option: closer in than the defender covering
    /// him, and hard against the touchline rather than on the diagonal,
    /// so the two are not standing on the same blade of grass.
    fn short_option(&self) -> Vector3<f32> {
        Vector3::new(
            self.goal_x + self.into * 60.0,
            self.flag.y - self.near * 18.0,
            0.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::MatchPlayer;
    use crate::{PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills};
    use chrono::NaiveDate;

    const WIDTH: f32 = 840.0;
    const HEIGHT: f32 = 545.0;
    const T442: [PlayerPositionType; 11] = [
        PlayerPositionType::Goalkeeper,
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenter,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardCenter,
        PlayerPositionType::Striker,
    ];

    /// Two XIs to plan a corner for.
    struct CornerFixture;

    impl CornerFixture {
        fn player(
            id: u32,
            side: PlayerSide,
            position: PlayerPositionType,
            at: (f32, f32),
        ) -> MatchPlayer {
            // Skills vary a little with the id so the "best header" and
            // "quickest forward" picks have something to rank.
            let mut skills = PlayerSkills::default();
            let spread = (id % 100) as f32;
            skills.technical.heading = 6.0 + spread;
            skills.physical.jumping = 6.0 + spread;
            skills.physical.pace = 6.0 + spread;
            skills.technical.long_shots = 6.0 + spread;
            MatchPlayer::from_inputs(
                id,
                id / 100,
                [at.0, at.1, 0.0],
                [at.0, at.1, 0.0],
                PersonAttributes::default(),
                PlayerAttributes::default(),
                skills,
                position,
                Some(side),
                Vec::new(),
                NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
                false,
                10000,
                0.0,
                1.0,
                1.0,
                false,
            )
        }

        /// An ordinary XI per side, all of them standing in the middle of
        /// the pitch — the "we just countered and lost it" shape that
        /// produced the empty box in the first place.
        fn squads() -> Vec<MatchPlayer> {
            let mut out = Vec::new();
            for (i, position) in T442.iter().enumerate() {
                out.push(Self::player(
                    100 + i as u32,
                    PlayerSide::Left,
                    *position,
                    (420.0, 200.0),
                ));
                out.push(Self::player(
                    200 + i as u32,
                    PlayerSide::Right,
                    *position,
                    (420.0, 340.0),
                ));
            }
            out
        }

        /// The plan for a corner the right-hand side is taking at the
        /// left-hand goal, from the flag at `flag_y`.
        fn plan(players: &[MatchPlayer], flag_y: f32) -> Vec<CornerStation> {
            CornerShape::plan(
                players,
                PlayerSide::Right,
                210,
                0.0,
                Vector3::new(2.0, flag_y, 0.0),
                WIDTH,
                HEIGHT,
            )
        }
    }

    #[test]
    fn defending_side_fills_its_box() {
        let players = CornerFixture::squads();
        // Right side attacking the left-hand goal, corner off the low-y flag.
        let stations = CornerFixture::plan(&players, 2.0);

        let in_box = stations
            .iter()
            .filter(|s| {
                s.role.is_defensive() && CornerShape::is_in_penalty_area(s.position, 0.0, HEIGHT)
            })
            .count();
        assert!(
            in_box >= 7,
            "a defended corner must have most of the side inside its own area, got {in_box}"
        );

        // …and exactly one man left up the pitch.
        let outlets = stations
            .iter()
            .filter(|s| s.role == CornerRole::Outlet)
            .count();
        assert_eq!(outlets, 1, "exactly one outlet stays up");
    }

    #[test]
    fn every_defender_gets_one_station_and_no_two_share_a_spot() {
        let players = CornerFixture::squads();
        let stations = CornerFixture::plan(&players, HEIGHT - 2.0);

        let defensive: Vec<_> = stations.iter().filter(|s| s.role.is_defensive()).collect();
        assert_eq!(defensive.len(), 10, "ten outfielders, ten stations");

        let mut ids: Vec<u32> = stations.iter().map(|s| s.player_id).collect();
        ids.sort_unstable();
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "nobody is given two jobs");

        for (i, a) in defensive.iter().enumerate() {
            for b in defensive.iter().skip(i + 1) {
                assert!(
                    (a.position - b.position).magnitude() > 8.0,
                    "{:?} and {:?} are standing on each other",
                    a.role,
                    b.role
                );
            }
        }
    }

    #[test]
    fn near_post_mirrors_with_the_flag() {
        let players = CornerFixture::squads();
        let low = CornerFixture::plan(&players, 2.0);
        let high = CornerFixture::plan(&players, HEIGHT - 2.0);
        // The first man of the zonal screen is the near-post one, so he
        // has to be on opposite sides of the goal in the two plans.
        assert!(NearPostScreen::y(&low) < HEIGHT * 0.5);
        assert!(NearPostScreen::y(&high) > HEIGHT * 0.5);
    }

    /// Where the near-post man of the zonal screen ended up.
    struct NearPostScreen;

    impl NearPostScreen {
        fn y(plan: &[CornerStation]) -> f32 {
            plan.iter()
                .find(|s| s.role == CornerRole::ZonalScreen)
                .map(|s| s.position.y)
                .expect("a corner is screened at the near post")
        }
    }

    #[test]
    fn short_corner_cover_stands_ten_yards_off_the_flag() {
        let players = CornerFixture::squads();
        let flag = Vector3::new(2.0, 2.0, 0.0);
        let stations = CornerFixture::plan(&players, flag.y);
        let cover = stations
            .iter()
            .find(|s| s.role == CornerRole::ShortCover)
            .expect("a corner is covered short");
        let distance = (cover.position - flag).magnitude();
        assert!(
            (distance - CORNER_RETREAT).abs() < 1.0,
            "cover stood {distance}u from the flag, the laws say {CORNER_RETREAT}u"
        );
    }

    #[test]
    fn attacking_side_loads_the_box_but_leaves_cover() {
        let players = CornerFixture::squads();
        let stations = CornerFixture::plan(&players, 2.0);
        let attacking: Vec<_> = stations.iter().filter(|s| !s.role.is_defensive()).collect();
        let in_box = attacking
            .iter()
            .filter(|s| CornerShape::is_in_penalty_area(s.position, 0.0, HEIGHT))
            .count();
        assert!(
            (4..=6).contains(&in_box),
            "attacking box load should be a corner, not an evacuation: {in_box}"
        );
        // Taker + the stations above must still leave men back.
        assert!(
            attacking.len() <= 7,
            "too much of the side committed: {}",
            attacking.len()
        );
    }

    #[test]
    fn a_short_handed_side_still_gets_a_coherent_shape() {
        let mut players = CornerFixture::squads();
        // Two sent off.
        for p in players.iter_mut().filter(|p| p.id == 101 || p.id == 105) {
            p.is_sent_off = true;
        }
        let stations = CornerFixture::plan(&players, 2.0);
        let defensive = stations.iter().filter(|s| s.role.is_defensive()).count();
        assert_eq!(defensive, 8, "eight left, eight stations");
        // The highest-priority jobs survive; the edge is what gets dropped.
        assert!(
            stations.iter().any(|s| s.role == CornerRole::ZonalScreen),
            "the screen in front of goal is never the thing that gets cut"
        );
    }

    #[test]
    fn the_outlet_holds_the_last_line_rather_than_the_halfway_line() {
        // Attacking side leaving its cover deep in its own half: the man
        // left up must drop with it or he is offside the instant the
        // clearance is played.
        let mut players = CornerFixture::squads();
        for p in players
            .iter_mut()
            .filter(|p| p.side == Some(PlayerSide::Right))
        {
            p.position = Vector3::new(300.0, 272.0, 0.0);
        }
        let outlet_x = OutletStation::x(&CornerFixture::plan(&players, 2.0));
        assert!(
            (outlet_x - 288.0).abs() < 1.0,
            "outlet stood at {outlet_x}, a stride onside of 300 is 288"
        );

        // …and with the cover camped on its own goal line he still stops
        // at halfway rather than following them into the other half.
        for p in players
            .iter_mut()
            .filter(|p| p.side == Some(PlayerSide::Right))
        {
            p.position = Vector3::new(800.0, 272.0, 0.0);
        }
        let outlet_x = OutletStation::x(&CornerFixture::plan(&players, 2.0));
        assert!(
            (outlet_x - OUTLET_MAX_DEPTH).abs() < 1.0,
            "outlet wandered to {outlet_x}, past the halfway line at {OUTLET_MAX_DEPTH}"
        );
    }

    /// Where the man left up the pitch ended up.
    struct OutletStation;

    impl OutletStation {
        fn x(plan: &[CornerStation]) -> f32 {
            plan.iter()
                .find(|s| s.role == CornerRole::Outlet)
                .map(|s| s.position.x)
                .expect("a side defending a corner leaves one man up")
        }
    }
}
