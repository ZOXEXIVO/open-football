use crate::club::player::player::Player;
use crate::club::{PlayerResult, PlayerStatusType};
use crate::utils::DateUtils;
use crate::{ContractType, HappinessEventType, PlayerSquadStatus};
use chrono::NaiveDate;

/// Snapshot of the team's current competitive standing — fed into
/// `calculate_ambition_fit` so a high-ambition player at a club who's
/// bottom of the table reacts to the season, not just to the club badge.
#[derive(Debug, Clone, Copy, Default)]
pub struct TeamSeasonState {
    /// 1-based league position; 0 if unknown.
    pub league_position: u8,
    /// Number of teams in the league; 0 if unknown.
    pub league_size: u8,
    /// Season progress 0.0..1.0 (matches played / total league matches).
    pub season_progress: f32,
    /// League reputation (0..10000). Contextualises position — top of
    /// a Tier-4 league isn't the same as top of the Premier League.
    pub league_reputation: u16,
}

impl Player {
    /// Weekly happiness evaluation with 6 real-world factors
    pub(crate) fn process_happiness(
        &mut self,
        result: &mut PlayerResult,
        now: NaiveDate,
        team_reputation: f32,
        season_state: TeamSeasonState,
    ) {
        let age = DateUtils::age(self.birth_date, now);
        let age_sensitivity = if age >= 24 && age <= 30 { 1.3 } else { 1.0 };

        // Decay old events weekly
        self.happiness.decay_events();

        // 1. Playing time vs squad status
        let playing_time_factor = self.calculate_playing_time_factor(age_sensitivity);
        self.happiness.factors.playing_time = playing_time_factor;

        // 2. Salary vs ability
        let mut salary_factor = self.calculate_salary_factor(age);

        // After 2 years of unresolved salary unhappiness, player accepts situation
        // and salary frustration dampens — prevents permanent unhappiness loops.
        // Must be applied BEFORE recalculate_morale() so dampening actually affects morale.
        let gave_up_on_salary = salary_factor <= -5.0
            && self.happiness.last_salary_negotiation
                .map(|d| (now - d).num_days() > 730)
                .unwrap_or(false);

        if gave_up_on_salary {
            salary_factor = (salary_factor * 0.5).clamp(-5.0, 0.0);
        }

        self.happiness.factors.salary_satisfaction = salary_factor;

        // 3. Manager relationship
        let manager_factor = self.calculate_manager_relationship_factor();
        self.happiness.factors.manager_relationship = manager_factor;

        // 4. Injury frustration
        let injury_factor = self.calculate_injury_frustration();
        self.happiness.factors.injury_frustration = injury_factor;

        // 5. Ambition vs club level (structural) plus season trajectory
        // (dynamic). A high-ambition player at a big club fighting
        // relegation is unhappy even though the prestige fits.
        let ambition_factor = self.calculate_ambition_fit(team_reputation, &season_state);
        self.happiness.factors.ambition_fit = ambition_factor;

        // 6. Praise/discipline from recent events (tracked separately)
        let praise: f32 = self.happiness.recent_events.iter()
            .filter(|e| e.event_type == HappinessEventType::ManagerPraise)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        self.happiness.factors.recent_praise = praise.clamp(0.0, 10.0);

        let discipline: f32 = self.happiness.recent_events.iter()
            .filter(|e| e.event_type == HappinessEventType::ManagerDiscipline)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        self.happiness.factors.recent_discipline = discipline.clamp(-10.0, 0.0);

        // Recalculate overall morale (now uses dampened salary factor)
        self.happiness.recalculate_morale();

        // Salary unhappy: player wants contract renegotiation (with 1-year cooldown)
        if salary_factor <= -5.0 && !gave_up_on_salary {
            let cooldown_passed = self.happiness.last_salary_negotiation
                .map(|d| (now - d).num_days() >= 365)
                .unwrap_or(true);

            if cooldown_passed {
                result.contract.want_improve_contract = true;
                if self.happiness.last_salary_negotiation.is_none() {
                    self.happiness.last_salary_negotiation = Some(now);
                }
            }
        } else if salary_factor > -5.0 && !gave_up_on_salary {
            // Salary is acceptable now — reset negotiation tracking
            self.happiness.last_salary_negotiation = None;
        }
        // If gave_up_on_salary: keep last_salary_negotiation but don't request improvements

        // Set Unh status if morale < 35. Recovery back to "normal" happens
        // when morale climbs above 50 — OR when morale is above 40 and the
        // player is clearly getting the match minutes they expect (the
        // manager's visible trust is enough to pull them out of the slump).
        if self.happiness.morale < 35.0 {
            if !self.statuses.get().contains(&PlayerStatusType::Unh) {
                self.statuses.add(now, PlayerStatusType::Unh);
            }
            result.unhappy = true;
        } else if self.happiness.morale > 50.0
            || (self.happiness.morale > 40.0 && playing_time_factor >= 10.0)
        {
            self.statuses.remove(PlayerStatusType::Unh);
            result.unhappy = false;
        } else {
            result.unhappy = !self.happiness.is_happy();
        }
    }

