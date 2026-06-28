//! Performance-breakout discovery — the "great results travel" channel.
//!
//! The demand-driven scout pipeline only finds targets when a club has an
//! open positional need, and the listed-star sweep only surfaces players
//! who have *advertised* availability. Neither reacts to a player whose
//! *form itself* should be drawing eyes: a 22-year-old striker banging in
//! goals in a second division, leading the scoring chart, collecting
//! awards — yet sitting with zero interest because his club merely
//! loan-listed him over a contract dispute.
//!
//! This module supplies the missing signal as pure, deterministic scoring
//! over **observable** numbers (goals, assists, appearances, regressed
//! rating, scoring-chart standing, recent individual awards) — nothing
//! hidden (no CA/PA). League reputation discounts lower-division output so
//! it is taken seriously without making top clubs chase every flat-track
//! scorer.
//!
//! Two pieces:
//!   * [`BreakoutPerformanceSignal`] — the pure score, unit-tested in
//!     isolation; the rest of the pipeline only reads `score` /
//!     `is_breakout`.
//!   * [`LeaguePerformanceLookup`] — builds the per-player scoring-rank and
//!     recent-award maps once per country from `country.leagues`, so the
//!     scorer can be fed the top-scorer / award signals without re-walking
//!     the league archives per player.
//!
//! Per project convention everything is a method on a struct (no free
//! functions) and every type is reached through a `use` at the file header.

use std::collections::HashMap;

use crate::{Country, Player, PlayerFieldPositionGroup};

/// Observable, season-to-date performance signals for one player. Built by
/// the pipeline from snapshots and the [`LeaguePerformanceLookup`]; kept
/// free of any `Player` borrow so [`BreakoutPerformanceSignal::compute`]
/// stays pure.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct BreakoutInputs {
    pub position_group: PlayerFieldPositionGroup,
    pub goals: u16,
    pub assists: u16,
    pub appearances: u16,
    /// Sample-size-regressed season average (`realistic_average_rating`),
    /// NOT the raw form — a nine-game 8.2 has already been pulled toward
    /// the positional neutral before it reaches here.
    pub average_rating: f32,
    #[allow(dead_code)]
    pub age: u8,
    /// Player's league reputation, 0..10000. Lower-division output is
    /// discounted (weaker defending) but never ignored.
    pub league_reputation: u16,
    /// Player currently leads his league's scoring chart.
    pub is_league_top_scorer: bool,
    /// 1-indexed rank on the league scoring chart, if known and within the
    /// tracked top group. `None` when unknown or outside it.
    pub scoring_rank: Option<u8>,
    /// Weighted count of recent individual awards (season POTY / young
    /// POTY / top-scorer / golden glove / Team of the Season / recent
    /// monthly POM). 0 when none or unknown.
    pub recent_award_points: f32,
}

/// Breakout verdict — a single discoverability score the pipeline reads.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct BreakoutPerformanceSignal {
    /// 0..100 — how strongly this player's *results* should draw interest
    /// from clubs above his level. League-reputation discounted. A ranking
    /// / admission signal, never a gate that relaxes affordability.
    pub score: f32,
}

impl BreakoutPerformanceSignal {
    /// Minimum appearances before an output *rate* is trusted at all — a
    /// two-in-two hot start is not a season's output.
    const MIN_SAMPLE: f32 = 6.0;

    /// `score` at/above which a player reads as a genuine breakout worth
    /// surfacing to stronger clubs (year-round monitoring) and worth
    /// treating a loan-listed player as "available enough" for a permanent
    /// approach. Deliberately a meaningful bar: a bare lower-league scorer
    /// with no corroboration (top-scorer standing, awards, strong rating)
    /// sits below it.
    pub(in crate::transfers::pipeline) const BREAKOUT_THRESHOLD: f32 = 45.0;

