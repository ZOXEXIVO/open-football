//! Goalkeeper-only timing derived from the recording; no simulation changes.
use super::*;

impl Actors {
    /// Load the legs in the last 150 ms before a recorded take-off. Looking
    /// beyond that point rejects the tiny split-step hops. Never extrapolate
    /// across missing chunks or a restart teleport.
    pub(super) fn keeper_coil(track: &mut Track, now: f64) -> f32 {
        let Some(here) = track.position_ahead(now) else { return 0.0 };
        if here[2] > Self::AIRBORNE_FEET {
            return 0.0;
        }
        for step in 1..=5 {
            let delay = step as f64 * 30.0;
            let Some(next) = track.position_ahead(now + delay) else { return 0.0 };
            if next[2] <= Self::AIRBORNE_FEET {
                continue;
            }
            let Some(up) = track.position_ahead(now + delay + 90.0) else { return 0.0 };
            let distance = Vec2::new(next[0] - here[0], next[1] - here[1]).length()
                * Field::METERS_PER_UNIT;
            if up[2] < Self::HOP_CEILING || distance > Self::TELEPORT * delay as f32 * 0.001 {
                return 0.0;
            }
            return 0.65 * Self::ease(1.0 - delay as f32 / 180.0);
        }
        0.0
    }
}

impl PlayerActor {
    /// Ballistic progress: knees drive up during ascent, then unfold before
    /// touchdown. The height gate handles short, low flights as well.
    pub(super) fn jump_progress(&self) -> f32 {
        if self.vertical_speed < 0.0 {
            0.5 + 0.5 * Actors::ease(1.0 - self.height / 0.28)
        } else if self.climb > 0.1 {
            0.5 * (1.0 - self.vertical_speed / self.climb).clamp(0.0, 1.0)
        } else {
            0.5
        }
    }

    /// A short give in the elbows and chest after contact. Suppressed once
    /// another action owns the hands. Never anticipates impact.
    pub(super) fn save_recoil(&self) -> f32 {
        let Some(contact) = self.save_time else { return 0.0 };
        let since = self.clock - contact;
        if !(0.0..0.32).contains(&since) {
            return 0.0;
        }
        let pulse = Actors::ease(since / 0.07) * (1.0 - Actors::ease((since - 0.07) / 0.25));
        pulse * self.reaction * (1.0 - self.carry)
            * (1.0 - self.despair.max(self.elation))
    }

    /// Disbelief, a held gesture, then arms dropping as he exhales. The
    /// individual reaction stays the same; its timing is no longer a statue.
    pub(super) fn keeper_gesture(&self) -> f32 {
        let Some(since) = self.goal_since.filter(|_| self.is_goalkeeper) else { return 1.0 };
        let delay = 0.25 + 0.15 * Complexion::carriage(self.id);
        Actors::ease((since - delay) / 0.7)
            * (1.0 - Actors::ease((since - 3.8 - delay) / 2.0))
    }

    /// Release the kneeling hold before the restart, even when the engine
    /// leaves his position unchanged through the celebration.
    pub(super) fn keeper_grief(&self) -> f32 {
        self.goal_since.map_or(1.0, |since| 1.0 - Actors::ease((since - 4.0) / 2.5))
    }
}
