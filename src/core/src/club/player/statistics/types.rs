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
    /// (scouting, awards, contracts, selection, etc.) should call
    /// [`Self::average_rating_realistic`] instead — a player with nine
    /// 8.2 appearances will read 8.20 here but ~7.25 there. See
    /// [`Self::weighted_average_rating`] / [`Self::average_rating_raw`]
    /// for the explicit accessors.
    ///
    /// This is a *judgement* currency, not a display one: nothing the
    /// user reads should come from here or from the regressed variant.
    /// See [`Self::mean_rating`] for the number that goes on screen.
    pub average_rating: f32,
    /// Σ(effective_rating × minutes_weight). Paired with [`rating_weight`]
    /// to produce a minutes-weighted season average — a 10-minute cameo
    /// no longer counts the same as a 90-minute start.
    pub rating_points: f32,
    /// Σ(minutes_weight). Acts as the denominator for [`rating_points`].
    pub rating_weight: f32,
    /// Σ(effective_rating) over rated appearances, unweighted. Paired
    /// with [`rating_matches`] to produce the plain arithmetic mean that
    /// every user-facing surface displays — Football Manager's `Av Rat`,
    /// where a 5-minute cameo counts exactly as much as a 90-minute
    /// start and no regression is applied. Kept alongside the weighted
    /// ledger rather than replacing it, because the two answer different
    /// questions: this one reports what happened, the weighted/regressed
    /// pair judges how much to believe it.
    pub rating_sum: f32,
    /// Count of rated appearances. Denominator for [`rating_sum`]. Only
    /// incremented by [`Self::record_match_rating`] for ratings that
    /// pass the validity guard, so unused subs and the 0.0 "no rating"
    /// sentinel never enter the mean.
    pub rating_matches: u16,

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