    pub(in crate::transfers::pipeline) fn compute(inp: &BreakoutInputs) -> BreakoutPerformanceSignal {
        // ── League-reputation discount ──
        // Lower-division output is real but worth less on the wider market.
        // Discount, never erase: a strong enough lower-league breakout still
        // clears the bar for clubs a tier or two up.
        let rep_frac = (inp.league_reputation as f32 / 10000.0).clamp(0.0, 1.0);
        let discount = 0.45 + 0.55 * rep_frac;
        let score = (Self::raw_output(inp) * discount).clamp(0.0, 100.0);

        BreakoutPerformanceSignal { score }
    }

    /// Breakout score WITHOUT the league-reputation discount — used for youth
    /// discovery. Youth squads play friendly-classified age-group football whose
    /// "league reputation" is near zero and is NOT a meaningful
    /// opposition-strength proxy (every U18 plays other U18s), so applying the
    /// discount would crush a genuine prospect's output below the bar purely
    /// because his league has no senior standing. Skipping it judges the talent
    /// on its raw output, the way a scout in the stands at an academy game would.
    /// The senior pass keeps the discount.
    pub(in crate::transfers::pipeline) fn compute_undiscounted(
        inp: &BreakoutInputs,
    ) -> BreakoutPerformanceSignal {
        BreakoutPerformanceSignal {
            score: Self::raw_output(inp).clamp(0.0, 100.0),
        }
    }

    /// Observable, league-reputation-independent output score: the shared core
    /// of [`Self::compute`] (which then applies the reputation discount) and
    /// [`Self::compute_undiscounted`] (which doesn't). Built only from visible
    /// numbers — goals, assists, regressed rating, scoring-chart standing,
    /// recent awards — never hidden ability.
    fn raw_output(inp: &BreakoutInputs) -> f32 {
        let appearances = inp.appearances as f32;

        // ── Position-adjusted output rate (goals + assists per app) ──
        // Goals/assists are the headline for forwards; a scoring midfielder
        // or defender is rarer and so weighted higher per contribution.
        // Goalkeepers earn nothing here — their breakout rides on rating
        // and awards alone.
        let (goal_w, assist_w) = match inp.position_group {
            PlayerFieldPositionGroup::Forward => (1.0, 0.6),
            PlayerFieldPositionGroup::Midfielder => (1.15, 0.9),
            PlayerFieldPositionGroup::Defender => (1.7, 1.2),
            PlayerFieldPositionGroup::Goalkeeper => (0.0, 0.0),
        };
        let contribution = inp.goals as f32 * goal_w + inp.assists as f32 * assist_w;

        // Gate tiny samples out, then regress the rate by sample size so a
        // long run is trusted more than a short one.
        let output_points = if appearances < Self::MIN_SAMPLE {
            0.0
        } else {
            let per_app = contribution / appearances;
            let reliability = appearances / (appearances + 5.0);
            (per_app * reliability * 70.0).clamp(0.0, 55.0)
        };

        // ── Rating excellence (already regressed upstream) ──
        let neutral = match inp.position_group {
            PlayerFieldPositionGroup::Goalkeeper => 6.65,
            PlayerFieldPositionGroup::Defender => 6.55,
            PlayerFieldPositionGroup::Midfielder => 6.60,
            PlayerFieldPositionGroup::Forward => 6.55,
        };
        let rating_points = ((inp.average_rating - neutral).max(0.0) * 18.0).clamp(0.0, 30.0);

        // ── Scoring-chart standing ──
        let standing_points = if inp.is_league_top_scorer {
            16.0
        } else {
            match inp.scoring_rank {
                Some(r) if r <= 3 => 8.0,
                Some(r) if r <= 5 => 4.0,
                _ => 0.0,
            }
        };

        // ── Recent individual awards (corroboration, capped) ──
        let award_points = (inp.recent_award_points * 3.0).clamp(0.0, 16.0);

        (output_points + rating_points + standing_points + award_points).clamp(0.0, 100.0)
    }

