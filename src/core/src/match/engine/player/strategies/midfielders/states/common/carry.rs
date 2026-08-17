use crate::r#match::StateProcessingContext;
use crate::r#match::midfielders::states::common::Opportunity;
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;

/// Engine units per metre. The pitch is 840u × 545u, i.e. 105 m × 68 m,
/// so 1u = 0.125 m. Everything below is written in metres and converted
/// here, because the whole reason the carry logic did nothing was that
/// its radii were written as bare unit counts that read like metres:
/// "no opponent within 30" looks like thirty metres of space and is
/// actually three and three-quarters.
pub const U_PER_M: f32 = 8.0;

/// How far up the pitch the carrier looks for the man in his way.
/// Beyond this it is not a decision he is making now.
const LANE_SCAN: f32 = 26.0 * U_PER_M;

/// Half-width of the running channel at the carrier's own feet, and how
/// fast it flares with depth. A channel, not a ray: a defender a metre
/// to the side at four metres is in the way, one twelve metres to the
/// side at four metres is not, and at twenty metres ahead the channel
/// he could still cut off is much wider than at two.
const LANE_HALF_WIDTH: f32 = 1.6 * U_PER_M;
const LANE_FLARE: f32 = 0.22;

/// Running room at which the lane counts as fully open. Eighteen metres
/// is a genuine driving run — the picture where a midfielder puts his
/// head down and goes.
const RUNNING_ROOM: f32 = 18.0 * U_PER_M;

/// How far behind the man in front his cover has to be to count as part
/// of the same duel. Beyond this he is a separate problem for later,
/// not a reason to decline this one.
const COVER_DEPTH: f32 = 7.0 * U_PER_M;

/// Where the carrier's field of view for the take-on starts and stops.
/// A man already at your feet is a shielding problem, not a take-on; a
/// man fifteen metres away is not yet a decision. The commitment builds
/// between them.
const ENGAGE_NEAR_M: f32 = 1.2;
const ENGAGE_FULL_M: f32 = 3.5;
const ENGAGE_FADE_M: f32 = 8.0;
const ENGAGE_GONE_M: f32 = 17.0;

/// Bar the take-on appetite is compared against. Drawn once per
/// (possession, defender) rather than per tick — see [`TakeOn::decide`].
const TAKE_ON_BAR_BASE: f32 = 0.30;
const TAKE_ON_BAR_SPREAD: f32 = 0.34;
const TAKE_ON_SALT: u64 = 0xD6E8_FEB8_6659_FD93;

/// What the carrier can see in front of him.
///
/// One continuous read replacing three binary predicates that each drew
/// their own arbitrary circle: `has_open_space_ahead` (no opponent
/// within 3.75 m), `has_running_lane` (none within 10 m in a 40° cone)
/// and the take-on gate's own 4.4 m count. Because they disagreed, the
/// window in which the engine would let a midfielder try to beat a man
/// was the gap between two of them — an annulus 0.6 m wide. Measured
/// over 60 matches, 91% of every on-ball tick exited at "carry on
/// running" and only 1.3% ever reached the take-on question at all.
#[derive(Debug, Clone, Copy)]
pub struct LaneAhead {
    /// Distance along the line to goal of the nearest opponent standing
    /// in the running channel, in engine units. `None` when the channel
    /// is empty out to [`LANE_SCAN`].
    pub nearest: Option<f32>,
    /// Id of that opponent — the man to beat.
    pub nearest_id: Option<u32>,
    /// How square-on he is: 1.0 when he is planted directly in the
    /// channel, falling to 0 at its edge. A defender drifting across
    /// the outside of the channel is less of an obstacle than one stood
    /// in the middle of it.
    pub nearest_centrality: f32,
    /// Opponents around the man in front — him plus whoever is close
    /// enough behind him to cover. This is the difference between a duel
    /// and dribbling into a crowd.
    ///
    /// It counted the WHOLE channel out to [`LANE_SCAN`] and measured
    /// 3-or-more on 59% of ticks, because a midfielder in his own half
    /// has the entire opposition between him and the goal. That is not
    /// a crowd to dribble into; it is a pitch.
    pub occupancy: usize,
    /// 0..1 running room: 0 when a defender is on top of him, 1 when
    /// there is [`RUNNING_ROOM`] of clear grass. This is the continuous
    /// quantity the old booleans were quantising.
    pub openness: f32,
}