/// League appearances a player made for a team OTHER than his rostered
/// (active-spell) team this season — e.g. a reserve/Second-team player
/// borrowed up to the main team for a top-division fixture, or a main
/// player fielded for the club's lower-division "2" side. Stored per team
/// (the full identity is captured at match time, the one thing only the
/// match knows) on [`super::history::PlayerStatisticsHistory`], so career
/// history can show a separate row for every team the player turned out
/// for in a season instead of folding both teams' games under the active
/// spell. The projection renders it directly for the in-progress season;
/// the season-end snapshot freezes it into the canonical `season_ledger`
/// like every other completed-season record.
#[derive(Debug, Clone, Default)]
pub struct SecondaryTeamStatistics {
    /// Season this slice belongs to (`Season::from_date(match_date)`),
    /// so a missed season-end can still freeze each slice under the right
    /// year rather than collapsing them together.
    pub season_start_year: u16,
    pub team_slug: String,
    pub team_name: String,
    pub team_reputation: u16,
    pub league_slug: String,
    pub league_name: String,
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
        self.rating_sum += effective_rating;
        self.rating_matches += 1;
        self.average_rating = self.weighted_average_rating();
    }

    /// The displayed season average: a plain arithmetic mean over every
    /// rated appearance, exactly like Football Manager's `Av Rat`. No
    /// minute weighting, no regression toward a positional neutral — a
    /// player whose three matches read 7.2 / 8.0 / 7.4 displays 7.53,
    /// the number a reader can reconcile against the match list sitting
    /// next to it.
    ///
    /// Small samples are kept honest the way FM keeps them honest —
    /// with a minimum-appearances filter on ranking surfaces (see
    /// `leagues::get`) and by *sorting* those surfaces on
    /// [`Self::average_rating_realistic`] — not by bending the number
    /// under the reader's eyes.
    ///
    /// Falls back to the weighted ledger when no unweighted samples
    /// exist: imported career history and test fixtures that poke
    /// `rating_points` / `average_rating` directly never populate it.
    #[inline]
    pub fn mean_rating(&self) -> f32 {
        if self.rating_matches > 0 {
            self.rating_sum / self.rating_matches as f32
        } else {
            self.weighted_average_rating()
        }
    }

    /// [`Self::mean_rating`] formatted for display, "-" for zero.
    #[inline]
    pub fn mean_rating_str(&self) -> String {
        Self::format_rating(self.mean_rating())
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
        RatingLedger::regress(raw, effective, position)
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

    /// Preferred public API for UI / table rendering: the plain
    /// arithmetic mean over rated appearances, formatted.
    ///
    /// Deliberately takes no position argument. It used to render
    /// [`Self::average_rating_realistic`], which meant a page could
    /// print an average that contradicted the very match list it sat
    /// above — three matches of 7.2 / 8.0 / 7.4 displayed 6.79, because
    /// four fifths of the shown number was the positional neutral. FM
    /// never does this: `Av Rat` is the mean, and small samples are
    /// filtered off leaderboards rather than rewritten. Ranking
    /// surfaces still *sort* on the regressed value; only the printed
    /// number changed.
    #[inline]
    pub fn display_average_rating(&self) -> String {
        self.mean_rating_str()
    }

    /// Merge another stat set into this one (combine stints at same club in one season).
    /// Weighted-averages the rating, sums everything else.
    pub fn merge_from(&mut self, other: &PlayerStatistics) {
        // Snapshot BOTH ledgers for both sides before writing anything.
        // The legacy-synthesis branches read `average_rating`, `played`
        // and `played_subs`, all of which this method overwrites — so
        // reconstructing the mean ledger after the weighted one had
        // already rewritten `average_rating` folded the blended value
        // back in as if it were the original stint's.
        let (mut self_points, mut self_weight) = RatingLedger::weighted(self);
        let (other_points, other_weight) = RatingLedger::weighted(other);
        let (mut self_sum, mut self_matches) = RatingLedger::mean(self);
        let (other_sum, other_matches) = RatingLedger::mean(other);

        // Promote both sides to the weighted ledger before summing. Old
        // saves with `average_rating > 0` but `rating_weight == 0` get
        // synthesised weight from game counts so the merge stays
        // arithmetic-equivalent.
        self_points += other_points;
        self_weight += other_weight;
        self.rating_points = self_points;
        self.rating_weight = self_weight;
        self.average_rating = self.weighted_average_rating();

        // The unweighted mean ledger merges the same way, with its own
        // legacy synthesis so a stint imported from career history still
        // contributes its games to the combined denominator.
        self_sum += other_sum;
        self_matches += other_matches;
        self.rating_sum = self_sum;
        self.rating_matches = self_matches;

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
        let (a_points, a_weight) = RatingLedger::weighted(self);
        let (b_points, b_weight) = RatingLedger::weighted(other);
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
        let (_, a_weight) = RatingLedger::weighted(self);
        let (_, b_weight) = RatingLedger::weighted(other);
        RatingLedger::regress(raw, a_weight + b_weight, position)
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

    /// Combined plain mean across two stat sets (official + friendly) —
    /// every rated appearance on both sides counted once. The display
    /// counterpart to [`Self::combined_realistic_average_rating`], which
    /// stays the judgement/sort currency.
    pub fn combined_mean_rating(&self, other: &PlayerStatistics) -> f32 {
        let (a_sum, a_matches) = RatingLedger::mean(self);
        let (b_sum, b_matches) = RatingLedger::mean(other);
        let total = a_matches + b_matches;
        if total == 0 {
            return 0.0;
        }
        (a_sum + b_sum) / total as f32
    }

    /// Combined rating, formatted for display. Preferred public API for
    /// two-bucket display rows.
    pub fn combined_display_rating(&self, other: &PlayerStatistics) -> String {
        Self::format_rating(self.combined_mean_rating(other))
    }
}

/// Lower / upper bounds for a valid `effective_rating` supplied to
/// [`PlayerStatistics::record_match_rating`]. Anything outside this
/// range (typically the 0.0 sentinel used for "no rating computed")
/// is rejected so the ledger stays clean.
const RATING_MIN_INPUT: f32 = 1.0;
const RATING_MAX_INPUT: f32 = 10.0;

/// The two rating ledgers a [`PlayerStatistics`] carries, and the
/// constants that shape them.
///
/// A stat block records every rated appearance twice: once unweighted
/// (`rating_sum` / `rating_matches`) for the number the user reads, and
/// once minute-weighted (`rating_points` / `rating_weight`) for the
/// number the simulator judges on. This struct owns the reconstruction
/// rules both ledgers need when one side of a merge predates them —
/// career history imported from the database, or a fixture that set
/// `average_rating` directly.
pub(super) struct RatingLedger;

impl RatingLedger {
    /// Reliability parameter for sample-size regression: the cross-over
    /// point where the weighted average and the positional neutral
    /// contribute equally. ~12 effective full-match equivalents.
    const RELIABILITY_GAMES: f32 = 12.0;

    /// Positional neutral baseline used by reliability regression.
    /// Numbers reflect league-average per-90 ratings for each role —
    /// keepers and defenders sit slightly under, midfielders slightly
    /// above, forwards match the league mean because finishing-driven
    /// variance averages out.
    fn neutral(pos: PlayerFieldPositionGroup) -> f32 {
        match pos {
            PlayerFieldPositionGroup::Goalkeeper => 6.65,
            PlayerFieldPositionGroup::Defender => 6.55,
            PlayerFieldPositionGroup::Midfielder => 6.60,
            PlayerFieldPositionGroup::Forward => 6.55,
        }
    }

    /// Shrink `raw` toward the positional neutral by `effective` sample
    /// size. The single place the regression curve is expressed, so the
    /// per-bucket and combined variants can't drift apart.
    fn regress(raw: f32, effective: f32, pos: PlayerFieldPositionGroup) -> f32 {
        let neutral = Self::neutral(pos);
        let reliability = effective / (effective + Self::RELIABILITY_GAMES);
        neutral + (raw - neutral) * reliability
    }

    /// Synthesise the (rating_points, rating_weight) pair for merging
    /// when one side may be a legacy stat block whose ledger is zero.
    fn weighted(s: &PlayerStatistics) -> (f32, f32) {
        if s.rating_weight > 0.0 {
            (s.rating_points, s.rating_weight)
        } else if s.average_rating > 0.0 {
            // Treat starter games as full weight, sub games as 0.35 —
            // keeps the merge consistent with the new ledger's typical
            // magnitudes.
            let synth_weight = s.played as f32 + s.played_subs as f32 * 0.35;
            (s.average_rating * synth_weight, synth_weight)
        } else {
            (0.0, 0.0)
        }
    }

    /// Synthesise the (rating_sum, rating_matches) pair for merging when
    /// one side never went through
    /// [`PlayerStatistics::record_match_rating`]. Unweighted, so every
    /// appearance counts once regardless of start or cameo.
    fn mean(s: &PlayerStatistics) -> (f32, u16) {
        if s.rating_matches > 0 {
            (s.rating_sum, s.rating_matches)
        } else if s.average_rating > 0.0 {
            let games = s.total_games();
            (s.average_rating * games as f32, games)
        } else {
            (0.0, 0)
        }
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
    fn display_average_rating_is_the_plain_mean_not_the_regressed_value() {
        // A 9-app forward who rated 8.2 every week displays 8.20, the
        // way FM's `Av Rat` does. This helper used to render the
        // regressed ~7.26 instead, which meant the summary row
        // contradicted the match list printed beneath it.
        let mut s = PlayerStatistics::default();
        for _ in 0..9 {
            s.played += 1;
            s.record_match_rating(8.2, 90, true);
        }
        assert_eq!(s.display_average_rating(), "8.20");

        // The regressed value still exists for ranking and decision
        // surfaces — it just isn't what gets printed.
        let judged = s.average_rating_realistic(PlayerFieldPositionGroup::Forward);
        assert!(
            judged > 7.0 && judged < 7.6,
            "judgement currency should still regress to ~7.25, got {}",
            judged
        );
    }

    #[test]
    fn display_average_rating_reconciles_with_a_three_match_run() {
        // The reported case: three matches of 7.2 / 8.0 / 7.4 displayed
        // 6.79, because at three appearances only 3/15 of the distance
        // from the 6.60 midfielder neutral survived the regression. The
        // arithmetic mean is 7.53 and that is what a reader can check
        // against the fixtures listed next to it.
        let mut s = PlayerStatistics::default();
        for r in [7.2_f32, 8.0, 7.4] {
            s.played += 1;
            s.record_match_rating(r, 90, true);
        }
        assert_eq!(s.display_average_rating(), "7.53");
    }

    #[test]
    fn mean_rating_counts_a_cameo_as_a_full_appearance() {
        // FM applies no minute weighting to `Av Rat`: a 10-minute 6.0
        // off the bench counts exactly as much as a 90-minute 8.0, so
        // the mean is 7.00. The weighted ledger, which still feeds
        // judgement code, leans toward the start instead.
        let mut s = PlayerStatistics::default();
        s.played += 1;
        s.record_match_rating(8.0, 90, true);
        s.played_subs += 1;
        s.record_match_rating(6.0, 10, false);

        assert_eq!(s.mean_rating_str(), "7.00");
        assert!(
            s.average_rating_raw() > 7.2,
            "minute-weighted average should still favour the start, got {}",
            s.average_rating_raw()
        );
    }

    #[test]
    fn mean_rating_falls_back_to_legacy_average_without_samples() {
        // Career history imported from the database carries a season
        // average and game counts but no per-match ledger.
        let s = make_stats(30, 4, 12, 7.15);
        assert_eq!(s.mean_rating_str(), "7.15");
        assert_eq!(PlayerStatistics::default().mean_rating_str(), "-");
    }

    #[test]
    fn merge_from_sums_the_mean_ledger_unweighted() {
        // Two stints in one season: 2 starts at 8.0 and 2 cameos at
        // 6.0 merge to a flat 7.00 mean across four appearances,
        // regardless of the minutes behind them.
        let mut a = PlayerStatistics::default();
        let mut b = PlayerStatistics::default();
        for _ in 0..2 {
            a.played += 1;
            a.record_match_rating(8.0, 90, true);
            b.played_subs += 1;
            b.record_match_rating(6.0, 15, false);
        }
        a.merge_from(&b);
        assert_eq!(a.rating_matches, 4);
        assert_eq!(a.mean_rating_str(), "7.00");
        assert_eq!(a.display_average_rating(), "7.00");
    }

    #[test]
    fn merge_from_promotes_a_legacy_side_into_the_mean_ledger() {
        // Legacy side has `average_rating` + game counts only; the
        // merge has to synthesise 10 appearances at 7.0 for it so the
        // combined mean lands halfway to the new-style 6.0 side.
        let legacy = make_stats(10, 0, 0, 7.0);
        let mut modern = PlayerStatistics::default();
        for _ in 0..10 {
            modern.played += 1;
            modern.record_match_rating(6.0, 90, true);
        }
        let mut merged = legacy.clone();
        merged.merge_from(&modern);
        assert_eq!(merged.rating_matches, 20);
        assert_eq!(merged.mean_rating_str(), "6.50");
    }

    #[test]
    fn combined_mean_rating_counts_every_appearance_once() {
        // Official + friendly buckets blend by appearance count, not
        // by minutes: 3 at 8.0 and 1 at 6.0 gives 7.50.
        let mut official = PlayerStatistics::default();
        for _ in 0..3 {
            official.played += 1;
            official.record_match_rating(8.0, 90, true);
        }
        let mut friendly = PlayerStatistics::default();
        friendly.played_subs += 1;
        friendly.record_match_rating(6.0, 20, false);

        assert_eq!(official.combined_display_rating(&friendly), "7.50");
        assert_eq!(
            PlayerStatistics::default().combined_display_rating(&PlayerStatistics::default()),
            "-"
        );
    }

    #[test]
    fn rejected_ratings_never_enter_the_mean_ledger() {
        // The validity guard covers both ledgers: an unused sub, the
        // 0.0 "no rating" sentinel, and out-of-range values must not
        // move the displayed mean.
        let mut s = PlayerStatistics::default();
        s.record_match_rating(7.5, 0, false);
        s.record_match_rating(0.0, 90, true);
        s.record_match_rating(11.0, 90, true);
        s.record_match_rating(f32::NAN, 90, true);
        assert_eq!(s.rating_matches, 0);
        assert_eq!(s.rating_sum, 0.0);

        s.played += 1;
        s.record_match_rating(7.0, 90, true);
        assert_eq!(s.rating_matches, 1);
        assert_eq!(s.mean_rating_str(), "7.00");
    }

    #[test]
    fn gk_season_display_anchors_on_six_six_five_not_match_baseline() {
        // A keeper's season-display regression target is the GK positional
        // neutral (6.65), NOT the 6.0 per-match baseline. Clean sheets and
        // protected shutouts make even an ordinary keeper's season currency
        // sit in the high sixes, so a small-sample or middling GK must
        // regress toward ~6.65 — quietly dropping the anchor toward 6.0
        // would re-bury exactly the protected-shutout seasons the rating
        // model was fixed to reward. This pins the display path so a future
        // change can't silently undo that.
        //
        // A one-app sample regresses almost entirely onto the anchor:
        // reliability 1/(1+12) ≈ 0.077, so a flat 6.0 line displays
        // 6.65 + (6.0 - 6.65)*0.077 ≈ 6.60 — pulled UP toward 6.65, the
        // opposite of what a 6.0 anchor would do.
        let mut gk = PlayerStatistics::default();
        gk.played += 1;
        gk.record_match_rating(6.0, 90, true);
        let gk_anchored = gk.realistic_average_rating(PlayerFieldPositionGroup::Goalkeeper);
        assert!(
            gk_anchored > 6.55 && gk_anchored < 6.66,
            "1-app 6.0 GK regresses to {:.3} — the display anchor must be \
             the 6.65 GK neutral, not the 6.0 match baseline",
            gk_anchored
        );

        // The anchor is positional, not a flat 6.0: the identical one-app
        // 6.0 line for a lower-neutral position regresses to a lower value.
        let mut def = PlayerStatistics::default();
        def.played += 1;
        def.record_match_rating(6.0, 90, true);
        let def_anchored = def.realistic_average_rating(PlayerFieldPositionGroup::Defender);
        assert!(
            gk_anchored > def_anchored,
            "GK display anchor ({:.3}) must sit above the defender anchor \
             ({:.3}) — the regression neutral is position-aware",
            gk_anchored,
            def_anchored
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
