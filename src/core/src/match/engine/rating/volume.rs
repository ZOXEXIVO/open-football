//! Engine→real volume conversion for rating inputs.
//!
//! The rating model's saturation scales and evidence-tier thresholds are
//! calibrated for REAL football per-90 volumes — `season_tests.rs` feeds
//! hand-built real stat lines and pins the FM-parity season bands. The
//! engine's raw counters run several times hotter: its 10ms physics loop
//! counts every ball-winning touch, forward nudge and take-on that a real
//! statistician would collapse into one event (or not count at all).
//! Feeding those counters to the model unconverted saturates every credit
//! curve and promotes ordinary shifts up the evidence ladder — measured
//! August 2026 (RATING VOLUME PROFILE in `.dev/match stats`, n=200 at
//! levels 6 and 14): midfielders averaged 97 progressive carries (real
//! ~2), 10 passes into the box (real ~1.5) and 6-14 interceptions (real
//! ~1) per match, so 85-98% of midfield samples cleared the
//! `routine_def >= 7` Strong bar that real football clears in <5% of
//! shifts. Every midfielder's season average landed 7.1-7.4 — the "too
//! many players above 7.10" incident.
//!
//! [`EngineVolumeCalibration::normalize`] converts an engine stat line
//! into real-equivalent units. It is applied ONLY at the engine's rating
//! call sites (`engine/run.rs` pass 2b, live sub scoring, the newsroom
//! texture band) — persisted / displayed statistics stay raw, and
//! hand-built fixtures (already in real units) never pass through it.
//!
//! Divisors are engine per-player-per-match mean ÷ real per-90 mean,
//! averaged across levels 6 and 14, PER POSITION GROUP — the engine's
//! over-emission is position-skewed (its defenders win ~2× real
//! ball-winning volume while its midfielders and forwards, who dominate
//! loose-ball chases, run 3-50×), and a flat divisor under-credited the
//! back line by a whole rating tier (DEF band miss 6.34 vs 6.55-6.95 in
//! the first calibration pass). Two rules keep the table honest:
//!
//!   * **Never amplify.** Counters the engine UNDER-emits for a group
//!     (key passes everywhere, defender box service, forward crosses)
//!     keep divisor 1.0 — a missing producer is an engine gap to fix at
//!     the source, not something to fake at the rating boundary.
//!   * **Families share a divisor.** Zone counters convert with the
//!     family they're read with (own-box defending, final-third work,
//!     attacking box service), not with their parent counter — box
//!     events are gated by real crossing/corner situations and inflate
//!     far less than open-field volume.

use crate::PlayerFieldPositionGroup;
use crate::r#match::PlayerMatchEndStats;

/// Per-position-group divisor set. One instance per group, values from
/// the RATING VOLUME PROFILE table (see module docs). `1.0` = identity.
struct GroupDivisors {
    tackles: f32,
    interceptions: f32,
    /// `passes_into_box` + the attacking box-service zone family
    /// (carries / half-space / central into box, switches of play).
    box_service: f32,
    prog_passes: f32,
    prog_carries: f32,
    /// Successful AND attempted dribbles — same factor preserves the
    /// success ratio the failed-dribble drag reads.
    dribbles: f32,
    /// Completed AND attempted crosses — preserves completion rate.
    crosses: f32,
    /// Own-box + six-yard defensive actions.
    zone_box_def: f32,
    /// Final-third pressures-won / tackles, middle-third interceptions.
    zone_field: f32,
    /// GK save volume: `saves` AND `shots_faced` together, so save%
    /// is preserved while the volume-driven credit is normalised.
    gk_volume: f32,
}

impl GroupDivisors {
    const IDENTITY: GroupDivisors = GroupDivisors {
        tackles: 1.0,
        interceptions: 1.0,
        box_service: 1.0,
        prog_passes: 1.0,
        prog_carries: 1.0,
        dribbles: 1.0,
        crosses: 1.0,
        zone_box_def: 1.0,
        zone_field: 1.0,
        gk_volume: 1.0,
    };

    /// Engine ~3.4 tackles vs real ~1.6; ~2.0 ints vs ~1.3 — the back
    /// line is the LEAST inflated group (it holds shape while mids and
    /// forwards chase). Box service is under-emitted → identity.
    ///
    /// All three tables lean toward the LOW end of the measured
    /// engine-mean range rather than its midpoint. Two reasons: the
    /// engine under-emits some real credit sources entirely (key passes
    /// ~0.1 vs ~1.0 real, blocks 0.0 — producer gaps that real-scale
    /// fixtures earn from but engine players can't), and the real
    /// per-90 references carry their own uncertainty. Mean-centred
    /// divisors measured 0.1-0.15 BELOW every reference band
    /// (n=400 × levels 6+14); the low-lean lands the bands.
    const DEFENDER: GroupDivisors = GroupDivisors {
        tackles: 1.8,
        interceptions: 1.6,
        box_service: 1.0,
        prog_passes: 4.3,
        prog_carries: 9.0,
        dribbles: 3.0,
        crosses: 6.0,
        zone_box_def: 1.5,
        zone_field: 1.0,
        gk_volume: 1.0,
    };

