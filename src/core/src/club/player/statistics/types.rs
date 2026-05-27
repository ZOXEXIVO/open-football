use crate::PlayerFieldPositionGroup;

/// Info about a team context for recording history events.
#[derive(Debug, Clone)]
pub struct TeamInfo {
    pub name: String,
    pub slug: String,
    pub reputation: u16,
    pub league_name: String,
    pub league_slug: String,
}

#[derive(Debug, Clone, Default)]
pub struct PlayerStatistics {
    pub played: u16,
    pub played_subs: u16,

    pub goals: u16,
    pub assists: u16,
    pub penalties: u16,
    pub player_of_the_match: u8,
    pub yellow_cards: u8,
    pub red_cards: u8,

    pub shots_on_target: f32,
    pub tackling: f32,
    pub passes: u8,

    /// Minutes-weighted RAW season average — kept in sync with the
    /// ledger so legacy readers (and pre-ledger save data) still see a
    /// sensible number. Note: this is *raw* form, NOT the sample-size
    /// regressed value. Code that's making a season-long judgement
    /// (display, scouting, awards, contracts, etc.) should call
    /// [`Self::average_rating_realistic`] instead — a player with nine
    /// 8.2 appearances will read 8.20 here but ~7.25 there. See
    /// [`Self::weighted_average_rating`] / [`Self::average_rating_raw`]
    /// for the explicit accessors.
    pub average_rating: f32,
    /// Σ(effective_rating × minutes_weight). Paired with [`rating_weight`]
    /// to produce a minutes-weighted season average — a 10-minute cameo
    /// no longer counts the same as a 90-minute start.
    pub rating_points: f32,
    /// Σ(minutes_weight). Acts as the denominator for [`rating_points`].
    pub rating_weight: f32,

    pub conceded: u16,
    pub clean_sheets: u16,
}

/// One competition's slice of a player's cup statistics, tagged with the
/// competition it was earned in. Recorded at match time keyed by the
/// match's `league_slug`, so every cup the player features in keeps its
/// own line on the player overview — continental cups today, domestic
/// cups once they're modelled — instead of collapsing into a single
/// hardcoded row. The rolled-up [`Player::cup_statistics`] aggregate is
/// recomputed from these, so existing aggregate readers are unaffected.
#[derive(Debug, Clone, Default)]
pub struct CompetitionStatistics {
    /// Stable competition identifier — the match's `league_slug`
    /// (e.g. `"copa-libertadores"`, `"champions-league"`). The display
    /// layer resolves a localized name from it.
    pub competition_slug: String,
    pub statistics: PlayerStatistics,
}

impl PlayerStatistics {
    /// Total appearances (started + substitute)
    #[inline]
    pub fn total_games(&self) -> u16 {
        self.played + self.played_subs
    }

    /// Format any rating value for display (e.g. "6.75"), returns "-" for zero
    #[inline]
    pub fn format_rating(value: f32) -> String {
        if value == 0.0 {
            "-".to_string()
        } else {
            format!("{:.2}", value)
        }
    }

    /// Average rating formatted for display (e.g. "6.75"), returns "-" for zero
    #[inline]
    pub fn average_rating_str(&self) -> String {
        Self::format_rating(self.average_rating)
    }

    /// Record one match's contribution to the minutes-weighted rolling
    /// average. `minutes_played` is the actual on-pitch minutes; starters
    /// get a higher minimum weight than substitutes so a starter who was
    /// injured at minute 5 still influences the average more than a sub
    /// who came on at 85.
    ///
    /// Keeps the legacy `average_rating` field in sync so any downstream
    /// reader that hasn't migrated to the weighted points sees the same
    /// number — and so old save files (which only stored `average_rating`)
    /// still display correctly until enough matches feed the new ledger.
    pub fn record_match_rating(
        &mut self,
        effective_rating: f32,
        minutes_played: u16,
        is_starter: bool,
    ) {
        // Guard against bad inputs slipping into the ledger: out-of-range
        // ratings (NaN, the 0.0 sentinel used when no rating was assigned,
        // or anything outside [1.0, 10.0]) and zero-minute "appearances"
        // would otherwise quietly poison the season average. An unused
        // sub still gets booked in `played_subs`, but their nonexistent
        // rating should not be counted here.
        if !effective_rating.is_finite()
            || !(RATING_MIN_INPUT..=RATING_MAX_INPUT).contains(&effective_rating)
            || minutes_played == 0
        {
            return;
        }
        let raw = (minutes_played as f32 / 90.0).clamp(0.0, 1.0);
        let min_weight = if is_starter { 0.65 } else { 0.20 };
        let minutes_weight = raw.max(min_weight);
        self.rating_points += effective_rating * minutes_weight;
        self.rating_weight += minutes_weight;
        self.average_rating = self.weighted_average_rating();
    }

