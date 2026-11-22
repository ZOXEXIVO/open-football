use chrono::NaiveDate;

/// Deterministic perception noise shared by the coaching staff model. Kept
/// on one type so the noise, roll and week arithmetic read as one API.
pub struct PerceptionNoise;

impl PerceptionNoise {
    pub fn raw(coach_seed: u32, player_id: u32, salt: u32) -> f32 {
        let hash = coach_seed
            .wrapping_mul(2654435761)
            .wrapping_add(player_id.wrapping_mul(2246822519))
            .wrapping_add(salt.wrapping_mul(3266489917));
        let hash = hash ^ (hash >> 16);
        let hash = hash.wrapping_mul(0x45d9f3b);
        let hash = hash ^ (hash >> 16);
        (hash & 0xFFFF) as f32 / 32768.0 - 1.0
    }

    pub fn seeded_decision(probability: f32, seed: u32) -> bool {
        if probability >= 1.0 {
            return true;
        }
        if probability <= 0.0 {
            return false;
        }
        let hash = seed.wrapping_mul(2654435761).wrapping_add(0xdeadbeef);
        let hash = hash ^ (hash >> 16);
        let hash = hash.wrapping_mul(0x45d9f3b);
        let hash = hash ^ (hash >> 16);
        let roll = (hash & 0xFFFF) as f32 / 65536.0;
        roll < probability
    }

    pub fn week(date: NaiveDate) -> u32 {
        let epoch = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        let days = date.signed_duration_since(epoch).num_days();
        (days / 7).max(0) as u32
    }
}