impl LaneAhead {
    /// Read the channel between the carrier and the goal he is
    /// attacking.
    pub fn read(ctx: &StateProcessingContext) -> Self {
        let from = ctx.player.position;
        let goal = ctx.player().opponent_goal_position();
        let to_goal = goal - from;
        let len = to_goal.magnitude();
        if len < f32::EPSILON {
            return Self::open();
        }
        let to_goal = to_goal / len;

        let mut nearest = f32::INFINITY;
        let mut nearest_id = None;
        let mut nearest_centrality = 0.0f32;
        // Depths of everyone in the channel, so the cover around the man
        // in front can be counted once `nearest` is known. A fixed array
        // rather than a Vec — this runs on every carrier tick and the
        // hot path is allocation-free by design (see `match_engine_perf`);
        // a side can only field eleven, so it cannot overflow.
        let mut in_lane = [0.0f32; 11];
        let mut in_lane_len = 0usize;

        for opp in ctx.players().opponents().nearby(LANE_SCAN) {
            let offset = opp.position - from;
            let along = offset.dot(&to_goal);
            if along <= 0.0 || along > LANE_SCAN {
                continue;
            }
            let lateral = (offset - to_goal * along).magnitude();
            let half_width = LANE_HALF_WIDTH + along * LANE_FLARE;
            if lateral > half_width {
                continue;
            }
            if in_lane_len < in_lane.len() {
                in_lane[in_lane_len] = along;
                in_lane_len += 1;
            }
            if along < nearest {
                nearest = along;
                nearest_id = Some(opp.id);
                nearest_centrality = 1.0 - (lateral / half_width).clamp(0.0, 1.0);
            }
        }

        if nearest_id.is_none() {
            return Self::open();
        }

        let cover_edge = nearest + COVER_DEPTH;
        let occupancy = in_lane[..in_lane_len]
            .iter()
            .filter(|along| **along <= cover_edge)
            .count();

        LaneAhead {
            nearest: Some(nearest),
            nearest_id,
            nearest_centrality,
            occupancy,
            openness: (nearest / RUNNING_ROOM).clamp(0.0, 1.0),
        }
    }

    fn open() -> Self {
        LaneAhead {
            nearest: None,
            nearest_id: None,
            nearest_centrality: 0.0,
            occupancy: 0,
            openness: 1.0,
        }
    }

    /// Is there anybody at all between him and the goal he is running
    /// at? Not a decision — the decision is [`TakeOn::decide`].
    #[inline]
    pub fn has_man_to_beat(&self) -> bool {
        self.nearest_id.is_some()
    }

    /// Distance in metres to the man in the way, for the readable form
    /// of the engagement curve.
    #[inline]
    pub fn nearest_m(&self) -> f32 {
        self.nearest.map(|d| d / U_PER_M).unwrap_or(f32::INFINITY)
    }
}

/// Whether the carrier goes at the man in front of him.
///
/// The thing this replaces was three booleans hanging off one composite
/// score — carry into space at 0.32, beat one man at 0.40, beat two at
/// 0.58. A uniform 10/20 midfielder scores 0.39, so the league-average
/// central midfielder was on the wrong side of the take-on line by a
/// hundredth and never attempted one in his career, while an 11/20
/// attempted every single one that geometry allowed. That is the shape
/// of a script, not of a footballer: the same player takes a man on in
/// one moment and gives it in the next, and which he does depends on
/// where he is, how much room he has to run at him, who else is around,
/// and what kind of player he is.
pub struct TakeOn;