    /// The pedestal group: ~5.7 tackles vs ~1.8, ~10 ints vs ~1.0
    /// (interceptions swing 6→14 from level 6 to 14 — the divisor leans
    /// low so the youth end doesn't go dead), ~97 progressive carries
    /// vs ~2, ~10 passes into the box vs ~1.5, ~13 dribbles vs ~1.
    const MIDFIELDER: GroupDivisors = GroupDivisors {
        tackles: 3.2,
        interceptions: 6.5,
        box_service: 6.0,
        prog_passes: 5.3,
        prog_carries: 40.0,
        dribbles: 10.0,
        crosses: 1.5,
        zone_box_def: 3.3,
        zone_field: 5.0,
        gk_volume: 1.0,
    };

    /// Engine forwards lead every loose-ball chase: ~10 tackles vs
    /// ~0.8 real, ~16 interceptions vs ~0.6, ~20 final-third pressure
    /// events vs ~1. Their crosses are under-emitted → identity.
    const FORWARD: GroupDivisors = GroupDivisors {
        tackles: 10.0,
        interceptions: 22.0,
        box_service: 4.5,
        prog_passes: 3.7,
        prog_carries: 16.0,
        dribbles: 1.7,
        crosses: 1.0,
        zone_box_def: 1.0,
        zone_field: 16.0,
        gk_volume: 1.0,
    };

    /// Claims and command actions are real-scale, but SAVE VOLUME is
    /// not: the engine puts ~8 shots on target per team per match
    /// against a real ~4.3, so a keeper accumulates roughly double the
    /// save credit the model was calibrated for (season means hit 7.7
    /// vs the 6.65-7.10 band). `saves` and `shots_faced` scale together
    /// so save PERCENTAGE — the quality signal — is untouched.
    /// gk_volume 1.9 → 1.0 (2026-08 keeper-quality pass). The divisor was
    /// set when the engine put ~8 shots on target per team against a real
    /// ~4.3. Shot volume is now REAL — 13.4 shots/team, ~3.7 on target —
    /// so a keeper who genuinely faced 3.5 shots was arriving at the
    /// rating model having faced 2, and that is below the `shots_faced`
    /// sample gate on the save-percentage band. The one genuine quality
    /// signal a keeper has was therefore inert for the typical match:
    /// whatever he saved, the model saw a keeper too untested to judge.
    ///
    /// An earlier attempt at 1.0 was reverted for making the population
    /// mean WORSE (7.01 → 7.23). That measurement no longer applies — it
    /// was taken when `gk_command_actions` fired ~15 times a match, so
    /// every keeper was classified `GkBusy` and collected the top
    /// clean-sheet tier regardless of workload. With the claim producer
    /// corrected, restoring real save volume pays the keeper who actually
    /// saves rather than the keeper who happens to be busy.
    const GOALKEEPER: GroupDivisors = GroupDivisors {
        gk_volume: 1.0,
        ..Self::IDENTITY
    };

    fn for_group(group: PlayerFieldPositionGroup) -> &'static GroupDivisors {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => &Self::GOALKEEPER,
            PlayerFieldPositionGroup::Defender => &Self::DEFENDER,
            PlayerFieldPositionGroup::Midfielder => &Self::MIDFIELDER,
            PlayerFieldPositionGroup::Forward => &Self::FORWARD,
        }
    }
}

/// Engine xG → real xG. Engine population xG/shot measures ~0.31
/// against a real ~0.11 and against its own ~9% conversion rate.
/// Identity: the engine→real xG conversion now happens upstream at
/// the recording site (`handle_shoot_event`), so `stats.xg` already
/// arrives in real units and must not be scaled twice.
const XG_TO_REAL: f32 = 1.0;

pub struct EngineVolumeCalibration;

impl EngineVolumeCalibration {
    #[inline]
    fn scale(value: u16, divisor: f32) -> u16 {
        (value as f32 / divisor).round() as u16
    }

