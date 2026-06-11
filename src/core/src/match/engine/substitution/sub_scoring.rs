//! Role- and game-state-aware substitution scoring (Section 6).
//!
//! The classic substitution loop in `substitutions.rs` handles the
//! fatigue / injury / youth-protection passes. This module layers a
//! tactical scoring on top so the coach can also pull "the right type
//! of player off and bring the right type on" — chasing → swap a
//! tired CB for a fresh attacker; protecting a lead → swap a luxury
//! forward for a defender / DM; etc.
//!
//! All scores are unitless [0.0, ~1.5] values; the higher the score
//! the stronger the case for the swap. The substitution loop combines
//! a `sub_off_score` and a `sub_in_score` to choose pairs.

use crate::r#match::engine::rating::RatingContext;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{MatchPlayer, engine::coach::TacticalNeed};
use crate::{PlayerFieldPositionGroup, PlayerPositionType};

/// Lightweight live-performance snapshot for a single on-field player.
///
/// Built from the same `PlayerMatchEndStats` shape that the post-match
/// rating helper consumes, but evaluated against the *current* scoreline
/// so the substitution loop can talk about "what has this player done
/// in this match so far". The substitution scorer reads this to protect
/// goal scorers and high-rated starters from routine removal — without
/// reaching into hidden ability or any pre-computed reputation flag.
///
/// The live rating is an approximation of the eventual post-match
/// rating: the same `RatingContext` is fed the same stats, with the
/// scoreline at the moment of decision. Cameos saturate naturally via
/// the existing minute-confidence damp.
#[derive(Debug, Clone)]
pub struct LiveSubstitutionStats {
    pub minutes_played: u16,
    pub goals: u16,
    pub assists: u16,
    pub key_passes: u16,
    pub shots_on_target: u16,
    pub xg: f32,
    pub errors_leading_to_goal: u16,
    pub yellow_cards: u16,
    pub red_cards: u16,
    /// Estimated current rating computed from the snapshot stats and
    /// current scoreline. Used to gate star protection.
    pub live_rating: f32,
    pub condition: i16,
    /// `team_goals - opponent_goals` from this player's perspective.
    pub goal_diff: i32,
    /// Whole-minute view of `context.total_match_time` at snapshot time.
    pub match_minute: u32,
}

impl LiveSubstitutionStats {
    /// Snapshot the player's contribution at the current match time.
    /// `own_goals` / `opp_goals` are the team-perspective scoreline so
    /// `goal_diff` reads positive for a team that is winning.
    pub fn from_player(
        player: &MatchPlayer,
        total_match_time_ms: u64,
        own_goals: u8,
        opp_goals: u8,
    ) -> Self {
        let minutes_played = player.minutes_played_at(total_match_time_ms);
        let stats = player.to_match_end_stats(minutes_played);
        let live_rating = RatingContext::new(&stats, own_goals, opp_goals).calculate();
        Self {
            minutes_played,
            goals: stats.goals,
            assists: stats.assists,
            key_passes: stats.key_passes,
            shots_on_target: stats.shots_on_target,
            xg: stats.xg,
            errors_leading_to_goal: stats.errors_leading_to_goal,
            yellow_cards: stats.yellow_cards,
            red_cards: stats.red_cards,
            live_rating,
            condition: player.player_attributes.condition,
            goal_diff: own_goals as i32 - opp_goals as i32,
            match_minute: (total_match_time_ms / 60_000) as u32,
        }
    }

    #[inline]
    pub fn yellow_carded(&self) -> bool {
        self.yellow_cards >= 1
    }

    #[inline]
    pub fn is_scorer(&self) -> bool {
        self.goals >= 1
    }

    /// Goals + assists. The "decisive contributions" footprint that
    /// star protection reads.
    #[inline]
    pub fn major_contributions(&self) -> u16 {
        self.goals + self.assists
    }
}

/// Stateless namespace for substitution-decision scoring. Bundles
/// star-protection, sub-off / sub-in fits, and the per-slot timing window
/// used by the substitution loop.
pub struct SubScoring;