impl TakeOn {
    /// Appetite for beating the man in front, 0..1-ish. Continuous in
    /// every input, so there is no line to sit on the wrong side of.
    pub fn appetite(
        ctx: &StateProcessingContext,
        lane: &LaneAhead,
        profile: &MidfielderSkillProfile,
    ) -> f32 {
        if !lane.has_man_to_beat() {
            return 0.0;
        }

        // ── Room to run at him ────────────────────────────────────────
        // You beat a man by attacking the space in front of him, so the
        // urge builds as he comes into range and fades once he is close
        // enough that the ball is already in the tackle.
        let d = lane.nearest_m();
        let closing = ((d - ENGAGE_NEAR_M) / (ENGAGE_FULL_M - ENGAGE_NEAR_M)).clamp(0.0, 1.0);
        let fading = 1.0 - ((d - ENGAGE_FADE_M) / (ENGAGE_GONE_M - ENGAGE_FADE_M)).clamp(0.0, 1.0);
        let engagement = closing * fading * (0.55 + lane.nearest_centrality * 0.45);

        // ── How many of them ──────────────────────────────────────────
        // One man is a duel. Two is a risk a good dribbler still takes.
        // Three is a crowd, and only a flair player goes into it — which
        // is why this tails off instead of stopping.
        let crowd = 1.0 / (1.0 + 0.85 * (lane.occupancy.saturating_sub(1)) as f32);

        // ── Where on the pitch ────────────────────────────────────────
        // Losing it thirty metres from your own goal is a chance
        // against; losing it thirty metres from theirs is a throw-in.
        // Territory is the whole of the risk model — there is no
        // "final third only" fence anywhere in this decision.
        let width = ctx.context.field_size.width as f32;
        let progress = 1.0 - (ctx.ball().distance_to_opponent_goal() / width).clamp(0.0, 1.0);
        let territory = 0.42 + progress * 0.58;

        // ── What kind of player ───────────────────────────────────────
        // Flair and bravery decide who backs himself; a tired player
        // stops trying. Narrow band — temperament colours the decision,
        // it does not make it.
        let flair = (ctx.player.skills.mental.flair / 20.0).clamp(0.0, 1.0);
        let bravery = (ctx.player.skills.mental.bravery / 20.0).clamp(0.0, 1.0);
        let temperament = 0.82 + flair * 0.26 + bravery * 0.10;

        // `carry_selection` is a weighted mean of eight curved skills, so
        // it clusters hard in the middle of its range; the root spreads
        // it back out over the population without changing the order.
        let craft = profile.carry_selection.clamp(0.0, 1.0).powf(0.80);

        craft * engagement * crowd * territory * temperament * profile.mid_condition_mult
    }

    /// Does he go?
    ///
    /// The bar is drawn once per (possession, man in front) — the same
    /// device the shot decision uses, and for the same reason. Asking a
    /// fresh random question every tick makes holding the ball a way of
    /// buying more chances to succeed, and it makes the answer flicker
    /// while the situation has not changed. Hashing the possession
    /// means he either fancies this one or he does not; hashing the
    /// defender means that when the next man steps across, it is a new
    /// question.
    pub fn decide(
        ctx: &StateProcessingContext,
        lane: &LaneAhead,
        profile: &MidfielderSkillProfile,
    ) -> bool {
        let Some(defender_id) = lane.nearest_id else {
            return false;
        };
        let appetite = Self::appetite(ctx, lane, profile);
        if appetite <= 0.0 {
            return false;
        }

        let spread = Opportunity::draw_vs(ctx, TAKE_ON_SALT, defender_id);
        appetite >= TAKE_ON_BAR_BASE + spread * TAKE_ON_BAR_SPREAD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_scan_covers_a_real_midfield_sight_line() {
        // The bug this whole module exists to fix: the old reads topped
        // out at 4.4 m, so the man a carrier actually runs at was never
        // visible to the decision.
        assert!(LANE_SCAN / U_PER_M >= 20.0);
        assert!(RUNNING_ROOM / U_PER_M >= 12.0);
    }

    #[test]
    fn channel_flares_but_stays_a_channel() {
        // At the feet it is about a body's width either side; twenty
        // metres up the pitch it is a lane, not the whole half.
        let near = LANE_HALF_WIDTH / U_PER_M;
        let far = (LANE_HALF_WIDTH + 20.0 * U_PER_M * LANE_FLARE) / U_PER_M;
        assert!((1.0..3.0).contains(&near), "near half-width {near} m");
        assert!((4.0..9.0).contains(&far), "far half-width {far} m");
    }
}
