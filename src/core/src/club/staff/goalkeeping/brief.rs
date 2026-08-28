//! What the manager carries into a matchday from the goalkeeping room.
//!
//! The plan itself is a monthly document about four squads. The team sheet
//! needs four names and one number: who is the number one, who is the
//! deputy, who is third, who has the department asked him to play — and how
//! much notice he takes of the man asking.
//!
//! Keeping the matchday read this small matters. It is copied per fixture,
//! it must not borrow the club while the selector runs, and every magnitude
//! that can move a team sheet lives here in one place rather than scattered
//! through the scoring engine.

use chrono::NaiveDate;

use super::plan::KeeperRoomPlan;

/// The goalkeeping department's word, as the selector hears it.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeeperSelectionBrief {
    /// The declared number one.
    pub number_one: Option<u32>,
    /// The deputy — the man who goes in without the team dropping.
    pub deputy: Option<u32>,
    /// Third choice.
    pub third: Option<u32>,
    /// The keeper the department has asked the manager to start, if that
    /// request still stands today. Deliberately allowed to be a keeper who
    /// is not the best at the club.
    pub nominated: Option<u32>,
    /// 0..1 — how far the manager acts on the department's word.
    pub authority: f32,
}

impl KeeperSelectionBrief {
    /// Standing of the declared number one, before the manager's deference
    /// scales it.
    ///
    /// What matters is the NET figure against the deputy, since he is the
    /// only man who can realistically take the shirt: `NUMBER_ONE_STANDING -
    /// DEPUTY_STANDING`, which is 2.8. `goalkeeper_score` moves about three
    /// points per ten of assessed ability, so that is some nine ability
    /// points of stickiness at full authority — deliberately a little more
    /// than the department's own `USURP_MARGIN`, so the order changes
    /// because the department changed it at a review and never because a
    /// deputy turned up a point fresher on a Saturday. That is how keepers
    /// are actually handled, and it is the whole reason for declaring an
    /// order at all.
    const NUMBER_ONE_STANDING: f32 = 4.0;

    /// The deputy's standing over the rest of the room. It only has to
    /// settle who goes in when the number one cannot, so the club does not
    /// field its fourth keeper because he happens to be fresh.
    const DEPUTY_STANDING: f32 = 1.2;
    const THIRD_STANDING: f32 = 0.4;

    /// The nomination.
    ///
    /// Not a nudge — a decision, in the same sense as the cup deputy's
    /// designation. A goalkeeping coach who says a boy is ready is asking
    /// for him to be picked ahead of a better keeper, because that is the
    /// only way a young keeper is ever made, and an amount that merely
    /// competes with the ability gap would never actually get anybody
    /// played: the point of a nomination is that it is not the marginal
    /// call the argmax was already making.
    ///
    /// At full authority it is worth around seventy ability points, and
    /// that is the right order of magnitude, because the bound on who plays
    /// is not this number. It is the department's own gate: a nominated
    /// keeper has cleared the readiness band against the club's LAST senior
    /// keeper, is at least eighteen, is not injured, is starved of senior
    /// minutes, and is the only keeper nominated at the club. What is left
    /// for the magnitude to decide is whether the manager acts on all that
    /// — so it scales with his deference, and a department he half-listens
    /// to gets its boy a game only when the gap is modest anyway.
    const NOMINATION_PULL: f32 = 22.0;

    /// Fixture importance at or below which the club can afford to hand a
    /// prospect the gloves, and the importance by which it can no longer
    /// afford it at all.
    const STAKES_FREE: f32 = 0.35;
    const STAKES_CLOSED: f32 = 0.62;

    /// Build the matchday read from the standing plan.
    pub fn from_plan(plan: &KeeperRoomPlan, today: NaiveDate) -> Self {
        KeeperSelectionBrief {
            number_one: plan.number_one(),
            deputy: plan.deputy(),
            third: plan.third(),
            nominated: plan.nominated(today),
            authority: plan.authority(),
        }
    }

    /// True when the department has nothing to say — the brief then adds
    /// nothing anywhere and selection behaves exactly as it did before the
    /// department existed.
    pub fn is_silent(&self) -> bool {
        self.number_one.is_none() && self.nominated.is_none()
    }

    /// How much room the fixture leaves for developing a keeper. Full at a
    /// dead rubber or an early cup tie, nothing once the result matters.
    fn stakes_room(match_importance: f32) -> f32 {
        if match_importance <= Self::STAKES_FREE {
            return 1.0;
        }
        if match_importance >= Self::STAKES_CLOSED {
            return 0.0;
        }
        1.0 - (match_importance - Self::STAKES_FREE) / (Self::STAKES_CLOSED - Self::STAKES_FREE)
    }

    /// The department's contribution to one keeper's selection score.
    ///
    /// Additive and signed, like every other term the selector folds in. A
    /// silent brief, or a keeper nobody has an opinion about, returns zero.
    pub fn selection_adjustment(&self, player_id: u32, match_importance: f32) -> f32 {
        let authority = self.authority.clamp(0.0, 1.0);
        if authority <= f32::EPSILON {
            return 0.0;
        }

        // How far this fixture belongs to the boy. Zero once the result
        // matters, and then the room is the number one's as usual.
        let room = if self.nominated.is_some() {
            Self::stakes_room(match_importance)
        } else {
            0.0
        };

        if self.nominated == Some(player_id) && room > 0.0 {
            return Self::NOMINATION_PULL * authority * room;
        }

        if self.number_one == Some(player_id) {
            // A live nomination is a request to leave the number one out of
            // THIS game, so his standing gives way as the fixture becomes
            // the one the club has set aside — and comes straight back the
            // moment the stakes rise. Without this the two halves of the
            // department's own advice would compete with each other.
            return Self::NUMBER_ONE_STANDING * authority * (1.0 - room);
        }
        if self.deputy == Some(player_id) {
            return Self::DEPUTY_STANDING * authority;
        }
        if self.third == Some(player_id) {
            return Self::THIRD_STANDING * authority;
        }
        0.0
    }
}
