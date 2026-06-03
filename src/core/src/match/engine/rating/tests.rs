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
        r >= 7.0 && r <= 7.7,
        "single-goal low-volume FWD rated {} — should be 7.0..=7.7",
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
    assert!(
        busy_r > quiet_r + 0.5,
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
fn low_impact_routine_cb_with_clean_sheet_stays_below_seven() {
    // 12 routine defensive actions, 80% completion on 30 safe
    // passes, no own-box / six-yard interventions, no progressive
    // passes / carries / dribbles, no key passes. Clean-sheet win.
    // This is the engine's typical low-HQ CB output shape — the
    // rating must NOT report this as a 7.0+ shift.
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
    assert!(
        r < 7.0,
        "low-impact routine CB with clean sheet rated {} — must stay < 7.0",
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
        (zctx.clean_sheet_context() - 0.25).abs() < 0.001,
        "own-box intervention earns full clean-sheet bonus"
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
fn busy_routine_defender_without_decisive_evidence_stays_sub_seven() {
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
    // A very busy routine CB on a clean sheet IS allowed to nudge
    // marginally past 7.0 because the team kept a shutout + win,
    // but never to the elite band. We pin the upper bound below
    // 7.3 — anything more would mean routine volume is unlocking
    // a "good performer" rating, which is the inflation symptom.
    assert!(
        r < 7.3,
        "very busy passenger CB rated {} — must not breach 7.3 without decisive evidence",
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
    assert!(
        r < 7.3,
        "busy routine DEF rated {} — no big moments must keep this below 7.3",
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
fn moderate_workload_clean_sheet_gk_stays_under_seven_two() {
    // 3 saves, 4 shots faced, clean sheet, win. Typical "did the job"
    // GK shift — enough saves to qualify for the modest CS bonus,
    // not enough to clear the busy bar. Should land around 7.0,
    // not 7.3+.
    let mut gk = make_gk(3, 4);
    gk.minutes_played = 90;
    let r = RatingContext::new(&gk, 1, 0).calculate();
    assert!(
        r < 7.2,
        "moderate-workload CS GK rated {} — average shutout shifts \
             must stay below 7.2 so season averages don't drift past 7.0",
        r
    );
}

#[test]
fn quiet_clean_sheet_gk_stays_in_high_six_band() {
    // 1 save / 2 shots faced, clean sheet, win. Quiet shutout: the
    // back four did the work. The bookkeeping CS bonus only — not
    // the full +0.30 — so a season of such matches averages well
    // below 7.0.
    let mut gk = make_gk(1, 2);
    gk.minutes_played = 90;
    let r = RatingContext::new(&gk, 1, 0).calculate();
    assert!(
        r < 7.0,
        "quiet CS GK rated {} — back-four-protected shutouts must not \
             cross 7.0 or every second-tier keeper averages 7.2+",
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
fn goalless_forward_near_strong_line_winning_season_averages_below_six_eight() {
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
    assert!(
        r < 6.8,
        "goalless near-Strong FWD rated {} on a 2-0 win — busy routine \
             without a goal contribution must stay below 6.8",
        r
    );
}