    /// Minutes-weighted average rating. Falls back to the legacy plain
    /// average when no weighted data is recorded yet (e.g. save data
    /// from before the rating-weight ledger was added).
    #[inline]
    pub fn weighted_average_rating(&self) -> f32 {
        if self.rating_weight > 0.0 {
            self.rating_points / self.rating_weight
        } else {
            self.average_rating
        }
    }

    /// Reliability-adjusted season average regressed toward a positional
    /// neutral. Small samples drift back to the league baseline; once
    /// enough minutes accumulate, the raw weighted average dominates.
    ///
    /// Use this for season awards, squad selection, scouting,
    /// development, and contract logic — anything that overreacts to a
    /// raw 8.2 over nine matches. Match-of-the-week / POM should keep
    /// using the raw rating because they're about individual match output.
    pub fn realistic_average_rating(&self, position: PlayerFieldPositionGroup) -> f32 {
        let raw = self.weighted_average_rating();
        if raw <= 0.0 {
            return 0.0;
        }
        // Synthesise an effective full-match equivalent. The weighted
        // ledger uses minute-weight in [0.20, 1.00] (clamped), so a
        // starter season with N games sits near N; a sub-only season
        // is naturally compressed below.
        let effective = if self.rating_weight > 0.0 {
            self.rating_weight
        } else {
            // Backward compatibility: pre-ledger saves only had
            // `average_rating` and game counts. Treat starter games as
            // full weight and substitute games as 0.35 effective.
            self.played as f32 + self.played_subs as f32 * 0.35
        };
        let neutral = neutral_rating(position);
        let reliability = effective / (effective + RELIABILITY_GAMES);
        neutral + (raw - neutral) * reliability
    }

    /// Reliability-adjusted average formatted for display, returns "-" for zero.
    #[inline]
    pub fn realistic_average_rating_str(&self, position: PlayerFieldPositionGroup) -> String {
        Self::format_rating(self.realistic_average_rating(position))
    }

    /// Raw minutes-weighted season average. Alias for
    /// [`Self::weighted_average_rating`] under a name that makes the
    /// raw-vs-realistic distinction explicit at the call site. Use for
    /// single-match-relative comparisons (form deltas, weekly awards),
    /// NOT for long-form judgements.
    #[inline]
    pub fn average_rating_raw(&self) -> f32 {
        self.weighted_average_rating()
    }

    /// Reliability-adjusted season average. Alias for
    /// [`Self::realistic_average_rating`] under a name that makes the
    /// raw-vs-realistic distinction explicit at the call site. Use for
    /// any decision that should not overreact to a small-sample 8.2.
    #[inline]
    pub fn average_rating_realistic(&self, position: PlayerFieldPositionGroup) -> f32 {
        self.realistic_average_rating(position)
    }

    /// Reliability-adjusted average formatted for display. Preferred
    /// public API for UI / table rendering — the position argument
    /// forces callers to make the regression position-aware instead of
    /// silently using the raw value.
    #[inline]
    pub fn display_average_rating(&self, position: PlayerFieldPositionGroup) -> String {
        Self::format_rating(self.average_rating_realistic(position))
    }

    /// Merge another stat set into this one (combine stints at same club in one season).
    /// Weighted-averages the rating, sums everything else.
    pub fn merge_from(&mut self, other: &PlayerStatistics) {
        // Promote both sides to the weighted ledger before summing. Old
        // saves with `average_rating > 0` but `rating_weight == 0` get
        // synthesised weight from game counts so the merge stays
        // arithmetic-equivalent.
        let (mut self_points, mut self_weight) = ledger_for_merge(self);
        let (other_points, other_weight) = ledger_for_merge(other);
        self_points += other_points;
        self_weight += other_weight;
        self.rating_points = self_points;
        self.rating_weight = self_weight;
        self.average_rating = self.weighted_average_rating();

        self.played += other.played;
        self.played_subs += other.played_subs;
        self.goals += other.goals;
        self.assists += other.assists;
        self.penalties += other.penalties;
        self.player_of_the_match += other.player_of_the_match;
        self.yellow_cards += other.yellow_cards;
        self.red_cards += other.red_cards;
        self.shots_on_target += other.shots_on_target;
        self.tackling += other.tackling;
        self.passes += other.passes;
        self.conceded += other.conceded;
        self.clean_sheets += other.clean_sheets;
    }

