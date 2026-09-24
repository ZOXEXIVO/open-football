//! Stable home-field assignments, shared by the server and review harness.

/// Version 1 maps immutable home team IDs onto the approved field IDs 1..=20.
/// Keep both the mixer and this fixed catalogue size unchanged: adding styles
/// later must not reshuffle existing teams. This is a pure mapping, so it also
/// applies to existing saves without a migration or process-random hash seed.
pub const fn for_home_team(team_id: u32) -> u8 {
    // SplitMix64's finalizer, with a fixed domain seed for home surfaces.
    let mut value = (team_id as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    (value % 20) as u8 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_one_assignments_never_change() {
        // Saved-world compatibility fixtures, including the full u32 ID range.
        for (team, style) in [
            (0, 16),
            (1, 6),
            (2, 11),
            (10, 7),
            (42, 14),
            (100, 5),
            (1000, 17),
            (123456, 18),
            (2_000_000_000, 14),
            (u32::MAX, 1),
        ] {
            assert_eq!(for_home_team(team), style, "home team {team}");
        }
    }

    #[test]
    fn all_twenty_fields_are_assigned_across_teams() {
        let mut counts = [0; 20];
        for team in 1..=10_000 {
            counts[for_home_team(team) as usize - 1] += 1;
        }
        assert!(counts.iter().all(|count| (400..=600).contains(count)));
    }
}