impl SubScoring {
    /// Star-protection bonus (always ≥ 0) — subtracted from `sub_off_score`
    /// so the manager doesn't pull the player who is carrying the match.
    ///
    /// Thresholds match the engineering brief:
    /// * Goal scorer: 0.20 (replaced by 0.35 once they have 2+ G/A)
    /// * 2+ G/A: 0.35 (covers two-goal scorers and goal+assist creators)
    /// * Live rating ≥ 7.3: +0.18
    /// * Live rating ≥ 8.0: +0.35 (replaces the 7.3 tier)
    /// * Decisive scorer when leading by exactly one: +0.15 stacked on top
    ///
    /// Below 30% condition the bonus tapers linearly to zero at the
    /// in-match condition floor (15%). A scorer who is physically
    /// finished is still a worse asset on the pitch than a fresh sub —
    /// keeping the protection at full strength means the engine refuses
    /// to rest top players even when their legs are completely gone.
    /// Critical injury / red-card aftermath are NOT gated by this
    /// function — callers handle those branches before reaching the
    /// scoring pass.
    pub fn star_protection(live: &LiveSubstitutionStats) -> f32 {
        let major = live.major_contributions();
        let contrib_protection = if major >= 2 {
            0.35
        } else if live.goals >= 1 {
            0.20
        } else {
            0.0
        };

        let rating_protection = if live.live_rating >= 8.0 {
            0.35
        } else if live.live_rating >= 7.3 {
            0.18
        } else {
            0.0
        };

        // Decisive scorer in a narrow lead: pulling the only goal scorer
        // off the pitch when 1-0 up is the canonical "don't do that" move.
        let decisive_protection = if live.goals >= 1 && live.goal_diff == 1 {
            0.15
        } else {
            0.0
        };

        let raw = contrib_protection + rating_protection + decisive_protection;
        raw * Self::extreme_fatigue_dampening(live.condition)
    }

    /// Condition-based dampener applied to `star_protection`. Above 30%
    /// condition the protection stands at full strength; from 30% down
    /// to the in-match floor it fades linearly to zero so the
    /// substitution loop can hook a scorer whose legs are visibly gone
    /// without overriding tactical fatigue with star halo.
    #[inline]
    fn extreme_fatigue_dampening(condition: i16) -> f32 {
        let cond_pct = (condition as f32 / 10_000.0).clamp(0.0, 1.0);
        if cond_pct >= 0.30 {
            1.0
        } else if cond_pct <= 0.15 {
            0.0
        } else {
            ((cond_pct - 0.15) / 0.15).clamp(0.0, 1.0)
        }
    }

    /// Score a player as a sub-off candidate. Higher = more urgent to
    /// remove. Force-selected players are still respected by the loop;
    /// this score is purely about tactical / fatigue / risk fit. Star
    /// protection is layered on by [`sub_off_score_protected`].
    pub fn sub_off_score(
        player: &MatchPlayer,
        live: &LiveSubstitutionStats,
        need: TacticalNeed,
    ) -> f32 {
        let cond_pct = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
        let jaded = (player.player_attributes.jadedness as f32 / 10_000.0).clamp(0.0, 1.0);
        let mut s = 0.0;

        // Fatigue dimension.
        s += (1.0 - cond_pct) * 0.32;
        s += jaded * 0.14;

        // Match-load dimension. The fatigue-normalized engine now aims
        // to leave an average starter around 70-76% late in the match
        // instead of collapsing toward the old floor range. Absolute
        // condition alone no longer makes those players clear the routine
        // substitution threshold, so add urgency when a long-stint player
        // has materially drained from their own kickoff tank.
        let starting_cond =
            (player.starting_condition as f32 / 10_000.0).clamp(cond_pct, 1.0);
        let condition_drop = (starting_cond - cond_pct).max(0.0);
        let long_stint = ((live.minutes_played as f32 - 50.0) / 30.0).clamp(0.0, 1.0);
        s += condition_drop * long_stint * 0.85;

        // Performance dimension — clamp so we can't punish a 6.0 player
        // who simply hasn't done anything.
        let perf = ((6.2 - live.live_rating) / 2.0).clamp(0.0, 1.0);
        s += perf * 0.16;

        // Role exhaustion: high-press wingers / fullbacks / CMs at < 60%
        // condition are usually the first to be hooked.
        let pos_group = player.tactical_position.current_position.position_group();
        if cond_pct < 0.60
            && matches!(
                pos_group,
                PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder
            )
        {
            s += 0.08;
        }
        if cond_pct < 0.55 && pos_group == PlayerFieldPositionGroup::Defender {
            s += 0.05;
        }

        // Yellow-card risk: a yellow + aggression in a defensive role
        // is a "get him off before he sees red" signal. Aggression scales
        // the bite smoothly (sigmoid around 14/20) so a 13/20 player is
        // still treated as risky, just less than a 17/20 hothead.
        if live.yellow_carded()
            && matches!(
                pos_group,
                PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Midfielder
            )
        {
            s += 0.12 * SkillCurve::new(player.skills.mental.aggression, 14.0, 0.6).probability();
        }

        // Errors leading to goal: the player has already cost the team
        // once — the manager has good reason to hook them. Smaller bite
        // than a red card but still a clear signal.
        if live.errors_leading_to_goal >= 1 {
            s += 0.15;
        }

        // Tactical mismatch: chasing → defenders / DMs less needed; defending
        // a lead → luxury forwards less needed.
        s += match (need, pos_group) {
            (TacticalNeed::Chasing, PlayerFieldPositionGroup::Defender) => 0.08,
            (TacticalNeed::ProtectingLead, PlayerFieldPositionGroup::Forward) => 0.08,
            _ => 0.0,
        };

        s
    }

