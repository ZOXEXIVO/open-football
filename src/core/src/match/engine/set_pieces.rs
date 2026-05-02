//! Set-piece intelligence: taker selection, penalty conversion model,
//! free-kick shot/cross/short choice, wall sizing/blocking.
//!
//! All functions here are pure (no RNG, no mutation). Callers fold the
//! returned probabilities/scores into their own random rolls. Skill
//! attributes are the 0–20 scale exposed by `PlayerSkills` / `PersonAttributes`.

use crate::r#match::engine::environment::MatchEnvironment;

/// Distance band of a direct free kick from goal, in field units (where
/// 1u ≈ 0.125m). 90u ≈ 11m, 130u ≈ 16m.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeKickBand {
    /// 65–90u — close-in direct shot or near-post delivery.
    Close,
    /// 90–130u — standard shooting/crossing range.
    Mid,
    /// 130–180u — long, normally a cross or layoff.
    Long,
    /// >180u — too far to shoot, recycle or long delivery.
    Far,
}

impl FreeKickBand {
    pub fn from_distance(distance_u: f32) -> Self {
        if distance_u <= 90.0 {
            FreeKickBand::Close
        } else if distance_u <= 130.0 {
            FreeKickBand::Mid
        } else if distance_u <= 180.0 {
            FreeKickBand::Long
        } else {
            FreeKickBand::Far
        }
    }
}

/// What the FK taker chooses to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeKickChoice {
    /// Direct shot at goal.
    DirectShot,
    /// Whipped delivery to far/near post.
    BoxDelivery,
    /// Short layoff / pass to a teammate.
    ShortRoutine,
    /// Recycle to a deeper teammate (hold possession).
    Recycle,
}

/// Taker scores: numeric ranking, no RNG. Caller picks max.
#[derive(Debug, Clone, Copy, Default)]
pub struct TakerScore {
    pub player_id: u32,
    pub score: f32,
}

/// Score for picking a penalty taker. Higher = better.
///
/// `penalty_taking*0.38 + finishing*0.18 + composure*0.18 + pressure_attr*0.14
/// + technique*0.07 + confidence*0.05`
///
/// All skill inputs in 0–20 scale; `confidence_state` in -1..+1.
pub fn score_penalty_taker(
    penalty_taking_0_20: f32,
    finishing_0_20: f32,
    composure_0_20: f32,
    pressure_attr_0_20: f32,
    technique_0_20: f32,
    confidence_state: f32,
) -> f32 {
    let pt = (penalty_taking_0_20 / 20.0).clamp(0.0, 1.0);
    let fin = (finishing_0_20 / 20.0).clamp(0.0, 1.0);
    let comp = (composure_0_20 / 20.0).clamp(0.0, 1.0);
    let pres = (pressure_attr_0_20 / 20.0).clamp(0.0, 1.0);
    let tech = (technique_0_20 / 20.0).clamp(0.0, 1.0);
    let conf = ((confidence_state + 1.0) / 2.0).clamp(0.0, 1.0);
    pt * 0.38 + fin * 0.18 + comp * 0.18 + pres * 0.14 + tech * 0.07 + conf * 0.05
}

/// Score for picking a direct free-kick taker. Higher = better.
///
/// `free_kicks*0.34 + technique*0.18 + long_shots*0.12 + crossing*0.12
/// + vision*0.10 + composure*0.08 + pressure*0.06`.
pub fn score_free_kick_taker(
    free_kicks_0_20: f32,
    technique_0_20: f32,
    long_shots_0_20: f32,
    crossing_0_20: f32,
    vision_0_20: f32,
    composure_0_20: f32,
    pressure_attr_0_20: f32,
) -> f32 {
    let n = |x: f32| (x / 20.0).clamp(0.0, 1.0);
    n(free_kicks_0_20) * 0.34
        + n(technique_0_20) * 0.18
        + n(long_shots_0_20) * 0.12
        + n(crossing_0_20) * 0.12
        + n(vision_0_20) * 0.10
        + n(composure_0_20) * 0.08
        + n(pressure_attr_0_20) * 0.06
}