    fn calculate_playing_time_factor(&self, age_sensitivity: f32) -> f32 {
        let total = self.statistics.played + self.statistics.played_subs;
        if total < 5 {
            return 0.0;
        }

        // Only skilled players care strongly about playing time.
        // Low-ability players (bench warmers) accept their role more easily.
        let ability = self.player_attributes.current_ability as f32;
        // ability_factor: 0.0 at ability 40, 1.0 at ability 120+
        // Players below 40 CA don't get upset about playing time at all
        if ability < 40.0 {
            return 0.0;
        }
        let ability_factor = ((ability - 40.0) / 80.0).clamp(0.0, 1.0);

        let play_ratio = self.statistics.played as f32 / total as f32;

        let (expected_ratio, unhappy_threshold) = if let Some(ref contract) = self.contract {
            match contract.squad_status {
                PlayerSquadStatus::KeyPlayer => (0.70, 0.50),
                PlayerSquadStatus::FirstTeamRegular => (0.50, 0.30),
                PlayerSquadStatus::FirstTeamSquadRotation => (0.25, 0.15),
                PlayerSquadStatus::MainBackupPlayer => (0.20, 0.10),
                PlayerSquadStatus::HotProspectForTheFuture => (0.10, 0.05),
                PlayerSquadStatus::DecentYoungster => (0.10, 0.05),
                PlayerSquadStatus::NotNeeded => (0.05, 0.0),
                _ => (0.30, 0.15),
            }
        } else {
            (0.30, 0.15)
        };

        let factor = if play_ratio >= expected_ratio {
            // Meeting or exceeding expectations
            let excess = (play_ratio - expected_ratio) / (1.0 - expected_ratio).max(0.01);
            excess * 20.0
        } else if play_ratio < unhappy_threshold {
            // Below unhappy threshold — scaled by ability
            let deficit = (unhappy_threshold - play_ratio) / unhappy_threshold.max(0.01);
            -deficit * 20.0 * age_sensitivity * ability_factor
        } else {
            // Between unhappy and expected - mild dissatisfaction, scaled by ability
            let range = expected_ratio - unhappy_threshold;
            let position = (play_ratio - unhappy_threshold) / range.max(0.01);
            (position - 0.5) * 10.0 * ability_factor
        };

        factor.clamp(-20.0, 20.0)
    }

    fn calculate_salary_factor(&self, age: u8) -> f32 {
        let Some(ref contract) = self.contract else {
            return -5.0;
        };

        // Players on loan accept their temporary salary — no frustration
        if self.contract_loan.is_some() {
            return 0.0;
        }

        // Youth/amateur players don't evaluate salary competitively
        match contract.contract_type {
            ContractType::Youth | ContractType::Amateur | ContractType::NonContract => return 0.0,
            _ => {}
        }

        let ability = self.player_attributes.current_ability as f32;

        // Map ability to expected salary matching the generation curve:
        // Salaries are generated as random(2k + rep*30k, 10k + rep*190k)
        // Ability ~30-170 roughly maps to rep_factor 0.0-1.0
        let ability_ratio = ((ability - 30.0) / 140.0).clamp(0.0, 1.0);
        let expected_base = 6000.0 + ability_ratio * 110000.0;

        let age_factor = if age < 22 { 0.7 } else if age > 30 { 0.85 } else { 1.0 };
        let expected = expected_base * age_factor;

        if expected < 1.0 {
            return 0.0;
        }

        let ratio = contract.salary as f32 / expected;
        let factor = if ratio >= 1.2 {
            // Well paid
            10.0_f32.min(ratio * 5.0)
        } else if ratio >= 0.8 {
            // Fairly paid
            (ratio - 0.8) * 25.0 // 0 to 10
        } else {
            // Underpaid
            (ratio - 0.8) * 37.5
        };

        factor.clamp(-15.0, 15.0)
    }

    fn calculate_manager_relationship_factor(&mut self) -> f32 {
        // Driven by manager talks which write directly to the factor, but
        // without decay a single good (or bad) chat anchored a player's
        // morale forever. Drift the stored factor 12% toward zero every
        // week so the effect of any single talk fades over ~2 months.
        let decayed = self.happiness.factors.manager_relationship * 0.88;
        // Snap tiny residues to 0 so they don't drift forever.
        let decayed = if decayed.abs() < 0.1 { 0.0 } else { decayed };
        self.happiness.factors.manager_relationship = decayed;
        decayed
    }