    /// `true` when the score clears [`Self::BREAKOUT_THRESHOLD`].
    pub(in crate::transfers::pipeline) fn is_breakout(&self) -> bool {
        self.score >= Self::BREAKOUT_THRESHOLD
    }
}

/// Per-country lookup of the two breakout signals that can't be read off a
/// single player in isolation — where he sits on his league's scoring
/// chart, and what recent individual awards he has collected. Built once
/// per country per pass (cheap relative to the rest of the pipeline) and
/// queried per player.
pub(in crate::transfers::pipeline) struct LeaguePerformanceLookup {
    /// player_id → 1-indexed rank on his league's scoring chart (top group
    /// only). Absent when the player has no goals or sits outside the
    /// tracked group.
    scoring_rank: HashMap<u32, u8>,
    /// player_id → recent individual-award weight (season + recent months).
    award_points: HashMap<u32, f32>,
}

impl LeaguePerformanceLookup {
    /// How deep the scoring chart is tracked per league. Beyond the top
    /// handful the standing signal is no longer meaningful.
    const TRACKED_SCORERS: usize = 5;
    /// How many recent monthly awards to fold in per league.
    const RECENT_MONTHS: usize = 3;
    /// Per-player award-weight ceiling — awards corroborate a breakout,
    /// they never single-handedly manufacture one.
    const AWARD_CAP: f32 = 6.0;

    pub(in crate::transfers::pipeline) fn build(country: &Country) -> Self {
        // ── Scoring chart per league ──
        let mut by_league: HashMap<u32, Vec<(u32, u16)>> = HashMap::new();
        for club in &country.clubs {
            for team in &club.teams.teams {
                let Some(league_id) = team.league_id else {
                    continue;
                };
                for player in &team.players.players {
                    if player.statistics.goals > 0 {
                        by_league
                            .entry(league_id)
                            .or_default()
                            .push((player.id, player.statistics.goals));
                    }
                }
            }
        }
        let mut scoring_rank: HashMap<u32, u8> = HashMap::new();
        for (_league_id, mut chart) in by_league {
            // Goals desc, lower id as the deterministic tiebreak — mirrors
            // `LeagueStatistics::update_player_rankings`.
            chart.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            for (idx, (player_id, _goals)) in chart.iter().take(Self::TRACKED_SCORERS).enumerate() {
                scoring_rank.insert(*player_id, (idx + 1) as u8);
            }
        }

        // ── Recent individual-award weight per league ──
        let mut award_points: HashMap<u32, f32> = HashMap::new();
        let add = |map: &mut HashMap<u32, f32>, id: u32, weight: f32| {
            *map.entry(id).or_insert(0.0) += weight;
        };
        for league in &country.leagues.leagues {
            let awards = &league.awards;
            // Most recent season — prefer the snapshot pending at season
            // end, else the last archived one.
            let season = awards
                .pending_season_awards
                .as_ref()
                .or_else(|| awards.season_awards.last());
            if let Some(s) = season {
                if let Some(id) = s.player_of_season {
                    add(&mut award_points, id, 3.0);
                }
                if let Some(id) = s.young_player_of_season {
                    add(&mut award_points, id, 2.5);
                }
                if let Some(id) = s.top_scorer {
                    add(&mut award_points, id, 2.0);
                }
                if let Some(id) = s.top_assists {
                    add(&mut award_points, id, 1.5);
                }
                if let Some(id) = s.golden_glove {
                    add(&mut award_points, id, 2.0);
                }
                for id in &s.team_of_season {
                    add(&mut award_points, *id, 1.0);
                }
            }
            for award in awards.player_of_month.iter().rev().take(Self::RECENT_MONTHS) {
                add(&mut award_points, award.player_id, 1.0);
            }
            for award in awards
                .young_player_of_month
                .iter()
                .rev()
                .take(Self::RECENT_MONTHS)
            {
                add(&mut award_points, award.player_id, 0.8);
            }
        }
        for weight in award_points.values_mut() {
            *weight = weight.min(Self::AWARD_CAP);
        }

        LeaguePerformanceLookup {
            scoring_rank,
            award_points,
        }
    }