    /// Sub-off urgency net of star protection. Callers pass a
    /// `protection_dampening` in [0.0, 1.0]: 1.0 keeps full protection
    /// (default), <1.0 weakens it (e.g. in a comfortable late lead where
    /// resting a star is acceptable).
    ///
    /// Result can go negative, signalling "this player is a net asset on
    /// the pitch right now" — discretionary subs should pass them over.
    pub fn sub_off_score_protected(
        player: &MatchPlayer,
        live: &LiveSubstitutionStats,
        need: TacticalNeed,
        protection_dampening: f32,
    ) -> f32 {
        let raw = Self::sub_off_score(player, live, need);
        let protection = Self::star_protection(live) * protection_dampening.clamp(0.0, 1.0);
        raw - protection
    }

    /// Score a substitute as a sub-in candidate for the given tactical need.
    /// `position_fit` is in [0.0, 1.0] (1.0 = exact position match).
    pub fn sub_in_score(
        sub: &MatchPlayer,
        need: TacticalNeed,
        position_fit: f32,
        development_priority: f32,
    ) -> f32 {
        let ca = (sub.player_attributes.current_ability as f32 / 200.0).clamp(0.0, 1.0);
        let cond = (sub.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
        let mut s = 0.30 * position_fit + 0.20 * ca + 0.14 * cond;

        let trait_fit = Self::trait_fit_score(sub, need);
        let need_fit = Self::need_fit_score(sub, need);

        s += 0.20 * need_fit;
        s += 0.10 * trait_fit;
        s += 0.06 * development_priority;
        s
    }

    fn need_fit_score(sub: &MatchPlayer, need: TacticalNeed) -> f32 {
        let s = &sub.skills;
        let pos_group = sub.tactical_position.current_position.position_group();
        match need {
            TacticalNeed::Chasing => {
                if !matches!(
                    pos_group,
                    PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder
                ) {
                    return 0.1;
                }
                (s.physical.pace * 0.30
                    + s.mental.off_the_ball * 0.25
                    + s.technical.finishing * 0.25
                    + s.technical.crossing * 0.20)
                    / 20.0
            }
            TacticalNeed::ProtectingLead => {
                if !matches!(
                    pos_group,
                    PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Midfielder
                ) {
                    return 0.1;
                }
                (s.mental.positioning * 0.30
                    + s.technical.tackling * 0.25
                    + s.mental.concentration * 0.25
                    + s.mental.work_rate * 0.20)
                    / 20.0
            }
            TacticalNeed::LosingMidfield => {
                if pos_group != PlayerFieldPositionGroup::Midfielder {
                    return 0.1;
                }
                (s.technical.passing * 0.30
                    + s.mental.vision * 0.25
                    + s.mental.decisions * 0.25
                    + s.mental.composure * 0.20)
                    / 20.0
            }
            TacticalNeed::BeingPressed => {
                (s.mental.composure * 0.30
                    + s.technical.first_touch * 0.30
                    + s.technical.passing * 0.20
                    + s.technical.technique * 0.20)
                    / 20.0
            }
            TacticalNeed::NeedingCrosses => {
                let wide = matches!(
                    sub.tactical_position.current_position,
                    PlayerPositionType::WingbackLeft
                        | PlayerPositionType::WingbackRight
                        | PlayerPositionType::MidfielderLeft
                        | PlayerPositionType::MidfielderRight
                        | PlayerPositionType::ForwardLeft
                        | PlayerPositionType::ForwardRight
                );
                if !wide {
                    return 0.2;
                }
                (s.technical.crossing * 0.40 + s.physical.pace * 0.30 + s.physical.stamina * 0.30)
                    / 20.0
            }
            TacticalNeed::Fatigue => 0.5,
        }
    }

    fn trait_fit_score(sub: &MatchPlayer, need: TacticalNeed) -> f32 {
        use crate::club::player::traits::PlayerTrait;
        let mut s: f32 = 0.0;
        match need {
            TacticalNeed::Chasing => {
                if sub.has_trait(PlayerTrait::GetsIntoOppositionArea) {
                    s += 0.3;
                }
                if sub.has_trait(PlayerTrait::ArrivesLateInOppositionArea) {
                    s += 0.2;
                }
                if sub.has_trait(PlayerTrait::RunsWithBallOften) {
                    s += 0.2;
                }
                if sub.has_trait(PlayerTrait::PowersShots)
                    || sub.has_trait(PlayerTrait::PlacesShots)
                {
                    s += 0.1;
                }
            }
            TacticalNeed::ProtectingLead => {
                if sub.has_trait(PlayerTrait::StaysBack) {
                    s += 0.3;
                }
                if sub.has_trait(PlayerTrait::MarkTightly) {
                    s += 0.2;
                }
                if sub.has_trait(PlayerTrait::StaysOnFeet) {
                    s += 0.2;
                }
            }
            TacticalNeed::LosingMidfield => {
                if sub.has_trait(PlayerTrait::Playmaker) {
                    s += 0.4;
                }
                if sub.has_trait(PlayerTrait::PlaysShortPasses)
                    || sub.has_trait(PlayerTrait::TriesThroughBalls)
                    || sub.has_trait(PlayerTrait::LikesToSwitchPlay)
                {
                    s += 0.2;
                }
            }
            TacticalNeed::BeingPressed => {
                if sub.has_trait(PlayerTrait::Playmaker) {
                    s += 0.2;
                }
                if sub.has_trait(PlayerTrait::PlaysShortPasses) {
                    s += 0.2;
                }
            }
            TacticalNeed::NeedingCrosses => {
                if sub.has_trait(PlayerTrait::HugsLine) {
                    s += 0.4;
                }
                if sub.has_trait(PlayerTrait::CurlsBall) {
                    s += 0.2;
                }
            }
            TacticalNeed::Fatigue => {}
        }
        s.clamp(0.0, 1.0)
    }

    /// Sub-timing windows in minutes — used by callers to gate when each
    /// substitution slot may be used. Real coaches stagger their tactical
    /// changes around the 60–80 minute window; injuries and red-card
    /// fallout are exceptions.
    pub fn allowed_in_window(sub_index: u8, match_minute: u32, force_critical: bool) -> bool {
        if force_critical {
            return match_minute >= 5;
        }
        match sub_index {
            0 => match_minute >= 55 && match_minute <= 88,
            1 => match_minute >= 65 && match_minute <= 88,
            2 => match_minute >= 75 && match_minute <= 92,
            _ => match_minute >= 85,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(
        goals: u16,
        assists: u16,
        rating: f32,
        goal_diff: i32,
        condition: i16,
        minute: u32,
    ) -> LiveSubstitutionStats {
        LiveSubstitutionStats {
            minutes_played: minute.min(120) as u16,
            goals,
            assists,
            key_passes: 0,
            shots_on_target: 0,
            xg: 0.0,
            errors_leading_to_goal: 0,
            yellow_cards: 0,
            red_cards: 0,
            live_rating: rating,
            condition,
            goal_diff,
            match_minute: minute,
        }
    }

    #[test]
    fn star_protection_zero_for_anonymous_player() {
        let s = live(0, 0, 6.4, 0, 8000, 60);
        assert_eq!(SubScoring::star_protection(&s), 0.0);
    }

    #[test]
    fn star_protection_lifts_for_single_goal() {
        let s = live(1, 0, 7.0, 0, 8000, 60);
        // single goal → 0.20, rating < 7.3 → 0, goal_diff != 1 → 0
        assert!((SubScoring::star_protection(&s) - 0.20).abs() < 1e-4);
    }

    #[test]
    fn star_protection_two_g_a_replaces_single_goal_tier() {
        let s_two_g = live(2, 0, 7.0, 0, 8000, 70);
        let s_g_and_a = live(1, 1, 7.0, 0, 8000, 70);
        // both should land at the 0.35 tier — confirms 2+ G/A replaces
        // (not stacks with) the single-goal tier.
        assert!((SubScoring::star_protection(&s_two_g) - 0.35).abs() < 1e-4);
        assert!((SubScoring::star_protection(&s_g_and_a) - 0.35).abs() < 1e-4);
    }

    #[test]
    fn star_protection_rating_tier_replaces_lower_band() {
        let s_high = live(0, 0, 8.1, 0, 8000, 70);
        let s_mid = live(0, 0, 7.4, 0, 8000, 70);
        assert!((SubScoring::star_protection(&s_high) - 0.35).abs() < 1e-4);
        assert!((SubScoring::star_protection(&s_mid) - 0.18).abs() < 1e-4);
    }

    #[test]
    fn star_protection_decisive_lead_scorer_gets_extra() {
        let s_one_up = live(1, 0, 7.0, 1, 8000, 70);
        let s_two_up = live(1, 0, 7.0, 2, 8000, 70);
        // leading by exactly one → +0.15 stacked on top of the goal tier
        assert!((SubScoring::star_protection(&s_one_up) - 0.35).abs() < 1e-4);
        // leading by two → no decisive bonus
        assert!((SubScoring::star_protection(&s_two_up) - 0.20).abs() < 1e-4);
    }

    #[test]
    fn star_protection_caps_at_stacked_top_tier() {
        // 2 goals + rating 8.2 + leading by one
        let s = live(2, 1, 8.2, 1, 8000, 80);
        // 0.35 (G/A) + 0.35 (rating 8.0+) + 0.15 (decisive) = 0.85
        assert!((SubScoring::star_protection(&s) - 0.85).abs() < 1e-4);
    }

    #[test]
    fn star_protection_tapers_to_zero_at_extreme_fatigue() {
        // Same star, three condition rungs: fresh / fatigued boundary /
        // exhausted floor. Above 30% the protection stands; at 15% it
        // is gone so the substitution loop can hook a finished scorer.
        let fresh = live(1, 0, 7.0, 0, 8000, 80);
        let on_boundary = live(1, 0, 7.0, 0, 3000, 80);
        let broken = live(1, 0, 7.0, 0, 1500, 80);
        let fresh_prot = SubScoring::star_protection(&fresh);
        let boundary_prot = SubScoring::star_protection(&on_boundary);
        let broken_prot = SubScoring::star_protection(&broken);
        assert!((fresh_prot - 0.20).abs() < 1e-4);
        assert!((boundary_prot - 0.20).abs() < 1e-4);
        assert!(broken_prot < 1e-4);
        // Half-way through the taper window (~22%) the bonus should be
        // roughly half — the linear shape lets callers reason about it.
        let mid_taper = live(1, 0, 7.0, 0, 2250, 80);
        let mid_prot = SubScoring::star_protection(&mid_taper);
        assert!((mid_prot - 0.10).abs() < 0.02, "mid_prot {mid_prot}");
    }

    #[test]
    fn star_protection_taper_applies_to_full_stack() {
        // Top-tier stacked protection (0.85 at 80% condition) must
        // fade with the same condition curve — otherwise a 2-goal
        // scorer at 18% condition still gets a 0.8+ shield, defeating
        // the whole purpose of the taper.
        let fresh = live(2, 1, 8.2, 1, 8000, 80);
        let broken = live(2, 1, 8.2, 1, 1800, 80);
        let fresh_prot = SubScoring::star_protection(&fresh);
        let broken_prot = SubScoring::star_protection(&broken);
        assert!((fresh_prot - 0.85).abs() < 1e-4);
        // Condition 18% → dampening (0.18 - 0.15) / 0.15 = 0.20.
        // 0.85 * 0.20 = 0.17 — well below the fresh value.
        assert!(
            broken_prot < 0.25,
            "broken-star protection {broken_prot} should have tapered"
        );
    }
}