/// Score for picking a corner taker.
///
/// `corners*0.45 + crossing*0.30 + technique*0.15 + vision*0.10`.
pub fn score_corner_taker(
    corners_0_20: f32,
    crossing_0_20: f32,
    technique_0_20: f32,
    vision_0_20: f32,
) -> f32 {
    let n = |x: f32| (x / 20.0).clamp(0.0, 1.0);
    n(corners_0_20) * 0.45 + n(crossing_0_20) * 0.30 + n(technique_0_20) * 0.15 + n(vision_0_20) * 0.10
}

/// Penalty conversion probability.
///
/// Base 0.76 ± taker/keeper skill deltas, ± pressure. Clamped to 0.58–0.90
/// per spec (real-world penalties: 70–82% goals; 10–18% saves; 7–14% off
/// target — this returns *goal* probability, callers split off-target/save).
///
/// Inputs are scores already on 0..1 scale (use `score_penalty_taker` and
/// `score_keeper_save` to compute them).
pub fn penalty_conversion_prob(
    taker_score: f32,
    keeper_score: f32,
    match_pressure_0_1: f32,
    is_shootout: bool,
) -> f32 {
    let base = 0.76;
    // The 0.5-centered baseline — taker_score above 0.5 boosts conversion,
    // keeper_score above 0.5 reduces it.
    let taker_delta = (taker_score - 0.5) * 2.0; // -1..+1
    let keeper_delta = (keeper_score - 0.5) * 2.0;
    let pressure = match_pressure_0_1.clamp(0.0, 1.0);
    let shootout_pressure = if is_shootout { 1.0 } else { 0.0 };

    let raw = base + taker_delta * 0.16 - keeper_delta * 0.10
        - pressure * 0.04
        - shootout_pressure * 0.03;
    raw.clamp(0.58, 0.90)
}

/// Goalkeeper save score (0..1).
///
/// `reflexes*0.34 + agility*0.22 + handling*0.14 + anticipation*0.12
/// + pressure*0.10 + concentration*0.08`. All 0–20 inputs.
pub fn score_keeper_save(
    reflexes_0_20: f32,
    agility_0_20: f32,
    handling_0_20: f32,
    anticipation_0_20: f32,
    pressure_attr_0_20: f32,
    concentration_0_20: f32,
) -> f32 {
    let n = |x: f32| (x / 20.0).clamp(0.0, 1.0);
    n(reflexes_0_20) * 0.34
        + n(agility_0_20) * 0.22
        + n(handling_0_20) * 0.14
        + n(anticipation_0_20) * 0.12
        + n(pressure_attr_0_20) * 0.10
        + n(concentration_0_20) * 0.08
}

/// Wall size for a direct free kick. Increases close-in, drops at wide
/// angles (caller passes `is_wide_angle=true` if y-offset/distance is high).
///
/// Banded sizing per spec:
///   close (65–90u):  5–6  (centred), 4–5 wide
///   mid   (90–130u): 4–5,  3–4 wide
///   long  (130–180u): 3–4, 2–3 wide
///   far   (>180u):    2 (token wall)
pub fn wall_size_for(band: FreeKickBand, is_wide_angle: bool) -> u8 {
    let base: u8 = match band {
        FreeKickBand::Close => 6,
        FreeKickBand::Mid => 5,
        FreeKickBand::Long => 4,
        FreeKickBand::Far => 2,
    };
    let adj = if is_wide_angle && band != FreeKickBand::Far {
        base.saturating_sub(1)
    } else {
        base
    };
    adj.max(2)
}