    fn calculate_injury_frustration(&self) -> f32 {
        if !self.player_attributes.is_injured {
            return 0.0;
        }

        let injury_days = self.player_attributes.injury_days_remaining as f32;
        if injury_days <= 14.0 {
            return -2.0;
        }

        // Longer injuries cause more frustration: -5 to -10
        let severity = ((injury_days - 14.0) / 60.0).min(1.0);
        -(5.0 + severity * 5.0)
    }

    fn calculate_ambition_fit(&self, team_reputation: f32, season: &TeamSeasonState) -> f32 {
        let ambition = self.attributes.ambition;
        if ambition <= 10.0 {
            return 0.0;
        }

        let status_dampening = ambition_status_dampening(self.contract.as_ref().map(|c| &c.squad_status));
        let prestige = self.prestige_fit_component(ambition, team_reputation, status_dampening);
        let trajectory = self.season_trajectory_component(ambition, season, status_dampening);

        (prestige + trajectory).clamp(-15.0, 12.0)
    }

    /// Classic "I joined a club befitting my stature" piece — compares
    /// the player's ambition against the club's all-time reputation.
    fn prestige_fit_component(
        &self,
        ambition: f32,
        team_reputation: f32,
        status_dampening: f32,
    ) -> f32 {
        let club_rep = team_reputation * 10000.0;
        let expected_rep = (ambition - 10.0) * 800.0;

        let raw = if club_rep >= expected_rep {
            let excess = ((club_rep - expected_rep) / 2000.0).min(1.0);
            excess * 5.0
        } else {
            let deficit = ((expected_rep - club_rep) / expected_rep.max(1.0)).min(1.0);
            -deficit * 10.0 * status_dampening
        };

        raw.clamp(-10.0, 5.0)
    }

    /// "Where is this team actually going this season?" — league position
    /// relative to where a player of this ambition expects to finish,
    /// weighted by how far into the season we are.
    ///
    /// Drives the relegation / mid-table-slump exodus: a Key Player at a
    /// Premier League club sitting 18th with 30 matches played piles up
    /// enough negative magnitude to tip into Unh → Req.
    ///
    /// League reputation contextualises expectations: "top of a Tier-4
    /// league" doesn't satisfy a world-class ambition. An ambitious
    /// player at a minnow over-performing in the bottom division
    /// doesn't feel ambition is satisfied, just mildly less frustrated.
    fn season_trajectory_component(
        &self,
        ambition: f32,
        s: &TeamSeasonState,
        status_dampening: f32,
    ) -> f32 {
        if s.league_position == 0 || s.league_size < 4 {
            return 0.0;
        }

        // 0.0 = top, 1.0 = bottom
        let pos_pct = (s.league_position as f32 - 1.0)
            / (s.league_size as f32 - 1.0).max(1.0);

        // Ambition 20 expects top (~5%), ambition 15 expects top-third
        // (~33%), ambition 10 accepts mid-table (~70%).
        let expected_pct = ((20.0 - ambition) / 14.0).clamp(0.05, 0.7);

        let gap = pos_pct - expected_pct;

        // Early season is noisy — a 10-game blip isn't fate. Weight scales
        // from 0.25 at season start to 1.0 by the two-thirds mark.
        let weight = (s.season_progress * 1.5).clamp(0.25, 1.0);

        // Prestige ambition (20) scoring anywhere outside the top of a
        // top-tier league is disappointing. For a tier-4 league a high
        // ambition player already feels out of place — league reputation
        // shrinks the positive side of the factor here.
        let league_rep_factor = (s.league_reputation as f32 / 8000.0).clamp(0.2, 1.2);

        let raw = if gap <= 0.0 {
            // Better than expected — positive, but scaled by league rep
            // so "top of non-league" feels flatter than "top of Serie A".
            let excess = (-gap).min(0.5) / 0.5;
            excess * 4.0 * league_rep_factor
        } else {
            // Worse than expected — dampened by squad status.
            // Relegation zone (bottom 15%) gets an extra penalty.
            let mut deficit_mag = gap * 18.0;
            if pos_pct >= 0.85 {
                deficit_mag += 3.0;
            }
            -deficit_mag * status_dampening
        };

        (raw * weight).clamp(-10.0, 5.0)
    }
}

fn ambition_status_dampening(status: Option<&PlayerSquadStatus>) -> f32 {
    match status {
        Some(PlayerSquadStatus::KeyPlayer) => 1.0,
        Some(PlayerSquadStatus::FirstTeamRegular) => 0.8,
        Some(PlayerSquadStatus::FirstTeamSquadRotation) => 0.4,
        Some(PlayerSquadStatus::MainBackupPlayer) => 0.2,
        Some(PlayerSquadStatus::HotProspectForTheFuture)
        | Some(PlayerSquadStatus::DecentYoungster) => 0.1,
        Some(PlayerSquadStatus::NotNeeded) => 0.3,
        _ => 0.5,
    }
}
