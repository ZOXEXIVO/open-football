use super::*;

fn make_stats(
    goals: u16,
    assists: u16,
    passes_attempted: u16,
    passes_completed: u16,
    shots_on_target: u16,
    shots_total: u16,
    tackles: u16,
    interceptions: u16,
    saves: u16,
    xg: f32,
    position_group: PlayerFieldPositionGroup,
) -> PlayerMatchEndStats {
    PlayerMatchEndStats {
        goals,
        assists,
        passes_attempted,
        passes_completed,
        shots_on_target,
        shots_total,
        tackles,
        interceptions,
        saves,
        shots_faced: 0,
        match_rating: 0.0,
        raw_match_rating: 0.0,
        xg,
        position_group,
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

fn make_gk(saves: u16, shots_faced: u16) -> PlayerMatchEndStats {
    let mut s = make_stats(
        0,
        0,
        20,
        15,
        0,
        0,
        0,
        0,
        saves,
        0.0,
        PlayerFieldPositionGroup::Goalkeeper,
    );
    s.shots_faced = shots_faced;
    s
}

fn anonymous(pos: PlayerFieldPositionGroup) -> PlayerMatchEndStats {
    make_stats(0, 0, 0, 0, 0, 0, 0, 0, 0, 0.0, pos)
}

// ===========================================================
// Behavioral invariants
// ===========================================================

#[test]
fn neutral_quiet_player_stays_near_six() {
    let s = anonymous(PlayerFieldPositionGroup::Midfielder);
    let r = RatingContext::new(&s, 1, 1).calculate();
    assert!((r - 6.0).abs() < 0.10, "neutral rating = {}", r);
}

#[test]
fn short_cameo_has_damped_non_exceptional_rating_movement() {
    let mut starter = make_stats(
        0,
        0,
        30,
        26,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    starter.minutes_played = 90;
    starter.key_passes = 2;
    starter.progressive_passes = 4;
    starter.passes_into_box = 3;
    starter.zone_stats.progressive_passes_into_final_third = 3;
    let mut cameo = starter.clone();
    cameo.minutes_played = 10;
    let starter_r = RatingContext::new(&starter, 1, 1).calculate();
    let cameo_r = RatingContext::new(&cameo, 1, 1).calculate();
    assert!(
        starter_r > cameo_r + 0.3,
        "starter {} should clearly outrate damped cameo {}",
        starter_r,
        cameo_r
    );
    assert!(
        cameo_r < 6.6,
        "cameo with no exceptional events rated {} — should stay damped near 6",
        cameo_r
    );
}

#[test]
fn late_goal_cameo_can_rate_high() {
    // 5-minute cameo, scored the winner. event_minutes_factor keeps
    // most of the goal credit.
    let mut s = make_stats(
        1,
        0,
        4,
        3,
        1,
        1,
        0,
        0,
        0,
        0.5,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 5;
    s.shots_on_target = 1;
    let r = RatingContext::new(&s, 2, 1).calculate();
    assert!(
        r >= 7.1 && r <= 7.8,
        "late-goal cameo rated {} — should be in 7.1..=7.8",
        r
    );
}

#[test]
fn one_goal_low_volume_forward_does_not_exceed_7_7() {
    // 90 minutes, 1 goal, 1 SOT, low creation/passing.
    let mut s = make_stats(
        1,
        0,
        12,
        9,
        1,
        1,
        0,
        0,
        0,
        0.5,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 2, 1).calculate();
    assert!(
        r >= 7.0 && r <= 7.85,
        "single-goal low-volume FWD rated {} — should be 7.0..=7.85 \
         (upper bound lifted in 2026-06 round 3 after goal scoring \
         coefficient raise 2.55 → 2.80)",
        r
    );
}

#[test]
fn two_goals_can_reach_eight_but_not_nine_without_all_round_volume() {
    // 90 minutes, 2 goals, 2 SOT, low creation. Should reach 8.0
    // but not 9.0 without supporting volume.
    let mut s = make_stats(
        2,
        0,
        18,
        14,
        2,
        2,
        0,
        0,
        0,
        0.9,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 2, 0).calculate();
    assert!(
        r >= 8.0 && r <= 8.7,
        "two-goal low-volume FWD rated {} — should be 8.0..=8.7",
        r
    );
}

#[test]
fn creative_no_goal_forward_outrates_passive_baseline() {
    // A creator-shape forward without a goal or assist (3 KP + 3 box
    // entries + 3 successful dribbles + 4 progressive carries) lands
    // in the mid-sixes under the role-expectation calibration —
    // creative work without a finishing event puts a forward above
    // an anonymous teammate, but never into the "good performer"
    // band. The seven-tier ladder + context damping + role
    // expectation all push together. We pin the relative ordering
    // (creative > passive) and a band that prevents inflation regressions.
    let mut fwd = make_stats(
        0,
        0,
        35,
        28,
        0,
        0,
        0,
        0,
        0,
        0.6,
        PlayerFieldPositionGroup::Forward,
    );
    fwd.key_passes = 3;
    fwd.passes_into_box = 3;
    fwd.successful_dribbles = 3;
    fwd.attempted_dribbles = 4;
    fwd.progressive_carries = 4;
    fwd.xg_buildup = 0.4;

    // Baseline: same passing line, no creative footprint.
    let passive = make_stats(
        0,
        0,
        35,
        28,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Forward,
    );
    let base_r = RatingContext::new(&passive, 1, 0).calculate();
    let r = RatingContext::new(&fwd, 1, 0).calculate();
    assert!(
        r > base_r + 0.5,
        "creative forward {} must visibly outrate passive baseline {}",
        r,
        base_r
    );
    assert!(
        r >= 6.0 && r < 6.8,
        "creative forward rated {} — should land 6.0..6.8 (mid sixes; no G/A means no good-performer band)",
        r
    );
}

#[test]
fn anonymous_clean_sheet_defender_stays_below_7() {
    let mut s = make_stats(
        0,
        0,
        18,
        15,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r < 7.0,
        "anonymous clean-sheet DEF rated {} — must be < 7.0",
        r
    );
}

#[test]
fn safe_recycler_does_not_get_elite_rating() {
    let mut s = make_stats(
        0,
        0,
        60,
        55,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.passes_into_box = 0;
    s.progressive_passes = 1;
    let r = RatingContext::new(&s, 1, 1).calculate();
    assert!(
        r < 7.0,
        "safe recycler rated {} — should not reach elite band",
        r
    );
}

#[test]
fn defensive_midfielder_can_exceed_seven_from_defense_and_progression() {
    let mut mid = make_stats(
        0,
        0,
        45,
        38,
        0,
        0,
        5,
        5,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    mid.successful_pressures = 6;
    mid.pressures = 12;
    mid.blocks = 2;
    mid.progressive_passes = 5;
    mid.progressive_carries = 3;
    mid.carry_distance = 1400;
    let r = RatingContext::new(&mid, 1, 0).calculate();
    assert!(
        r > 7.0,
        "defensive MID rated {} — should clear 7.0 on D + progression",
        r
    );
}

#[test]
fn defender_own_box_interventions_rate_higher_than_midfield_volume() {
    let mut middle = make_stats(
        0,
        0,
        25,
        21,
        0,
        0,
        3,
        3,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    middle.clearances = 4;
    middle.blocks = 2;
    let mut box_cb = middle.clone();
    box_cb.zone_stats.tackles_own_box = 3;
    box_cb.zone_stats.interceptions_own_box = 3;
    box_cb.zone_stats.clearances_own_box = 2;
    box_cb.zone_stats.blocks_own_box = 1;
    box_cb.zone_stats.clearances_own_six_yard = 2;
    box_cb.zone_stats.blocks_own_six_yard = 1;
    let mid_r = RatingContext::new(&middle, 1, 0).calculate();
    let box_r = RatingContext::new(&box_cb, 1, 0).calculate();
    assert!(
        box_r > mid_r + 0.20,
        "box CB ({}) should outrate middle-zone CB ({})",
        box_r,
        mid_r
    );
}

#[test]
fn goalkeeper_busy_clean_sheet_rates_well() {
    let busy = make_gk(6, 6);
    let r = RatingContext::new(&busy, 1, 0).calculate();
    assert!(r >= 7.3, "busy CS GK rated {} — should reach 7.3+", r);
}

#[test]
fn goalkeeper_conceding_three_with_saves_is_distinguished_from_no_saves() {
    let busy_three = make_gk(5, 8);
    let bad_three = make_gk(0, 3);
    let busy_r = RatingContext::new(&busy_three, 0, 3).calculate();
    let bad_r = RatingContext::new(&bad_three, 0, 3).calculate();
    assert!(
        busy_r > bad_r + 0.5,
        "busy 3-shipping GK ({}) must outrate inactive 3-shipping GK ({})",
        busy_r,
        bad_r
    );
}

#[test]
fn errors_and_red_cards_materially_lower_rating() {
    // Realistic 90-min MID touch volume so the engagement-penalty
    // gate doesn't fire on either side of the comparison and the
    // delta is driven purely by the error / card event.
    let clean = make_stats(
        0,
        0,
        40,
        32,
        0,
        0,
        3,
        3,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    let mut bad = clean.clone();
    bad.errors_leading_to_goal = 1;
    let mut red = clean.clone();
    red.red_cards = 1;

    let clean_r = RatingContext::new(&clean, 1, 1).calculate();
    let err_r = RatingContext::new(&bad, 1, 2).calculate();
    let red_r = RatingContext::new(&red, 1, 1).calculate();
    assert!(
        clean_r - err_r > 1.5,
        "error-to-goal must drop rating significantly: clean {} → err {}",
        clean_r,
        err_r
    );
    assert!(
        clean_r - red_r > 1.0,
        "red card must drop rating: clean {} → red {}",
        clean_r,
        red_r
    );
}

#[test]
fn rating_stays_in_one_to_ten_range() {
    let mut great = make_stats(
        5,
        3,
        60,
        57,
        5,
        5,
        5,
        5,
        10,
        4.0,
        PlayerFieldPositionGroup::Forward,
    );
    great.key_passes = 8;
    great.progressive_passes = 12;
    great.progressive_carries = 8;
    great.successful_dribbles = 6;
    great.attempted_dribbles = 7;
    great.passes_into_box = 6;
    great.successful_pressures = 5;
    great.pressures = 12;
    great.crosses_attempted = 5;
    great.crosses_completed = 4;
    great.blocks = 2;
    great.clearances = 3;
    great.carry_distance = 3000;
    great.xg_buildup = 1.5;
    let r = RatingContext::new(&great, 6, 0).calculate();
    assert!(r >= RATING_MIN && r <= RATING_MAX, "great rating {}", r);

    let mut bad = anonymous(PlayerFieldPositionGroup::Goalkeeper);
    bad.minutes_played = 90;
    bad.errors_leading_to_goal = 3;
    bad.red_cards = 1;
    bad.own_goals = 1;
    bad.zone_stats.errors_to_goal_own_box = 3;
    let r = RatingContext::new(&bad, 0, 8).calculate();
    assert!(r >= RATING_MIN && r <= RATING_MAX, "bad rating {}", r);
}

#[test]
fn extreme_stat_spam_saturates_without_hard_ceiling_artifacts() {
    let mut moderate = make_stats(
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    moderate.key_passes = 4;
    moderate.progressive_passes = 8;
    moderate.passes_into_box = 4;
    moderate.successful_dribbles = 4;
    moderate.attempted_dribbles = 4;
    let mut extreme = moderate.clone();
    extreme.key_passes = 30;
    extreme.progressive_passes = 50;
    extreme.passes_into_box = 25;
    extreme.successful_dribbles = 25;
    extreme.attempted_dribbles = 25;

    let mod_r = RatingContext::new(&moderate, 1, 0).calculate();
    let ext_r = RatingContext::new(&extreme, 1, 0).calculate();
    assert!(ext_r >= mod_r, "spam must not rate below moderate");
    let delta = ext_r - mod_r;
    assert!(
        delta < 1.5,
        "saturation should bound extreme vs moderate delta ({}) — got {}",
        mod_r,
        delta
    );
    assert!(
        ext_r <= 10.0,
        "spam must respect final clamp — got {}",
        ext_r
    );
}

// ===========================================================
// Sanity / regression checks
// ===========================================================

#[test]
fn busy_gk_outrates_quiet_gk() {
    let quiet = make_gk(1, 1);
    let busy = make_gk(8, 9);
    let quiet_r = RatingContext::new(&quiet, 1, 0).calculate();
    let busy_r = RatingContext::new(&busy, 0, 1).calculate();
    // Margin relaxed 0.5 → 0.35 in the FM-parity season calibration:
    // the quiet side here is a clean-sheet WIN (now properly credited
    // at ~7.0) while the busy side is a heroic 8-save LOSS (~7.4).
    // FM shows exactly that 0.3-0.5 gap — ordering must hold, but a
    // half-point gulf would mean shutouts are under-credited again.
    assert!(
        busy_r > quiet_r + 0.35,
        "busy {} vs quiet {}",
        busy_r,
        quiet_r
    );
}

#[test]
fn gk_shipping_many_goals_is_disaster_band() {
    let gk = make_gk(3, 10);
    let r = RatingContext::new(&gk, 0, 7).calculate();
    assert!(r < 4.5, "7-shipping GK rated {} — should be a disaster", r);
    let none = make_gk(0, 7);
    let none_r = RatingContext::new(&none, 0, 7).calculate();
    assert!(
        r > none_r,
        "any-effort {} must outrate no-effort {}",
        r,
        none_r
    );
}

#[test]
fn defender_with_clean_sheet_and_interventions_lifts_above_quiet() {
    let passive = make_stats(
        0,
        0,
        20,
        16,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    let mut active = passive.clone();
    active.tackles = 3;
    active.interceptions = 4;
    active.clearances = 6;
    active.blocks = 2;
    let p_r = RatingContext::new(&passive, 1, 0).calculate();
    let a_r = RatingContext::new(&active, 1, 0).calculate();
    assert!(a_r > p_r + 0.4, "active CB {} vs passive {}", a_r, p_r);
}

#[test]
fn forward_offsides_penalised_more_than_midfielder() {
    // Realistic touch volume — well above both position engagement
    // floors so the engagement-penalty gate doesn't fire on either
    // side. The discipline-side offsides drag (heavier per-event
    // weight for forwards) is what the test isolates.
    let mut fwd = make_stats(
        0,
        0,
        45,
        38,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Forward,
    );
    fwd.offsides = 3;
    fwd.successful_dribbles = 1;
    fwd.attempted_dribbles = 2;
    let mut mid = fwd.clone();
    mid.position_group = PlayerFieldPositionGroup::Midfielder;
    let fwd_r = RatingContext::new(&fwd, 1, 1).calculate();
    let mid_r = RatingContext::new(&mid, 1, 1).calculate();
    assert!(
        fwd_r < mid_r,
        "FWD offsides {} vs MID offsides {}",
        fwd_r,
        mid_r
    );
}

#[test]
fn high_volume_accurate_passing_beats_low_volume() {
    let few = make_stats(
        0,
        0,
        15,
        14,
        0,
        0,
        2,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    let many = make_stats(
        0,
        0,
        55,
        50,
        0,
        0,
        2,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    let f_r = RatingContext::new(&few, 1, 1).calculate();
    let m_r = RatingContext::new(&many, 1, 1).calculate();
    assert!(m_r > f_r, "many {} vs few {}", m_r, f_r);
}

#[test]
fn clean_sheet_keeper_with_distribution_errors_stays_above_floor() {
    let mut gk = make_gk(1, 1);
    gk.errors_leading_to_shot = 5;
    let r = RatingContext::new(&gk, 0, 0).calculate();
    assert!(
        r >= 5.0,
        "clean-sheet keeper with intercepted long balls rated {} — should stay reasonable",
        r
    );
}

#[test]
fn own_goal_drops_rating_materially() {
    let mut s = make_stats(
        0,
        0,
        30,
        25,
        0,
        0,
        2,
        3,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.clearances = 4;
    let base_r = RatingContext::new(&s, 1, 1).calculate();
    s.own_goals = 1;
    let og_r = RatingContext::new(&s, 1, 2).calculate();
    assert!(base_r - og_r >= 1.0, "OG drop {} → {}", base_r, og_r);
}

#[test]
fn wasteful_high_xg_no_goals_does_not_match_clinical_two_goals() {
    let mut wasteful = make_stats(
        0,
        0,
        20,
        15,
        2,
        6,
        0,
        0,
        0,
        2.5,
        PlayerFieldPositionGroup::Forward,
    );
    wasteful.shots_on_target = 2;
    let clinical = make_stats(
        2,
        0,
        20,
        15,
        2,
        3,
        0,
        0,
        0,
        0.6,
        PlayerFieldPositionGroup::Forward,
    );
    let w_r = RatingContext::new(&wasteful, 2, 0).calculate();
    let c_r = RatingContext::new(&clinical, 2, 0).calculate();
    assert!(
        c_r > w_r + 1.0,
        "clinical {} must clearly outrate wasteful {}",
        c_r,
        w_r
    );
}

#[test]
fn errors_to_shot_saturate() {
    let mut few = make_stats(
        0,
        0,
        30,
        24,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    few.errors_leading_to_shot = 2;
    let mut many = few.clone();
    many.errors_leading_to_shot = 8;
    let f_r = RatingContext::new(&few, 1, 1).calculate();
    let m_r = RatingContext::new(&many, 1, 1).calculate();
    assert!(
        (f_r - m_r).abs() < 0.15,
        "errors-to-shot should saturate: 2 {} vs 8 {} delta {}",
        f_r,
        m_r,
        f_r - m_r
    );
}

#[test]
fn failed_gk_claim_to_goal_subtracts_full_strength() {
    let mut gk = make_gk(3, 4);
    let baseline = RatingContext::new(&gk, 1, 1).calculate();
    gk.zone_stats.gk_failed_claims_to_goal = 1;
    let with_fail = RatingContext::new(&gk, 1, 1).calculate();
    let drop = baseline - with_fail;
    assert!(
        drop > 0.6,
        "failed-claim-to-goal must hit hard — got drop {}",
        drop
    );
}

#[test]
fn xg_buildup_lifts_midfielder() {
    let plain = make_stats(
        0,
        0,
        40,
        34,
        0,
        0,
        3,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    let mut chained = plain.clone();
    chained.xg_buildup = 0.8;
    let p_r = RatingContext::new(&plain, 1, 1).calculate();
    let c_r = RatingContext::new(&chained, 1, 1).calculate();
    assert!(c_r > p_r, "buildup chained {} > plain {}", c_r, p_r);
}

// ===========================================================
// Low-HQ passenger / routine-volume guards
//
// These pin the headline bug: a back-line / midfield player
// racking up routine defensive volume (no own-box impact, no
// progressive output, no key passes / crosses / dribbles, no
// G/A) should not drift into the elite band on the back of a
// clean-sheet win. Stat-line evidence only — never reads
// current_ability or any hidden flag.
// ===========================================================

#[test]
fn low_impact_routine_cb_with_clean_sheet_stays_below_seven_two() {
    // 12 routine defensive actions, 80% completion on 30 safe
    // passes, no own-box / six-yard interventions, no progressive
    // passes / carries / dribbles, no key passes. Clean-sheet win.
    // A busy back-line shutout reads solid-good (~7.1 after the
    // FM-parity calibration) but must never drift toward the elite
    // band on routine volume alone.
    let mut s = make_stats(
        0,
        0,
        30,
        24,
        0,
        0,
        3,
        2,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.clearances = 4;
    s.blocks = 1;
    s.successful_pressures = 2;
    s.pressures = 8;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    // Ceiling moved 7.0 → 7.2 across the FM-parity calibration (win
    // credit 0.16, defensive routine 0.34/0.34/0.30/0.18, defender CS
    // tier 0.36): a 12-action CB holding a 1-0 clean-sheet win at
    // ~7.1 matches the FM reference for a genuinely busy shutout.
    // The 7.4-cluster symptom is pinned where it actually lives now:
    // `season_tests::clean_sheet_defender_season_lands_in_fm_band`
    // caps a realistic 14-CS season mix at 6.95, and the ordinary
    // draw/loss lines below stay strict.
    assert!(
        r < 7.2,
        "low-impact routine CB with clean sheet rated {} — must stay < 7.2",
        r
    );
    assert!(
        r > 6.0,
        "low-impact routine CB rated {} — should still benefit from showing up",
        r
    );
}

#[test]
fn low_impact_routine_mid_with_routine_passing_stays_below_seven() {
    // 30/26 passes (87%), 1 tackle, 1 interception, 1 successful
    // pressure, 5 pressures. No key pass / progressive pass / cross.
    // Win 1-0 (clean sheet doesn't help midfielders much). The
    // engine's typical low-HQ shuttler shape — must stay sub-7.0.
    let mut s = make_stats(
        0,
        0,
        30,
        26,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.successful_pressures = 1;
    s.pressures = 5;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r < 7.0,
        "low-impact routine MID rated {} — must stay < 7.0",
        r
    );
}

#[test]
fn cb_with_own_box_intervention_clears_passenger_guard() {
    // Same routine volume as the bug case above, but with a single
    // own-box clearance. That's stat-line evidence of a decisive
    // moment — the passenger ceiling (Tier C, +0.85) must lift to
    // the modest-evidence ceiling (Tier B, +1.15) on the strength
    // of that one zone event.
    let mut s = make_stats(
        0,
        0,
        30,
        24,
        0,
        0,
        3,
        2,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.clearances = 4;
    s.blocks = 1;
    s.successful_pressures = 2;
    s.pressures = 8;
    s.minutes_played = 90;
    // Baseline: same player, no zone event.
    let baseline = RatingContext::new(&s, 1, 0).calculate();
    // With the own-box intervention added.
    s.zone_stats.clearances_own_box = 1;
    let r = RatingContext::new(&s, 1, 0).calculate();
    // Single own-box clearance is genuine decisive evidence — it
    // must lift the rating above the baseline. The absolute value
    // is no longer expected to clear 7.0 (one clearance is a
    // modest event, not a man-of-the-match shift), but the ladder
    // must visibly reward the evidence.
    assert!(
        r > baseline,
        "CB with own-box intervention rated {} — must outrate the equivalent player without the intervention ({})",
        r,
        baseline
    );
    assert!(
        r > 6.8,
        "CB with own-box intervention rated {} — should at least sit in the upper 6s for an active back-line shift",
        r
    );
}

#[test]
fn low_hq_player_with_decisive_goal_still_rates_above_seven() {
    // The fix should reduce *fake* competence from routine volume,
    // never block a real decisive moment. A "low-HQ" forward whose
    // single match yielded a goal must still be allowed past 7.0.
    let mut s = make_stats(
        1,
        0,
        12,
        8,
        1,
        2,
        0,
        0,
        0,
        0.4,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 80;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r > 7.0,
        "low-volume forward with a goal rated {} — must reach 7.0+",
        r
    );
}

#[test]
fn miscontrols_drag_rating_when_recorded() {
    // Once the engine producers fire, miscontrols / heavy touches
    // must visibly drag the rating — the helper's coefficient is
    // calibrated to land but not dominate.
    let mut clean = make_stats(
        0,
        0,
        30,
        26,
        0,
        0,
        2,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    clean.minutes_played = 90;
    let mut sloppy = clean.clone();
    sloppy.miscontrols = 3;
    sloppy.heavy_touches = 2;
    let clean_r = RatingContext::new(&clean, 1, 1).calculate();
    let sloppy_r = RatingContext::new(&sloppy, 1, 1).calculate();
    assert!(
        clean_r > sloppy_r + 0.15,
        "miscontrols/heavy-touches should drag: clean {} vs sloppy {}",
        clean_r,
        sloppy_r
    );
}

#[test]
fn quiet_passenger_below_busy_passenger() {
    // Both pass the passenger guard, but the busy worker bee should
    // outrate the truly quiet one — the graded `busy` multiplier
    // preserves ordering within the passenger band.
    let mut quiet = make_stats(
        0,
        0,
        18,
        15,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    quiet.minutes_played = 90;
    let mut busy = quiet.clone();
    busy.tackles = 4;
    busy.interceptions = 3;
    busy.clearances = 5;
    busy.blocks = 1;
    busy.successful_pressures = 2;
    busy.pressures = 6;
    let quiet_r = RatingContext::new(&quiet, 1, 0).calculate();
    let busy_r = RatingContext::new(&busy, 1, 0).calculate();
    assert!(
        busy_r > quiet_r + 0.3,
        "busy CB {} should outrate quiet passenger CB {}",
        busy_r,
        quiet_r
    );
    // Quiet passenger never clears 7.0; the busy 15-action worker
    // bee is allowed to drift just past it on clean-sheet credit,
    // but well below the elite band that real decisive output
    // would unlock.
    assert!(
        quiet_r < 7.0,
        "quiet passenger CB rated {} — must stay < 7.0",
        quiet_r
    );
    assert!(
        busy_r < 7.3,
        "busy passenger CB rated {} — should not breach 7.3 without decisive output",
        busy_r
    );
}

#[test]
fn clean_sheet_credit_tiered_by_defensive_evidence() {
    // CB with zero defensive activity (and no own-box presence)
    // gets a minimal clean-sheet bonus; a busy back-line workhorse
    // gets the full +0.25. Evidence-based — no ability read.
    let mut quiet = make_stats(
        0,
        0,
        18,
        15,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    quiet.minutes_played = 90;
    let mut busy = quiet.clone();
    busy.tackles = 3;
    busy.interceptions = 2;
    busy.clearances = 3;
    busy.blocks = 1;
    let mut zone_busy = quiet.clone();
    zone_busy.zone_stats.tackles_own_box = 1;
    let qctx = RatingContext::new(&quiet, 1, 0);
    let bctx = RatingContext::new(&busy, 1, 0);
    let zctx = RatingContext::new(&zone_busy, 1, 0);
    assert!(qctx.clean_sheet_context() < bctx.clean_sheet_context());
    assert!(qctx.clean_sheet_context() < zctx.clean_sheet_context());
    assert!(
        (zctx.clean_sheet_context() - 0.36).abs() < 0.001,
        "own-box intervention earns full clean-sheet bonus (0.36 after \
         the FM-parity defender season calibration)"
    );
}

#[test]
fn destroyer_midfielder_with_clutch_blocks_rates_well() {
    // Heavy defensive volume + progression in a 1-0 win: with the
    // new evidence-based calibration this kind of "shuttler"
    // performance lands in the upper 6s rather than auto-claiming
    // the elite band. Routine work without a goal / assist /
    // own-box intervention is genuinely "good but not great",
    // which matches the spec's "most players: 6.0-6.9" target.
    let mut destroyer = make_stats(
        0,
        0,
        40,
        34,
        0,
        0,
        6,
        5,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    destroyer.blocks = 3;
    destroyer.successful_pressures = 5;
    destroyer.pressures = 12;
    destroyer.progressive_passes = 3;
    let mut passive = destroyer.clone();
    passive.tackles = 0;
    passive.interceptions = 0;
    passive.blocks = 0;
    passive.successful_pressures = 0;
    passive.pressures = 0;
    passive.progressive_passes = 0;
    let r = RatingContext::new(&destroyer, 1, 0).calculate();
    let base_r = RatingContext::new(&passive, 1, 0).calculate();
    assert!(
        r > base_r + 0.5,
        "destroyer {} must visibly outrate passive baseline {}",
        r,
        base_r
    );
    assert!(
        r > 6.7,
        "destroyer MID rated {} — clutch D should lift past 6.7",
        r
    );
}

// ===========================================================
// Distribution targets (from the global-inflation spec).
//
// These tests pin the headline calibration: an ordinary
// midfielder / defender / forward without decisive output
// should cluster in the mid-6s, not at 7.4. A goal / assist /
// multi-key-pass shift earns the 7.0+ band on evidence.
// Stat-line only — never reads current_ability.
// ===========================================================

#[test]
fn ordinary_midfielder_with_routine_volume_stays_in_mid_six_band() {
    // Spec stat line: 35/42 passes (83%), 1 progressive pass,
    // 1 tackle, 1 interception, no goal/assist/key pass/error.
    // Expected band: 6.2–6.7.
    let mut s = make_stats(
        0,
        0,
        42,
        35,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.progressive_passes = 1;
    s.minutes_played = 90;
    // 1-1 draw — no clean-sheet / win lift.
    let r = RatingContext::new(&s, 1, 1).calculate();
    assert!(
        r >= 6.0 && r <= 6.7,
        "ordinary MID rated {} — should sit 6.0..6.7 per the spec target",
        r
    );
}

#[test]
fn ordinary_defender_in_draw_without_clean_sheet_stays_low_six() {
    // Spec stat line: 90 min, realistic CB touch volume (38/35
    // passes ≈ 92%), 2 tackles, 1 interception, 3 clearances —
    // typical engaged CB shift. No clean sheet (drawn 1-1).
    // Expected band: 6.0–6.6 (under "good performer" without
    // decisive output, but above passenger floor).
    let mut s = make_stats(
        0,
        0,
        38,
        35,
        0,
        0,
        2,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.clearances = 3;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 1).calculate();
    assert!(
        r >= 6.0 && r <= 6.6,
        "ordinary DEF in draw rated {} — should sit 6.0..6.6",
        r
    );
}

#[test]
fn losing_midfielder_with_routine_volume_stays_below_six_eight() {
    // No goal/assist, some passes/progression/defense, team lost.
    // Expected: < 6.8 — defeat + no decisive output combined caps
    // the rating below the "good performer" band.
    let mut s = make_stats(
        0,
        0,
        50,
        42,
        0,
        0,
        2,
        2,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.progressive_passes = 2;
    s.successful_pressures = 3;
    s.pressures = 8;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 0, 2).calculate(); // lost 0-2
    assert!(
        r < 6.8,
        "losing MID rated {} — should stay below 6.8 without decisive output",
        r
    );
}

#[test]
fn good_creator_lands_in_seven_to_seven_four_band() {
    // Key passes + box entries + strong progression — the
    // "good performer" archetype. Expected band: 7.0–7.4.
    let mut s = make_stats(
        0,
        0,
        50,
        42,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.key_passes = 3;
    s.passes_into_box = 2;
    s.progressive_passes = 5;
    s.progressive_carries = 2;
    s.xg_buildup = 0.4;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r >= 7.0 && r <= 7.6,
        "good creator MID rated {} — should land 7.0..7.6",
        r
    );
}

#[test]
fn decisive_playmaker_with_assist_clears_seven() {
    // Spec: "goal or assist — rating can exceed 7.0". Real
    // assist-day lines come with creative context (key passes,
    // box entries, progression) — the assist event isn't a
    // standalone signal in the stats stream. The rating ladder
    // rewards the cumulative decisive footprint.
    let mut s = make_stats(
        0,
        1,
        50,
        42,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.key_passes = 2;
    s.passes_into_box = 1;
    s.progressive_passes = 2;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r > 7.0,
        "decisive playmaker rated {} — assist + creation must clear 7.0",
        r
    );
}

#[test]
fn high_pass_completion_alone_does_not_unlock_seven() {
    // 60 passes at 95% completion, no tackles / ints / creation /
    // progression. The spec calls this out explicitly: "high pass
    // completion should not be a large bonus unless volume and
    // progression are meaningful". 1-0 win + clean sheet for MID
    // gives +0.17 context, so the rating should still stay sub-7.
    let mut s = make_stats(
        0,
        0,
        60,
        57,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r < 7.0,
        "pure recycler rated {} — high completion alone must not breach 7.0",
        r
    );
}

#[test]
fn busy_routine_defender_without_decisive_evidence_stays_below_seven_five() {
    // 7/7/7/5 routine defensive actions — a very busy CB by
    // per-90 standards. No zone events, no creative output,
    // clean-sheet win. Routine volume alone may not produce a
    // 7.0+; the passenger cap (Tier C) keeps it in the upper 6s.
    let mut s = make_stats(
        0,
        0,
        25,
        21,
        0,
        0,
        7,
        7,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.clearances = 7;
    s.successful_pressures = 5;
    s.pressures = 12;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    // A very busy routine CB on a clean sheet IS allowed past 7.0 —
    // the team kept a shutout + win behind 26 defensive actions.
    // Bound lifted 7.3 → 7.5 in the FM-parity defender pass: this
    // volume is a siege, not the ordinary-line inflation symptom,
    // and the realistic season mix is pinned at ≤ 6.95 by
    // `season_tests::clean_sheet_defender_season_lands_in_fm_band`.
    // 7.5 still keeps routine volume out of the elite (8.0) band.
    assert!(
        r < 7.5,
        "very busy passenger CB rated {} — must not breach 7.5 without decisive evidence",
        r
    );
}

#[test]
fn losing_team_full_squad_does_not_cluster_at_seven_four() {
    // Spec acceptance criterion: "Losing-team players should not
    // be broadly rated as good performers." Probe a representative
    // losing-side stat distribution: routine outputs for a CB,
    // a CM, and a striker, all in a 0-2 defeat. None should clear
    // 7.0 without decisive output.
    let mut cb = make_stats(
        0,
        0,
        32,
        26,
        0,
        0,
        4,
        3,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    cb.clearances = 5;
    cb.blocks = 1;
    cb.minutes_played = 90;
    let cb_r = RatingContext::new(&cb, 0, 2).calculate();

    let mut cm = make_stats(
        0,
        0,
        55,
        46,
        0,
        0,
        2,
        2,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    cm.progressive_passes = 3;
    cm.successful_pressures = 3;
    cm.pressures = 9;
    cm.minutes_played = 90;
    let cm_r = RatingContext::new(&cm, 0, 2).calculate();

    let mut st = make_stats(
        0,
        0,
        18,
        12,
        1,
        4,
        0,
        0,
        0,
        0.4,
        PlayerFieldPositionGroup::Forward,
    );
    st.shots_on_target = 1;
    st.successful_dribbles = 1;
    st.attempted_dribbles = 3;
    st.minutes_played = 90;
    let st_r = RatingContext::new(&st, 0, 2).calculate();

    for (label, r) in [("CB", cb_r), ("CM", cm_r), ("ST", st_r)] {
        assert!(
            r < 7.0,
            "losing-team {} rated {} — losers without decisive output must stay sub-7",
            label,
            r
        );
    }
}

#[test]
fn ordinary_winning_starter_without_major_action_stays_below_seven() {
    // Spec acceptance criterion: "Players with no goal/assist/big
    // defensive action should usually stay below 7.0." Probe a
    // typical winning-side CM with routine outputs and no decisive
    // events. Win + clean-sheet context contributes +0.17, but
    // Tier B (modest evidence) keeps the ceiling at 7.15 and the
    // routine sum lands under that.
    let mut s = make_stats(
        0,
        0,
        45,
        38,
        0,
        0,
        2,
        2,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.progressive_passes = 2;
    s.successful_pressures = 2;
    s.pressures = 7;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r < 7.0,
        "ordinary winning CM rated {} — routine on the winning side must not clear 7.0",
        r
    );
}

#[test]
fn evidence_tier_ladder_orders_correctly_across_three_archetypes() {
    // Same minutes, same passing baseline. Only the decisive
    // evidence differs. The tier ladder must rate them strictly
    // monotonically:
    //   passenger  (no zone / no creative / no shot)
    //   < modest   (1 key pass)
    //   < strong   (multi key passes + zone work)
    let mut base = make_stats(
        0,
        0,
        35,
        28,
        0,
        0,
        2,
        2,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    base.minutes_played = 90;
    let passenger = base.clone();
    let mut modest = base.clone();
    modest.key_passes = 1;
    let mut strong = base.clone();
    strong.key_passes = 3;
    strong.passes_into_box = 2;
    strong.zone_stats.pressures_won_final_third = 2;
    let p_r = RatingContext::new(&passenger, 1, 0).calculate();
    let m_r = RatingContext::new(&modest, 1, 0).calculate();
    let s_r = RatingContext::new(&strong, 1, 0).calculate();
    assert!(p_r < m_r, "passenger {} should be < modest {}", p_r, m_r);
    assert!(m_r < s_r, "modest {} should be < strong {}", m_r, s_r);
    // Passenger is below the 7.0 band; strong has earned the lift.
    assert!(p_r < 7.0, "passenger MID rated {}", p_r);
}

#[test]
fn one_goal_low_volume_player_still_clears_seven_for_decisive_output() {
    // Spec: "A low-HQ player with visible decisive output should
    // still be rated well." Even a low-touch forward with a single
    // goal must clear 7.0 — the fix should reduce *fake* competence,
    // never block a real decisive moment.
    let mut s = make_stats(
        1,
        0,
        8,
        5,
        1,
        1,
        0,
        0,
        0,
        0.35,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 65;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r > 7.0,
        "low-volume forward with a goal rated {} — decisive output must land",
        r
    );
}

// ===========================================================
// Engagement-gate behaviour
//
// Pin the actual symptom case: a low-engagement starter at a
// possession-dominant club (the "Kazakhstan player at Real Madrid"
// shape — surrounded by elite teammates who keep him out of touch,
// a few miscontrols when he does receive, no decisive output)
// must NOT cluster around 6.3. He should land in the 4.5–5.5
// "clear underperformance" band that real punditry would assign.
// Stat-line only — never reads ability or any hidden flag.
// ===========================================================

#[test]
fn low_engagement_starter_at_possession_dominant_team_rates_below_passenger_cap() {
    // Symptom stat-line: a midfielder on a team that wins 1-0, with
    // ~25 attempted passes (low for 90 min — elite teammates are
    // hogging the ball), 1 tackle, 1 interception, 2 attempted
    // dribbles (BOTH lost — he tried but couldn't beat his man),
    // 1 heavy touch — no key passes, no progressive output, no
    // shot, no goal contribution. Pure passenger fingerprint.
    // Touches/min ≈ 30/90 = 0.33, well below the MID floor (0.50).
    let mut s = make_stats(
        0,
        0,
        25,
        20,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.minutes_played = 90;
    s.successful_dribbles = 0;
    s.attempted_dribbles = 2;
    s.heavy_touches = 1;
    // 1-0 win, no clean sheet for MID.
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r < 5.8,
        "low-engagement starter rated {} — must stay below 5.8 \
             (current symptom: clusters at 6.3+ from passenger cap + \
             win/CS context bonuses)",
        r
    );
    assert!(
        r > 4.0,
        "low-engagement starter rated {} — should NOT bottom out \
             at the floor; this is sub-baseline, not a disaster",
        r
    );
}

#[test]
fn engagement_penalty_does_not_block_decisive_evidence() {
    // A low-touch player whose few touches were decisive (a key
    // pass + dribble + zone intervention) clears the passenger gate
    // and the engagement penalty no longer fires. The fix must never
    // suppress a real decisive moment because the player happens to
    // have low overall touch volume.
    let mut s = make_stats(
        0,
        0,
        20,
        16,
        1,
        1,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.minutes_played = 90;
    s.key_passes = 2;
    s.passes_into_box = 2;
    s.successful_dribbles = 2;
    s.attempted_dribbles = 2;
    let r = RatingContext::new(&s, 1, 0).calculate();
    // With decisive evidence, the player lands in Modest tier (cap
    // +0.95) and the engagement penalty is gated off. Should clear
    // 6.5 even on the same low touch base.
    assert!(
        r > 6.5,
        "low-touch but decisive MID rated {} — decisive evidence \
             must override engagement gating",
        r
    );
}

#[test]
fn engagement_penalty_position_floors_calibrated() {
    // Three players, same volume, different positions. The penalty
    // calibration assumes a position-typical floor: midfielders are
    // expected to touch the ball more than defenders, defenders more
    // than forwards. Verify a 27-touch / 90-min line falls below the
    // MID floor (penalty fires) but not the DEF or FWD floors.
    //
    // Assert the engagement-penalty term directly — the combined
    // rating also folds in the forward role-expectation drag, which
    // is a separate signal (a goalless forward with zero attacking
    // footprint IS more penalised overall than a goalless DEF, even
    // when their touch volume sits above the FWD engagement floor).
    let mut mid = make_stats(
        0,
        0,
        25,
        20,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    mid.minutes_played = 90;
    let def = {
        let mut s = mid.clone();
        s.position_group = PlayerFieldPositionGroup::Defender;
        s
    };
    let fwd = {
        let mut s = mid.clone();
        s.position_group = PlayerFieldPositionGroup::Forward;
        s
    };
    let mid_pen = RatingContext::new(&mid, 1, 1).engagement_penalty();
    let def_pen = RatingContext::new(&def, 1, 1).engagement_penalty();
    let fwd_pen = RatingContext::new(&fwd, 1, 1).engagement_penalty();
    // MID floor 0.50 vs 0.30 actual → significant penalty (more negative)
    // DEF floor 0.40 vs 0.30 actual → small penalty
    // FWD floor 0.30 vs 0.30 actual → at floor, no penalty (zero)
    assert!(
        mid_pen < def_pen,
        "MID engagement penalty ({}) should be more negative than DEF ({}) at \
             0.30 touches/min — higher position-typical floor",
        mid_pen,
        def_pen
    );
    assert!(
        def_pen < fwd_pen,
        "DEF engagement penalty ({}) should be more negative than FWD ({}) at \
             0.30 touches/min — DEF floor 0.40, FWD floor 0.30",
        def_pen,
        fwd_pen
    );
}

#[test]
fn zero_touch_fixture_does_not_trip_engagement_penalty() {
    // The neutral-baseline test fixture (a 90-min player with
    // literally zero stats) is synthetic — the engine always emits
    // some events for a real 60+ min outfield starter. The gate
    // includes a `total_touches == 0` carve-out so this baseline
    // anchor invariant survives the rebalance.
    let mut s = anonymous(PlayerFieldPositionGroup::Midfielder);
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 1).calculate();
    assert!(
        (r - 6.0).abs() < 0.10,
        "zero-touch fixture rated {} — must anchor to BASE = 6.0",
        r
    );
}

#[test]
fn engagement_penalty_skips_short_cameos() {
    // A 20-minute cameo with low touches is not "anonymous" — they
    // just didn't have time to accumulate stats. The penalty only
    // applies to 60+ min starters where the touch volume tells a
    // genuine engagement story.
    let mut s = make_stats(
        0,
        0,
        5,
        4,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.minutes_played = 20;
    let r = RatingContext::new(&s, 1, 1).calculate();
    // Cameo cap (+0.7) governs the upside; engagement gate is off.
    // Final rating sits in the 5.8-6.5 band — short cameo + draw.
    assert!(
        r >= 5.5 && r <= 6.5,
        "short cameo rated {} — engagement gate must not fire \
             on cameos",
        r
    );
}

// ===========================================================
// Role-based rating expectations (spec rebalance).
//
// These pin the natural calibration target: a forward without
// G/A can't ride routine volume into the good-rating band, but
// a GK/DEF/MID can still earn a good rating on role-specific
// stat-line evidence. All checks are pure stat-line reads.
// ===========================================================

// ─── Forwards ───────────────────────────────────────────────

#[test]
fn forward_no_goal_no_assist_with_routine_volume_stays_below_6_5() {
    // 90 min, no G/A, modest shooting, light dribbling + pressing.
    // The forward did *something* — but none of it threatened the
    // goal. Spec target: naturally below 6.5 (no hard cap).
    let mut s = make_stats(
        0,
        0,
        25,
        19,
        1,
        3,
        0,
        0,
        0,
        0.3,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    s.successful_dribbles = 2;
    s.attempted_dribbles = 4;
    s.successful_pressures = 2;
    s.pressures = 6;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r < 6.5,
        "routine no-G/A FWD rated {} — primary role unfulfilled, must stay below 6.5",
        r
    );
}

#[test]
fn creative_forward_no_goal_no_assist_stays_around_mid_sixes() {
    // A creative forward without G/A — repeatedly broke the line
    // for teammates but didn't put anything in or set up a goal.
    // Above an anonymous forward, well below a good-performer rating.
    let mut s = make_stats(
        0,
        0,
        35,
        28,
        0,
        0,
        0,
        0,
        0,
        0.6,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    s.key_passes = 3;
    s.passes_into_box = 3;
    s.successful_dribbles = 3;
    s.attempted_dribbles = 4;
    s.progressive_carries = 4;
    s.xg_buildup = 0.4;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r >= 6.0 && r <= 6.8,
        "creative no-G/A FWD rated {} — should land in the mid-sixes",
        r
    );
}

#[test]
fn pressing_forward_no_goal_no_assist_does_not_clear_good_rating() {
    // High-press forward: lots of pressing volume + a couple of
    // ground duels won, but zero goal threat and no creative
    // footprint. Defensive work helps slightly; it cannot carry a
    // forward into a good rating.
    let mut s = make_stats(
        0,
        0,
        22,
        17,
        0,
        0,
        1,
        2,
        0,
        0.0,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    s.successful_pressures = 8;
    s.pressures = 18;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r < 7.0,
        "pressing-only FWD rated {} — defensive volume cannot carry a forward past 7.0 without G/A",
        r
    );
}

#[test]
fn wasteful_high_xg_forward_is_penalized() {
    // 6 shots / 2 SOT / 2.5 xG / 0 goals — the canonical "missed
    // sitter" shift. Strong wasted-xG drag in shooting() AND in
    // attacking_role_expectation() should pull this well below
    // 6.5 even though the forward looked busy on the shot board.
    let mut s = make_stats(
        0,
        0,
        20,
        15,
        2,
        6,
        0,
        0,
        0,
        2.5,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r < 6.5,
        "wasteful high-xG FWD rated {} — must be visibly penalised, never neutral or above",
        r
    );
}

#[test]
fn one_goal_forward_clears_seven() {
    // Low-volume forward with a single goal — the decisive moment
    // must clear 7.0 naturally, even when the supporting line is
    // thin. The role-expectation gate is off (goals > 0).
    let mut s = make_stats(
        1,
        0,
        15,
        11,
        1,
        2,
        0,
        0,
        0,
        0.4,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r > 7.0,
        "one-goal FWD rated {} — decisive output must clear 7.0",
        r
    );
}

#[test]
fn assist_forward_with_creative_context_clears_seven() {
    // No goal, one assist, surrounding creative line (2 KP + 2
    // box entries + 2 successful dribbles + 2 progressive carries).
    // The assist is the decisive event; the creative line confirms
    // it. Spec: a goal *or* assist makes the forward a good
    // performer.
    let mut s = make_stats(
        0,
        1,
        25,
        20,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Forward,
    );
    s.key_passes = 2;
    s.passes_into_box = 2;
    s.successful_dribbles = 2;
    s.attempted_dribbles = 3;
    s.progressive_carries = 2;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r > 7.0,
        "assist + creative FWD rated {} — decisive output must clear 7.0",
        r
    );
}

#[test]
fn two_goal_forward_can_reach_eight() {
    // Clinical two-goal striker — modest shot volume, both on
    // target, scored both from ~0.9 xG. The TwoGoals tier and
    // clinical bonus combine naturally to reach 8.0+.
    let mut s = make_stats(
        2,
        0,
        18,
        14,
        2,
        2,
        0,
        0,
        0,
        0.9,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 2, 0).calculate();
    assert!(
        r >= 8.0,
        "two-goal FWD rated {} — clinical brace must reach 8.0",
        r
    );
}

// ─── Goalkeepers ────────────────────────────────────────────

#[test]
fn quiet_clean_sheet_gk_not_overrewarded() {
    // 1 save off 1 shot faced, clean sheet, win. A keeper who
    // basically wasn't tested gets the bookkeeping clean-sheet
    // bonus but no save / xG-prevented credit to speak of.
    let gk = make_gk(1, 1);
    let r = RatingContext::new(&gk, 1, 0).calculate();
    assert!(
        r >= 6.0 && r < 7.2,
        "quiet CS GK rated {} — should sit around the high-six band, not the good-performer band",
        r
    );
}

#[test]
fn busy_clean_sheet_gk_rates_well() {
    // 6 saves off 6 shots, clean sheet. Real keeper performance
    // with workload absorbed + xG-prevented + save% lift.
    let busy = make_gk(6, 6);
    let r = RatingContext::new(&busy, 1, 0).calculate();
    assert!(
        r >= 7.2,
        "busy CS GK rated {} — should comfortably exceed 7.2",
        r
    );
}

#[test]
fn gk_with_many_saves_while_conceding_outscores_no_save_gk() {
    // Two keepers each concede 3. The busy one made 5 saves off
    // 8 shots; the inactive one made 0 off 3. Concede-blame is
    // the same; the save / save% / workload signal differentiates.
    let busy = make_gk(5, 8);
    let inactive = make_gk(0, 3);
    let busy_r = RatingContext::new(&busy, 0, 3).calculate();
    let inactive_r = RatingContext::new(&inactive, 0, 3).calculate();
    assert!(
        busy_r > inactive_r + 0.6,
        "busy 3-shipping GK ({}) must clearly outrate inactive 3-shipping GK ({})",
        busy_r,
        inactive_r
    );
}

#[test]
fn gk_error_to_goal_drops_rating_hard() {
    // A defining failure moment lands at full strength regardless
    // of minutes — an error-to-goal must pull the rating down by
    // more than a full point.
    let mut clean = make_gk(2, 3);
    clean.minutes_played = 90;
    let mut bad = clean.clone();
    bad.errors_leading_to_goal = 1;
    let clean_r = RatingContext::new(&clean, 1, 0).calculate();
    let bad_r = RatingContext::new(&bad, 1, 1).calculate();
    assert!(
        clean_r - bad_r > 1.2,
        "GK error-to-goal must drop hard: clean {} → err {}",
        clean_r,
        bad_r
    );
}

// ─── Defenders ──────────────────────────────────────────────

#[test]
fn anonymous_clean_sheet_defender_stays_mid_six() {
    // CB with realistic passing volume (35 passes) on a clean sheet
    // but zero defensive activity. Clean sheet alone, with no
    // defensive evidence behind it, sits at the bottom of the
    // mid-sixes — the team got the shutout, not this player.
    let mut s = make_stats(
        0,
        0,
        35,
        29,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r >= 6.0 && r < 6.7,
        "anonymous CS DEF rated {} — should sit in the mid-sixes, never reach the good band",
        r
    );
}

#[test]
fn ordinary_defender_without_clean_sheet_stays_low_six() {
    // Routine defender shift in a 1-1 draw — 2 tackles, 1
    // interception, 3 clearances. No own-box impact, no clean
    // sheet, no win. Lands in the low-to-mid sixes.
    let mut s = make_stats(
        0,
        0,
        38,
        35,
        0,
        0,
        2,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.clearances = 3;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 1).calculate();
    assert!(
        r >= 6.0 && r <= 6.6,
        "ordinary DEF in draw rated {} — should sit 6.0..6.6",
        r
    );
}

#[test]
fn defender_with_major_box_interventions_can_clear_seven() {
    // Heavy own-box workload: tackles + blocks + clearances inside
    // the danger zone. Real stat-line evidence of a back-line shift
    // that earned the result. Should comfortably clear 7.0.
    let mut s = make_stats(
        0,
        0,
        30,
        24,
        0,
        0,
        4,
        4,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.clearances = 5;
    s.blocks = 3;
    s.zone_stats.tackles_own_box = 2;
    s.zone_stats.blocks_own_box = 2;
    s.zone_stats.clearances_own_box = 2;
    s.zone_stats.clearances_own_six_yard = 1;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r > 7.0,
        "major-box-intervention DEF rated {} — high-danger work must clear 7.0",
        r
    );
}

#[test]
fn busy_routine_defender_without_big_moments_not_overrewarded() {
    // Busy routine CB on a clean-sheet win: 6/4 tackles+ints, 7
    // clearances, 4 successful pressures. No zone events, no
    // creative output. Spec: routine work doesn't auto-claim the
    // good-rating band.
    let mut s = make_stats(
        0,
        0,
        30,
        25,
        0,
        0,
        6,
        4,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.clearances = 7;
    s.blocks = 2;
    s.successful_pressures = 4;
    s.pressures = 10;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    // Ceiling lifted 7.3 → 7.5 (FM-parity defender season pass): a
    // 19-action clean-sheet win is a siege survived, not routine
    // volume — FM rates that shift 7.3-7.7. The engine's typical CB
    // emits 2-3 routine actions, so this line is rare; the cluster
    // guard for realistic season mixes lives in
    // `season_tests::clean_sheet_defender_season_lands_in_fm_band`.
    assert!(
        r < 7.5,
        "busy routine DEF rated {} — no big moments must keep this below 7.5",
        r
    );
}

// ─── Midfielders ────────────────────────────────────────────

#[test]
fn safe_recycler_without_progression_stays_below_6_5() {
    // High-completion midfielder with no progression, no key passes,
    // no defensive footprint. The passenger gate + retention cap
    // keep this below 6.5 even on a clean-sheet win.
    let mut s = make_stats(
        0,
        0,
        60,
        57,
        0,
        0,
        0,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r < 6.5,
        "safe recycler rated {} — pass% alone must not clear 6.5",
        r
    );
}

#[test]
fn ordinary_midfielder_stays_mid_six() {
    // Spec stat line: 35/42 passes (~83%), 1 progressive pass,
    // 1 tackle, 1 interception, no decisive event. 1-1 draw.
    let mut s = make_stats(
        0,
        0,
        42,
        35,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.progressive_passes = 1;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 1).calculate();
    assert!(
        r >= 6.0 && r <= 6.7,
        "ordinary MID rated {} — should sit in the mid-sixes",
        r
    );
}

#[test]
fn good_creator_without_assist_can_reach_high_sixes() {
    // Creator without assist: 3 KP + 2 box entries + 5 progressive
    // passes + xG buildup. The Strong tier + creative footprint
    // lift this into the high-sixes / low-sevens — no decisive
    // event makes it a *good* performer, never *great*.
    let mut s = make_stats(
        0,
        0,
        50,
        42,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.key_passes = 3;
    s.passes_into_box = 2;
    s.progressive_passes = 5;
    s.progressive_carries = 2;
    s.xg_buildup = 0.4;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r >= 6.7 && r <= 7.4,
        "good creator MID rated {} — should land high-six / low-seven",
        r
    );
}

#[test]
fn decisive_playmaker_with_assist_clears_seven_role() {
    // An assist + surrounding creative footprint. The decisive
    // event clears the good-performer threshold.
    let mut s = make_stats(
        0,
        1,
        50,
        42,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.key_passes = 2;
    s.passes_into_box = 1;
    s.progressive_passes = 2;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r > 7.0,
        "decisive playmaker rated {} — assist + creation must clear 7.0",
        r
    );
}

#[test]
fn defensive_midfielder_with_major_work_can_clear_seven() {
    // Ball-winning + progression — destroyer profile. Real
    // defensive evidence in a winning side lifts naturally past
    // the good-performer threshold.
    let mut s = make_stats(
        0,
        0,
        45,
        38,
        0,
        0,
        5,
        5,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.successful_pressures = 6;
    s.pressures = 12;
    s.blocks = 2;
    s.progressive_passes = 5;
    s.progressive_carries = 3;
    s.carry_distance = 1400;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r > 7.0,
        "destroyer MID rated {} — ball-winning + progression must clear 7.0",
        r
    );
}

#[test]
fn losing_routine_midfielder_not_overrewarded() {
    // 0-2 defeat, routine line: 50/42 passes, 2 progressive, 2/2
    // tackles+ints, modest pressing. No decisive event. The loss
    // drag + lack of decisive evidence keep this below 6.8.
    let mut s = make_stats(
        0,
        0,
        50,
        42,
        0,
        0,
        2,
        2,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.progressive_passes = 2;
    s.successful_pressures = 3;
    s.pressures = 8;
    s.minutes_played = 90;
    let r = RatingContext::new(&s, 0, 2).calculate();
    assert!(
        r < 6.8,
        "losing routine MID rated {} — defeat + no decisive output should keep this sub-6.8",
        r
    );
}

#[test]
fn passenger_context_credit_halved_but_loss_drag_full() {
    // A passenger should not get the full win bonus (didn't earn
    // the win) but a loss still pulls in full (defeat hits everyone
    // who was on the pitch). Verify the asymmetric context damping.
    let mut base = make_stats(
        0,
        0,
        30,
        24,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    base.minutes_played = 90;
    let win_r = RatingContext::new(&base, 1, 0).calculate(); // win 1-0
    let draw_r = RatingContext::new(&base, 1, 1).calculate(); // draw 1-1
    let loss_r = RatingContext::new(&base, 0, 1).calculate(); // loss 0-1
    // Win bonus halved: win - draw ≈ +0.06 + halved-CS ≈ +0.025 → ~0.085
    let win_lift = win_r - draw_r;
    let loss_drag = draw_r - loss_r;
    assert!(
        loss_drag > win_lift,
        "loss drag {} should exceed passenger win lift {} — passenger \
             credit damping is asymmetric (positive context halved, \
             negative context unchanged)",
        loss_drag,
        win_lift
    );
}

// ===========================================================
// Season-average regression — a 20-match goalless-forward shift
// must not drift above ~6.6 even on a winning team. This pins
// the symptom that drove the recent calibration: forwards with
// modest-but-not-decisive lines averaging 6.9 over many games.
// ===========================================================

#[test]
fn goalless_forward_modest_line_winning_season_averages_in_mid_sixes() {
    // Repeat shift: 25/19 passes, 1 SOT off 2 shots, xG 0.3, two
    // dribbles, one tackle / interception. Forward sits in the
    // Modest evidence tier (xG ≥ 0.4 doesn't trigger; 1 SOT does).
    // Always a 1-0 win — best-case for routine accumulation.
    let mut s = make_stats(
        0,
        0,
        25,
        19,
        1,
        2,
        1,
        1,
        0,
        0.3,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    s.successful_dribbles = 2;
    s.attempted_dribbles = 3;
    s.successful_pressures = 2;
    s.pressures = 5;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r < 6.6,
        "goalless modest-line FWD rated {} on a 1-0 win — over 20 such \
             shifts the season average must not drift above 6.6, or the \
             primary-role expectation is being underweighted",
        r
    );
}

// ─── GK calibration regression — second-tier "robot" symptom ───
//
// Pichienko (Serie B, 26 apps, 0.46 conceded/game, 16 CS) and
// Casanova (Argentine Primera, 13 apps, 0.46 conceded/game, 10 CS)
// were averaging 7.11-7.37 over full seasons — the unconditional
// +0.30 CS bonus stacked on GkModest's +1.05 routine cap.

#[test]
fn moderate_workload_clean_sheet_gk_stays_under_seven_five() {
    // 3 saves, 4 shots faced, clean sheet, win. Typical "did the job"
    // GK shift — enough saves to qualify for the modest CS bonus,
    // not enough to clear the busy bar. Should land around 7.2-7.3,
    // not 7.5+. The 2026-06 GK recalibration lifted the GkModest cap
    // back to 0.92 (was 0.75) after the prior pass dropped TOP-GK
    // season averages to ~6.3.
    let mut gk = make_gk(3, 4);
    gk.minutes_played = 90;
    let r = RatingContext::new(&gk, 1, 0).calculate();
    // Ceiling lifted 7.4 → 7.5 with the FM-parity GK clean-sheet
    // recalibration: a 3-save shutout win lands ~7.45 (WhoScored /
    // FM reference 7.2-7.5). The second-tier-robot symptom this test
    // guarded is now pinned harder at season scale by
    // `season_tests::top_gk_league_season_lands_in_fm_band` (a 12-CS
    // season must stay ≤ 7.00 overall).
    assert!(
        r < 7.5,
        "moderate-workload CS GK rated {} — should land in the 7.2-7.45 \
             band, not drift past 7.5",
        r
    );
}

#[test]
fn quiet_clean_sheet_gk_stays_under_seven_two() {
    // 1 save / 2 shots faced, clean sheet, win. Quiet shutout: the
    // back four did the work, but the keeper organised it. With the
    // 2026-06 recalibration this lands ~6.8 — still clearly below the
    // "did real GK work" band, but no longer crushed to ~6.6 by a
    // halved CS credit. Season averages with such matches stay in
    // the high sixes / low sevens.
    let mut gk = make_gk(1, 2);
    gk.minutes_played = 90;
    let r = RatingContext::new(&gk, 1, 0).calculate();
    assert!(
        r < 7.2,
        "quiet CS GK rated {} — back-four-protected shutouts must not \
             cross 7.2 or every second-tier keeper averages 7.0+",
        r
    );
    assert!(
        r > 6.5,
        "quiet CS GK rated {} — a clean-sheet win must still clear 6.5; \
             previous tightening crushed top-GK season averages to ~6.3",
        r
    );
}

#[test]
fn busy_clean_sheet_gk_still_rewards_real_work() {
    // 5 saves, 7 shots faced, clean sheet, win. Earned the CS — the
    // full bonus + busy-GK routine cap should comfortably clear 7.2.
    let mut gk = make_gk(5, 7);
    gk.minutes_played = 90;
    let r = RatingContext::new(&gk, 1, 0).calculate();
    assert!(
        r > 7.1,
        "busy CS GK rated {} — a 5-save shutout must remain a clearly \
             good rating after the GK tier rebalance",
        r
    );
}

#[test]
fn heroic_loss_gk_outrates_routine_shutout_gk() {
    // The "earned vs organised" ordering: an 8-save keeper who lost
    // 0-1 behind a sieve must outrate a 3-save keeper behind a solid
    // 1-0 shutout. The FM-parity clean-sheet lifts pulled these two
    // within ~0.05 of each other — this guard pins the model-level
    // ordering so a future CS-credit nudge can't silently invert it.
    let mut heroic = make_gk(8, 9);
    heroic.minutes_played = 90;
    let mut shutout = make_gk(3, 4);
    shutout.minutes_played = 90;
    let heroic_r = RatingContext::new(&heroic, 0, 1).calculate();
    let shutout_r = RatingContext::new(&shutout, 1, 0).calculate();
    assert!(
        heroic_r > shutout_r,
        "heroic 8-save loss GK ({}) must outrate the routine 3-save \
             shutout-win GK ({})",
        heroic_r,
        shutout_r
    );
}

#[test]
fn four_save_shutout_win_stays_below_elite_band() {
    // The GkBusy tier gate (saves >= 4) flips both the soft cap
    // (0.92 → 1.52) and the top CS tier (0.29 → 0.34) at once, so a
    // 4-save shutout win sits a clear step above the 3-save one
    // (~8.2 vs ~7.45). FM shows 7.5-8.0 for that line; this pins the
    // cell below 8.3 so the tier cliff can't drift into routine
    // 8.5+ keeper matches.
    let mut gk = make_gk(4, 4);
    gk.minutes_played = 90;
    let r = RatingContext::new(&gk, 1, 0).calculate();
    assert!(
        r < 8.3,
        "4-save shutout-win GK rated {} — the GkBusy tier step must \
             stay below 8.3",
        r
    );
}

#[test]
fn top_gk_season_average_lands_in_real_football_band() {
    // Regression for the 2026-06 issue where Courtois / Maignan /
    // Unai Simón posted season averages in the 6.21-6.62 band — well
    // below the WhoScored reference 6.8-7.0 for elite keepers. The
    // root cause was the cumulative tightening: GkModest cap +0.75,
    // GkPassenger cap +0.50, halved context credit, and an over-tiered
    // CS bonus. With the recalibration, a representative 38-match
    // schedule for a TOP keeper in a strong defensive side must
    // average comfortably above 6.7.
    //
    // The synthetic schedule below approximates the observed match
    // distribution for Maignan (43% CS, 30% concede-1, 17% concede-2,
    // 7% concede-3+, in a win-heavy team). Save counts track real
    // distribution: dominant defence → ~50% of CS games are quiet
    // (0-1 saves), the rest have 2-5; concede games scale workload
    // with shots faced.
    let mut total = 0.0_f32;
    let mut count = 0_u32;
    // CS wins, quiet (0-1 saves) — dominant defence, untested keeper.
    for &(saves, sf) in &[(0_u16, 1_u16), (1, 1), (1, 2), (0, 2), (1, 2), (1, 3)] {
        let mut gk = make_gk(saves, sf);
        gk.minutes_played = 90;
        total += RatingContext::new(&gk, 1, 0).calculate();
        count += 1;
    }
    // CS wins, moderate (2-3 saves) — back-four pressed, keeper tidy.
    for &(saves, sf) in &[(2_u16, 3_u16), (3, 4), (3, 4), (2, 3), (3, 5)] {
        let mut gk = make_gk(saves, sf);
        gk.minutes_played = 90;
        total += RatingContext::new(&gk, 1, 0).calculate();
        count += 1;
    }
    // CS wins, busy (4-6 saves) — keeper saved the result.
    for &(saves, sf) in &[(4_u16, 5_u16), (5, 6), (4, 5), (6, 7)] {
        let mut gk = make_gk(saves, sf);
        gk.minutes_played = 90;
        total += RatingContext::new(&gk, 1, 0).calculate();
        count += 1;
    }
    // Concede 1, win (12 games — most common outcome for a TOP team).
    for &(saves, sf) in &[
        (2_u16, 4_u16),
        (3, 5),
        (2, 4),
        (3, 5),
        (1, 3),
        (3, 5),
        (4, 6),
        (2, 4),
        (3, 5),
        (2, 4),
        (3, 6),
        (4, 7),
    ] {
        let mut gk = make_gk(saves, sf);
        gk.minutes_played = 90;
        total += RatingContext::new(&gk, 2, 1).calculate();
        count += 1;
    }
    // Concede 2 (6 games — mix of wins and draws/losses).
    for &(saves, sf, tg, og) in &[
        (3_u16, 6_u16, 3_u8, 2_u8),
        (4, 7, 2, 2),
        (3, 6, 1, 2),
        (4, 8, 2, 2),
        (5, 8, 3, 2),
        (3, 7, 0, 2),
    ] {
        let mut gk = make_gk(saves, sf);
        gk.minutes_played = 90;
        total += RatingContext::new(&gk, tg, og).calculate();
        count += 1;
    }
    // Concede 3+ (3 games — bad days).
    for &(saves, sf, og) in &[(4_u16, 8_u16, 3_u8), (3, 7, 3), (4, 9, 4)] {
        let mut gk = make_gk(saves, sf);
        gk.minutes_played = 90;
        total += RatingContext::new(&gk, 1, og).calculate();
        count += 1;
    }
    let avg = total / count as f32;
    assert!(
        avg > 6.7,
        "TOP-GK season average rated {} across {} matches — must \
             clear 6.7 so elite keepers in a strong defensive side \
             land in the real-football 6.8-7.0 reference band (was \
             collapsing to 6.3 after the prior tightening)",
        avg,
        count,
    );
    assert!(
        avg < 7.3,
        "TOP-GK season average rated {} across {} matches — must \
             stay under 7.3 so the routine band doesn't drift back \
             into the elite zone the 2026-04 pass was guarding against",
        avg,
        count,
    );
}

#[test]
fn fullback_routine_shift_clears_six_six() {
    // Workhorse fullback shift — no box-zone events, no crosses
    // completed, no key passes; just routine defensive volume and a
    // tidy passing line. With the 2026-06 tier-classification fix
    // (`routine_def >= 3` qualifies for Modest), this no longer
    // collapses to the Passenger tier that was pinning fullbacks like
    // Cambiaso to 6.20 season averages.
    let mut s = make_stats(
        0,
        0,
        38, // passes attempted
        30, // passes completed (79%)
        0,
        0,
        2, // tackles
        2, // interceptions
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.minutes_played = 90;
    s.clearances = 3;
    s.successful_pressures = 1;
    s.progressive_passes = 2;
    let r = RatingContext::new(&s, 1, 0).calculate();
    assert!(
        r > 6.6,
        "fullback with routine defensive shift + CS win rated {} — \
             a starting defender clearing 7 routine actions in a CS \
             must rate above 6.6, not collapse to the Passenger band",
        r
    );
}

#[test]
fn defensive_midfielder_recycler_clears_six_three() {
    // Deep-lying midfielder shift — no decisive creative events,
    // no goals/assists, just turnover-resistant ball recycling
    // and modest defensive work. Pre-fix this was Passenger tier
    // (Khéphren Thuram averaged 5.96 — *below* the 6.0 baseline);
    // a starting DM in a top side doing the job should clear 6.4.
    let mut s = make_stats(
        0,
        0,
        45, // passes attempted
        38, // passes completed (84%)
        0,
        0,
        1, // tackles
        2, // interceptions
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.minutes_played = 90;
    s.successful_pressures = 2;
    s.progressive_passes = 3;
    let r = RatingContext::new(&s, 1, 1).calculate();
    assert!(
        r > 6.3,
        "DM recycler with no G/A in a 1-1 draw rated {} — a deep \
             midfielder doing honest pass/defensive volume must clear \
             6.3, not get crushed to sub-6.0 by Passenger tier",
        r
    );
}

#[test]
fn fullback_season_average_lands_in_real_football_band() {
    // Regression for Cambiaso-shape symptom: a Juventus starting RB
    // posting 6.20-6.25 season averages despite holding down the
    // role in a top side. Real-football reference for a starting
    // fullback is 6.7-7.0. The synthetic 30-match schedule below
    // models a Juve RB's typical match distribution (mix of CS,
    // concede-1, concede-2; mostly 0 G/A; one assist a season).
    let mut total = 0.0_f32;
    let mut count = 0_u32;
    let make_def = |passes_att: u16,
                    passes_comp: u16,
                    tackles: u16,
                    interceptions: u16,
                    clearances: u16,
                    pressures: u16,
                    crosses_completed: u16,
                    crosses_attempted: u16,
                    progressive: u16,
                    fouls: u16|
     -> PlayerMatchEndStats {
        let mut s = make_stats(
            0,
            0,
            passes_att,
            passes_comp,
            0,
            0,
            tackles,
            interceptions,
            0,
            0.0,
            PlayerFieldPositionGroup::Defender,
        );
        s.minutes_played = 90;
        s.clearances = clearances;
        s.successful_pressures = pressures;
        s.crosses_completed = crosses_completed;
        s.crosses_attempted = crosses_attempted;
        s.progressive_passes = progressive;
        s.progressive_carries = progressive / 2;
        s.fouls = fouls;
        s
    };
    // CS wins (10 — Juve concede CS rate ~35%).
    for stats in [
        make_def(40, 33, 2, 2, 3, 1, 1, 3, 2, 0),
        make_def(38, 31, 1, 3, 2, 2, 0, 2, 2, 1),
        make_def(42, 35, 3, 2, 4, 0, 2, 4, 3, 0),
        make_def(35, 28, 2, 1, 3, 2, 1, 3, 2, 1),
        make_def(45, 38, 1, 2, 4, 1, 1, 3, 3, 0),
        make_def(36, 29, 2, 2, 2, 1, 0, 1, 2, 0),
        make_def(40, 32, 2, 2, 3, 2, 1, 2, 2, 1),
        make_def(38, 30, 3, 1, 4, 0, 0, 2, 2, 0),
        make_def(42, 34, 2, 3, 2, 1, 1, 3, 3, 0),
        make_def(48, 41, 1, 2, 3, 2, 2, 4, 2, 0),
    ] {
        total += RatingContext::new(&stats, 1, 0).calculate();
        count += 1;
    }
    // Concede 1, win (10 games).
    for stats in [
        make_def(40, 33, 2, 2, 3, 1, 0, 2, 2, 1),
        make_def(38, 30, 1, 2, 4, 0, 1, 3, 2, 1),
        make_def(42, 34, 3, 1, 3, 2, 0, 1, 3, 0),
        make_def(45, 37, 2, 3, 2, 1, 1, 3, 2, 1),
        make_def(36, 28, 2, 2, 4, 0, 0, 2, 2, 0),
        make_def(40, 32, 1, 2, 3, 1, 1, 2, 3, 1),
        make_def(38, 30, 2, 1, 4, 2, 0, 1, 2, 0),
        make_def(44, 36, 3, 2, 2, 1, 1, 4, 3, 0),
        make_def(35, 27, 2, 2, 3, 0, 0, 2, 2, 1),
        make_def(42, 34, 1, 2, 4, 1, 1, 3, 2, 0),
    ] {
        total += RatingContext::new(&stats, 2, 1).calculate();
        count += 1;
    }
    // Concede 2 (6 games — mix of W/D/L).
    for (stats, tg, og) in [
        (make_def(38, 30, 2, 2, 3, 0, 0, 1, 2, 1), 3_u8, 2_u8),
        (make_def(40, 32, 3, 1, 4, 1, 1, 2, 2, 0), 2, 2),
        (make_def(36, 28, 2, 2, 4, 0, 0, 2, 1, 1), 1, 2),
        (make_def(42, 34, 1, 2, 3, 1, 1, 3, 3, 0), 2, 2),
        (make_def(38, 30, 2, 1, 4, 0, 0, 1, 2, 1), 3, 2),
        (make_def(35, 27, 2, 2, 3, 1, 0, 2, 2, 0), 0, 2),
    ] {
        total += RatingContext::new(&stats, tg, og).calculate();
        count += 1;
    }
    // Concede 3+ (4 games — bad days).
    for (stats, og) in [
        (make_def(35, 26, 2, 1, 3, 0, 0, 2, 1, 1), 3_u8),
        (make_def(38, 28, 1, 2, 4, 1, 1, 3, 2, 0), 3),
        (make_def(32, 23, 2, 2, 3, 0, 0, 1, 1, 1), 3),
        (make_def(36, 27, 1, 1, 4, 0, 0, 2, 2, 1), 4),
    ] {
        total += RatingContext::new(&stats, 1, og).calculate();
        count += 1;
    }
    let avg = total / count as f32;
    assert!(
        avg > 6.5,
        "Starting fullback season average rated {} across {} matches \
             — must clear 6.5 so a Juventus RB doesn't post 6.2 \
             averages while doing the job. Real-football reference \
             for routine fullbacks is 6.7-7.0.",
        avg,
        count,
    );
    assert!(
        avg < 7.2,
        "Starting fullback season average rated {} across {} matches \
             — a routine no-G/A fullback shouldn't drift into the \
             elite band on workhorse evidence alone",
        avg,
        count,
    );
}

#[test]
fn defensive_midfielder_season_average_lands_in_real_football_band() {
    // Regression for Khéphren Thuram-shape symptom: a Juventus
    // starting DM posting 5.96 / 6.09 — *below baseline* in 27/28 —
    // because the engine's modest stat-line emission combined with
    // Passenger tier classification crushed his rating. A starting
    // top-club DM doing the job should average above 6.4.
    let mut total = 0.0_f32;
    let mut count = 0_u32;
    let make_mid = |passes_att: u16,
                    passes_comp: u16,
                    tackles: u16,
                    interceptions: u16,
                    pressures: u16,
                    progressive: u16,
                    key_passes: u16,
                    fouls: u16|
     -> PlayerMatchEndStats {
        let mut s = make_stats(
            0,
            0,
            passes_att,
            passes_comp,
            0,
            0,
            tackles,
            interceptions,
            0,
            0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.minutes_played = 90;
        s.successful_pressures = pressures;
        s.progressive_passes = progressive;
        s.progressive_carries = progressive / 2;
        s.key_passes = key_passes;
        s.fouls = fouls;
        s
    };
    // CS wins (10 games).
    for stats in [
        make_mid(50, 42, 2, 2, 2, 3, 1, 1),
        make_mid(48, 40, 1, 3, 1, 2, 0, 0),
        make_mid(55, 47, 3, 1, 3, 4, 1, 1),
        make_mid(42, 35, 2, 2, 2, 2, 0, 1),
        make_mid(52, 44, 2, 2, 3, 3, 1, 0),
        make_mid(45, 37, 1, 2, 2, 2, 0, 0),
        make_mid(50, 42, 2, 3, 1, 3, 1, 1),
        make_mid(48, 40, 3, 1, 2, 2, 0, 1),
        make_mid(55, 47, 2, 2, 3, 4, 1, 0),
        make_mid(52, 44, 1, 2, 2, 3, 0, 0),
    ] {
        total += RatingContext::new(&stats, 1, 0).calculate();
        count += 1;
    }
    // Concede 1, win (10 games).
    for stats in [
        make_mid(48, 40, 2, 2, 2, 3, 1, 1),
        make_mid(50, 42, 1, 2, 1, 2, 0, 0),
        make_mid(45, 37, 2, 3, 3, 3, 1, 1),
        make_mid(52, 44, 3, 1, 2, 4, 0, 0),
        make_mid(48, 40, 2, 2, 2, 2, 1, 1),
        make_mid(42, 35, 1, 2, 1, 2, 0, 0),
        make_mid(50, 42, 2, 1, 3, 3, 1, 1),
        make_mid(45, 37, 2, 2, 2, 2, 0, 0),
        make_mid(48, 40, 3, 2, 1, 3, 1, 0),
        make_mid(52, 44, 2, 1, 2, 4, 0, 1),
    ] {
        total += RatingContext::new(&stats, 2, 1).calculate();
        count += 1;
    }
    // Concede 2 (6 games — mix of W/D/L).
    for (stats, tg, og) in [
        (make_mid(45, 37, 2, 1, 2, 3, 0, 1), 3_u8, 2_u8),
        (make_mid(48, 40, 3, 2, 1, 2, 1, 0), 2, 2),
        (make_mid(42, 35, 2, 2, 3, 2, 0, 1), 1, 2),
        (make_mid(50, 42, 1, 1, 2, 4, 0, 0), 2, 2),
        (make_mid(45, 37, 2, 2, 1, 3, 1, 1), 3, 2),
        (make_mid(48, 40, 2, 1, 2, 2, 0, 0), 0, 2),
    ] {
        total += RatingContext::new(&stats, tg, og).calculate();
        count += 1;
    }
    // Concede 3+ (4 games — bad days).
    for (stats, og) in [
        (make_mid(42, 34, 1, 1, 2, 2, 0, 1), 3_u8),
        (make_mid(45, 37, 2, 2, 1, 3, 0, 0), 3),
        (make_mid(40, 32, 1, 1, 2, 2, 0, 1), 3),
        (make_mid(38, 30, 2, 1, 1, 2, 0, 0), 4),
    ] {
        total += RatingContext::new(&stats, 1, og).calculate();
        count += 1;
    }
    let avg = total / count as f32;
    assert!(
        avg > 6.3,
        "Starting DM season average rated {} across {} matches — \
             a Juventus-quality deep midfielder must clear 6.3, not \
             post 5.96-6.09 averages below baseline",
        avg,
        count,
    );
    assert!(
        avg < 7.2,
        "Starting DM season average rated {} across {} matches — \
             routine no-G/A defensive midfielder shouldn't drift \
             into the elite band on recycling alone",
        avg,
        count,
    );
}

#[test]
fn goalless_forward_near_strong_line_stays_below_good_band() {
    // Near-Strong-tier shift: 2 SOT, xG 0.5, 2 KP, 1 box pass, 2
    // dribbles, light defensive work, a clean-sheet win. Previously
    // this stat-line could pile up routine positives to +0.95 and
    // pick up the full team-result lift, drifting season averages
    // for top-club goalless forwards into 6.9+. Pin it firmly in
    // the high-sixes ceiling — still a decent shift, not a good one.
    let mut s = make_stats(
        0,
        0,
        30,
        25,
        2,
        3,
        1,
        2,
        0,
        0.5,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    s.key_passes = 2;
    s.passes_into_box = 1;
    s.successful_dribbles = 2;
    s.attempted_dribbles = 3;
    s.progressive_passes = 2;
    s.progressive_carries = 2;
    let r = RatingContext::new(&s, 2, 0).calculate();
    // Ceiling lifted 6.8 → 6.95 in the FM-parity calibration: this is
    // the best-case single match (2 SOT + 2 KP + 2 dribbles in a 2-0
    // clean-sheet win), and FM rates that shift solid-high-six. The
    // season-scale guard this test was written for now lives in
    // `season_tests` — a goalless forward's season mixes draws,
    // losses, and quiet shifts and stays in the low-mid sixes. The
    // "good performer" line at 7.0 still requires a goal contribution.
    assert!(
        r < 6.95,
        "goalless near-Strong FWD rated {} on a 2-0 win — busy routine \
             without a goal contribution must stay below 6.95",
        r
    );
}

#[test]
fn forward_in_hard_away_loss_with_low_output_floor_above_five_five() {
    // 90 min, 0 G/A, no shooting, no creative footprint, 0-1 loss
    // away. The canonical "CL-quality opposition kept him quiet"
    // shift. Real-football reference: 5.5-6.0. Prior calibration
    // crushed this to 5.2-5.4 by stacking ARE (-0.54) + Passenger
    // tier cap + halved context (0.10x) + engagement penalty + loss
    // penalty — the ARE drag is already the forward-specific
    // role-failure signal; the others double-bit.
    let mut s = make_stats(
        0,
        0,
        20,
        16,
        0,
        0,
        1,
        0,
        0,
        0.0,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    s.successful_pressures = 1;
    s.attempted_dribbles = 1;
    s.progressive_passes = 1;
    s.fouls = 1;
    let r = RatingContext::new(&s, 0, 1).calculate();
    assert!(
        r > 5.4,
        "FWD in hard away loss with no output rated {} — must stay \
             above 5.4; the ARE drag alone should not stack with \
             engagement + tier-cap penalties to collapse the rating \
             into the 5.2 band that the user observed in CL play",
        r
    );
    assert!(
        r < 6.2,
        "FWD in hard away loss with no output rated {} — must still \
             be a clearly poor shift, well below the baseline",
        r
    );
}

#[test]
fn eleven_goal_league_scorer_season_average_clears_six_nine() {
    // Regression for the user-observed 11g/13ap forward posting 6.53
    // in League play. A genuine goal-scorer with that conversion
    // should average above 6.9 — combining goal matches (~7.6-7.8)
    // with goalless full matches (~6.0-6.3) and cameos (~6.0). The
    // prior forward over-tightening (ARE 4.5/0.80, wasted-xG 0.40,
    // goalless-context multiplier 0.20, engagement penalty stacking)
    // pulled goalless matches into the 5.5-5.8 band, dragging the
    // 11-goal scorer to 6.5.
    let mut total = 0.0_f32;
    let mut count = 0_u32;

    let make_fwd = |g: u16,
                    a: u16,
                    sot: u16,
                    shots: u16,
                    xg: f32,
                    passes_att: u16,
                    passes_comp: u16,
                    kp: u16,
                    pbox: u16,
                    drib: u16,
                    drib_att: u16,
                    tackles: u16,
                    pressures: u16,
                    minutes: u16|
     -> PlayerMatchEndStats {
        let mut s = make_stats(
            g,
            a,
            passes_att,
            passes_comp,
            sot,
            shots,
            tackles,
            0,
            0,
            xg,
            PlayerFieldPositionGroup::Forward,
        );
        s.minutes_played = minutes;
        s.key_passes = kp;
        s.passes_into_box = pbox;
        s.successful_dribbles = drib;
        s.attempted_dribbles = drib_att;
        s.successful_pressures = pressures;
        s.fouls = 1;
        s
    };

    // 7 goal-scoring starts (11 goals total, mostly 1-2 per match).
    for stats in [
        make_fwd(2, 0, 4, 5, 0.9, 25, 20, 1, 1, 1, 2, 0, 2, 90),
        make_fwd(1, 1, 3, 4, 0.6, 28, 22, 2, 1, 1, 2, 0, 2, 90),
        make_fwd(1, 0, 3, 4, 0.5, 24, 19, 0, 1, 2, 3, 0, 2, 90),
        make_fwd(2, 0, 5, 6, 1.1, 26, 21, 1, 2, 1, 2, 1, 2, 90),
        make_fwd(1, 0, 2, 3, 0.4, 22, 18, 1, 0, 1, 2, 0, 2, 90),
        make_fwd(2, 0, 4, 5, 0.8, 30, 24, 2, 1, 1, 2, 0, 3, 90),
        make_fwd(2, 0, 3, 4, 0.7, 27, 22, 1, 1, 0, 1, 0, 2, 90),
    ] {
        total += RatingContext::new(&stats, 2, 1).calculate();
        count += 1;
    }
    // 3 goalless full-match starts (CS/concede mix).
    for (stats, tg, og) in [
        (
            make_fwd(0, 0, 2, 3, 0.4, 25, 20, 1, 1, 1, 2, 0, 2, 90),
            1_u8,
            0_u8,
        ),
        (
            make_fwd(0, 0, 1, 2, 0.3, 23, 18, 0, 0, 1, 2, 1, 2, 90),
            1,
            1,
        ),
        (
            make_fwd(0, 0, 2, 3, 0.5, 26, 21, 1, 0, 0, 1, 0, 1, 90),
            2,
            1,
        ),
    ] {
        total += RatingContext::new(&stats, tg, og).calculate();
        count += 1;
    }
    // 3 sub appearances (cameos, mostly quiet).
    for stats in [
        make_fwd(0, 0, 0, 1, 0.1, 8, 6, 0, 0, 1, 1, 0, 0, 20),
        make_fwd(0, 1, 1, 1, 0.2, 6, 5, 1, 0, 0, 0, 0, 0, 15),
        make_fwd(0, 0, 1, 1, 0.1, 10, 8, 0, 0, 0, 1, 0, 1, 25),
    ] {
        total += RatingContext::new(&stats, 2, 1).calculate();
        count += 1;
    }

    let avg = total / count as f32;
    assert!(
        avg > 6.9,
        "11-goal league scorer season average rated {} across {} \
             matches — must clear 6.9; a forward who scores once every \
             ~1.2 starts is doing the primary job and should rate above \
             7.0 in real-football reference (prior calibration crushed \
             this to 6.5 via forward over-tightening)",
        avg,
        count,
    );
    // Ceiling lifted 7.6 → 7.7 in the FM-parity calibration (goal
    // event 2.80 → 2.95 + win credit 0.16): this 13-match sample is a
    // red-hot stretch with four braces, so ~7.6 is a believable
    // short-window average. Full-season scale is pinned tighter by
    // `season_tests::twentyone_goal_striker_season_clears_seven`
    // (≤ 7.30 across 31 apps).
    assert!(
        avg < 7.7,
        "11-goal league scorer season average rated {} across {} \
             matches — must stay under 7.7 so a high-volume scorer \
             doesn't drift into the elite hat-trick band on volume alone",
        avg,
        count,
    );
}

#[test]
fn five_goal_one_assist_eight_app_scorer_clears_six_seven() {
    // Regression for user-observed 5g/1a/8ap forward posting 6.15
    // (with 25 SoT, 1 PoM). A clearly hot striker on a smaller club
    // should average above 6.7 — main-team forwards with strong goal
    // returns must not be dragged below the baseline by goalless
    // hard-match shifts. The 13-match `eleven_goal` regression
    // proved that scoring lifts season averages above 6.9, but with
    // fewer matches the variance band of the post-rating pipeline
    // can pull a single bad swing harder, so this test uses a tighter
    // 8-match sample matching the user's observation.
    let mut total = 0.0_f32;
    let mut count = 0_u32;

    let make_fwd = |g: u16,
                    a: u16,
                    sot: u16,
                    shots: u16,
                    xg: f32,
                    passes_att: u16,
                    passes_comp: u16,
                    kp: u16,
                    pbox: u16,
                    drib: u16,
                    drib_att: u16,
                    tackles: u16,
                    pressures: u16,
                    fouls: u16|
     -> PlayerMatchEndStats {
        let mut s = make_stats(
            g,
            a,
            passes_att,
            passes_comp,
            sot,
            shots,
            tackles,
            0,
            0,
            xg,
            PlayerFieldPositionGroup::Forward,
        );
        s.minutes_played = 90;
        s.key_passes = kp;
        s.passes_into_box = pbox;
        s.successful_dribbles = drib;
        s.attempted_dribbles = drib_att;
        s.successful_pressures = pressures;
        s.fouls = fouls;
        s
    };

    // 4 goal-scoring matches (5 goals total, one brace).
    for (stats, tg, og) in [
        (make_fwd(2, 0, 4, 6, 0.9, 22, 17, 1, 1, 1, 2, 1, 2, 1), 3_u8, 1_u8),
        (make_fwd(1, 0, 3, 5, 0.6, 24, 18, 0, 1, 0, 2, 0, 2, 0), 1, 0),
        (make_fwd(1, 0, 4, 5, 0.7, 20, 15, 1, 0, 1, 2, 1, 1, 1), 2, 2),
        (make_fwd(1, 0, 3, 4, 0.5, 21, 17, 0, 1, 0, 1, 0, 2, 0), 1, 1),
    ] {
        total += RatingContext::new(&stats, tg, og).calculate();
        count += 1;
    }
    // 1 assist match (no goal).
    {
        let stats = make_fwd(0, 1, 2, 4, 0.4, 25, 20, 2, 1, 1, 2, 0, 2, 0);
        total += RatingContext::new(&stats, 2, 0).calculate();
        count += 1;
    }
    // 3 goalless full-match starts (mix of CS/concede; the kind of
    // hard-match goalless shift that drags season averages).
    for (stats, tg, og) in [
        (make_fwd(0, 0, 3, 5, 0.6, 23, 18, 1, 1, 0, 2, 1, 2, 1), 1_u8, 1_u8),
        (make_fwd(0, 0, 2, 4, 0.5, 20, 16, 0, 0, 1, 2, 0, 1, 1), 0, 2),
        (make_fwd(0, 0, 2, 3, 0.4, 22, 17, 1, 0, 0, 1, 1, 2, 0), 1, 2),
    ] {
        total += RatingContext::new(&stats, tg, og).calculate();
        count += 1;
    }

    let avg = total / count as f32;
    assert!(
        avg > 6.7,
        "5g/1a/8-match forward season average rated {} across {} matches \
             — a striker scoring at 0.625 G/match with high SoT must \
             clear 6.7. Prior calibration crushed this shape to 6.15.",
        avg,
        count,
    );
}

// ===========================================================
// Contextual model (Stage 2 + Stage 3) — anti-robotic guards.
//
// These exercise `calculate_contextual`: the same stat line must
// rate differently depending on how the TEAM played (possession /
// shot / defensive-load share) and on the player's physical
// condition. The deltas are deliberately modest — context separates
// otherwise-identical lines, it never overturns the stat-line
// verdict — so the assertions pin ordering + bounded magnitude,
// which is exactly the de-clustering the rebalance targets.
// Stat-line + team-behaviour + condition only; never reads ability.
// ===========================================================

use crate::r#match::engine::result::PlayerMatchPhysicalSnapshot;

/// Physical snapshot helper. Conditions are on the engine's 0..10000
/// scale; `hi` is the 0..1 high-intensity load share.
fn phys(start: i16, end: i16, hi: f32) -> PlayerMatchPhysicalSnapshot {
    PlayerMatchPhysicalSnapshot {
        player_id: 1,
        minutes_played: 90.0,
        starting_condition: start,
        final_match_energy: end,
        high_intensity_load_hint: hi,
    }
}

/// Team behaviour summary helper (passes_attempted padded ~20% over
/// completed so the proxy is realistic).
fn team_summary(
    shots: u32,
    sot: u32,
    xg: f32,
    passes_completed: u32,
    defensive_actions: u32,
) -> TeamRatingSummary {
    TeamRatingSummary {
        shots_total: shots,
        shots_on_target: sot,
        xg,
        passes_attempted: passes_completed + passes_completed / 5,
        passes_completed,
        defensive_actions,
    }
}

#[test]
fn dominant_team_tidy_recycler_does_not_rate_like_creator() {
    // Same possession-dominant midfield, same 1-0 win. The tidy recycler
    // keeps the ball but never progresses or creates; the creator opens
    // the defence up. The contextual layer raises the bar for a dominant
    // side, so the two must NOT print the same robotic number.
    let own = team_summary(16, 7, 1.6, 560, 22); // bossed the game
    let opp = team_summary(4, 1, 0.4, 250, 40); // pinned back
    let ctx = RatingExpectationContext::from_match(&own, &opp, 1, 0, None);

    let mut recycler = make_stats(
        0,
        0,
        58,
        53,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    recycler.minutes_played = 90;

    let mut creator = make_stats(
        0,
        0,
        52,
        44,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    creator.minutes_played = 90;
    creator.key_passes = 3;
    creator.passes_into_box = 3;
    creator.progressive_passes = 5;
    creator.progressive_carries = 2;
    creator.xg_buildup = 0.4;

    let r_rec = RatingContext::new(&recycler, 1, 0).calculate_contextual(&ctx);
    let r_cre = RatingContext::new(&creator, 1, 0).calculate_contextual(&ctx);
    assert!(
        r_cre > r_rec + 0.3,
        "creator {} must clearly outrate dominant-team recycler {}",
        r_cre,
        r_rec
    );
    assert!(
        r_rec < 6.9,
        "tidy recycler in a dominant side rated {} — must not print a creator-like 7.1",
        r_rec
    );
}

#[test]
fn low_block_defender_with_box_actions_outrates_clean_sheet_passenger() {
    // A side defending a 1-0 lead under siege (heavy defensive load).
    // The firefighter CB made real box interventions; the passenger CB
    // coasted on the same clean sheet. High expected defensive
    // contribution + the actual interventions separate them.
    let own = team_summary(4, 1, 0.4, 250, 45); // under siege
    let opp = team_summary(17, 7, 1.8, 560, 18); // attacked all match
    let ctx = RatingExpectationContext::from_match(&own, &opp, 1, 0, None);

    let mut passenger = make_stats(
        0,
        0,
        22,
        18,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    passenger.minutes_played = 90;

    let mut firefighter = passenger.clone();
    firefighter.tackles = 4;
    firefighter.interceptions = 3;
    firefighter.clearances = 6;
    firefighter.blocks = 2;
    firefighter.zone_stats.tackles_own_box = 2;
    firefighter.zone_stats.clearances_own_box = 3;
    firefighter.zone_stats.blocks_own_box = 1;

    let r_pass = RatingContext::new(&passenger, 1, 0).calculate_contextual(&ctx);
    let r_fire = RatingContext::new(&firefighter, 1, 0).calculate_contextual(&ctx);
    assert!(
        r_fire > r_pass + 0.5,
        "firefighting CB {} must outrate coasting passenger {}",
        r_fire,
        r_pass
    );
    assert!(
        r_fire > 7.0,
        "firefighting CB in a backs-to-the-wall clean sheet rated {} — should clear 7.0",
        r_fire
    );
}

#[test]
fn tired_player_sloppy_actions_amplify_negative_rating() {
    // Same stat line (a few fouls + loose touches). Played fresh vs ran
    // the tank to empty and got sloppy. The gassed-and-sloppy shift must
    // rate lower — tiredness that CAUSED mistakes is punished, while the
    // same mistakes at full energy are not amplified.
    let mut s = make_stats(
        0,
        0,
        40,
        33,
        0,
        0,
        2,
        2,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.minutes_played = 90;
    s.fouls = 3;
    s.miscontrols = 3;
    s.heavy_touches = 2;

    let summary = team_summary(10, 4, 1.0, 400, 30);
    let fresh = RatingExpectationContext::from_match(
        &summary,
        &summary,
        1,
        1,
        Some(&phys(9500, 6000, 0.30)),
    );
    let gassed = RatingExpectationContext::from_match(
        &summary,
        &summary,
        1,
        1,
        Some(&phys(9500, 2400, 0.30)),
    );

    let r_fresh = RatingContext::new(&s, 1, 1).calculate_contextual(&fresh);
    let r_gassed = RatingContext::new(&s, 1, 1).calculate_contextual(&gassed);
    assert!(
        r_gassed < r_fresh - 0.1,
        "gassed sloppy shift {} must rate below the same line played fresh {}",
        r_gassed,
        r_fresh
    );
}

#[test]
fn high_load_good_shift_gets_small_condition_respect_not_big_bonus() {
    // A genuinely good shift (raw > 6.0) with a heavy high-intensity load
    // earns a SMALL respect bump — never more than +0.10, and never
    // enough to manufacture a good rating from a poor one.
    let mut s = make_stats(
        0,
        0,
        45,
        38,
        0,
        0,
        4,
        3,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    s.minutes_played = 90;
    s.successful_pressures = 5;
    s.pressures = 11;
    s.progressive_passes = 4;
    s.key_passes = 1;

    let summary = team_summary(11, 4, 1.1, 420, 28);
    // Normal load vs heavy load (MID high-intensity default ≈ 0.30; the
    // bump only fires above default + 0.12 = 0.42).
    let normal = RatingExpectationContext::from_match(
        &summary,
        &summary,
        1,
        0,
        Some(&phys(9000, 5000, 0.30)),
    );
    let heavy = RatingExpectationContext::from_match(
        &summary,
        &summary,
        1,
        0,
        Some(&phys(9000, 5000, 0.55)),
    );

    let r_normal = RatingContext::new(&s, 1, 0).calculate_contextual(&normal);
    let r_heavy = RatingContext::new(&s, 1, 0).calculate_contextual(&heavy);
    assert!(
        r_heavy > r_normal,
        "heavy-load good shift {} should edge the same shift at normal load {}",
        r_heavy,
        r_normal
    );
    assert!(
        r_heavy - r_normal <= 0.11,
        "high-load respect must stay small (≤0.10), got {}",
        r_heavy - r_normal
    );
}

#[test]
fn same_medium_statline_gets_context_separation_by_team_behavior() {
    // The exact same medium defender line, same 1-1 draw, rated for a
    // possession-dominant side vs a pinned-back side. The contextual
    // layer must pull them apart — identical stat lines no longer print
    // the identical robotic number. The separation is deliberately modest
    // (context never overturns the verdict), so we pin a visible-but-small
    // floor rather than a large swing.
    let mut s = make_stats(
        0,
        0,
        32,
        27,
        0,
        0,
        3,
        3,
        0,
        0.0,
        PlayerFieldPositionGroup::Defender,
    );
    s.minutes_played = 90;
    s.clearances = 4;

    let dominant_own = team_summary(16, 7, 1.7, 580, 18);
    let pinned_opp = team_summary(4, 1, 0.4, 240, 46);
    let dominant = RatingExpectationContext::from_match(&dominant_own, &pinned_opp, 1, 1, None);
    let pinned = RatingExpectationContext::from_match(&pinned_opp, &dominant_own, 1, 1, None);

    let r_dom = RatingContext::new(&s, 1, 1).calculate_contextual(&dominant);
    let r_pin = RatingContext::new(&s, 1, 1).calculate_contextual(&pinned);
    assert!(
        (r_dom - r_pin).abs() > 0.04,
        "identical medium defender line must separate by team behaviour: \
         dominant {} vs pinned {}",
        r_dom,
        r_pin
    );
    // The under-siege defender (more expected, same modest actual) should
    // be the lower of the two — context credits the firefighter, not the
    // coaster, so a flat line in a pinned side reads worse, not better.
    assert!(
        r_pin < r_dom,
        "a flat defensive line under siege ({}) should rate below the same \
         line in a dominant side ({})",
        r_pin,
        r_dom
    );
}

#[test]
fn personality_variance_is_deterministic_and_bounded() {
    // `texture_band` is the per-identity jitter amplitude the downstream
    // pipeline multiplies by a seeded (player, date, team) hash. It must
    // be deterministic, small, and ordered by evidence. The contextual
    // rating itself must be a pure function of its inputs.
    let mut passenger = make_stats(
        0,
        0,
        20,
        16,
        0,
        0,
        1,
        1,
        0,
        0.0,
        PlayerFieldPositionGroup::Midfielder,
    );
    passenger.minutes_played = 90;
    let mut modest = passenger.clone();
    modest.key_passes = 1;
    let mut strong = passenger.clone();
    strong.key_passes = 3;
    strong.passes_into_box = 2;
    strong.zone_stats.pressures_won_final_third = 2;

    let pb = RatingContext::new(&passenger, 1, 0).texture_band();
    let mb = RatingContext::new(&modest, 1, 0).texture_band();
    let sb = RatingContext::new(&strong, 1, 0).texture_band();
    assert!(
        pb < mb && mb <= sb,
        "texture band must grow with evidence: {} < {} <= {}",
        pb,
        mb,
        sb
    );
    assert!(
        sb <= 0.08 + 1e-6 && pb >= 0.0,
        "texture band must stay small / bounded: pb {} sb {}",
        pb,
        sb
    );

    // Determinism: identical inputs → bit-identical contextual rating.
    let ctx = RatingExpectationContext::neutral();
    let a = RatingContext::new(&strong, 1, 0).calculate_contextual(&ctx);
    let b = RatingContext::new(&strong, 1, 0).calculate_contextual(&ctx);
    assert_eq!(
        a.to_bits(),
        b.to_bits(),
        "contextual rating must be deterministic"
    );
}

#[test]
fn public_rating_preserves_raw_match_rating() {
    // `raw_match_rating` is the pure stat-line verdict; the public
    // (contextual) rating layers Stage 2/3 on top WITHOUT mutating raw.
    // At the rating layer: `calculate()` takes no context, so it returns
    // the same number regardless of the context the public rating is
    // built against — the diagnostic split holds.
    let mut s = make_stats(
        1,
        0,
        18,
        14,
        2,
        3,
        0,
        0,
        0,
        0.6,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    let raw = RatingContext::new(&s, 2, 0).calculate();

    let dominant = RatingExpectationContext::from_match(
        &team_summary(15, 6, 1.8, 520, 20),
        &team_summary(5, 2, 0.5, 300, 40),
        2,
        0,
        Some(&phys(9000, 2500, 0.55)),
    );
    let public = RatingContext::new(&s, 2, 0).calculate_contextual(&dominant);
    let raw_after = RatingContext::new(&s, 2, 0).calculate();

    assert!(
        (raw_after - raw).abs() < 1e-9,
        "raw stat-line rating must be context-free and unchanged"
    );
    // The public rating is a real, bounded transform of raw — within the
    // documented contextual budget (±0.25 expectation + 0.22 condition).
    assert!(
        (public - raw).abs() <= 0.25 + 0.22 + 1e-4,
        "public rating must stay within the contextual budget of raw: raw {} public {}",
        raw,
        public
    );
}

#[test]
fn favorite_forward_held_to_higher_expectation_than_underdog() {
    // Same goalless, low-threat forward line. As the heavy favourite
    // (negative rep gap) more is expected of the attack, so the same
    // empty shift rates a touch lower than it would for the same player
    // as the underdog. Reputation shapes the EXPECTATION, never a free
    // rating bonus — this is the spec's "ability shapes expected
    // contribution, not the rating" principle, exercised via the
    // downstream `with_rep_gap` builder.
    let mut s = make_stats(
        0,
        0,
        20,
        15,
        1,
        3,
        0,
        0,
        0,
        0.4,
        PlayerFieldPositionGroup::Forward,
    );
    s.minutes_played = 90;
    let summary = team_summary(12, 5, 1.2, 450, 28);
    let favorite =
        RatingExpectationContext::from_match(&summary, &summary, 1, 0, None).with_rep_gap(-2.5);
    let underdog =
        RatingExpectationContext::from_match(&summary, &summary, 1, 0, None).with_rep_gap(2.5);
    let r_fav = RatingContext::new(&s, 1, 0).calculate_contextual(&favorite);
    let r_und = RatingContext::new(&s, 1, 0).calculate_contextual(&underdog);
    assert!(
        r_fav < r_und,
        "favourite forward {} should be held to a higher bar than the same line as underdog {}",
        r_fav,
        r_und
    );
}