/// Probability that the wall blocks/deflects a direct free-kick shot.
///
/// `wall_positioning*0.18 + bravery_avg*0.12 + taker_error*0.20
/// + distance_close_factor*0.16` clamped 0.08–0.34.
pub fn wall_block_prob(
    wall_positioning_0_1: f32,
    wall_bravery_avg_0_20: f32,
    taker_error_0_1: f32,
    band: FreeKickBand,
) -> f32 {
    let bravery = (wall_bravery_avg_0_20 / 20.0).clamp(0.0, 1.0);
    let dist_close = match band {
        FreeKickBand::Close => 1.0,
        FreeKickBand::Mid => 0.55,
        FreeKickBand::Long => 0.20,
        FreeKickBand::Far => 0.05,
    };
    let raw = wall_positioning_0_1.clamp(0.0, 1.0) * 0.18
        + bravery * 0.12
        + taker_error_0_1.clamp(0.0, 1.0) * 0.20
        + dist_close * 0.16;
    raw.clamp(0.08, 0.34)
}

/// Score a free-kick choice for the given context. Returns weighted scores
/// for each option; caller normalises to a probability distribution and
/// rolls. Pure function — same inputs → same outputs.
///
/// The scores already incorporate the spec's per-band base probabilities
/// (8–22% direct shot, 35–55% box delivery, 20–35% short, 8–18% recycle).
#[derive(Debug, Clone, Copy)]
pub struct FreeKickChoiceScores {
    pub direct_shot: f32,
    pub box_delivery: f32,
    pub short_routine: f32,
    pub recycle: f32,
}

impl FreeKickChoiceScores {
    /// Largest-score winner. Ties broken in priority order
    /// DirectShot > BoxDelivery > ShortRoutine > Recycle.
    pub fn winner(&self) -> FreeKickChoice {
        let mut best = (FreeKickChoice::DirectShot, self.direct_shot);
        if self.box_delivery > best.1 {
            best = (FreeKickChoice::BoxDelivery, self.box_delivery);
        }
        if self.short_routine > best.1 {
            best = (FreeKickChoice::ShortRoutine, self.short_routine);
        }
        if self.recycle > best.1 {
            best = (FreeKickChoice::Recycle, self.recycle);
        }
        best.0
    }
}

pub fn score_free_kick_choices(
    band: FreeKickBand,
    is_indirect: bool,
    taker_free_kicks_0_20: f32,
    taker_crossing_0_20: f32,
    target_aerial_advantage_0_1: f32,
    chasing_late: bool,
    protecting_lead_late: bool,
    env: &MatchEnvironment,
) -> FreeKickChoiceScores {
    let fk = (taker_free_kicks_0_20 / 20.0).clamp(0.0, 1.0);
    let crossing = (taker_crossing_0_20 / 20.0).clamp(0.0, 1.0);

    // Per-band base probabilities (sum to ~1.0 within each band).
    let (mut shot, mut delivery, mut short, mut recycle): (f32, f32, f32, f32) = match band {
        FreeKickBand::Close => (0.22, 0.45, 0.20, 0.13),
        FreeKickBand::Mid => (0.15, 0.50, 0.25, 0.10),
        FreeKickBand::Long => (0.04, 0.55, 0.30, 0.11),
        FreeKickBand::Far => (0.00, 0.40, 0.40, 0.20),
    };

    // Strong FK skill biases toward direct shot in close/mid bands.
    if band == FreeKickBand::Close || band == FreeKickBand::Mid {
        shot += (fk - 0.5).max(0.0) * 0.20;
    }
    // Strong crossing biases toward box delivery.
    delivery += (crossing - 0.5).max(0.0) * 0.10;

    // Aerial advantage on targets pushes toward delivery.
    delivery += (target_aerial_advantage_0_1 - 0.5).max(0.0) * 0.20;
    // Poor aerial targets push toward short routines.
    if target_aerial_advantage_0_1 < 0.4 {
        short += 0.12;
    }

    // Match state.
    if chasing_late {
        delivery += 0.08;
        recycle -= 0.06;
    }
    if protecting_lead_late {
        short += 0.06;
        recycle += 0.10;
        shot -= 0.05;
    }

    // Wind suppresses long deliveries and shifts to short.
    if env.weather == crate::r#match::engine::environment::Weather::Wind {
        delivery -= 0.10;
        short += 0.07;
        shot -= 0.03;
    }

    // Heavy rain / muddy pitch makes short routines safer.
    if env.weather == crate::r#match::engine::environment::Weather::HeavyRain
        || env.pitch == crate::r#match::engine::environment::Pitch::Muddy
    {
        short += 0.05;
        delivery -= 0.03;
    }

    // Indirect FKs cannot be shot directly at goal — applied last so any
    // skill/state bonuses to shot are zeroed out, redistributed to delivery.
    if is_indirect {
        delivery += shot;
        shot = 0.0;
    }

    // Clamp non-negative; caller normalises if needed.
    FreeKickChoiceScores {
        direct_shot: shot.max(0.0),
        box_delivery: delivery.max(0.0),
        short_routine: short.max(0.0),
        recycle: recycle.max(0.0),
    }
}