    /// Combined raw minutes-weighted average rating across two stat
    /// sets (official + friendly). Uses the same legacy-fallback ledger
    /// synthesis as [`Self::merge_from`] so a 10-minute cameo doesn't
    /// count as a full game in the blend.
    ///
    /// Returns the *raw* weighted value — for display, prefer
    /// [`Self::combined_display_rating`] which applies the same
    /// sample-size regression as the per-bucket display helper.
    pub fn combined_weighted_average_rating(&self, other: &PlayerStatistics) -> f32 {
        let (a_points, a_weight) = ledger_for_merge(self);
        let (b_points, b_weight) = ledger_for_merge(other);
        let total_weight = a_weight + b_weight;
        if total_weight <= 0.0 {
            return 0.0;
        }
        (a_points + b_points) / total_weight
    }

    /// Reliability-adjusted combined average across two stat sets,
    /// applied with positional regression so a tiny sample doesn't
    /// dominate the displayed combined number. Used by the team-squad
    /// view to summarise "form across all matches" without the same
    /// 9-app-8.2 overreaction the per-bucket helper guards against.
    pub fn combined_realistic_average_rating(
        &self,
        other: &PlayerStatistics,
        position: PlayerFieldPositionGroup,
    ) -> f32 {
        let raw = self.combined_weighted_average_rating(other);
        if raw <= 0.0 {
            return 0.0;
        }
        let (_, a_weight) = ledger_for_merge(self);
        let (_, b_weight) = ledger_for_merge(other);
        let effective = a_weight + b_weight;
        let neutral = neutral_rating(position);
        let reliability = effective / (effective + RELIABILITY_GAMES);
        neutral + (raw - neutral) * reliability
    }

    /// Combined RAW weighted rating, formatted for display ("-" for
    /// zero). Kept for backward compatibility with views that don't
    /// have positional context; new display call sites should prefer
    /// [`Self::combined_display_rating`].
    pub fn combined_rating_str(&self, other: &PlayerStatistics) -> String {
        let combined = self.combined_weighted_average_rating(other);
        if combined <= 0.0 {
            return "-".to_string();
        }
        format!("{:.2}", combined)
    }

    /// Combined regressed rating, formatted for display. Preferred
    /// public API when a position is known.
    pub fn combined_display_rating(
        &self,
        other: &PlayerStatistics,
        position: PlayerFieldPositionGroup,
    ) -> String {
        Self::format_rating(self.combined_realistic_average_rating(other, position))
    }
}

/// Lower / upper bounds for a valid `effective_rating` supplied to
/// [`PlayerStatistics::record_match_rating`]. Anything outside this
/// range (typically the 0.0 sentinel used for "no rating computed")
/// is rejected so the ledger stays clean.
const RATING_MIN_INPUT: f32 = 1.0;
const RATING_MAX_INPUT: f32 = 10.0;

/// Positional neutral baseline used by reliability regression. Numbers
/// reflect league-average per-90 ratings for each role — keepers and
/// defenders sit slightly under, midfielders slightly above, forwards
/// match the league mean because finishing-driven variance averages out.
fn neutral_rating(pos: PlayerFieldPositionGroup) -> f32 {
    match pos {
        PlayerFieldPositionGroup::Goalkeeper => 6.65,
        PlayerFieldPositionGroup::Defender => 6.55,
        PlayerFieldPositionGroup::Midfielder => 6.60,
        PlayerFieldPositionGroup::Forward => 6.55,
    }
}

/// Reliability parameter for sample-size regression: the cross-over
/// point where the weighted average and the positional neutral
/// contribute equally. ~12 effective full-match equivalents.
const RELIABILITY_GAMES: f32 = 12.0;

