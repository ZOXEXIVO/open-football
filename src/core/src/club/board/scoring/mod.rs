//! Component scoring for the board's monthly review. Instead of folding
//! everything into one linear confidence delta, the board scores four
//! independent dimensions — sporting, financial, squad-building, strategy
//! — each roughly in `[-40, 40]`. They're stored on `ClubBoard` so the UI
//! and tests can read *why* the board is happy or angry, and combined
//! (phase-weighted) into the confidence delta that actually moves mood.

use super::context::{BoardContext, FfpStatus};
use super::promise::PromiseLedger;
use super::strategy::SquadProfile;
use super::{ClubVision, SeasonTargets, VisionYouthFocus};

/// Where in the season we are. Early months are judged softly; the run-in
/// is judged harshly and is the only window in which sackings/table
/// judgments are allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeasonPhase {
    TooEarly,
    Early,
    Mid,
    RunIn,
}

impl SeasonPhase {
    /// Classify from matches played / total. Wrapped as an associated fn
    /// so the timing logic lives on the type.
    pub fn classify(matches_played: u8, total_matches: u8) -> SeasonPhase {
        if total_matches == 0 {
            return if matches_played < 10 {
                SeasonPhase::TooEarly
            } else {
                SeasonPhase::Mid
            };
        }

        let progress = matches_played as f32 / total_matches.max(1) as f32;
        if matches_played < 5 {
            SeasonPhase::TooEarly
        } else if progress < 0.30 {
            SeasonPhase::Early
        } else if progress < 0.75 {
            SeasonPhase::Mid
        } else {
            SeasonPhase::RunIn
        }
    }

    pub fn performance_weight(self) -> i32 {
        match self {
            SeasonPhase::TooEarly => 1,
            SeasonPhase::Early => 2,
            SeasonPhase::Mid => 3,
            SeasonPhase::RunIn => 4,
        }
    }

    pub fn can_judge_table(self) -> bool {
        matches!(self, SeasonPhase::Mid | SeasonPhase::RunIn)
    }

    pub fn can_sack_manager(self) -> bool {
        matches!(self, SeasonPhase::Mid | SeasonPhase::RunIn)
    }

    /// Sporting-score multiplier — early-season results count for less.
    fn sporting_scale(self) -> f32 {
        match self {
            SeasonPhase::TooEarly => 0.4,
            SeasonPhase::Early => 0.7,
            SeasonPhase::Mid => 1.0,
            SeasonPhase::RunIn => 1.25,
        }
    }
}

/// The four component scores from one board review. Roughly `[-40, 40]`
/// each; positive = pleasing the board.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BoardComponentScores {
    pub sporting: f32,
    pub financial: f32,
    pub squad_building: f32,
    pub strategy: f32,
}

impl BoardComponentScores {
    /// Run the full four-dimension evaluation. `style_drag` is the
    /// board's precomputed playing-style mismatch (0 = ideal, up to 2 =
    /// clash) so the scorer doesn't duplicate the tactic table.
    pub fn evaluate(
        ctx: &BoardContext,
        targets: &SeasonTargets,
        vision: &ClubVision,
        promises: &PromiseLedger,
        phase: SeasonPhase,
        style_drag: i32,
    ) -> Self {
        BoardComponentScores {
            sporting: Self::sporting_score(ctx, targets, phase),
            financial: Self::financial_score(ctx),
            squad_building: Self::squad_score(ctx, targets, vision),
            strategy: Self::strategy_score(ctx, vision, promises, style_drag),
        }
    }