/// Corner routine flavour. Drives delivery target + cross trajectory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CornerRoutine {
    /// Inswing/outswing to near-post run.
    NearPost,
    /// High whipped cross to penalty spot.
    PenaltySpot,
    /// Floated/driven cross to far post.
    FarPost,
    /// Short pass to a teammate near the corner flag.
    Short,
    /// Cutback / lay-off to the edge of the box.
    EdgeCutback,
}

/// Throw-in routine flavour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThrowRoutine {
    /// Long throw delivered into the box.
    LongBox,
    /// Short throw to recycle possession.
    ShortRecycle,
}

#[derive(Debug, Clone, Copy)]
pub struct CornerScores {
    pub near_post: f32,
    pub penalty_spot: f32,
    pub far_post: f32,
    pub short: f32,
    pub edge_cutback: f32,
}

impl CornerScores {
    pub fn winner(&self) -> CornerRoutine {
        let mut best = (CornerRoutine::PenaltySpot, self.penalty_spot);
        let candidates = [
            (CornerRoutine::NearPost, self.near_post),
            (CornerRoutine::FarPost, self.far_post),
            (CornerRoutine::Short, self.short),
            (CornerRoutine::EdgeCutback, self.edge_cutback),
        ];
        for (r, s) in candidates {
            if s > best.1 {
                best = (r, s);
            }
        }
        best.0
    }
}

/// Score the five corner routines.
///
/// Inputs:
/// - taker_corners_0_20 — taker's corners attribute
/// - taker_crossing_0_20 — taker's crossing attribute
/// - target_aerial_advantage_0_1 — average attacking-target aerial advantage
///   (heading + jumping + strength avg of attackers minus defenders, normalised)
/// - opponent_gk_aerial_score_0_1 — GK command_of_area+aerial_reach combined
///   (0..1, higher = better at claiming crosses)
/// - chasing_late, protecting_lead — match state
/// - env — for wind effects
pub fn score_corner_routines(
    taker_corners_0_20: f32,
    taker_crossing_0_20: f32,
    target_aerial_advantage_0_1: f32,
    opponent_gk_aerial_score_0_1: f32,
    chasing_late: bool,
    protecting_lead: bool,
    env: &MatchEnvironment,
) -> CornerScores {
    let corners = (taker_corners_0_20 / 20.0).clamp(0.0, 1.0);
    let crossing = (taker_crossing_0_20 / 20.0).clamp(0.0, 1.0);

    // Per-spec base probabilities.
    let mut near = 0.22_f32;
    let mut spot = 0.32_f32;
    let mut far = 0.22_f32;
    let mut short = 0.14_f32;
    let mut edge = 0.10_f32;

    let elite_corners = corners >= 0.75 || crossing >= 0.75;
    if elite_corners {
        spot += 0.05;
        far += 0.03;
    }

    let poor_aerial = target_aerial_advantage_0_1 < 0.40;
    if poor_aerial {
        short += 0.07;
        edge += 0.05;
        near -= 0.04;
        spot -= 0.04;
        far -= 0.04;
    }

    // Strong opponent GK in the air — short/edge becomes more attractive.
    if opponent_gk_aerial_score_0_1 >= 0.75 {
        short += 0.06;
        edge += 0.04;
        spot -= 0.05;
        far -= 0.05;
    }

    if chasing_late {
        near += 0.03;
        spot += 0.04;
        far += 0.01;
        short -= 0.04;
        edge -= 0.04;
    }
    if protecting_lead {
        short += 0.06;
        edge += 0.04;
        spot -= 0.04;
        far -= 0.04;
        near -= 0.02;
    }

    if env.weather == crate::r#match::engine::environment::Weather::Wind {
        short += 0.06;
        near += 0.06;
        far -= 0.10;
        spot -= 0.02;
    }
    if env.weather == crate::r#match::engine::environment::Weather::HeavyRain {
        short += 0.04;
        spot -= 0.02;
        far -= 0.02;
    }

    CornerScores {
        near_post: near.max(0.0),
        penalty_spot: spot.max(0.0),
        far_post: far.max(0.0),
        short: short.max(0.0),
        edge_cutback: edge.max(0.0),
    }
}

