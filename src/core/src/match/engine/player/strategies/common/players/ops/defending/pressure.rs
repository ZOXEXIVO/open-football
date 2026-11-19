use crate::r#match::{MatchPlayer, MatchPlayerLite, StateProcessingContext};
use std::env::var;
use std::sync::OnceLock;

/// Operations for assessing pressure situations on the field
pub struct PressureOperationsImpl<'p> {
    ctx: &'p StateProcessingContext<'p>,
}

impl<'p> PressureOperationsImpl<'p> {
    pub fn new(ctx: &'p StateProcessingContext<'p>) -> Self {
        PressureOperationsImpl { ctx }
    }

    /// Check if player is under immediate pressure (at least one opponent within 1m)
    pub fn is_under_immediate_pressure(&self) -> bool {
        self.is_under_immediate_pressure_with_distance(5.0)
    }

    /// Check if player is under immediate pressure with custom distance
    pub fn is_under_immediate_pressure_with_distance(&self, distance: f32) -> bool {
        self.ctx.players().opponents().exists(distance)
    }

    /// Check if player is under heavy pressure (multiple opponents within 1m)
    pub fn is_under_heavy_pressure(&self) -> bool {
        self.is_under_heavy_pressure_with_params(3.0, 2)
    }

    /// Check if player is under heavy pressure with custom parameters
    pub fn is_under_heavy_pressure_with_params(&self, distance: f32, threshold: usize) -> bool {
        self.pressing_opponents_count(distance) >= threshold
    }

    /// Count pressing opponents within distance
    pub fn pressing_opponents_count(&self, distance: f32) -> usize {
        self.ctx.players().opponents().nearby(distance).count()
    }

    /// Check if a teammate is marked by opponents
    pub fn is_teammate_marked(&self, teammate: &MatchPlayerLite, marking_distance: f32) -> bool {
        // Use pre-computed distances: opponents of teammate = our players near them,
        // but we need opponents near teammate, so from teammate's POV our team are opponents
        self.ctx
            .tick_context
            .grid
            .opponents(teammate.id, marking_distance)
            .count()
            >= 1
    }

    /// Check if a teammate is heavily marked (multiple opponents or very close marking)
    pub fn is_teammate_heavily_marked(&self, teammate: &MatchPlayerLite) -> bool {
        // Single scan at max distance, bucket by distance
        let mut markers = 0;
        let mut close_markers = 0;
        for (_id, dist) in self.ctx.tick_context.grid.opponents(teammate.id, 8.0) {
            markers += 1;
            if dist <= 3.0 {
                close_markers += 1;
            }
        }

        markers >= 2 || (markers >= 1 && close_markers > 0)
    }

    /// Get the closest pressing opponent
    pub fn closest_pressing_opponent(&self, max_distance: f32) -> Option<MatchPlayerLite> {
        self.ctx
            .players()
            .opponents()
            .nearby(max_distance)
            .min_by(|a, b| {
                let dist_a = a.distance(self.ctx);
                let dist_b = b.distance(self.ctx);
                dist_a.total_cmp(&dist_b)
            })
    }

    /// Calculate pressure intensity (0.0 = no pressure, 1.0 = extreme pressure)
    pub fn pressure_intensity(&self) -> f32 {
        // Single scan at max distance, bucket by distance
        let mut close: f32 = 0.0;
        let mut medium: f32 = 0.0;
        let mut far: f32 = 0.0;
        for (_id, dist) in self
            .ctx
            .tick_context
            .grid
            .opponents(self.ctx.player.id, 30.0)
        {
            far += 1.0;
            if dist <= 20.0 {
                medium += 1.0;
            }
            if dist <= 10.0 {
                close += 1.0;
            }
        }

        // Weight closer opponents more heavily
        let intensity = (close * 0.5 + medium * 0.3 + far * 0.2) / 3.0;
        intensity.min(1.0)
    }

    /// Check if there's space around the player (inverse of pressure)
    pub fn has_space_around(&self, min_distance: f32) -> bool {
        !self.ctx.players().opponents().exists(min_distance)
    }
}

/// **Does he press because his team is pressing?**
///
/// `work_rate` says how much a player runs. `teamwork` says whether he
/// runs WITH them, and the counter-press is the one moment in football
/// where that distinction is the whole action: a side either arrives
/// together in the two seconds after it loses the ball, or one man
/// sprints at the carrier while the other ten walk.
///
/// Deliberately an INTERACTION with the team's own press call rather
/// than a fourth additive weight beside `work_rate`. An additive term is
/// how `teamwork` came to be live in nine composites at 0.07-0.14 and
/// measure at nothing: pinned 6 against 18 across a whole side it moved
/// the goal differential by **+0.18**, inside the noise floor and the
/// wrong way round. Multiplying by the team's intensity means the
/// attribute only bites when there IS a unit to join, which is what the
/// word means.
pub struct PressUnit;

impl PressUnit {
    /// Widest the attribute may swing an individual's counter-press
    /// appetite, at full team press intensity. ±20% at the extremes of
    /// the attribute; nothing at all when the team is not pressing.
    const BAND: f32 = 0.40;

    /// Compliance multiplier for `player` when his side presses at
    /// `press_intensity`, centred on 1.0 at the population. At zero there
    /// is no unit and the attribute is silent.
    pub fn compliance(player: &MatchPlayer, press_intensity: f32) -> f32 {
        if !Self::armed() {
            return 1.0;
        }
        let unit = (player.skills.mental.teamwork / 20.0).clamp(0.0, 1.0);
        let intensity = press_intensity.clamp(0.0, 1.0);
        (1.0 + (unit - 0.5) * intensity * Self::BAND).clamp(0.80, 1.20)
    }

    /// `OF_TEAMWORK_PRESS_OFF=1` removes the compliance term — the
    /// pre-2026-08-26 engine. The A/B control for the channel.
    #[inline]
    pub fn armed() -> bool {
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| {
            !var("OF_TEAMWORK_PRESS_OFF")
                .map(|v| v != "0" && !v.is_empty())
                .unwrap_or(false)
        })
    }
}
