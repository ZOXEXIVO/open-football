//! A senior team-mate's mentoring reaches a player.

use crate::club::player::player::Player;

/// What a mentor brings to one spell of mentoring: the values he holds, and
/// how far a spell moves his mentee toward them.
#[derive(Debug, Clone, Copy)]
pub struct Mentoring {
    pub professionalism: f32,
    pub ambition: f32,
    pub determination: f32,
    pub step: f32,
}

impl Player {
    /// A spell of mentoring by a senior team-mate. Squad life states who
    /// mentored him and how well they fit; his character and determination
    /// drift toward the mentor's by the step it brought.
    pub fn on_mentored(&mut self, mentoring: &Mentoring) {
        let drift = |current: f32, target: f32| -> f32 {
            let delta = target - current;
            if delta.abs() < 0.05 {
                0.0
            } else {
                delta.signum() * mentoring.step
            }
        };

        let cur = &mut self.attributes;
        cur.professionalism = (cur.professionalism
            + drift(cur.professionalism, mentoring.professionalism))
        .clamp(0.0, 20.0);
        cur.ambition = (cur.ambition + drift(cur.ambition, mentoring.ambition)).clamp(0.0, 20.0);

        let det_now = self.skills.mental.determination;
        let det_new = det_now + drift(det_now, mentoring.determination);
        self.skills.mental.determination = det_new.clamp(0.0, 20.0);
    }
}