/// Pick a throw-in routine. Returns LongBox only when the thrower has the
/// long-throw skill and is in the attacking third with available targets.
pub fn pick_throw_routine(
    long_throws_0_20: f32,
    in_attacking_third: bool,
    have_aerial_targets_in_box: bool,
    chasing_late: bool,
) -> ThrowRoutine {
    let lt = (long_throws_0_20 / 20.0).clamp(0.0, 1.0);
    if in_attacking_third && lt >= 0.70 && have_aerial_targets_in_box {
        return ThrowRoutine::LongBox;
    }
    if chasing_late && in_attacking_third && lt >= 0.55 {
        return ThrowRoutine::LongBox;
    }
    ThrowRoutine::ShortRecycle
}

/// Tracks the most-recent set-piece routines per team. Used to suppress
/// "same exact routine twice in a row" repetition unless the previous
/// routine produced a high-quality chance.
#[derive(Debug, Clone, Default)]
pub struct SetPieceHistory {
    /// Last two corner routines for the home team (most-recent last).
    pub home_corners: Vec<CornerRoutine>,
    /// Last two corner routines for the away team.
    pub away_corners: Vec<CornerRoutine>,
    /// xG produced by each entry above (parallel index).
    pub home_corner_xg: Vec<f32>,
    pub away_corner_xg: Vec<f32>,
}

const ROUTINE_HISTORY_DEPTH: usize = 3;
/// xG threshold above which a routine is deemed successful enough to
/// repeat without penalty.
pub const ROUTINE_REPEAT_XG_THRESHOLD: f32 = 0.10;

impl SetPieceHistory {
    pub fn record_corner(&mut self, is_home: bool, routine: CornerRoutine, xg: f32) {
        let (rs, xs) = if is_home {
            (&mut self.home_corners, &mut self.home_corner_xg)
        } else {
            (&mut self.away_corners, &mut self.away_corner_xg)
        };
        rs.push(routine);
        xs.push(xg);
        if rs.len() > ROUTINE_HISTORY_DEPTH {
            rs.remove(0);
            xs.remove(0);
        }
    }

    /// Should the team be blocked from repeating the same corner routine
    /// they just used? True when the last 2 corners were the same routine
    /// AND neither produced a chance above the xG threshold.
    pub fn should_block_corner_repeat(&self, is_home: bool, routine: CornerRoutine) -> bool {
        let (rs, xs) = if is_home {
            (&self.home_corners, &self.home_corner_xg)
        } else {
            (&self.away_corners, &self.away_corner_xg)
        };
        if rs.len() < 2 {
            return false;
        }
        let n = rs.len();
        let last_two_same = rs[n - 1] == routine && rs[n - 2] == routine;
        if !last_two_same {
            return false;
        }
        let last_was_successful =
            xs[n - 1] >= ROUTINE_REPEAT_XG_THRESHOLD || xs[n - 2] >= ROUTINE_REPEAT_XG_THRESHOLD;
        !last_was_successful
    }
}