    fn sporting_score(ctx: &BoardContext, targets: &SeasonTargets, phase: SeasonPhase) -> f32 {
        let mut s = 0.0f32;

        // League position vs expectation (the dominant term once there's
        // enough of a sample).
        if ctx.league_position > 0 && ctx.matches_played >= 5 {
            let delta = targets.expected_position as i32 - ctx.league_position as i32;
            s += delta as f32 * 3.0;
        }

        // Points per match against a competitive baseline.
        if ctx.points_per_match > 0.0 {
            s += (ctx.points_per_match - 1.3) * 9.0;
        }

        // Recent form and goal trend.
        s += (ctx.recent_wins as f32 - ctx.recent_losses as f32) * 2.5;
        s += (ctx.recent_goal_difference as f32).clamp(-8.0, 8.0) * 0.6;
        s += (ctx.goal_difference as f32).clamp(-25.0, 25.0) * 0.2;

        // Relegation jeopardy bites; comfortable cushion soothes.
        if ctx.distance_to_relegation <= 0 {
            s -= 10.0;
        } else if ctx.distance_to_relegation <= 2 {
            s -= 4.0;
        }

        let mut total = s * phase.sporting_scale();
        // An injury crisis softens the blame for poor results — you can't
        // judge a depleted side as harshly. Only pulls negatives towards
        // zero; it never rewards bad form.
        if ctx.injury_crisis_score > 0.3 && total < 0.0 {
            total *= 1.0 - (ctx.injury_crisis_score * 0.4).clamp(0.0, 0.4);
        }
        total.clamp(-40.0, 40.0)
    }

    fn financial_score(ctx: &BoardContext) -> f32 {
        let mut s = 0.0f32;

        s += match ctx.ffp_status {
            FfpStatus::Clean => 4.0,
            FfpStatus::Watchlist => -8.0,
            FfpStatus::Breach => -20.0,
        };

        // Balance health, scaled to wage size so it's wealth-relative.
        let wage_scale = (ctx.total_annual_wages as f32 / 12.0).max(1.0);
        let months_cash = ctx.balance as f32 / wage_scale;
        s += months_cash.clamp(-12.0, 6.0);

        // Wage discipline.
        if ctx.wage_budget_usage > 1.1 {
            s -= 12.0;
        } else if ctx.wage_budget_usage > 1.0 {
            s -= 6.0;
        } else if ctx.wage_budget_usage > 0.0 && ctx.wage_budget_usage < 0.85 {
            s += 4.0;
        }

        // Transfer spending discipline (0 usage is neutral/unknown).
        if ctx.transfer_budget_usage > 1.05 {
            s -= 6.0;
        } else if ctx.transfer_budget_usage > 0.0 && ctx.transfer_budget_usage < 0.6 {
            s += 3.0;
        }

        // Profit/loss trend.
        if ctx.profit_loss_12m > 0 {
            s += 4.0;
        } else if ctx.profit_loss_12m < 0 {
            s -= 4.0;
        }

        // Debt load.
        if ctx.debt_ratio > 1.0 {
            s -= 8.0;
        } else if ctx.debt_ratio > 0.5 {
            s -= 3.0;
        }

        s.clamp(-40.0, 40.0)
    }

    fn squad_score(ctx: &BoardContext, targets: &SeasonTargets, vision: &ClubVision) -> f32 {
        let mut s = 0.0f32;

        let total_squad = ctx.main_squad_size + ctx.reserve_squad_size;
        if total_squad > targets.max_squad_size as usize + 5 {
            s -= 6.0;
        } else if ctx.main_squad_size < targets.min_squad_size as usize {
            s -= 6.0;
        } else {
            s += 2.0;
        }

        // Age profile vs the preferred squad profile / youth focus.
        if ctx.squad_avg_age > 0 {
            let age = ctx.squad_avg_age as f32;
            let target_age = match vision.preferred_squad_profile {
                SquadProfile::Youth => 23.0,
                SquadProfile::ResaleValue => 24.0,
                SquadProfile::Balanced => 26.0,
                SquadProfile::Domestic => 26.0,
                SquadProfile::PrimeAge => 27.0,
                SquadProfile::Stars => 28.0,
            };
            s += (4.0f32 - (age - target_age).abs()).clamp(-6.0, 4.0);
        }

        // Youth usage vs vision.
        match vision.youth_focus {
            VisionYouthFocus::DevelopYouth => {
                s += (ctx.u21_minutes_share * 20.0).clamp(0.0, 8.0);
                s += (ctx.academy_graduates_this_season as f32 * 1.5).clamp(0.0, 6.0);
            }
            VisionYouthFocus::SignExperienced => {
                // Over-reliance on kids is off-brief here.
                if ctx.u21_minutes_share > 0.35 {
                    s -= 3.0;
                }
            }
            VisionYouthFocus::Balanced => {}
        }

        // Ability gap vs where the club expects to be: a squad punching
        // below its league standing worries the board.
        if ctx.league_position > 0 && ctx.league_size > 0 {
            let standing = 1.0 - (ctx.league_position as f32 / ctx.league_size as f32);
            let ability_norm = ctx.avg_squad_ability as f32 / 200.0;
            s += ((ability_norm - 0.5) * 8.0 + (standing - 0.5) * 4.0).clamp(-6.0, 6.0);
        }

        // Injury crisis softens the blame — you can't judge a depleted
        // squad as harshly. Pull negatives back towards zero.
        if ctx.injury_crisis_score > 0.3 && s < 0.0 {
            s *= 1.0 - (ctx.injury_crisis_score * 0.5).clamp(0.0, 0.5);
        }

        s.clamp(-40.0, 40.0)
    }

