//! Post-match player effects: stats bookkeeping, morale events,
//! reputation update.
//!
//! All cross-cutting effects of "a match happened" live here instead
//! of leaking into the league-result pipeline. Role-transition tracking
//! (the `WonStartingPlace` / `LostStartingPlace` one-shots) is dispatched
//! to [`super::role`]; physical exertion / injury rolls live in
//! [`super::match_exertion`].

use super::scaling;
use super::types::{MatchOutcome, MatchParticipation};
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::player::Player;
use crate::{HappinessEventType, PlayerStatistics};

impl Player {
    /// React to finishing a match: stats bookkeeping, morale events,
    /// reputation update. All cross-cutting effects of "a match happened"
    /// live here instead of leaking into the league-result pipeline.
    pub fn on_match_played(&mut self, o: &MatchOutcome<'_>) {
        self.record_match_appearance(o);
        self.record_match_stats(o);
        self.record_match_events(o);
        self.record_match_reputation(o);
    }

    /// Named to a squad but never got off the bench. Small morale hit.
    pub fn on_match_dropped(&mut self) {
        self.happiness
            .add_event_default(HappinessEventType::MatchDropped);

        // Bench-only appearance: feeds the rolling starter ratio with a 0.0
        // sample so chronic dropping eventually flips the role state.
        const ALPHA: f32 = 0.25;
        self.happiness.starter_ratio = self.happiness.starter_ratio * (1.0 - ALPHA);
        self.happiness.appearances_tracked = self.happiness.appearances_tracked.saturating_add(1);
        self.evaluate_role_transition();
    }

    fn record_match_appearance(&mut self, o: &MatchOutcome<'_>) {
        let s = stats_bucket_mut(self, o.is_cup, o.is_friendly);
        match o.participation {
            MatchParticipation::Starter => s.played += 1,
            MatchParticipation::Substitute => s.played_subs += 1,
        }
    }

    fn record_match_stats(&mut self, o: &MatchOutcome<'_>) {
        // Feed the per-player form EMA before we mutate any stat bucket —
        // `effective_rating` is the post-settlement rating already used for
        // season averages and POM selection, so form stays consistent.
        if !o.is_friendly {
            self.load.update_form(o.effective_rating);
        }

        let s = stats_bucket_mut(self, o.is_cup, o.is_friendly);
        s.goals += o.stats.goals;
        s.assists += o.stats.assists;
        s.shots_on_target += o.stats.shots_on_target as f32;
        s.tackling += o.stats.tackles as f32;
        s.yellow_cards = s.yellow_cards.saturating_add(o.stats.yellow_cards as u8);
        s.red_cards = s.red_cards.saturating_add(o.stats.red_cards as u8);

        if o.stats.passes_attempted > 0 {
            let match_pct =
                (o.stats.passes_completed as f32 / o.stats.passes_attempted as f32 * 100.0) as u8;
            let games = s.played + s.played_subs;
            s.passes = if games <= 1 {
                match_pct
            } else {
                let prev = s.passes as f32;
                ((prev * (games - 1) as f32 + match_pct as f32) / games as f32) as u8
            };
        }

        let games = s.played + s.played_subs;
        s.average_rating = if games <= 1 {
            o.effective_rating
        } else {
            let prev = s.average_rating;
            (prev * (games - 1) as f32 + o.effective_rating) / games as f32
        };

        if o.is_motm {
            s.player_of_the_match = s.player_of_the_match.saturating_add(1);
        }

        // GK conceded / clean-sheet bookkeeping — only for starting GKs.
        // Subs who came on briefly don't get attributed the full team conceded.
        if self.position().is_goalkeeper()
            && matches!(o.participation, MatchParticipation::Starter)
        {
            let s = stats_bucket_mut(self, o.is_cup, o.is_friendly);
            s.conceded += o.team_goals_against as u16;
            if o.team_goals_against == 0 {
                s.clean_sheets += 1;
            }
        }
    }