    pub(in crate::transfers::pipeline) fn scoring_rank(&self, player_id: u32) -> Option<u8> {
        self.scoring_rank.get(&player_id).copied()
    }

    pub(in crate::transfers::pipeline) fn is_top_scorer(&self, player_id: u32) -> bool {
        self.scoring_rank.get(&player_id) == Some(&1)
    }

    pub(in crate::transfers::pipeline) fn award_points(&self, player_id: u32) -> f32 {
        self.award_points.get(&player_id).copied().unwrap_or(0.0)
    }

    /// Assemble the full [`BreakoutInputs`] for a player from the observable
    /// fields the caller already has plus the league-derived signals this
    /// lookup owns. Keeps the per-call wiring in one place so every caller
    /// (data pre-filter, recommendation snapshots, circulation, year-round
    /// watch) builds the inputs identically.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::transfers::pipeline) fn breakout_inputs(
        &self,
        player_id: u32,
        position_group: PlayerFieldPositionGroup,
        goals: u16,
        assists: u16,
        appearances: u16,
        average_rating: f32,
        age: u8,
        league_reputation: u16,
    ) -> BreakoutInputs {
        BreakoutInputs {
            position_group,
            goals,
            assists,
            appearances,
            average_rating,
            age,
            league_reputation,
            is_league_top_scorer: self.is_top_scorer(player_id),
            scoring_rank: self.scoring_rank(player_id),
            recent_award_points: self.award_points(player_id),
        }
    }

    /// Compute the breakout score for one player straight from a live
    /// `Player` reference, given his observable performance plus league
    /// context. The single helper the snapshot builders call so the
    /// observable-only contract (no CA/PA) is enforced in one place.
    pub(in crate::transfers::pipeline) fn breakout_for_player(
        &self,
        player: &Player,
        appearances: u16,
        average_rating: f32,
        age: u8,
        league_reputation: u16,
    ) -> BreakoutPerformanceSignal {
        let inputs = self.breakout_inputs(
            player.id,
            player.position().position_group(),
            player.statistics.goals,
            player.statistics.assists,
            appearances,
            average_rating,
            age,
            league_reputation,
        );
        BreakoutPerformanceSignal::compute(&inputs)
    }

    /// Breakout signal for a YOUTH-squad player. Reads his age-group output from
    /// the FRIENDLY bucket — youth football is friendly-classified, so his
    /// `statistics` (official) are empty and only `friendly_statistics` carry his
    /// goals/assists/rating — and uses the undiscounted score so the near-zero
    /// youth-league reputation doesn't bury him. Scoring-chart standing and
    /// awards stay at the youth-empty defaults (a youngster never enters a senior
    /// chart), so the signal rides on output and rating: exactly the visible
    /// evidence a scout takes away from watching the U18s.
    pub(in crate::transfers::pipeline) fn breakout_for_youth(
        &self,
        player: &Player,
        appearances: u16,
        average_rating: f32,
        age: u8,
    ) -> BreakoutPerformanceSignal {
        let inputs = self.breakout_inputs(
            player.id,
            player.position().position_group(),
            player.friendly_statistics.goals,
            player.friendly_statistics.assists,
            appearances,
            average_rating,
            age,
            0, // league_reputation unused — the youth path skips the discount
        );
        BreakoutPerformanceSignal::compute_undiscounted(&inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Baseline: an anonymous, low-output forward in a strong league.
    /// Tests tweak individual axes off this. Wrapped in a unit struct per
    /// the project's no-free-helpers convention.
    struct BreakoutFixtures;

    impl BreakoutFixtures {
        fn anonymous_forward() -> BreakoutInputs {
            BreakoutInputs {
                position_group: PlayerFieldPositionGroup::Forward,
                goals: 2,
                assists: 1,
                appearances: 18,
                average_rating: 6.5,
                age: 24,
                league_reputation: 8000,
                is_league_top_scorer: false,
                scoring_rank: None,
                recent_award_points: 0.0,
            }
        }

        /// The reported player: 22-y-o striker, 10 goals in 14 starts,
        /// league top scorer, decorated, in a second division.
        fn lower_division_breakout_striker() -> BreakoutInputs {
            BreakoutInputs {
                position_group: PlayerFieldPositionGroup::Forward,
                goals: 10,
                assists: 2,
                appearances: 14,
                average_rating: 7.1,
                age: 22,
                league_reputation: 3000,
                is_league_top_scorer: true,
                scoring_rank: Some(1),
                recent_award_points: 6.0,
            }
        }

        /// A 17-year-old academy striker dominating age-group football: 20 goals
        /// in 28 youth (friendly) games, strong rating — but his youth "league"
        /// has near-zero reputation and he has no senior scoring-chart standing
        /// or awards. The exact profile that was sitting invisible.
        fn youth_academy_striker() -> BreakoutInputs {
            BreakoutInputs {
                position_group: PlayerFieldPositionGroup::Forward,
                goals: 20,
                assists: 5,
                appearances: 28,
                average_rating: 7.0,
                age: 17,
                league_reputation: 300,
                is_league_top_scorer: false,
                scoring_rank: None,
                recent_award_points: 0.0,
            }
        }
    }

    #[test]
    fn lower_division_top_scorer_clears_breakout_bar() {
        let signal =
            BreakoutPerformanceSignal::compute(&BreakoutFixtures::lower_division_breakout_striker());
        assert!(
            signal.is_breakout(),
            "a decorated, league-top-scoring lower-division striker must read as a breakout (score {})",
            signal.score
        );
    }

    #[test]
    fn anonymous_low_output_player_is_not_a_breakout() {
        let signal = BreakoutPerformanceSignal::compute(&BreakoutFixtures::anonymous_forward());
        assert!(
            !signal.is_breakout(),
            "a 2-goal anonymous forward must not read as a breakout (score {})",
            signal.score
        );
    }

    #[test]
    fn top_scorer_and_awards_raise_the_score() {
        // Same output / rating, but recognised (top scorer + awards) — the
        // recognition must lift the discovery score. (Req: the top-scorer /
        // award signal affects discovery.)
        let mut bare = BreakoutFixtures::lower_division_breakout_striker();
        bare.is_league_top_scorer = false;
        bare.scoring_rank = None;
        bare.recent_award_points = 0.0;

        let bare_score = BreakoutPerformanceSignal::compute(&bare).score;
        let decorated_score =
            BreakoutPerformanceSignal::compute(&BreakoutFixtures::lower_division_breakout_striker())
                .score;
        assert!(
            decorated_score > bare_score + 5.0,
            "recognition must lift the score meaningfully: decorated {} vs bare {}",
            decorated_score,
            bare_score
        );
    }

    #[test]
    fn lower_division_output_is_discounted_versus_top_flight() {
        // Identical results in a top league vs a second division: the
        // discounted score must be lower for the weaker league — but the
        // weaker-league player is not zeroed out. (Req: league reputation
        // discount, lower-division output discounted but not ignored.)
        let mut top = BreakoutFixtures::lower_division_breakout_striker();
        top.league_reputation = 9000;
        let mut lower = BreakoutFixtures::lower_division_breakout_striker();
        lower.league_reputation = 2500;

        let top_score = BreakoutPerformanceSignal::compute(&top).score;
        let lower_score = BreakoutPerformanceSignal::compute(&lower).score;
        assert!(
            top_score > lower_score,
            "top-flight output must out-score identical second-division output: {} vs {}",
            top_score,
            lower_score
        );
        assert!(
            lower_score > 0.0,
            "second-division breakout must not be zeroed out: {}",
            lower_score
        );
    }

    #[test]
    fn tiny_sample_hot_start_does_not_count_as_output() {
        // Two goals in two games is a 1.0/game pace, but far too small a
        // sample to read as a breakout — the appearance gate must reject it.
        let hot_start = BreakoutInputs {
            goals: 2,
            assists: 0,
            appearances: 2,
            average_rating: 7.0,
            is_league_top_scorer: false,
            scoring_rank: None,
            recent_award_points: 0.0,
            ..BreakoutFixtures::lower_division_breakout_striker()
        };
        let signal = BreakoutPerformanceSignal::compute(&hot_start);
        assert!(
            !signal.is_breakout(),
            "a two-game hot start must not register as a breakout (score {})",
            signal.score
        );
    }

    #[test]
    fn longer_run_is_trusted_more_than_a_short_one() {
        // Same per-game rate, more games → higher output (sample-size
        // reliability), so the score is non-decreasing in appearances.
        let mut short = BreakoutFixtures::lower_division_breakout_striker();
        short.goals = 6;
        short.assists = 0;
        short.appearances = 8;
        let mut long = short;
        long.goals = 24;
        long.appearances = 32; // same 0.75 goals/game

        assert!(
            BreakoutPerformanceSignal::compute(&long).score
                >= BreakoutPerformanceSignal::compute(&short).score,
            "a longer run at the same rate must be trusted at least as much"
        );
    }

    #[test]
    fn keeper_breakout_rides_on_rating_not_goals() {
        // A keeper has no goal output, so his breakout depends entirely on
        // rating + awards. A strong-rated, decorated keeper can still be a
        // breakout; a plain one cannot.
        let strong_keeper = BreakoutInputs {
            position_group: PlayerFieldPositionGroup::Goalkeeper,
            goals: 0,
            assists: 0,
            appearances: 26,
            average_rating: 7.4,
            age: 24,
            league_reputation: 6000,
            is_league_top_scorer: false,
            scoring_rank: None,
            recent_award_points: 4.0,
        };
        let plain_keeper = BreakoutInputs {
            average_rating: 6.6,
            recent_award_points: 0.0,
            ..strong_keeper
        };
        assert!(BreakoutPerformanceSignal::compute(&strong_keeper).score > 0.0);
        assert!(!BreakoutPerformanceSignal::compute(&plain_keeper).is_breakout());
    }

    #[test]
    fn undiscounted_score_is_at_least_the_discounted_score() {
        // The undiscounted score is the pre-discount ceiling, so for any input
        // it can never be below the league-rep-discounted score.
        let inp = BreakoutFixtures::lower_division_breakout_striker();
        let discounted = BreakoutPerformanceSignal::compute(&inp).score;
        let undiscounted = BreakoutPerformanceSignal::compute_undiscounted(&inp).score;
        assert!(
            undiscounted >= discounted,
            "undiscounted {undiscounted} must be >= discounted {discounted}"
        );
    }

    #[test]
    fn youth_standout_surfaces_only_without_the_reputation_discount() {
        // A dominant academy striker in a near-zero-reputation youth league.
        // With the league-rep discount applied he is buried below the bar (the
        // bug that left U18 stars invisible); the youth path scores him
        // undiscounted, so a genuine prospect reads as a breakout and surfaces.
        let inp = BreakoutFixtures::youth_academy_striker();
        assert!(
            !BreakoutPerformanceSignal::compute(&inp).is_breakout(),
            "discounted: the youth-league reputation should bury the score ({})",
            BreakoutPerformanceSignal::compute(&inp).score
        );
        assert!(
            BreakoutPerformanceSignal::compute_undiscounted(&inp).is_breakout(),
            "undiscounted: a dominant academy striker must read as a breakout ({})",
            BreakoutPerformanceSignal::compute_undiscounted(&inp).score
        );
    }
}