/// Synthesise the (rating_points, rating_weight) pair for merging when
/// one side may be a legacy stat block whose ledger is still zero.
fn ledger_for_merge(s: &PlayerStatistics) -> (f32, f32) {
    if s.rating_weight > 0.0 {
        (s.rating_points, s.rating_weight)
    } else if s.average_rating > 0.0 {
        // Treat starter games as full weight, sub games as 0.35 — keeps
        // the merge consistent with the new ledger's typical magnitudes.
        let synth_weight = s.played as f32 + s.played_subs as f32 * 0.35;
        (s.average_rating * synth_weight, synth_weight)
    } else {
        (0.0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stats(played: u16, played_subs: u16, goals: u16, rating: f32) -> PlayerStatistics {
        let mut s = PlayerStatistics::default();
        s.played = played;
        s.played_subs = played_subs;
        s.goals = goals;
        s.average_rating = rating;
        s
    }

    // === PlayerStatistics ===

    #[test]
    fn total_games_sums_played_and_subs() {
        let s = make_stats(20, 5, 3, 7.0);
        assert_eq!(s.total_games(), 25);
    }

    #[test]
    fn total_games_zero_when_empty() {
        let s = PlayerStatistics::default();
        assert_eq!(s.total_games(), 0);
    }

    #[test]
    fn format_rating_two_decimals() {
        assert_eq!(PlayerStatistics::format_rating(6.5), "6.50");
        assert_eq!(PlayerStatistics::format_rating(7.123), "7.12");
        assert_eq!(PlayerStatistics::format_rating(0.0), "-");
    }

    #[test]
    fn average_rating_str_delegates_to_format_rating() {
        let s = make_stats(10, 0, 0, 6.75);
        assert_eq!(s.average_rating_str(), "6.75");
    }

    #[test]
    fn combined_rating_str_zero_games_returns_dash() {
        let a = PlayerStatistics::default();
        let b = PlayerStatistics::default();
        assert_eq!(a.combined_rating_str(&b), "-");
    }

    #[test]
    fn combined_rating_str_one_side_zero() {
        let a = make_stats(10, 0, 0, 7.0);
        let b = PlayerStatistics::default();
        assert_eq!(a.combined_rating_str(&b), "7.00");
    }

    #[test]
    fn combined_rating_str_weighted_average() {
        let a = make_stats(10, 0, 0, 7.0);
        let b = make_stats(10, 0, 0, 6.0);
        assert_eq!(a.combined_rating_str(&b), "6.50");
    }

    #[test]
    fn combined_rating_str_unequal_games() {
        let a = make_stats(30, 0, 0, 7.0);
        let b = make_stats(10, 0, 0, 6.0);
        assert_eq!(a.combined_rating_str(&b), "6.75");
    }

    #[test]
    fn combined_rating_str_subs_count_less_than_starts() {
        // Legacy stats path: a player whose 7.0s came mostly from
        // starts (8 + 2) and another whose 6.0s came half from
        // substitute appearances (5 + 5) should NOT blend exactly down
        // the middle — the cameo half drags less on the average. With
        // the synth-ledger fallback (starts = weight 1.0, subs = 0.35):
        //   a_weight = 8 + 2 * 0.35 = 8.7  → points = 60.9
        //   b_weight = 5 + 5 * 0.35 = 6.75 → points = 40.5
        //   combined ≈ 101.4 / 15.45 ≈ 6.56 (vs. 6.50 under game-count math)
        let a = make_stats(8, 2, 0, 7.0);
        let b = make_stats(5, 5, 0, 6.0);
        let result = a.combined_rating_str(&b);
        let parsed: f32 = result.parse().unwrap();
        assert!(
            parsed > 6.50 && parsed < 6.62,
            "expected combined ~6.56 with subs weighted at 0.35, got {}",
            result
        );
    }

    #[test]
    fn combined_rating_str_pure_starts_unchanged() {
        // No subs in either side → synthesised weight == total_games,
        // so the new ledger-aware blend matches the old game-count
        // arithmetic. Locks in backward compatibility with views that
        // don't track substitute appearances separately.
        let a = make_stats(10, 0, 0, 7.0);
        let b = make_stats(10, 0, 0, 6.0);
        assert_eq!(a.combined_rating_str(&b), "6.50");
        let c = make_stats(30, 0, 0, 7.0);
        let d = make_stats(10, 0, 0, 6.0);
        assert_eq!(c.combined_rating_str(&d), "6.75");
    }

    #[test]
    fn combined_rating_str_cameos_drag_less_than_starts() {
        // Player A: 5 full starts at 8.0 — proper top-rated season.
        // Player B: 5 ten-minute cameos at 8.0 — same per-match rating
        // but ~1/6 of the weight (0.20 min_weight per cameo).
        // Their blend should sit much closer to A's value than B's.
        let mut starter = PlayerStatistics::default();
        for _ in 0..5 {
            starter.played += 1;
            starter.record_match_rating(8.0, 90, true);
        }
        let mut cameo = PlayerStatistics::default();
        for _ in 0..5 {
            cameo.played_subs += 1;
            cameo.record_match_rating(6.0, 10, false);
        }
        let blend = starter.combined_weighted_average_rating(&cameo);
        // Both groups have rating-weight ledgers — 5 starts at ~1.0 vs
        // 5 cameos at ~0.20. The cameos should pull the blend by
        // ~5*0.20/(5+1) ≈ 17%, not by 50%.
        assert!(
            blend > 7.5,
            "expected starter average to dominate over cameos, got {}",
            blend
        );
    }

    // === Minutes-weighted ledger ===

    #[test]
    fn record_match_rating_starter_weights_higher_than_substitute() {
        let mut starter = PlayerStatistics::default();
        starter.played = 1;
        starter.record_match_rating(7.5, 90, true);

        let mut cameo = PlayerStatistics::default();
        cameo.played_subs = 1;
        cameo.record_match_rating(7.5, 10, false);

        // Same per-match rating, but a 90-minute start should carry
        // measurably more weight than a 10-minute cameo.
        assert!(
            starter.rating_weight > cameo.rating_weight + 0.3,
            "starter weight {} vs cameo weight {}",
            starter.rating_weight,
            cameo.rating_weight
        );
    }

    #[test]
    fn short_cameo_rating_has_lower_average_weight_than_starter() {
        // Two seasons: one player rests as starter for 89 minutes, the
        // other comes on for 10 minutes only. Both get 7.0 raw rating
        // per match. The season average for the cameo player should be
        // dragged less toward 7.0 than the starter's.
        let mut starter = PlayerStatistics::default();
        for _ in 0..5 {
            starter.played += 1;
            starter.record_match_rating(7.0, 90, true);
        }
        for _ in 0..5 {
            starter.played += 1;
            starter.record_match_rating(6.0, 90, true);
        }

        let mut cameo = PlayerStatistics::default();
        for _ in 0..5 {
            cameo.played_subs += 1;
            cameo.record_match_rating(7.0, 10, false);
        }
        for _ in 0..5 {
            cameo.played += 1;
            cameo.record_match_rating(6.0, 90, true);
        }

        // Both players got the same per-match raw ratings, but the
        // cameo player's 7.0 came in tiny doses. The starter's weighted
        // average should sit closer to 6.5 (equal weight); the cameo's
        // should lean toward 6.0 (where the actual minutes were).
        let starter_avg = starter.weighted_average_rating();
        let cameo_avg = cameo.weighted_average_rating();
        assert!(
            cameo_avg < starter_avg,
            "cameo avg {} should be < starter avg {}",
            cameo_avg,
            starter_avg
        );
        assert!(
            (starter_avg - 6.5).abs() < 0.05,
            "starter avg should average evenly: got {}",
            starter_avg
        );
        assert!(
            cameo_avg < 6.4,
            "cameo avg should lean toward the 90-min 6.0 matches: got {}",
            cameo_avg
        );
    }

    #[test]
    fn nine_games_two_goals_regresses_below_elite_average() {
        // Reproduces the reported bug: a young prospect with nine
        // appearances and a raw average rating of 8.2 should NOT show
        // as an 8.0+ regressed average. With reliability ≈ 9/(9+12) ≈
        // 0.43, the regressed value sits around 6.55 + (8.2-6.55)*0.43 ≈ 7.26.
        let mut s = PlayerStatistics::default();
        for _ in 0..9 {
            s.played += 1;
            s.record_match_rating(8.2, 90, true);
        }
        let regressed = s.realistic_average_rating(PlayerFieldPositionGroup::Forward);
        assert!(
            regressed > 7.0 && regressed < 7.6,
            "9-app 8.2-raw forward regressed = {} — expected ~7.2..7.4",
            regressed
        );
        // Sanity: the raw weighted average is still 8.2.
        assert!(
            (s.weighted_average_rating() - 8.2).abs() < 0.01,
            "weighted raw avg should be 8.2, got {}",
            s.weighted_average_rating()
        );
    }

    #[test]
    fn realistic_average_handles_legacy_stats_without_ledger() {
        // Legacy save data only has `average_rating` and game counts.
        // The realistic helper should still regress sensibly using a
        // synthesised weight from games.
        let s = make_stats(9, 0, 2, 8.2);
        let regressed = s.realistic_average_rating(PlayerFieldPositionGroup::Forward);
        assert!(
            regressed > 7.0 && regressed < 7.6,
            "legacy stats regression = {} — expected ~7.2..7.4",
            regressed
        );
    }

    #[test]
    fn realistic_average_full_season_barely_regresses() {
        // A full season of 30 starts at 7.6 should regress only mildly:
        // reliability ≈ 30/(30+12) = 0.71, so regressed ≈ 6.55 + 1.05*0.71 ≈ 7.30.
        let mut s = PlayerStatistics::default();
        for _ in 0..30 {
            s.played += 1;
            s.record_match_rating(7.6, 90, true);
        }
        let regressed = s.realistic_average_rating(PlayerFieldPositionGroup::Forward);
        assert!(
            regressed > 7.2 && regressed < 7.5,
            "30-app 7.6 forward regressed = {} — expected ~7.3",
            regressed
        );
    }

    #[test]
    fn merge_from_with_weighted_ledgers_preserves_average() {
        let mut a = PlayerStatistics::default();
        a.played = 5;
        for _ in 0..5 {
            a.record_match_rating(7.0, 90, true);
        }
        let mut b = PlayerStatistics::default();
        b.played = 5;
        for _ in 0..5 {
            b.record_match_rating(6.0, 90, true);
        }
        a.merge_from(&b);
        assert!(
            (a.weighted_average_rating() - 6.5).abs() < 0.01,
            "merged average should be 6.5, got {}",
            a.weighted_average_rating()
        );
        assert_eq!(a.played, 10);
    }

    #[test]
    fn record_match_rating_rejects_zero_minutes() {
        // An unused substitute still gets booked in `played_subs`, but
        // their nonexistent rating must not contaminate the ledger.
        let mut s = PlayerStatistics::default();
        s.record_match_rating(7.5, 0, false);
        assert_eq!(s.rating_points, 0.0);
        assert_eq!(s.rating_weight, 0.0);
        assert_eq!(s.average_rating, 0.0);
    }

    #[test]
    fn record_match_rating_rejects_zero_sentinel() {
        // 0.0 is the "no rating" sentinel from the engine — the guard
        // prevents it from being averaged into the ledger as a literal
        // "the player rated 0.0".
        let mut s = PlayerStatistics::default();
        s.record_match_rating(0.0, 90, true);
        assert_eq!(s.rating_weight, 0.0);
        assert_eq!(s.average_rating, 0.0);
    }

    #[test]
    fn record_match_rating_rejects_out_of_range_and_nan() {
        let mut s = PlayerStatistics::default();
        s.record_match_rating(11.0, 90, true);
        s.record_match_rating(-1.0, 90, true);
        s.record_match_rating(f32::NAN, 90, true);
        s.record_match_rating(f32::INFINITY, 90, true);
        assert_eq!(s.rating_weight, 0.0);
        assert_eq!(s.average_rating, 0.0);
    }

    #[test]
    fn display_average_rating_regresses_the_reported_bug() {
        // The exact reported scenario: a 9-app forward with raw 8.2
        // should NOT render 8.20 on any UI surface that uses the
        // public display helper. Regressed value is ~7.26.
        let mut s = PlayerStatistics::default();
        for _ in 0..9 {
            s.played += 1;
            s.record_match_rating(8.2, 90, true);
        }
        let displayed = s.display_average_rating(PlayerFieldPositionGroup::Forward);
        let parsed: f32 = displayed.parse().unwrap();
        assert!(
            parsed > 7.0 && parsed < 7.6,
            "9-app 8.2 forward should display ~7.25, got {}",
            displayed
        );
        // The raw weighted form is still 8.2 — exposed via the
        // explicitly-named accessor — so single-match callers and
        // debug DTOs can still see the raw value when they need it.
        let raw = s.average_rating_raw();
        assert!(
            (raw - 8.2).abs() < 0.01,
            "raw accessor should still expose 8.2, got {}",
            raw
        );
    }

    #[test]
    fn merge_from_promotes_legacy_into_ledger() {
        // a is legacy (no rating_weight), b is new-style. Merge should
        // synthesise a weight for a and produce a sensible blended avg.
        let a = make_stats(10, 0, 0, 7.0);
        let mut b = PlayerStatistics::default();
        b.played = 10;
        for _ in 0..10 {
            b.record_match_rating(6.0, 90, true);
        }
        let mut merged = a.clone();
        merged.merge_from(&b);
        let avg = merged.weighted_average_rating();
        assert!(
            (avg - 6.5).abs() < 0.05,
            "legacy + new merge avg = {} — expected ~6.5",
            avg
        );
    }
}
