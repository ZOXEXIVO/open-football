use core::Player;
use core::Staff;

/// Star-rating projector for the web crate. Absolute scale only —
/// CA stars come from `current_ability` / 200 and PA stars from
/// `potential_ability` / 200, with a staff-judging noise on the
/// potential read when an observer is available. No club-relative
/// baselines: the same player always reads the same stars regardless
/// of which squad you're viewing them in.
pub struct PotentialStarsView;

impl PotentialStarsView {
    /// Stars (0..5) from the player's absolute current ability.
    pub fn current(player: &Player) -> u8 {
        (5.0 * (player.player_attributes.current_ability as f32 / 200.0))
            .round()
            .clamp(0.0, 5.0) as u8
    }

    /// Staff-noisy potential stars on the absolute 1..200 PA scale.
    /// Floor at the player's current-ability star count — staff don't
    /// tell you the ceiling is below where the player already plays.
    pub fn potential_by_staff(player: &Player, staff: &Staff) -> u8 {
        let staff_judging = staff.staff_attributes.knowledge.judging_player_potential;
        let raw_stars = 5.0 * (player.player_attributes.potential_ability as f32 / 200.0);
        let accuracy = (staff_judging as f32 / 20.0).clamp(0.0, 1.0);
        let noise_scale = (1.0 - accuracy) * 1.5;

        let hash = staff
            .id
            .wrapping_mul(2654435761)
            .wrapping_add(player.id.wrapping_mul(2246822519));
        let hash = hash ^ (hash >> 16);
        let hash = hash.wrapping_mul(0x45d9f3b);
        let hash = hash ^ (hash >> 16);
        let noise = (hash & 0xFFFF) as f32 / 32768.0 - 1.0;

        let stars = (raw_stars + noise * noise_scale).round().clamp(0.0, 5.0) as u8;
        stars.max(Self::current(player))
    }

    /// Absolute potential stars without a staff observer — used for
    /// free-agent and retired views where no employing club exists.
    /// Floored at the current-ability star so a senior player never
    /// reads as having a lower ceiling than their present level.
    pub fn potential_absolute(player: &Player) -> u8 {
        let raw = (5.0 * (player.player_attributes.potential_ability as f32 / 200.0))
            .round()
            .clamp(0.0, 5.0) as u8;
        raw.max(Self::current(player))
    }
}