    /// Convert an engine stat line to real-equivalent volumes for the
    /// rating model. Decisive outcomes (goals, assists, shots, xG,
    /// saves, errors, cards) and the pass-accuracy pair are identity —
    /// they are already in real units or are ratios the model reads
    /// relative to a baseline.
    pub fn normalize(stats: &PlayerMatchEndStats) -> PlayerMatchEndStats {
        let d = GroupDivisors::for_group(stats.position_group);
        let mut s = stats.clone();
        // xG is the one "decisive" field that is NOT already in real
        // units. The engine's shot-quality model reports ~0.31 xG per
        // shot while those same shots convert at ~9%, i.e. it overstates
        // chance quality ~3x against its own outcomes. The rating model
        // is calibrated on real xG, so unconverted it reads every
        // forward as permanently wasteful — the wasted-xG drags and the
        // attacking-role-expectation lane fire constantly while the
        // clinical (goals-over-xG) bonus can never pay, holding FWD
        // season means ~0.3 below their band. Scaled here rather than in
        // the shot model so gameplay gates (min_xg floors, willingness
        // quality pull, the Tier bars) keep their calibration.
        s.xg *= XG_TO_REAL;
        s.xg_prevented *= XG_TO_REAL;
        s.xg_chain *= XG_TO_REAL;
        s.xg_buildup *= XG_TO_REAL;
        s.saves = Self::scale(s.saves, d.gk_volume);
        s.shots_faced = Self::scale(s.shots_faced, d.gk_volume);
        s.tackles = Self::scale(s.tackles, d.tackles);
        s.interceptions = Self::scale(s.interceptions, d.interceptions);
        s.passes_into_box = Self::scale(s.passes_into_box, d.box_service);
        s.progressive_passes = Self::scale(s.progressive_passes, d.prog_passes);
        s.progressive_carries = Self::scale(s.progressive_carries, d.prog_carries);
        s.successful_dribbles = Self::scale(s.successful_dribbles, d.dribbles);
        s.attempted_dribbles = Self::scale(s.attempted_dribbles, d.dribbles);
        s.crosses_completed = Self::scale(s.crosses_completed, d.crosses);
        s.crosses_attempted = Self::scale(s.crosses_attempted, d.crosses);

        let z = &mut s.zone_stats;
        z.tackles_own_box = Self::scale(z.tackles_own_box, d.zone_box_def);
        z.tackles_own_six_yard = Self::scale(z.tackles_own_six_yard, d.zone_box_def);
        z.interceptions_own_box = Self::scale(z.interceptions_own_box, d.zone_box_def);
        z.interceptions_own_six_yard = Self::scale(z.interceptions_own_six_yard, d.zone_box_def);
        z.blocks_own_box = Self::scale(z.blocks_own_box, d.zone_box_def);
        z.blocks_own_six_yard = Self::scale(z.blocks_own_six_yard, d.zone_box_def);
        z.clearances_own_box = Self::scale(z.clearances_own_box, d.zone_box_def);
        z.clearances_own_six_yard = Self::scale(z.clearances_own_six_yard, d.zone_box_def);
        z.pressures_won_final_third = Self::scale(z.pressures_won_final_third, d.zone_field);
        z.tackles_final_third = Self::scale(z.tackles_final_third, d.zone_field);
        z.interceptions_middle_third = Self::scale(z.interceptions_middle_third, d.zone_field);
        z.carries_into_box = Self::scale(z.carries_into_box, d.box_service);
        z.half_space_passes_into_box = Self::scale(z.half_space_passes_into_box, d.box_service);
        z.central_passes_into_box = Self::scale(z.central_passes_into_box, d.box_service);
        z.switches_of_play = Self::scale(z.switches_of_play, d.box_service);
        z.progressive_passes_into_final_third =
            Self::scale(z.progressive_passes_into_final_third, d.prog_passes);
        z.progressive_carries_into_final_third =
            Self::scale(z.progressive_carries_into_final_third, d.prog_carries);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::engine::rating::RatingContext;
    use crate::r#match::engine::zones::ZoneStats;

    fn stats_with(pos: PlayerFieldPositionGroup) -> PlayerMatchEndStats {
        PlayerMatchEndStats {
            goals: 0,
            assists: 0,
            passes_attempted: 0,
            passes_completed: 0,
            shots_on_target: 0,
            shots_total: 0,
            tackles: 0,
            interceptions: 0,
            saves: 0,
            shots_faced: 0,
            match_rating: 0.0,
            raw_match_rating: 0.0,
            xg: 0.0,
            position_group: pos,
            fouls: 0,
            yellow_cards: 0,
            red_cards: 0,
            minutes_played: 90,
            key_passes: 0,
            progressive_passes: 0,
            progressive_carries: 0,
            successful_dribbles: 0,
            attempted_dribbles: 0,
            successful_pressures: 0,
            pressures: 0,
            blocks: 0,
            clearances: 0,
            passes_into_box: 0,
            crosses_attempted: 0,
            crosses_completed: 0,
            xg_chain: 0.0,
            xg_buildup: 0.0,
            miscontrols: 0,
            heavy_touches: 0,
            carry_distance: 0,
            errors_leading_to_shot: 0,
            errors_leading_to_goal: 0,
            xg_prevented: 0.0,
            offsides: 0,
            own_goals: 0,
            zone_stats: ZoneStats::default(),
        }
    }

    /// The measured August-2026 engine midfielder line must convert to
    /// roughly the real per-90 profile the saturation scales were
    /// calibrated for.
    #[test]
    fn engine_midfielder_line_converts_to_real_scale() {
        let mut s = stats_with(PlayerFieldPositionGroup::Midfielder);
        s.tackles = 6;
        s.interceptions = 10;
        s.passes_into_box = 10;
        s.progressive_passes = 26;
        s.progressive_carries = 97;
        s.successful_dribbles = 13;
        let n = EngineVolumeCalibration::normalize(&s);
        assert_eq!(n.tackles, 2);
        assert_eq!(n.interceptions, 2);
        assert_eq!(n.passes_into_box, 2);
        assert_eq!(n.progressive_passes, 5);
        assert_eq!(n.progressive_carries, 2);
        assert_eq!(n.successful_dribbles, 1);
    }

    /// Defenders convert with their own, much gentler divisors — the
    /// flat first-pass table cost the back line a whole rating tier.
    #[test]
    fn defender_volumes_convert_gently() {
        let mut s = stats_with(PlayerFieldPositionGroup::Defender);
        s.tackles = 3;
        s.interceptions = 2;
        s.clearances = 5;
        s.zone_stats.clearances_own_box = 2;
        let n = EngineVolumeCalibration::normalize(&s);
        assert_eq!(n.tackles, 2);
        assert_eq!(n.interceptions, 1);
        assert_eq!(n.clearances, 5, "clearances are already real-scale");
        assert_eq!(n.zone_stats.clearances_own_box, 1);
    }

    /// Decisive outcomes and pass accuracy pass through untouched —
    /// except xG, which is unit-converted (see XG_TO_REAL).
    #[test]
    fn decisive_outcomes_are_identity() {
        let mut s = stats_with(PlayerFieldPositionGroup::Forward);
        s.goals = 2;
        s.assists = 1;
        s.shots_total = 5;
        s.shots_on_target = 3;
        s.xg = 1.7;
        s.saves = 4;
        s.passes_attempted = 40;
        s.passes_completed = 33;
        s.errors_leading_to_goal = 1;
        let n = EngineVolumeCalibration::normalize(&s);
        assert_eq!(n.goals, 2);
        assert_eq!(n.assists, 1);
        assert_eq!(n.shots_total, 5);
        assert_eq!(n.shots_on_target, 3);
        // xG is deliberately NOT identity — it is converted from the
        // engine's ~3x-hot shot model to real units (see XG_TO_REAL).
        assert!(
            (n.xg - 1.7 * XG_TO_REAL).abs() < 1e-6,
            "xg should convert to real units, got {}",
            n.xg
        );
        assert_eq!(n.saves, 4);
        assert_eq!(n.passes_attempted, 40);
        assert_eq!(n.passes_completed, 33);
        assert_eq!(n.errors_leading_to_goal, 1);
    }

    /// An ordinary engine midfield shift (the measured per-match means)
    /// must not rate above the real-football "solid, nothing decisive"
    /// band once converted — this is the direct regression pin for the
    /// 2026-08 "every midfielder averages 7.2+" incident.
    #[test]
    fn ordinary_engine_midfield_shift_stays_below_good_band() {
        let mut s = stats_with(PlayerFieldPositionGroup::Midfielder);
        s.tackles = 6;
        s.interceptions = 10;
        s.passes_into_box = 10;
        s.progressive_passes = 26;
        s.progressive_carries = 97;
        s.successful_dribbles = 13;
        s.attempted_dribbles = 18;
        s.passes_attempted = 45;
        s.passes_completed = 33;
        s.successful_pressures = 2;
        s.pressures = 8;
        let n = EngineVolumeCalibration::normalize(&s);
        let r = RatingContext::new(&n, 1, 1).calculate();
        assert!(
            r < 7.0,
            "ordinary converted midfield shift should sit below 7.0, got {r:.2}"
        );
    }
}