/// Pick a corner routine, blocking the recent-history repeat per
/// `SetPieceHistory::should_block_corner_repeat`. If the winner is
/// blocked, returns the next-highest scoring option.
pub fn pick_corner_routine(
    scores: &CornerScores,
    history: &SetPieceHistory,
    is_home: bool,
) -> CornerRoutine {
    let winner = scores.winner();
    if !history.should_block_corner_repeat(is_home, winner) {
        return winner;
    }
    let alternatives = [
        (CornerRoutine::NearPost, scores.near_post),
        (CornerRoutine::PenaltySpot, scores.penalty_spot),
        (CornerRoutine::FarPost, scores.far_post),
        (CornerRoutine::Short, scores.short),
        (CornerRoutine::EdgeCutback, scores.edge_cutback),
    ];
    alternatives
        .iter()
        .filter(|(r, _)| *r != winner)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(r, _)| *r)
        .unwrap_or(winner)
}

/// Picks an explicit taker if they are still on the field; otherwise
/// returns the candidate with the highest score from `candidates`.
pub fn pick_taker(
    explicit_id: Option<u32>,
    on_field_ids: &[u32],
    candidates: &[TakerScore],
) -> Option<u32> {
    if let Some(eid) = explicit_id {
        if on_field_ids.iter().any(|&i| i == eid) {
            return Some(eid);
        }
    }
    candidates
        .iter()
        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
        .map(|t| t.player_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fk_band_classification() {
        assert_eq!(FreeKickBand::from_distance(70.0), FreeKickBand::Close);
        assert_eq!(FreeKickBand::from_distance(110.0), FreeKickBand::Mid);
        assert_eq!(FreeKickBand::from_distance(160.0), FreeKickBand::Long);
        assert_eq!(FreeKickBand::from_distance(220.0), FreeKickBand::Far);
    }

    #[test]
    fn high_skilled_penalty_taker_outscores_low() {
        let high = score_penalty_taker(18.0, 17.0, 16.0, 15.0, 17.0, 0.3);
        let low = score_penalty_taker(8.0, 9.0, 10.0, 8.0, 10.0, -0.2);
        assert!(high > low);
        assert!(high <= 1.0 && low >= 0.0);
    }

    #[test]
    fn high_skilled_fk_taker_outscores_low() {
        let high = score_free_kick_taker(18.0, 17.0, 16.0, 16.0, 15.0, 16.0, 14.0);
        let low = score_free_kick_taker(8.0, 9.0, 8.0, 9.0, 10.0, 10.0, 10.0);
        assert!(high > low);
    }

    #[test]
    fn penalty_prob_clamped_into_band() {
        // Even an elite taker vs a very weak keeper shouldn't break 0.90.
        let taker = score_penalty_taker(20.0, 20.0, 20.0, 20.0, 20.0, 1.0);
        let keeper = score_keeper_save(2.0, 2.0, 2.0, 2.0, 2.0, 2.0);
        let p = penalty_conversion_prob(taker, keeper, 0.0, false);
        assert!((0.58..=0.90).contains(&p));
        assert!(p > 0.80);

        // Worst case still floors at 0.58.
        let taker_low = score_penalty_taker(2.0, 2.0, 2.0, 2.0, 2.0, -1.0);
        let keeper_high = score_keeper_save(20.0, 20.0, 20.0, 20.0, 20.0, 20.0);
        let p_low = penalty_conversion_prob(taker_low, keeper_high, 1.0, true);
        assert!((0.58..=0.90).contains(&p_low));
    }

    #[test]
    fn shootout_pressure_lowers_conversion() {
        let taker = score_penalty_taker(15.0, 15.0, 15.0, 15.0, 15.0, 0.0);
        let keeper = score_keeper_save(15.0, 15.0, 15.0, 15.0, 15.0, 15.0);
        let normal = penalty_conversion_prob(taker, keeper, 0.5, false);
        let shootout = penalty_conversion_prob(taker, keeper, 0.5, true);
        assert!(shootout <= normal);
    }

    #[test]
    fn wall_size_scales_with_distance() {
        assert!(wall_size_for(FreeKickBand::Close, false) >= wall_size_for(FreeKickBand::Mid, false));
        assert!(wall_size_for(FreeKickBand::Mid, false) >= wall_size_for(FreeKickBand::Long, false));
        assert!(wall_size_for(FreeKickBand::Long, false) >= wall_size_for(FreeKickBand::Far, false));
    }

    #[test]
    fn wide_angle_reduces_wall_size_except_far() {
        let close_centre = wall_size_for(FreeKickBand::Close, false);
        let close_wide = wall_size_for(FreeKickBand::Close, true);
        assert!(close_wide < close_centre);
        // Far band already at minimum, no further reduction.
        assert_eq!(
            wall_size_for(FreeKickBand::Far, false),
            wall_size_for(FreeKickBand::Far, true)
        );
    }

    #[test]
    fn wall_block_prob_clamped() {
        let p_low = wall_block_prob(0.0, 0.0, 0.0, FreeKickBand::Far);
        let p_high = wall_block_prob(1.0, 20.0, 1.0, FreeKickBand::Close);
        assert!((0.08..=0.34).contains(&p_low));
        assert!((0.08..=0.34).contains(&p_high));
        assert!(p_high > p_low);
    }

    #[test]
    fn indirect_fk_never_chooses_direct_shot() {
        let env = MatchEnvironment::default();
        let scores = score_free_kick_choices(
            FreeKickBand::Close,
            true,
            18.0,
            10.0,
            0.5,
            false,
            false,
            &env,
        );
        assert_eq!(scores.direct_shot, 0.0);
        assert_ne!(scores.winner(), FreeKickChoice::DirectShot);
    }

    #[test]
    fn long_distance_fk_avoids_direct_shot() {
        let env = MatchEnvironment::default();
        let scores = score_free_kick_choices(
            FreeKickBand::Far,
            false,
            18.0,
            14.0,
            0.5,
            false,
            false,
            &env,
        );
        assert_eq!(scores.direct_shot, 0.0);
    }

    #[test]
    fn wind_pushes_fk_choice_toward_short() {
        let calm = MatchEnvironment::default();
        let windy = MatchEnvironment {
            weather: crate::r#match::engine::environment::Weather::Wind,
            ..Default::default()
        };
        let calm_scores = score_free_kick_choices(
            FreeKickBand::Long,
            false,
            14.0,
            14.0,
            0.55,
            false,
            false,
            &calm,
        );
        let windy_scores = score_free_kick_choices(
            FreeKickBand::Long,
            false,
            14.0,
            14.0,
            0.55,
            false,
            false,
            &windy,
        );
        assert!(windy_scores.box_delivery < calm_scores.box_delivery);
        assert!(windy_scores.short_routine > calm_scores.short_routine);
    }

    #[test]
    fn pick_taker_prefers_explicit_when_on_field() {
        let candidates = vec![
            TakerScore { player_id: 1, score: 0.5 },
            TakerScore { player_id: 2, score: 0.9 },
        ];
        // Explicit taker on field — wins regardless of score.
        let on_field = vec![1, 2, 3];
        assert_eq!(pick_taker(Some(1), &on_field, &candidates), Some(1));
        // Explicit taker off field — falls back to highest score.
        let on_field = vec![2, 3];
        assert_eq!(pick_taker(Some(1), &on_field, &candidates), Some(2));
        // No explicit — highest score.
        assert_eq!(pick_taker(None, &on_field, &candidates), Some(2));
    }

    #[test]
    fn pick_taker_handles_empty() {
        assert_eq!(pick_taker(None, &[1], &[]), None);
        assert_eq!(pick_taker(Some(1), &[2], &[]), None);
    }

    #[test]
    fn corner_short_increases_against_strong_gk() {
        let env = MatchEnvironment::default();
        let weak_gk = score_corner_routines(15.0, 15.0, 0.55, 0.30, false, false, &env);
        let strong_gk = score_corner_routines(15.0, 15.0, 0.55, 0.85, false, false, &env);
        assert!(strong_gk.short > weak_gk.short);
        assert!(strong_gk.penalty_spot < weak_gk.penalty_spot);
    }

    #[test]
    fn corner_short_increases_with_poor_aerial_targets() {
        let env = MatchEnvironment::default();
        let aerial = score_corner_routines(15.0, 15.0, 0.85, 0.50, false, false, &env);
        let no_aerial = score_corner_routines(15.0, 15.0, 0.20, 0.50, false, false, &env);
        assert!(no_aerial.short > aerial.short);
    }

    #[test]
    fn corner_wind_pushes_short_and_near_post() {
        let calm = MatchEnvironment::default();
        let windy = MatchEnvironment {
            weather: crate::r#match::engine::environment::Weather::Wind,
            ..Default::default()
        };
        let calm_scores = score_corner_routines(15.0, 15.0, 0.55, 0.50, false, false, &calm);
        let wind_scores = score_corner_routines(15.0, 15.0, 0.55, 0.50, false, false, &windy);
        assert!(wind_scores.short > calm_scores.short);
        assert!(wind_scores.near_post > calm_scores.near_post);
        assert!(wind_scores.far_post < calm_scores.far_post);
    }

    #[test]
    fn corner_protecting_lead_pushes_short_recycle() {
        let env = MatchEnvironment::default();
        let neutral = score_corner_routines(15.0, 15.0, 0.55, 0.50, false, false, &env);
        let protecting = score_corner_routines(15.0, 15.0, 0.55, 0.50, false, true, &env);
        assert!(protecting.short > neutral.short);
        assert!(protecting.penalty_spot < neutral.penalty_spot);
    }

    #[test]
    fn long_throw_only_with_skill_in_attacking_third() {
        // Off attacking third — never long.
        assert_eq!(
            pick_throw_routine(20.0, false, true, true),
            ThrowRoutine::ShortRecycle
        );
        // Low skill — never long.
        assert_eq!(
            pick_throw_routine(8.0, true, true, true),
            ThrowRoutine::ShortRecycle
        );
        // High skill, attacking third, targets — long.
        assert_eq!(
            pick_throw_routine(16.0, true, true, false),
            ThrowRoutine::LongBox
        );
        // Chasing late, mid skill — long.
        assert_eq!(
            pick_throw_routine(12.0, true, false, true),
            ThrowRoutine::LongBox
        );
    }

    #[test]
    fn corner_repeat_blocked_after_two_failures() {
        let mut hist = SetPieceHistory::default();
        // Two NearPost corners with low xG.
        hist.record_corner(true, CornerRoutine::NearPost, 0.04);
        hist.record_corner(true, CornerRoutine::NearPost, 0.05);
        assert!(hist.should_block_corner_repeat(true, CornerRoutine::NearPost));
        assert!(!hist.should_block_corner_repeat(true, CornerRoutine::PenaltySpot));
    }

    #[test]
    fn corner_repeat_allowed_after_success() {
        let mut hist = SetPieceHistory::default();
        // Successful first corner — repeat allowed.
        hist.record_corner(false, CornerRoutine::PenaltySpot, 0.18);
        hist.record_corner(false, CornerRoutine::PenaltySpot, 0.06);
        assert!(!hist.should_block_corner_repeat(false, CornerRoutine::PenaltySpot));
    }

    #[test]
    fn corner_repeat_history_per_team() {
        let mut hist = SetPieceHistory::default();
        hist.record_corner(true, CornerRoutine::Short, 0.02);
        hist.record_corner(true, CornerRoutine::Short, 0.02);
        // Home is blocked, away is unaffected.
        assert!(hist.should_block_corner_repeat(true, CornerRoutine::Short));
        assert!(!hist.should_block_corner_repeat(false, CornerRoutine::Short));
    }

    #[test]
    fn pick_corner_routine_falls_back_when_blocked() {
        let mut hist = SetPieceHistory::default();
        hist.record_corner(true, CornerRoutine::PenaltySpot, 0.03);
        hist.record_corner(true, CornerRoutine::PenaltySpot, 0.04);
        // Force PenaltySpot to be the natural winner.
        let scores = CornerScores {
            near_post: 0.20,
            penalty_spot: 0.50,
            far_post: 0.18,
            short: 0.10,
            edge_cutback: 0.05,
        };
        // PenaltySpot blocked → next-best = NearPost (0.20).
        let pick = pick_corner_routine(&scores, &hist, true);
        assert_eq!(pick, CornerRoutine::NearPost);
    }
}