    fn strategy_score(
        ctx: &BoardContext,
        vision: &ClubVision,
        promises: &PromiseLedger,
        style_drag: i32,
    ) -> f32 {
        let mut s = 0.0f32;

        // Playing-style alignment.
        s -= style_drag as f32 * 4.0;

        // Promise track record.
        s += promises.track_record_score();

        // Outstanding promises are a low-grade nag.
        s -= (promises.active_count() as f32 * 1.0).min(4.0);

        // Youth pathway delivering against a development vision.
        if matches!(vision.youth_focus, VisionYouthFocus::DevelopYouth)
            && ctx.academy_graduates_this_season > 0
        {
            s += 3.0;
        }

        s.clamp(-40.0, 40.0)
    }

    /// Phase-weighted confidence delta this review. Sporting dominates but
    /// finances/squad/strategy meaningfully shift the dial.
    pub fn confidence_delta(&self, phase: SeasonPhase) -> i32 {
        let weighted = self.sporting * 0.45
            + self.financial * 0.22
            + self.squad_building * 0.15
            + self.strategy * 0.18;
        // Late-season swings hit confidence a touch harder.
        let phase_mult = match phase {
            SeasonPhase::TooEarly => 0.5,
            SeasonPhase::Early => 0.75,
            SeasonPhase::Mid => 1.0,
            SeasonPhase::RunIn => 1.15,
        };
        (weighted * phase_mult / 2.2).round() as i32
    }

