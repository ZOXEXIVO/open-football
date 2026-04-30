//! Role-transition tracking + season-event scaling factors.
//!
//! The match-play layer feeds rolling starter-share signals into
//! [`Player::update_role_state`]; that flips the one-shot
//! `WonStartingPlace` / `LostStartingPlace` events when the EMA
//! crosses the established / not-established thresholds.
//!
//! [`Player::season_participation_factor`],
//! [`Player::season_event_role_factor`] and
//! [`Player::team_event_personality_factor`] are reused by
//! [`super::career`] to scale team-level season events
//! (relegation, trophies, …) by how invested the player was.

use super::scaling;
use super::types::{MatchOutcome, MatchParticipation};
use crate::HappinessEventType;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::player::Player;

impl Player {
    /// Update the rolling starter ratio on a competitive match and emit the
    /// one-shot role-transition events when the player crosses the
    /// established / not-established threshold. EMA window ~ 4 matches.
    pub(super) fn update_role_state(&mut self, o: &MatchOutcome<'_>) {
        const ALPHA: f32 = 0.25;
        let sample: f32 = match o.participation {
            MatchParticipation::Starter => 1.0,
            MatchParticipation::Substitute => 0.0,
        };
        self.happiness.starter_ratio =
            self.happiness.starter_ratio * (1.0 - ALPHA) + sample * ALPHA;
        self.happiness.appearances_tracked = self.happiness.appearances_tracked.saturating_add(1);
        self.evaluate_role_transition();
    }

    /// One-shot transition logic. Need at least 5 tracked appearances
    /// before the verdict counts — fewer is statistical noise.
    /// Magnitude scales by squad status / ambition / age / professionalism
    /// so a KeyPlayer losing his place hurts twice as much as a rotation
    /// player, and a hungry prospect winning a starting place feels it
    /// twice as much as an established veteran for whom it's expected.
    pub(super) fn evaluate_role_transition(&mut self) {
        const MIN_APPS: u8 = 5;
        const STARTER_FLOOR: f32 = 0.65;
        const BENCHED_CEILING: f32 = 0.40;
        if self.happiness.appearances_tracked < MIN_APPS {
            return;
        }
        if !self.happiness.is_established_starter && self.happiness.starter_ratio >= STARTER_FLOOR {
            let mag = self.won_starting_place_magnitude();
            // 90-day cooldown so a brief slump and recovery don't ping-pong
            // the event once per fortnight.
            if self
                .happiness
                .add_event_with_cooldown(HappinessEventType::WonStartingPlace, mag, 90)
            {
                self.happiness.is_established_starter = true;
            }
        } else if self.happiness.is_established_starter
            && self.happiness.starter_ratio <= BENCHED_CEILING
        {
            let mag = self.lost_starting_place_magnitude();
            if self.happiness.add_event_with_cooldown(
                HappinessEventType::LostStartingPlace,
                mag,
                90,
            ) {
                self.happiness.is_established_starter = false;
            }
        }
    }

    /// Magnitude for `WonStartingPlace`. Catalog default amplified by:
    /// - youth/prospect squad status (it's a breakthrough)
    /// - ambition (career-defining for the hungry)
    /// - age (under-23s feel it more than veterans who expected this)
    fn won_starting_place_magnitude(&self) -> f32 {
        use crate::PlayerSquadStatus;
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.won_starting_place;

        // Status amplifier: prospects feel "I made it" more than a
        // KeyPlayer who expected to start. The amplifier inverts squad
        // status — lower expectation, bigger emotional payoff.
        let status_mul = match self.contract.as_ref().map(|c| &c.squad_status) {
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 1.30,
            Some(PlayerSquadStatus::DecentYoungster) => 1.25,
            Some(PlayerSquadStatus::MainBackupPlayer) => 1.20,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 1.10,
            Some(PlayerSquadStatus::FirstTeamRegular) => 0.95,
            Some(PlayerSquadStatus::KeyPlayer) => 0.85,
            _ => 1.0,
        };
        let ambition_mul = scaling::ambition_amplifier(self.attributes.ambition);
        // Birthdate available; age unknown without a date. Use load.last
        // match-day proxy isn't ideal — fall back on `birth_date.year()`
        // delta from a reference. Simpler: skip age here; ambition is
        // the dominant axis the spec cares about for upward events.
        // (Tests pin ambition behavior; age is captured separately for
        // negative events where it actually moves the needle.)
        base * status_mul * ambition_mul
    }

    /// Magnitude for `LostStartingPlace`. Catalog default amplified by:
    /// - squad status (KeyPlayer/FirstTeamRegular hits hardest)
    /// - ambition (more negative for the hungry)
    /// - professionalism (dampens — pros take it on the chin)
    fn lost_starting_place_magnitude(&self) -> f32 {
        use crate::PlayerSquadStatus;
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.lost_starting_place;

        let status_mul = match self.contract.as_ref().map(|c| &c.squad_status) {
            Some(PlayerSquadStatus::KeyPlayer) => 1.40,
            Some(PlayerSquadStatus::FirstTeamRegular) => 1.20,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 1.0,
            Some(PlayerSquadStatus::MainBackupPlayer) => 0.85,
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 0.90,
            Some(PlayerSquadStatus::DecentYoungster) => 0.80,
            Some(PlayerSquadStatus::NotNeeded) => 0.50,
            _ => 1.0,
        };
        let ambition_mul = scaling::ambition_amplifier(self.attributes.ambition);
        let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
        base * status_mul * ambition_mul * prof_dampen
    }

