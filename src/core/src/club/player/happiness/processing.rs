use crate::club::player::calculators::{
    ContractValuation, ValuationContext, expected_annual_value, package_inputs_from_contract,
};
use crate::club::player::player::Player;
use crate::club::{PlayerResult, PlayerStatusType};
use crate::utils::DateUtils;
use crate::{ContractType, HappinessEventType, PlayerSquadStatus};
use chrono::NaiveDate;

/// Club / coaching context fed into the six derived morale factors.
/// Decoupled from `GlobalContext` so the helpers can be unit-tested
/// without spinning up a simulator. Built once per weekly tick by the
/// caller (see `Player::simulate`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ClubMoraleContext {
    /// Best technical coach score on the club staff (0..20). Drives
    /// coach_credibility for outfield players.
    pub coach_best_technical: u8,
    /// Best mental coach score on the club staff (0..20). Drives
    /// coach_credibility weight for high-pressure / tactical players.
    pub coach_best_mental: u8,
    /// Best fitness coach score on the club staff (0..20). Drives
    /// coach_credibility for athleticism-dependent roles.
    pub coach_best_fitness: u8,
    /// Best goalkeeping coach score on the club staff (0..20). Drives
    /// coach_credibility for goalkeepers specifically.
    pub coach_best_goalkeeping: u8,
    /// Training facility quality (0..1) — feeds club_fit for ambitious
    /// pros who expect modern facilities.
    pub training_facility_quality: f32,
    /// Youth facility quality (0..1) — feeds club_fit for young players.
    pub youth_facility_quality: f32,
}

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
    /// Weekly happiness evaluation. Computes the seven legacy factors
    /// plus six derived "life in the team" factors (role clarity,
    /// coach credibility, dressing-room status, club fit, pressure
    /// load, promise trust). Takes `ClubMoraleContext` so the derived
    /// axes can read coach scores and facility quality.
    pub(crate) fn process_happiness_full(
        &mut self,
        result: &mut PlayerResult,
        now: NaiveDate,
        team_reputation: f32,
        season_state: TeamSeasonState,
        club_ctx: ClubMoraleContext,
    ) {
        let age = DateUtils::age(self.birth_date, now);
        let age_sensitivity = if age >= 24 && age <= 30 { 1.3 } else { 1.0 };

        // Decay old events weekly
        self.happiness.decay_events();

        // 1. Playing time vs squad status
        let playing_time_factor = self.calculate_playing_time_factor(age_sensitivity);
        self.happiness.factors.playing_time = playing_time_factor;

        // 2. Salary vs ability
        let mut salary_factor =
            self.calculate_salary_factor(age, team_reputation, season_state.league_reputation);

        // After 2 years of unresolved salary unhappiness, player accepts situation
        // and salary frustration dampens — prevents permanent unhappiness loops.
        // Must be applied BEFORE recalculate_morale() so dampening actually affects morale.
        let gave_up_on_salary = salary_factor <= -5.0
            && self
                .happiness
                .last_salary_negotiation
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
        let praise: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ManagerPraise)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        self.happiness.factors.recent_praise = praise.clamp(0.0, 10.0);

        let discipline: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ManagerDiscipline)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        self.happiness.factors.recent_discipline = discipline.clamp(-10.0, 0.0);

        // Loan-specific weekly modulation — extends the loan audit with
        // per-player morale signals (out-of-position, too-good, young
        // enjoying responsibility, veteran humiliation).
        if self.contract_loan.is_some() {
            self.process_loan_morale(team_reputation, season_state.league_reputation);
        }

        // ── Derived factors ───────────────────────────────────
        self.happiness.factors.role_clarity = self.calculate_role_clarity();
        self.happiness.factors.coach_credibility = self.calculate_coach_credibility(&club_ctx);
        self.happiness.factors.dressing_room_status = self.calculate_dressing_room_status();
        self.happiness.factors.club_fit =
            self.calculate_club_fit(team_reputation, season_state.league_reputation, &club_ctx);
        self.happiness.factors.pressure_load =
            self.calculate_pressure_load(team_reputation, season_state.league_reputation);
        self.happiness.factors.promise_trust = self.calculate_promise_trust(now);

        // Recalculate overall morale (now uses dampened salary factor + derived axes)
        self.happiness.recalculate_morale();

        // Salary unhappy: player wants contract renegotiation (with 1-year cooldown)
        if salary_factor <= -5.0 && !gave_up_on_salary {
            let cooldown_passed = self
                .happiness
                .last_salary_negotiation
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

    /// Salary factor uses the same `ContractValuation` as the renewal AI
    /// and personal-terms negotiation. Otherwise the three systems disagree
    /// on what a fair wage looks like — happiness might shout "underpaid"
    /// while the renewal AI is happily renewing on the same terms.
    ///
    /// Inputs the valuation already accounts for: ability, age, position,
    /// reputation, league prestige, club tier, status premium. The factor
    /// here is the gap between actual salary and expected; bonuses and
    /// recent renewals dampen frustration.
    fn calculate_salary_factor(
        &self,
        age: u8,
        team_reputation: f32,
        league_reputation: u16,
    ) -> f32 {
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

        // Pass the real club + league reputation so an elite Premier League
        // player's expectation isn't computed against a generic "mid-tier"
        // baseline. Falls back to the neutral 0.5 / 5000 only when the
        // caller couldn't provide context (zero values).
        let club_rep = if team_reputation > 0.0 {
            team_reputation.clamp(0.0, 1.0)
        } else {
            0.5
        };
        let league_rep = if league_reputation > 0 {
            league_reputation
        } else {
            5_000
        };
        let ctx = ValuationContext {
            age,
            club_reputation_score: club_rep,
            league_reputation: league_rep,
            squad_status: contract.squad_status.clone(),
            current_salary: contract.salary,
            // months_remaining doesn't affect expected_wage (only the
            // leverage band), so a constant keeps the factor stable.
            months_remaining: 24,
            has_market_interest: false,
        };

        let valuation = ContractValuation::evaluate(self, &ctx);
        let expected = valuation.expected_wage as f32;
        if expected < 1.0 {
            return 0.0;
        }

        // Use the shared package-value helper so happiness, acceptance,
        // and renewal scoring agree on what the package is "really
        // worth" annually. The helper amortises the signing bonus
        // (zeroing it out once paid via `signing_bonus_paid`),
        // probability-weights promotion / avoid-relegation bonuses, and
        // values per-event bonuses by realistic season frequencies.
        let inputs = package_inputs_from_contract(contract, self);
        let effective_salary = expected_annual_value(&inputs) as f32;

        let ratio = effective_salary / expected;
        let mut factor = if ratio >= 1.20 {
            (5.0 + (ratio - 1.20) * 8.0).min(12.0)
        } else if ratio >= 1.00 {
            (ratio - 1.0) * 25.0
        } else if ratio >= 0.80 {
            (ratio - 1.0) * 30.0
        } else if ratio >= 0.60 {
            -6.0 + (ratio - 0.80) * 30.0
        } else {
            -12.0 + (ratio - 0.60) * 15.0
        };

        // Loyalty veterans tolerate slightly below market — agent isn't
        // pushing them to chase every dollar.
        if self.attributes.loyalty >= 16.0 && ratio >= 0.85 {
            factor = (factor + 2.0).min(10.0);
        }

        // Just signed → don't resent the wage you just negotiated. The
        // renewal handler stamps `last_salary_negotiation` on accept; if
        // it's set and the factor is negative, soften the blow.
        if self.happiness.last_salary_negotiation.is_some() && factor < 0.0 {
            factor *= 0.85;
        }

        factor.clamp(-15.0, 15.0)
    }

    fn calculate_manager_relationship_factor(&mut self) -> f32 {
        // Manager-relationship factor is now a **derived** weekly summary,
        // not a free-floating accumulator. Three sources feed it:
        //
        //   * Staff relation level — long-term trust/respect with the
        //     coaching staff. The strongest negative or strongest positive
        //     dominates so a single broken-down relationship with the head
        //     coach isn't drowned out by neutral assistants.
        //   * Coach rapport — short-term delivery / training-talk rapport,
        //     stored on `PlayerRapport`. Maps the strongest existing rapport
        //     score onto a smaller −5..+5 contribution.
        //   * Recent promise & praise/discipline events — kept promises and
        //     manager praise lift the factor; broken promises and discipline
        //     drag it down.
        //
        // Each weekly evaluation overwrites the stored factor so a single
        // good (or bad) talk doesn't anchor morale forever — the underlying
        // staff relation has to actually be there for the factor to persist.
        // Old in-place writes from team_talks / behaviour result still feed
        // the staff relation + rapport stores, so their effect now decays
        // naturally with the rest of the social graph.

        // Strongest staff relation level — pick the signed maximum-magnitude
        // entry rather than averaging, so a single really bad coach
        // relationship registers.
        let staff_level: f32 = {
            let mut strongest = 0.0f32;
            for (_id, rel) in self.relations.staff_relations_iter() {
                if rel.level.abs() > strongest.abs() {
                    strongest = rel.level;
                }
            }
            // Map [-100, 100] → [-8, +8].
            (strongest / 100.0 * 8.0).clamp(-8.0, 8.0)
        };

        // Strongest rapport entry — same magnitude logic.
        let rapport_score: f32 = {
            let mut strongest: i16 = 0;
            for entry in self.rapport.coaches.iter() {
                if entry.score.abs() > strongest.abs() {
                    strongest = entry.score;
                }
            }
            // Map roughly [-50, 100] → [-5, +5]. Asymmetric because rapport
            // floor is -50 not -100.
            let normalised = if strongest >= 0 {
                (strongest as f32 / 100.0) * 5.0
            } else {
                (strongest as f32 / 50.0) * 5.0
            };
            normalised.clamp(-5.0, 5.0)
        };

        // Recent promise outcomes — kept lifts, broken hits hard.
        // Limit window to 60 days so the contribution decays naturally.
        let promise_kept: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == crate::HappinessEventType::PromiseKept && e.days_ago <= 60)
            .count() as f32;
        let promise_broken: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                e.event_type == crate::HappinessEventType::PromiseBroken && e.days_ago <= 60
            })
            .count() as f32;
        // Cap so a flurry of promise events doesn't dominate.
        let promise_contribution = (promise_kept * 4.0 - promise_broken * 8.0).clamp(-12.0, 8.0);

        // Recent praise / discipline — softer than promises, since the
        // dedicated factors `recent_praise` / `recent_discipline` already
        // count those events. We include them here too to anchor the
        // relationship summary on actual interactions.
        let praise_count: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                e.event_type == crate::HappinessEventType::ManagerPraise && e.days_ago <= 30
            })
            .count() as f32;
        let discipline_count: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                e.event_type == crate::HappinessEventType::ManagerDiscipline && e.days_ago <= 30
            })
            .count() as f32;
        let praise_contribution = (praise_count * 2.0 - discipline_count * 3.0).clamp(-6.0, 5.0);

        let derived = (staff_level + rapport_score + promise_contribution + praise_contribution)
            .clamp(-15.0, 15.0);

        // Persist the snapshot so external consumers (UI, debug tools) see
        // the same number this week.
        self.happiness.factors.manager_relationship = derived;
        derived
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

        let status_dampening =
            ambition_status_dampening(self.contract.as_ref().map(|c| &c.squad_status));
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
        let pos_pct = (s.league_position as f32 - 1.0) / (s.league_size as f32 - 1.0).max(1.0);

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

    // ── Six derived morale factors ───────────────────────────
    //
    // Each factor returns a signed value in roughly the band stated on
    // its `HappinessFactors` field doc. They're recomputed every weekly
    // tick and rolled into morale at 0.6× weight inside
    // `recalculate_morale`.

    /// role_clarity: does the player understand his role?
    fn calculate_role_clarity(&self) -> f32 {
        let mut score: f32 = 0.0;

        // Recent RoleMismatch events drag clarity down hard.
        let mismatch_pull: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::RoleMismatch)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 90.0).max(0.0))
            .sum();
        score += mismatch_pull * 0.5;

        // Squad status alignment with appearances. KeyPlayer with a
        // healthy starter ratio knows where he stands; KeyPlayer barely
        // playing has zero clarity even though the badge says regular.
        if let Some(c) = self.contract.as_ref() {
            let starter = self.happiness.starter_ratio;
            let alignment = match c.squad_status {
                PlayerSquadStatus::KeyPlayer => starter - 0.65,
                PlayerSquadStatus::FirstTeamRegular => starter - 0.50,
                PlayerSquadStatus::FirstTeamSquadRotation => starter - 0.30,
                PlayerSquadStatus::MainBackupPlayer => 0.20 - (starter - 0.20).abs(),
                _ => 0.0,
            };
            score += alignment * 6.0;
        }

        // Repeated tactical-role talks in the log signal the player
        // has been chasing clarity. One ask is normal; three+ in 90
        // days means he's not getting it.
        let tactical_asks = self
            .interactions
            .entries
            .iter()
            .filter(|e| {
                matches!(
                    e.topic,
                    crate::club::player::interaction::InteractionTopic::TacticalRole
                )
            })
            .count() as f32;
        if tactical_asks >= 3.0 {
            score -= (tactical_asks - 2.0) * 1.5;
        }

        score.clamp(-8.0, 5.0)
    }

    /// coach_credibility: do the coaches feel competent enough to coach
    /// this player? Compares the best-coach-on-staff scores against
    /// the player's CA. Goalkeepers weight the goalkeeping coach;
    /// outfield players weight technical+mental.
    fn calculate_coach_credibility(&self, ctx: &ClubMoraleContext) -> f32 {
        let player_ca = self.player_attributes.current_ability as f32;
        if player_ca < 60.0 {
            // Sub-60 CA players don't outgrow their coaches.
            return 0.0;
        }

        let is_gk = matches!(self.position(), crate::PlayerPositionType::Goalkeeper);
        let coach_score = if is_gk {
            ctx.coach_best_goalkeeping as f32
        } else {
            (ctx.coach_best_technical as f32 + ctx.coach_best_mental as f32) / 2.0
        };

        // Player expects coach quality scaled with own ability:
        // CA 100 → expect coach ≥ 10
        // CA 150 → expect coach ≥ 15
        // CA 180+ → expect coach ≥ 18 (top ten in the world)
        let expected_coach = (player_ca / 10.0).clamp(6.0, 19.0);
        let gap = coach_score - expected_coach;

        // Above expectations: respect, capped. Below: contempt scaling
        // by how big a star the player is. World-class player at a
        // small club coached by amateurs feels this most.
        if gap >= 0.0 {
            (gap * 0.8).clamp(0.0, 6.0)
        } else {
            let star_factor = (player_ca / 160.0).clamp(0.6, 1.8);
            (gap * 1.4 * star_factor).clamp(-8.0, 0.0)
        }
    }

    /// dressing_room_status: where does the player sit in the squad
    /// pecking order? Built from leadership skill, world reputation,
    /// and the player's own social-graph signals — bonding events lift,
    /// conflict events drag.
    fn calculate_dressing_room_status(&self) -> f32 {
        let leadership = self.skills.mental.leadership;
        let reputation = self.player_attributes.current_reputation as f32;

        // Base — leadership 0..20 → -2..+4, reputation lifts top end.
        let mut score: f32 = ((leadership - 10.0) * 0.3).clamp(-3.0, 4.0);
        score += (reputation / 10000.0).clamp(0.0, 1.0) * 3.0;

        // Recent bonding lifts standing; conflicts drag.
        let bonding: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::TeammateBonding)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        let conflict: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ConflictWithTeammate)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        score += (bonding * 0.4).clamp(0.0, 3.0);
        score += (conflict * 0.6).clamp(-4.0, 0.0);

        // Isolation events knock standing further.
        let isolated = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::FeelingIsolated && e.days_ago <= 60)
            .count() as f32;
        score -= isolated * 0.5;

        score.clamp(-6.0, 8.0)
    }

    /// club_fit: cultural / structural fit with the club. Reads
    /// language fluency, league level, and facilities against the
    /// player's ambition. Distinct from ambition_fit (which is purely
    /// reputation-vs-expectation) — club_fit captures the *day-to-day*
    /// experience of being at this club.
    fn calculate_club_fit(
        &self,
        team_reputation: f32,
        league_reputation: u16,
        ctx: &ClubMoraleContext,
    ) -> f32 {
        let mut score: f32 = 0.0;

        // Facility fit — ambitious pros expect modern training. Below
        // 0.4 facility quality and ambition ≥ 14 → noticeable hit.
        let ambition = self.attributes.ambition;
        let avg_facility = (ctx.training_facility_quality + ctx.youth_facility_quality) / 2.0;
        if avg_facility > 0.0 {
            let facility_gap = avg_facility - (ambition / 30.0);
            score += facility_gap * 6.0;
        }

        // League prestige fit — pros at a tier-4 league with high
        // ambition feel out of place even if the club is doing well.
        let league_norm = (league_reputation as f32 / 8000.0).clamp(0.2, 1.2);
        score += (league_norm - 0.5) * (ambition - 10.0) * 0.3;

        // Compatriot count + language progress events — cultural roots.
        let compatriots = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::CompatriotJoined && e.days_ago <= 180)
            .count() as f32;
        score += compatriots.min(2.0) * 0.8;

        let lang_progress = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::LanguageProgress && e.days_ago <= 180)
            .count() as f32;
        score += lang_progress.min(3.0) * 0.5;

        // (Favorite-club bonus is applied at signing time via the
        // DreamMove pathway — duplicating it weekly would double-count.)

        // Use team reputation only as a tiny tie-breaker — ambition_fit
        // already covers the "wrong size club" axis.
        score += (team_reputation - 0.5) * 1.5;

        score.clamp(-8.0, 6.0)
    }

    /// pressure_load: how heavy is the fan/media/board expectation
    /// relative to the player's pressure tolerance? High-rep players at
    /// big clubs always carry pressure; low-pressure personalities
    /// crack first. Outlier-above players (e.g. Messi at a small club)
    /// get an extra spotlight multiplier — the press follows them
    /// regardless of where the badge sits.
    fn calculate_pressure_load(&self, team_reputation: f32, league_reputation: u16) -> f32 {
        use crate::club::player::adaptation::ReputationGap;
        let rep_gap = ReputationGap::compute(self, team_reputation, league_reputation);
        let pressure = self.attributes.pressure;
        let player_rep = self.player_attributes.current_reputation as f32;
        let club_rep_score = team_reputation.clamp(0.0, 1.0) * 100.0;
        let league_score = (league_reputation as f32 / 100.0).clamp(0.0, 100.0);

        // Pressure index — bigger club, bigger league, higher player
        // rep → more eyes. Player_rep doubled because the public talks
        // about the player, not just the badge.
        let pressure_index =
            (club_rep_score * 0.4 + league_score * 0.3 + (player_rep / 100.0) * 0.6)
                .clamp(0.0, 100.0);

        // Tolerance: pressure attribute 0..20 → 0..100.
        let tolerance = pressure * 5.0;
        let index_gap = pressure_index - tolerance;

        // High rep player having a poor recent stretch under spotlight
        // (low form rating) — extra hit. Form ≥ 0 means we have data.
        let form = self.statistics.average_rating;
        let form_penalty = if form > 0.0 && form < 6.0 && pressure_index > 50.0 {
            -1.5
        } else {
            0.0
        };

        // Outlier-above amplifier: Messi at a small club draws every
        // camera regardless of league or club rep. Adds a floor of
        // pressure that scales with how far the rep gap is.
        let outlier_pull: f32 = if rep_gap.is_outlier_above() {
            (rep_gap.player_vs_club.max(rep_gap.player_vs_league) as f32 / 1000.0).clamp(0.0, 8.0)
        } else {
            0.0
        };

        let raw = if index_gap <= 0.0 {
            (-index_gap * 0.05).clamp(0.0, 3.0) - outlier_pull * 0.5
        } else {
            (-(index_gap / 12.0) + form_penalty - outlier_pull * 0.5).clamp(-8.0, 0.0)
        };
        raw.clamp(-8.0, 3.0)
    }

    /// promise_trust: the player's belief in the manager's word. Built
    /// from kept-vs-broken promise events, recent broken-promise
    /// frequency, and current credibility of the manager-relationship.
    fn calculate_promise_trust(&self, _now: NaiveDate) -> f32 {
        let kept: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::PromiseKept && e.days_ago <= 180)
            .count() as f32;
        let broken: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::PromiseBroken && e.days_ago <= 180)
            .count() as f32;

        // Kept builds slowly; broken cuts trust hard. Asymmetry
        // mirrors the rapport "easy to lose" model.
        let mut score = kept * 1.5 - broken * 3.5;

        // Pending promises with high importance and low credibility
        // erode trust just by sitting there — the player notices the
        // manager is overpromising.
        let overhang: f32 = self
            .promises
            .iter()
            .filter(|p| p.credibility_at_creation < 40 && p.importance_to_player >= 60)
            .count() as f32;
        score -= overhang * 1.5;

        score.clamp(-10.0, 6.0)
    }

    /// Weekly loan-life modulation — only called when the player is on
    /// loan. Emits the "too good for this level" frustration for elite
    /// veterans, the "first taste of responsibility" lift for young
    /// loanees, an "out of position" hit when role mismatch lingers,
    /// and an underperformance signal when the player's form is poor
    /// at a smaller club (the loan isn't working out).
    fn process_loan_morale(&mut self, team_reputation: f32, league_reputation: u16) {
        use crate::club::player::adaptation::ReputationGap;
        let gap = ReputationGap::compute(self, team_reputation, league_reputation);
        let age = DateUtils::age(
            self.birth_date,
            self.last_transfer_date.unwrap_or_else(|| {
                self.contract_loan
                    .as_ref()
                    .and_then(|c| c.started)
                    .unwrap_or(self.birth_date)
            }),
        );
        let world_rep = self.player_attributes.world_reputation;

        // Veteran humiliation — elite vet (32+, world_rep ≥ 6500)
        // sitting in a clearly smaller league. Gentle ongoing drag.
        if age >= 32 && world_rep >= 6500 && gap.is_outlier_above() {
            self.happiness
                .add_event_with_cooldown(HappinessEventType::AmbitionShock, -3.0, 30);
        }

        // Young loanee enjoying responsibility — under-23, in a
        // smaller club / lower league than parent, getting starts.
        // Trigger only if the player has ≥ 5 starts since arriving.
        let starts = self.statistics.played;
        if age <= 22
            && world_rep < 5000
            && starts >= 5
            && (gap.player_vs_club <= 0 || gap.player_vs_league <= 0)
        {
            self.happiness
                .add_event_with_cooldown(HappinessEventType::SettledIntoSquad, 2.5, 60);
        }

        // Used out of position — RoleMismatch event still active and
        // the player hasn't been moved back. Recurring small hit.
        let recent_mismatch = self
            .happiness
            .recent_events
            .iter()
            .any(|e| e.event_type == HappinessEventType::RoleMismatch && e.days_ago <= 28);
        if recent_mismatch {
            self.happiness
                .add_event_with_cooldown(HappinessEventType::LackOfPlayingTime, -2.0, 21);
        }

        // Loan underperformance — apps but rating < 6.0 means the loan
        // isn't producing minutes the parent club would value. Adds
        // the "I'm not enjoying this" hit.
        let apps = self.statistics.played + self.statistics.played_subs;
        let form = self.statistics.average_rating;
        if apps >= 6 && form > 0.0 && form < 6.0 {
            self.happiness
                .add_event_with_cooldown(HappinessEventType::PoorTraining, -1.5, 28);
        }
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