    fn record_match_events(&mut self, o: &MatchOutcome<'_>) {
        if !o.is_friendly {
            // Rolling starter-share tracking — drives the WonStartingPlace /
            // LostStartingPlace one-shot transitions. Only competitive
            // matches count: pre-season minutes don't tell us anything
            // about the manager's matchday trust.
            self.update_role_state(o);
        }

        if o.is_motm {
            self.happiness
                .add_event_default(HappinessEventType::PlayerOfTheMatch);
        }

        // Friendlies don't generate the rest of the football-life events —
        // pre-season form, suspensions, derby narratives don't apply.
        if o.is_friendly {
            return;
        }

        // Sent off — embarrassing, plus the suspension fallout. Flat hit.
        if o.stats.red_cards > 0 {
            self.happiness
                .add_event_default(HappinessEventType::RedCardFallout);
        }

        // First competitive goal at this club. Stats are reset on club
        // change (see `on_transfer` / `on_loan`), so the only way the
        // running competitive total equals this match's goals is when
        // this is the first scoring match of the tenure. Long cooldown
        // prevents the milestone from firing again later in the spell.
        if o.stats.goals > 0 {
            let total_competitive = self.statistics.goals + self.cup_statistics.goals;
            if total_competitive == o.stats.goals
                && !self
                    .happiness
                    .has_recent_event(&HappinessEventType::FirstClubGoal, 300)
            {
                self.happiness
                    .add_event_default(HappinessEventType::FirstClubGoal);
            }
        }

        // Substitute impact: came on and made it count. Skip if already
        // tagged POM — no point double-firing for the same standout shift.
        if !o.is_motm
            && o.participation == MatchParticipation::Substitute
            && (o.stats.goals > 0 || o.stats.assists > 0 || o.effective_rating >= 7.3)
        {
            self.happiness
                .add_event_default(HappinessEventType::SubstituteImpact);
        }

        // Clean sheet pride for goalkeepers and defenders — both roles
        // genuinely care about a shutout. Starters get the full event;
        // unused subs aren't on the field but still share the team result
        // (skipped here — they don't even hit `record_match_events`).
        if o.team_goals_against == 0
            && (self.position().is_goalkeeper() || self.position().is_defender())
        {
            self.happiness
                .add_event_default(HappinessEventType::CleanSheetPride);
        }

        // Match-rating debrief. The catastrophic floor is now its own event
        // (`CostlyMistake`) instead of overloaded ManagerCriticism, mid-low
        // ratings still fire the manager event for a routine talking-to,
        // and the derby effect is moved out to `DerbyHero` / `DerbyDefeat`
        // so we don't double-count derby weight on top of personal form.
        if o.stats.match_rating >= 1.0 {
            if o.effective_rating < 5.5 {
                let extra = (5.5 - o.effective_rating).clamp(0.0, 2.0);
                self.happiness
                    .add_event(HappinessEventType::CostlyMistake, -(2.0 + extra));
            } else if o.effective_rating < 6.3 {
                let mag = -(2.0 + (6.3 - o.effective_rating).clamp(0.0, 0.8));
                self.happiness
                    .add_event(HappinessEventType::ManagerCriticism, mag);
            } else if o.effective_rating >= 7.5 {
                let mag = 1.5 + (o.effective_rating - 7.5).clamp(0.0, 2.5);
                self.happiness
                    .add_event(HappinessEventType::ManagerEncouragement, mag);
            }
        }

        // ── Decisive goal / fan / media reactions ───────────────────
        let cfg = HappinessConfig::default();
        let had_contribution = o.stats.goals > 0 || o.stats.assists > 0;

        // DecisiveGoal — scored or assisted in a single-goal team win.
        // Captures the late winner / only-goal-of-the-game moment without
        // needing minute-of-goal data. Cooldown 14d so a hot scoring run
        // still feels punctuated rather than fired every weekend.
        if had_contribution && o.team_won && o.goal_margin() == 1 {
            let pressure_mul = scaling::pressure_amplifier(
                self.attributes.important_matches,
                self.attributes.pressure,
            );
            let scene_mul = if o.is_cup || o.is_derby { 1.25 } else { 1.0 };
            let rep_mul =
                scaling::reputation_amplifier(self.player_attributes.current_reputation);
            let mag = cfg.catalog.decisive_goal * pressure_mul * scene_mul * rep_mul;
            self.happiness
                .add_event_with_cooldown(HappinessEventType::DecisiveGoal, mag, 14);
        }

        // FanPraise — supporters latch onto a stand-out display. Triggered
        // by POM, an excellent rating, or a goal/assist contribution in a
        // win. Reputation-amplified so high-profile players feel it more.
        let fan_praise_trigger =
            o.is_motm || o.effective_rating >= 8.0 || (o.team_won && had_contribution);
        if fan_praise_trigger {
            let rep_mul =
                scaling::reputation_amplifier(self.player_attributes.current_reputation);
            let scene_mul = if o.is_cup || o.is_derby { 1.2 } else { 1.0 };
            let mag = cfg.catalog.fan_praise * rep_mul * scene_mul;
            self.happiness
                .add_event_with_cooldown(HappinessEventType::FanPraise, mag, 21);
        }

        // FanCriticism — fans turn on a poor display, especially in
        // defeat or after a red card. Amplified by controversy/low
        // temperament; dampened by professionalism (settles ego).
        let fan_criticism_trigger = o.stats.red_cards > 0
            || o.effective_rating < 5.7
            || (o.team_lost && o.effective_rating < 6.2);
        if fan_criticism_trigger {
            let rep_mul =
                scaling::reputation_amplifier(self.player_attributes.current_reputation);
            let provoke_mul = scaling::criticism_amplifier(
                self.attributes.controversy,
                self.attributes.temperament,
            );
            let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
            let mag = cfg.catalog.fan_criticism * rep_mul * provoke_mul * prof_dampen;
            self.happiness
                .add_event_with_cooldown(HappinessEventType::FanCriticism, mag, 21);
        }

        // MediaPraise — strictly rarer than fan reaction; only fires for
        // a genuinely elite shift or a story-defining moment. Cooldown
        // 30d so press inches don't pile up week after week.
        let exceptional_gk_shutout = self.position().is_goalkeeper()
            && o.team_goals_against == 0
            && (o.is_cup || o.is_derby)
            && matches!(o.participation, MatchParticipation::Starter);
        let media_praise_trigger = o.effective_rating >= 8.3
            || (o.is_motm && (o.is_cup || o.is_derby))
            || exceptional_gk_shutout;
        if media_praise_trigger {
            let rep_mul =
                scaling::reputation_amplifier(self.player_attributes.current_reputation);
            let mag = cfg.catalog.media_praise * rep_mul;
            self.happiness
                .add_event_with_cooldown(HappinessEventType::MediaPraise, mag, 30);
        }

        // Derby outcome — proper rivalry-day events instead of recycled
        // manager talks. DerbyHero is reserved for standout performers
        // (scored, assisted, POM, ≥7.5 rating, or GK/DEF clean sheet
        // ≥7.2). Ordinary squad members on the winning side get the
        // squad-wide DerbyWin instead, so the event log doesn't claim
        // every fullback was the hero of the match.
        if o.is_derby {
            if o.team_won {
                let is_back_line =
                    self.position().is_goalkeeper() || self.position().is_defender();
                let standout = o.stats.goals > 0
                    || o.stats.assists > 0
                    || o.is_motm
                    || o.effective_rating >= 7.5
                    || (is_back_line
                        && o.team_goals_against == 0
                        && o.effective_rating >= 7.2);
                if standout {
                    let bonus = if o.stats.goals > 0 || o.is_motm {
                        2.0
                    } else if o.effective_rating >= 7.5 {
                        1.0
                    } else {
                        0.0
                    };
                    self.happiness.add_event(
                        HappinessEventType::DerbyHero,
                        cfg.catalog.derby_hero + bonus,
                    );
                } else {
                    self.happiness
                        .add_event_default(HappinessEventType::DerbyWin);
                }
            } else if o.team_lost {
                // Squad-wide base hit, with extra for poor performers /
                // red cards. Base around -3 (catalog), extra up to -3.0
                // for a red-card collapse, capped to keep magnitudes sane.
                let mut extra = 0.0f32;
                if o.effective_rating < 6.0 {
                    extra += (6.0 - o.effective_rating).clamp(0.0, 1.0) * 1.5;
                }
                if o.stats.red_cards > 0 {
                    extra += 1.5;
                }
                let extra = extra.clamp(0.0, 3.0);
                self.happiness.add_event(
                    HappinessEventType::DerbyDefeat,
                    cfg.catalog.derby_defeat - extra,
                );
            }
        }
    }