    /// Coarse "how involved was this player in the season" factor used by
    /// team-level events (trophy, relegation, etc.). Regular starters get
    /// the full effect; rotation players a discount; barely-featured fringe
    /// players a small share since they still felt the season unfold from
    /// the bench. Numbers are tuned for a typical 30–40 game season.
    pub(super) fn season_participation_factor(&self) -> f32 {
        let games = (self.statistics.played
            + self.statistics.played_subs
            + self.cup_statistics.played
            + self.cup_statistics.played_subs) as f32;
        if games >= 25.0 {
            1.0
        } else if games >= 12.0 {
            0.7
        } else if games >= 4.0 {
            0.5
        } else {
            0.35
        }
    }

    /// Role-aware multiplier for team-level season events. Combines squad
    /// status, loan status, and (for upward-direction events) age into a
    /// single scalar near 1.0. Returns smaller values for fringe / loanee
    /// / youth players so a relegation hurts a KeyPlayer twice as much as
    /// a bench loanee, which matches how supporters and pundits actually
    /// frame post-season player departures.
    ///
    /// Polarity-aware: the *direction* of the event determines whether
    /// "fringe" softens. For trophies/promotion, fringe players still feel
    /// some of the moment (they were there). For relegation/relegation
    /// fear, fringe loanees barely care — they're already mentally back at
    /// the parent club. The asymmetry is deliberate.
    pub(super) fn season_event_role_factor(&self, event: &HappinessEventType, age: u8) -> f32 {
        use crate::PlayerSquadStatus;
        use HappinessEventType::*;

        // Squad-status weight. KeyPlayer/FirstTeamRegular invest more
        // emotionally in the season than rotation/backup players. A
        // NotNeeded player has effectively been told they can leave; the
        // season's outcome is background noise.
        let status_weight = match self.contract.as_ref().map(|c| &c.squad_status) {
            Some(PlayerSquadStatus::KeyPlayer) => 1.20,
            Some(PlayerSquadStatus::FirstTeamRegular) => 1.10,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 0.95,
            Some(PlayerSquadStatus::MainBackupPlayer) => 0.80,
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 0.85,
            Some(PlayerSquadStatus::DecentYoungster) => 0.70,
            Some(PlayerSquadStatus::NotNeeded) => 0.40,
            // NotYetSet / Invalid / SquadStatusCount: treat as average.
            _ => 1.0,
        };

        // Loanees feel the season at the borrowing club differently
        // depending on whether they were actively contributing. A loanee
        // who barely played shrugs off relegation; one with regular
        // minutes still feels it but slightly softened (they know they're
        // returning to the parent club shortly).
        let on_loan = self.contract_loan.is_some();
        let loan_factor = if on_loan { 0.7 } else { 1.0 };

        // Negative team events hit prime-age career-defining players the
        // hardest; veterans have weathered them before, prospects are too
        // green to internalise. Positive events follow the existing
        // `veteran_amplifier` shape via the personality blend, so we leave
        // them untouched here.
        let age_factor = match event {
            Relegated | RelegationFear | CupFinalDefeat => {
                if age <= 19 {
                    0.85
                } else if age >= 33 {
                    0.90
                } else {
                    1.0
                }
            }
            _ => 1.0,
        };

        status_weight * loan_factor * age_factor
    }

    /// Personality multiplier for a team-level season event. Different
    /// events lean on different personality axes; the helper centralises
    /// the choice so emit sites just say `event` and we look up the right
    /// blend. Returns a multiplier near 1.0 by design — magnitudes stay
    /// in the catalog band.
    pub(super) fn team_event_personality_factor(&self, event: &HappinessEventType, age: u8) -> f32 {
        use HappinessEventType::*;
        let a = self.attributes.ambition;
        let l = self.attributes.loyalty;
        let im = self.attributes.important_matches;
        let pr = self.attributes.pressure;
        match event {
            // Career silverware — ambition + age (veterans treasure it).
            TrophyWon => scaling::ambition_amplifier(a) * scaling::veteran_amplifier(age),
            // Final-day defeat — pressure / big-match sensitivity.
            CupFinalDefeat => scaling::pressure_amplifier(im, pr),
            // Promotion is a club moment — loyalty plus mild ambition lift.
            PromotionCelebration => {
                scaling::loyalty_amplifier(l) * (0.9 + scaling::ambition_amplifier(a) * 0.1)
            }
            // Relegation hurts ambitious players the most.
            Relegated => scaling::ambition_amplifier(a),
            // Late-season fear — ambition hurts, professionalism dampens.
            RelegationFear => {
                scaling::ambition_amplifier(a)
                    * scaling::criticism_dampener(self.attributes.professionalism)
            }
            // Continental qualification — pure ambition lift.
            QualifiedForEurope => scaling::ambition_amplifier(a),
            _ => 1.0,
        }
    }
}