    /// Compact human-readable summary of the dominant grievance / strength
    /// for logs and (later) UI.
    pub fn headline(&self) -> &'static str {
        let worst = [
            (self.sporting, "sporting"),
            (self.financial, "financial"),
            (self.squad_building, "squad"),
            (self.strategy, "strategy"),
        ]
        .into_iter()
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, name)| name)
        .unwrap_or("none");
        worst
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_ctx() -> BoardContext {
        let mut c = BoardContext::new();
        c.total_annual_wages = 12_000_000;
        c.balance = 5_000_000;
        c.league_size = 20;
        c.matches_played = 19;
        c.total_matches = 38;
        c
    }

    fn targets() -> SeasonTargets {
        SeasonTargets {
            transfer_budget: 0,
            wage_budget: 0,
            max_squad_size: 30,
            min_squad_size: 18,
            expected_position: 8,
            min_acceptable_position: 13,
        }
    }

    #[test]
    fn overperformance_yields_positive_sporting() {
        let mut ctx = base_ctx();
        ctx.league_position = 3; // beating expected 8th
        ctx.points_per_match = 2.1;
        ctx.recent_wins = 4;
        ctx.recent_losses = 0;
        ctx.goal_difference = 18;
        ctx.distance_to_relegation = 15;
        let scores = BoardComponentScores::evaluate(
            &ctx,
            &targets(),
            &ClubVision::default(),
            &PromiseLedger::new(),
            SeasonPhase::Mid,
            0,
        );
        assert!(scores.sporting > 10.0, "got {}", scores.sporting);
        assert!(scores.confidence_delta(SeasonPhase::Mid) > 0);
    }

    #[test]
    fn relegation_and_breach_drag_scores_down() {
        let mut ctx = base_ctx();
        ctx.league_position = 19;
        ctx.points_per_match = 0.6;
        ctx.recent_losses = 4;
        ctx.goal_difference = -20;
        ctx.distance_to_relegation = -1;
        ctx.ffp_status = FfpStatus::Breach;
        ctx.wage_budget_usage = 1.2;
        let scores = BoardComponentScores::evaluate(
            &ctx,
            &targets(),
            &ClubVision::default(),
            &PromiseLedger::new(),
            SeasonPhase::RunIn,
            0,
        );
        assert!(scores.sporting < 0.0);
        assert!(scores.financial < 0.0);
        assert!(scores.confidence_delta(SeasonPhase::RunIn) < 0);
    }

    #[test]
    fn season_phase_thresholds() {
        assert_eq!(SeasonPhase::classify(4, 38), SeasonPhase::TooEarly);
        assert_eq!(SeasonPhase::classify(8, 38), SeasonPhase::Early);
        assert!(!SeasonPhase::classify(8, 38).can_sack_manager());
        assert!(SeasonPhase::classify(16, 38).can_sack_manager());
        assert_eq!(SeasonPhase::classify(32, 38), SeasonPhase::RunIn);
    }

    #[test]
    fn mid_table_club_meeting_expectations_is_roughly_neutral() {
        // A textbook mid-table side, finishing where expected on a par
        // points haul, should barely move board confidence.
        let mut ctx = base_ctx();
        ctx.league_position = 8;
        ctx.points_per_match = 1.3;
        ctx.recent_wins = 2;
        ctx.recent_losses = 2;
        ctx.goal_difference = 0;
        ctx.distance_to_relegation = 5;
        let scores = BoardComponentScores::evaluate(
            &ctx,
            &targets(), // expected 8
            &ClubVision::default(),
            &PromiseLedger::new(),
            SeasonPhase::Mid,
            0,
        );
        assert!(
            scores.sporting.abs() < 8.0,
            "meeting expectations should be near-neutral sporting: {}",
            scores.sporting
        );
        assert!(
            scores.confidence_delta(SeasonPhase::Mid).abs() <= 2,
            "confidence should barely move: {}",
            scores.confidence_delta(SeasonPhase::Mid)
        );
    }

    #[test]
    fn relegation_survivor_is_judged_more_kindly_than_a_big_club_in_the_same_spot() {
        // Same league position (15th), wildly different expectations.
        let mut ctx = base_ctx();
        ctx.league_position = 15;
        ctx.points_per_match = 1.0;
        ctx.distance_to_relegation = 3;

        let survivor_targets = SeasonTargets {
            expected_position: 17,
            min_acceptable_position: 20,
            ..targets()
        };
        let big_club_targets = SeasonTargets {
            expected_position: 3,
            min_acceptable_position: 8,
            ..targets()
        };

        let survivor =
            BoardComponentScores::sporting_score(&ctx, &survivor_targets, SeasonPhase::Mid);
        let big_club =
            BoardComponentScores::sporting_score(&ctx, &big_club_targets, SeasonPhase::Mid);
        assert!(
            survivor > big_club,
            "15th should hurt a title side far more than a survival one: {survivor} vs {big_club}"
        );
        assert!(
            survivor >= 0.0,
            "a survivor over-achieving in 15th isn't punished"
        );
    }

    #[test]
    fn ffp_breach_hits_finances_hard_without_drowning_sporting() {
        // A title-challenging side that breaches FFP.
        let mut ctx = base_ctx();
        ctx.league_position = 1;
        ctx.points_per_match = 2.3;
        ctx.recent_wins = 5;
        ctx.goal_difference = 30;
        ctx.distance_to_relegation = 18;
        ctx.ffp_status = FfpStatus::Breach;

        let scores = BoardComponentScores::evaluate(
            &ctx,
            &targets(),
            &ClubVision::default(),
            &PromiseLedger::new(),
            SeasonPhase::Mid,
            0,
        );
        assert!(
            scores.financial < -10.0,
            "breach must bite finances: {}",
            scores.financial
        );
        assert!(
            scores.sporting > 20.0,
            "a runaway leader's sporting stays strong: {}",
            scores.sporting
        );
        // Sporting still wins out: overall confidence stays positive despite
        // the financial hit, so good football isn't made irrelevant.
        assert!(
            scores.confidence_delta(SeasonPhase::Mid) > 0,
            "strong football should still lift confidence through an FFP breach"
        );
    }

    #[test]
    fn injury_crisis_softens_both_sporting_and_squad_blame() {
        let mut ctx = base_ctx();
        ctx.league_position = 16;
        ctx.points_per_match = 0.8;
        ctx.recent_losses = 4;
        ctx.goal_difference = -12;
        ctx.distance_to_relegation = 1;
        ctx.main_squad_size = 15; // under min → squad penalty

        let healthy = {
            let mut c = ctx.clone();
            c.injury_crisis_score = 0.0;
            BoardComponentScores::evaluate(
                &c,
                &targets(),
                &ClubVision::default(),
                &PromiseLedger::new(),
                SeasonPhase::Mid,
                0,
            )
        };
        let crisis = {
            let mut c = ctx.clone();
            c.injury_crisis_score = 0.6;
            BoardComponentScores::evaluate(
                &c,
                &targets(),
                &ClubVision::default(),
                &PromiseLedger::new(),
                SeasonPhase::Mid,
                0,
            )
        };
        assert!(
            crisis.sporting > healthy.sporting,
            "injury crisis should soften sporting blame: {} vs {}",
            crisis.sporting,
            healthy.sporting
        );
        assert!(
            crisis.squad_building >= healthy.squad_building,
            "injury crisis should soften squad blame: {} vs {}",
            crisis.squad_building,
            healthy.squad_building
        );
    }

    /// Table-driven archetype check: each club profile should produce the
    /// expected *signs* of component movement (not fragile exact scores).
    #[test]
    fn archetype_component_signs() {
        struct Case {
            name: &'static str,
            ctx: BoardContext,
            targets: SeasonTargets,
            vision: ClubVision,
            // Expected sign of each component: Some(true)=positive,
            // Some(false)=negative, None=don't care.
            sporting: Option<bool>,
            financial: Option<bool>,
            confidence_up: Option<bool>,
        }

        let elite_title = {
            let mut c = base_ctx();
            c.league_position = 1;
            c.points_per_match = 2.4;
            c.recent_wins = 5;
            c.goal_difference = 35;
            c.distance_to_relegation = 18;
            c.profit_loss_12m = 20_000_000;
            Case {
                name: "elite state-backed title challenger",
                ctx: c,
                targets: SeasonTargets {
                    expected_position: 1,
                    min_acceptable_position: 4,
                    ..targets()
                },
                vision: ClubVision::default(),
                sporting: Some(true),
                financial: Some(true),
                confidence_up: Some(true),
            }
        };

        let pe_selling = {
            let mut c = base_ctx();
            c.league_position = 9;
            c.points_per_match = 1.35;
            c.profit_loss_12m = 30_000_000; // sold to profit
            c.balance = 40_000_000;
            Case {
                name: "private-equity selling club",
                ctx: c,
                targets: SeasonTargets {
                    expected_position: 9,
                    min_acceptable_position: 14,
                    ..targets()
                },
                vision: ClubVision::default(),
                sporting: None,
                financial: Some(true),
                confidence_up: None,
            }
        };

        let relegation_survivor = {
            let mut c = base_ctx();
            c.league_position = 15;
            c.points_per_match = 1.1;
            c.distance_to_relegation = 4; // a real cushion, not in the scrap
            Case {
                name: "relegation survivor over-achieving",
                ctx: c,
                targets: SeasonTargets {
                    expected_position: 18,
                    min_acceptable_position: 20,
                    ..targets()
                },
                vision: ClubVision::default(),
                sporting: Some(true), // beating a survival brief
                financial: None,
                confidence_up: None,
            }
        };

        for case in [elite_title, pe_selling, relegation_survivor] {
            let scores = BoardComponentScores::evaluate(
                &case.ctx,
                &case.targets,
                &case.vision,
                &PromiseLedger::new(),
                SeasonPhase::Mid,
                0,
            );
            if let Some(pos) = case.sporting {
                assert_eq!(
                    scores.sporting > 0.0,
                    pos,
                    "{}: sporting sign wrong ({})",
                    case.name,
                    scores.sporting
                );
            }
            if let Some(pos) = case.financial {
                assert_eq!(
                    scores.financial > 0.0,
                    pos,
                    "{}: financial sign wrong ({})",
                    case.name,
                    scores.financial
                );
            }
            if let Some(up) = case.confidence_up {
                assert_eq!(
                    scores.confidence_delta(SeasonPhase::Mid) > 0,
                    up,
                    "{}: confidence direction wrong ({})",
                    case.name,
                    scores.confidence_delta(SeasonPhase::Mid)
                );
            }
        }
    }
}