    fn record_match_reputation(&mut self, o: &MatchOutcome<'_>) {
        let rating_delta = (o.effective_rating - 6.0) * 20.0;
        let goal_bonus = o.stats.goals.min(3) as f32 * 15.0;
        let assist_bonus = o.stats.assists.min(3) as f32 * 8.0;
        let motm_bonus = if o.is_motm { 25.0 } else { 0.0 };
        let raw_delta = rating_delta + goal_bonus + assist_bonus + motm_bonus;

        if o.is_friendly {
            let home_delta = (raw_delta * 0.4 * o.league_weight) as i16;
            self.player_attributes.update_reputation(0, home_delta, 0);
        } else {
            let current_delta = (raw_delta * o.league_weight) as i16;
            let home_delta = (raw_delta * 0.6 * o.league_weight) as i16;
            let world_delta = (raw_delta * o.world_weight * o.league_weight) as i16;
            self.player_attributes
                .update_reputation(current_delta, home_delta, world_delta);
        }
    }
}

/// Pick the right `PlayerStatistics` bucket for the match — league,
/// cup, or pre-season friendly — so the call sites read declaratively
/// (`stats_bucket_mut(p, is_cup, is_friendly).goals += …`).
fn stats_bucket_mut(player: &mut Player, is_cup: bool, is_friendly: bool) -> &mut PlayerStatistics {
    if is_cup {
        &mut player.cup_statistics
    } else if is_friendly {
        &mut player.friendly_statistics
    } else {
        &mut player.statistics
    }
}
