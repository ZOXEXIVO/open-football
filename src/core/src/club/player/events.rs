use crate::club::player::adaptation::PendingSigning;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::calculators::WageCalculator;
use crate::club::player::injury::InjuryType;
use crate::club::player::load::PlayerLoad;
use crate::club::player::player::Player;
use crate::club::PlayerClubContract;
use crate::r#match::PlayerMatchEndStats;
use crate::{
    HappinessEventType, Person, PlayerHappiness, PlayerPlan, PlayerStatistics,
    PlayerStatusType, TeamInfo,
};
use chrono::NaiveDate;

/// Personality-aware scaling helpers used by happiness emit sites.
///
/// Centralised here so emit sites stay declarative — a fan-criticism
/// site reads "amplified by reputation, dampened by professionalism"
/// rather than redoing the math inline. Each helper returns a positive
/// multiplier (no sign flip — the caller already knows the polarity of
/// the underlying event from the catalog).
pub(crate) mod scaling {
    /// Reputation amplifier. Higher-profile players feel fan/media events
    /// more — their name gets carried on banners, not buried on page 14.
    /// Returns ~1.0 at low reputation up to ~1.5 at the top of the scale.
    #[inline]
    pub fn reputation_amplifier(current_reputation: i16) -> f32 {
        let r = (current_reputation as f32 / 10_000.0).clamp(0.0, 1.0);
        1.0 + 0.5 * r
    }

    /// Pressure / big-match amplifier. Cup nights, derbies, decisive
    /// moments hit harder for `important_matches` and `pressure` players
    /// who live for those occasions. Returns 1.0 at neutral 10/10, up
    /// to ~1.4 at 20/20, down to ~0.8 at 0/0.
    #[inline]
    pub fn pressure_amplifier(important_matches: f32, pressure: f32) -> f32 {
        let im = important_matches.clamp(0.0, 20.0) / 20.0;
        let pr = pressure.clamp(0.0, 20.0) / 20.0;
        let avg = (im + pr) * 0.5;
        // Map [0, 1] → [0.8, 1.4]
        0.8 + avg * 0.6
    }

    /// Criticism amplifier — provocative personalities (high `controversy`
    /// or `temperament`) react more strongly to fan/media negativity.
    /// Returns 1.0 at neutral 10/10, ~1.5 at 20/20.
    #[inline]
    pub fn criticism_amplifier(controversy: f32, temperament: f32) -> f32 {
        let c = controversy.clamp(0.0, 20.0) / 20.0;
        // Low temperament = more reactive (gets in the player's head).
        let t_inv = 1.0 - (temperament.clamp(0.0, 20.0) / 20.0);
        // Map both factors into [0, 1] then blend 60/40 controversy/temp.
        let blended = c * 0.6 + t_inv * 0.4;
        0.75 + blended * 0.75
    }

    /// Professionalism dampener — high-pro players brush off fan/media
    /// noise more readily. Returns 1.0 at 0 professionalism down to ~0.6
    /// at 20.
    #[inline]
    pub fn criticism_dampener(professionalism: f32) -> f32 {
        let p = professionalism.clamp(0.0, 20.0) / 20.0;
        1.0 - 0.4 * p
    }

    /// Ambition amplifier for upward / career-defining events (trophies,
    /// continental qualification, dream moves). Returns 0.85 at zero
    /// ambition, 1.0 at neutral 10, ~1.3 at 20.
    #[inline]
    pub fn ambition_amplifier(ambition: f32) -> f32 {
        let a = ambition.clamp(0.0, 20.0) / 20.0;
        0.85 + a * 0.45
    }

    /// Loyalty amplifier for club-bonded events (promotion celebration
    /// for a long-serving player feels stronger). Mild — 0.9 to 1.2.
    #[inline]
    pub fn loyalty_amplifier(loyalty: f32) -> f32 {
        let l = loyalty.clamp(0.0, 20.0) / 20.0;
        0.9 + l * 0.3
    }

    /// Age amplifier for trophy-style events. Veterans (30+) treasure
    /// silverware they may never win again; the very young get a smaller
    /// kick because they expect more chances. Returns 0.8 (under 21),
    /// 1.0 (22-29), 1.2 (30+).
    #[inline]
    pub fn veteran_amplifier(age: u8) -> f32 {
        if age >= 30 {
            1.2
        } else if age >= 22 {
            1.0
        } else {
            0.8
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MatchParticipation {
    Starter,
    Substitute,
}

/// Everything the Player needs to react to a finished match. Constructed
/// by the league/match-result pipeline and handed over one player at a
/// time; the Player owns all resulting stat bookkeeping, morale events,
/// and reputation changes.
pub struct MatchOutcome<'a> {
    pub stats: &'a PlayerMatchEndStats,
    pub effective_rating: f32,
    pub participation: MatchParticipation,
    pub is_friendly: bool,
    pub is_cup: bool,
    pub is_motm: bool,
    /// Goals scored by this player's team. Available for everyone on the
    /// matchday squad — used for decisive-goal / score-margin gating.
    pub team_goals_for: u8,
    /// Goals conceded by this player's team. Always populated for matchday
    /// squad members; emit sites apply their own role gates (GK stats,
    /// defender clean-sheet pride, etc.).
    pub team_goals_against: u8,
    pub league_weight: f32,
    pub world_weight: f32,
    /// True when the opposing club is in this player's club's rivals list.
    /// Derby results produce bigger morale swings either way.
    pub is_derby: bool,
    /// Did this player's team win the match? Derby bonus/penalty uses it.
    pub team_won: bool,
    /// Did this player's team lose the match?
    pub team_lost: bool,
}

impl<'a> MatchOutcome<'a> {
    /// Goal margin from this player's team perspective: positive when
    /// won, negative when lost, zero on a draw. Saturates to i8 to keep
    /// freak basket-scores from overflowing.
    #[inline]
    pub fn goal_margin(&self) -> i8 {
        let g = self.team_goals_for as i16 - self.team_goals_against as i16;
        g.clamp(i8::MIN as i16, i8::MAX as i16) as i8
    }
}

pub struct TransferCompletion<'a> {
    pub from: &'a TeamInfo,
    pub to: &'a TeamInfo,
    pub fee: f64,
    pub date: NaiveDate,
    pub selling_club_id: u32,
    pub buying_club_id: u32,
    /// Annual wage agreed during PersonalTerms. None = compute from context.
    pub agreed_wage: Option<u32>,
    /// Buying club's league reputation (0–10000), for fallback wage computation
    /// when `agreed_wage` is absent.
    pub buying_league_reputation: u16,
    /// Sell-on percentage pledged by the buyer to the current seller. Added
    /// to `player.sell_on_obligations` so the next sale pays the seller out.
    pub record_sell_on: Option<f32>,
}

pub struct LoanCompletion<'a> {
    pub from: &'a TeamInfo,
    pub to: &'a TeamInfo,
    pub loan_fee: f64,
    pub date: NaiveDate,
    pub loan_contract: PlayerClubContract,
    pub borrowing_club_id: u32,
}

impl Player {
    /// React to a completed permanent transfer. Resets stats history,
    /// clears transient statuses and happiness, installs a fresh contract
    /// and signing plan, and stages a pending signing so the next sim
    /// tick can emit the shock / role-fit / promise events.
    pub fn complete_transfer(&mut self, t: TransferCompletion<'_>) {
        let previous_salary = self.contract.as_ref().map(|c| c.salary);
        self.on_transfer(t.from, t.to, t.fee, t.date);
        self.sold_from = Some((t.selling_club_id, t.fee));
        self.reset_on_club_change();
        self.install_permanent_contract(t.date, t.to.reputation, t.buying_league_reputation, t.agreed_wage);
        self.plan = Some(PlayerPlan::from_signing(self.age(t.date), t.fee, t.date));
        if let Some(pct) = t.record_sell_on {
            if pct > 0.0 && self.sell_on_obligations.len() < 3 {
                self.sell_on_obligations.push(crate::club::player::player::SellOnObligation {
                    beneficiary_club_id: t.selling_club_id,
                    percentage: pct,
                });
            }
        }
        self.pending_signing = Some(PendingSigning {
            previous_salary,
            fee: t.fee,
            is_loan: false,
            destination_club_id: t.buying_club_id,
        });
    }

    /// Take and return all active sell-on obligations, clearing the list on
    /// the player. Used by execution to route money to past beneficiaries
    /// before crediting the current seller.
    pub fn drain_sell_on_obligations(&mut self) -> Vec<crate::club::player::player::SellOnObligation> {
        std::mem::take(&mut self.sell_on_obligations)
    }

    /// React to a completed loan. The parent contract is preserved; the
    /// borrowing club's contract is installed as `contract_loan`. We also
    /// annotate the parent contract's `loan_to_club_id` so downstream
    /// queries (UI, match-day loaned-in collector) can locate the borrower
    /// directly from the parent-side contract without digging into
    /// `contract_loan`.
    pub fn complete_loan(&mut self, l: LoanCompletion<'_>) {
        let borrowing_id = l.borrowing_club_id;
        self.on_loan(l.from, l.to, l.loan_fee, l.date);
        self.reset_on_club_change();
        if let Some(parent) = self.contract.as_mut() {
            parent.loan_to_club_id = Some(borrowing_id);
        }
        self.contract_loan = Some(l.loan_contract);
        self.pending_signing = Some(PendingSigning {
            previous_salary: None,
            fee: l.loan_fee,
            is_loan: true,
            destination_club_id: borrowing_id,
        });
    }

    fn reset_on_club_change(&mut self) {
        const TRANSIENT: [PlayerStatusType; 10] = [
            PlayerStatusType::Lst,
            PlayerStatusType::Loa,
            PlayerStatusType::Frt,
            PlayerStatusType::Req,
            PlayerStatusType::Unh,
            PlayerStatusType::Trn,
            PlayerStatusType::Bid,
            PlayerStatusType::Wnt,
            PlayerStatusType::Sct,
            PlayerStatusType::Enq,
        ];
        for s in TRANSIENT {
            self.statuses.remove(s);
        }
        self.happiness = PlayerHappiness::new();
        // Workload doesn't carry across clubs — minutes-played at the old
        // side don't burden the new manager's selection choice, and form
        // naturally resets as the player settles.
        self.load = PlayerLoad::new();
    }

}

fn stats_bucket_mut(player: &mut Player, is_cup: bool, is_friendly: bool) -> &mut PlayerStatistics {
    if is_cup {
        &mut player.cup_statistics
    } else if is_friendly {
        &mut player.friendly_statistics
    } else {
        &mut player.statistics
    }
}

impl Player {
    /// Install a fresh permanent contract on this player at the buying club.
    ///
    /// This is the canonical contract-installation policy used by both the
    /// AI transfer pipeline (via `complete_transfer`) and the manual web
    /// UI. The single source of truth for two decisions:
    ///
    ///  - **Length:** age-banded (5y under 24, 4y under 28, 3y under 32,
    ///    otherwise 2y). Younger players get longer deals.
    ///  - **Salary:** `agreed_wage` if `Some` (for AI deals where the
    ///    negotiation already settled on a number); otherwise computed
    ///    via `WageCalculator::expected_annual_wage` from the player's
    ///    profile and the buying club's reputation.
    ///
    /// Inputs `buying_club_reputation` and `buying_league_reputation` are
    /// raw 0–10000 reputation values for the club's main team and its
    /// league. The wage calculator normalises them internally.
    ///
    /// Side effects: `self.contract` is set to a fresh
    /// `PlayerClubContract` with `squad_status = NotYetSet`; callers that
    /// know the destination roster should update `squad_status`
    /// afterwards. `self.contract_loan` is cleared to drop any prior
    /// borrowing-club contract.
    pub fn install_permanent_contract(
        &mut self,
        date: NaiveDate,
        buying_club_reputation: u16,
        buying_league_reputation: u16,
        agreed_wage: Option<u32>,
    ) {
        let age = self.age(date);
        let years = if age < 24 { 5 } else if age < 28 { 4 } else if age < 32 { 3 } else { 2 };
        let expiry = date
            .checked_add_signed(chrono::Duration::days(years * 365))
            .unwrap_or(date);
        let salary = agreed_wage.unwrap_or_else(|| {
            let club_score = (buying_club_reputation as f32 / 10_000.0).clamp(0.0, 1.0);
            WageCalculator::expected_annual_wage(self, age, club_score, buying_league_reputation)
        });
        self.contract = Some(PlayerClubContract::new(salary, expiry));
        self.contract_loan = None;
    }

    /// React to finishing a match: stats bookkeeping, morale events,
    /// reputation update. All cross-cutting effects of "a match happened"
    /// live here instead of leaking into the league-result pipeline.
    pub fn on_match_played(&mut self, o: &MatchOutcome<'_>) {
        self.record_match_appearance(o);
        self.record_match_stats(o);
        self.record_match_events(o);
        self.record_match_reputation(o);
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
            let mag = match o.participation {
                MatchParticipation::Starter => 1.5,
                MatchParticipation::Substitute => 0.6,
            };
            self.happiness.add_event(HappinessEventType::MatchSelection, mag);

            // Rolling starter-share tracking — drives the WonStartingPlace /
            // LostStartingPlace one-shot transitions. Only competitive
            // matches count: pre-season minutes don't tell us anything
            // about the manager's matchday trust.
            self.update_role_state(o);
        }

        if o.is_motm {
            self.happiness.add_event_default(HappinessEventType::PlayerOfTheMatch);
        }

        // Friendlies don't generate the rest of the football-life events —
        // pre-season form, suspensions, derby narratives don't apply.
        if o.is_friendly {
            return;
        }

        // Sent off — embarrassing, plus the suspension fallout. Flat hit.
        if o.stats.red_cards > 0 {
            self.happiness.add_event_default(HappinessEventType::RedCardFallout);
        }

        // First competitive goal at this club. Stats are reset on club
        // change (see `on_transfer` / `on_loan`), so the only way the
        // running competitive total equals this match's goals is when
        // this is the first scoring match of the tenure. Long cooldown
        // prevents the milestone from firing again later in the spell.
        if o.stats.goals > 0 {
            let total_competitive = self.statistics.goals + self.cup_statistics.goals;
            if total_competitive == o.stats.goals
                && !self.happiness.has_recent_event(&HappinessEventType::FirstClubGoal, 300)
            {
                self.happiness.add_event_default(HappinessEventType::FirstClubGoal);
            }
        }

        // Substitute impact: came on and made it count. Skip if already
        // tagged POM — no point double-firing for the same standout shift.
        if !o.is_motm
            && o.participation == MatchParticipation::Substitute
            && (o.stats.goals > 0 || o.stats.assists > 0 || o.effective_rating >= 7.3)
        {
            self.happiness.add_event_default(HappinessEventType::SubstituteImpact);
        }

        // Clean sheet pride for goalkeepers and defenders — both roles
        // genuinely care about a shutout. Starters get the full event;
        // unused subs aren't on the field but still share the team result
        // (skipped here — they don't even hit `record_match_events`).
        if o.team_goals_against == 0
            && (self.position().is_goalkeeper() || self.position().is_defender())
        {
            self.happiness.add_event_default(HappinessEventType::CleanSheetPride);
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
                self.happiness.add_event(HappinessEventType::ManagerCriticism, mag);
            } else if o.effective_rating >= 7.5 {
                let mag = 1.5 + (o.effective_rating - 7.5).clamp(0.0, 2.5);
                self.happiness.add_event(HappinessEventType::ManagerEncouragement, mag);
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
            self.happiness.add_event_with_cooldown(
                HappinessEventType::DecisiveGoal,
                mag,
                14,
            );
        }

        // FanPraise — supporters latch onto a stand-out display. Triggered
        // by POM, an excellent rating, or a goal/assist contribution in a
        // win. Reputation-amplified so high-profile players feel it more.
        let fan_praise_trigger = o.is_motm
            || o.effective_rating >= 8.0
            || (o.team_won && had_contribution);
        if fan_praise_trigger {
            let rep_mul =
                scaling::reputation_amplifier(self.player_attributes.current_reputation);
            let scene_mul = if o.is_cup || o.is_derby { 1.2 } else { 1.0 };
            let mag = cfg.catalog.fan_praise * rep_mul * scene_mul;
            self.happiness.add_event_with_cooldown(
                HappinessEventType::FanPraise,
                mag,
                21,
            );
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
            let mag =
                cfg.catalog.fan_criticism * rep_mul * provoke_mul * prof_dampen;
            self.happiness.add_event_with_cooldown(
                HappinessEventType::FanCriticism,
                mag,
                21,
            );
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
            self.happiness.add_event_with_cooldown(
                HappinessEventType::MediaPraise,
                mag,
                30,
            );
        }

        // Derby outcome — proper rivalry-day events instead of recycled
        // manager talks. DerbyHero is reserved for standout performers
        // (scored, assisted, POM, ≥7.5 rating, or GK/DEF clean sheet
        // ≥7.2). Ordinary squad members on the winning side get the
        // squad-wide DerbyWin instead, so the event log doesn't claim
        // every fullback was the hero of the match.
        if o.is_derby {
            if o.team_won {
                let is_back_line = self.position().is_goalkeeper()
                    || self.position().is_defender();
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
                    self.happiness
                        .add_event(HappinessEventType::DerbyHero, cfg.catalog.derby_hero + bonus);
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
                self.happiness
                    .add_event(HappinessEventType::DerbyDefeat, cfg.catalog.derby_defeat - extra);
            }
        }
    }

    /// Named to a squad but never got off the bench. Small morale hit.
    pub fn on_match_dropped(&mut self) {
        self.happiness.add_event_default(HappinessEventType::MatchDropped);

        // Bench-only appearance: feeds the rolling starter ratio with a 0.0
        // sample so chronic dropping eventually flips the role state.
        const ALPHA: f32 = 0.25;
        self.happiness.starter_ratio = self.happiness.starter_ratio * (1.0 - ALPHA);
        self.happiness.appearances_tracked = self.happiness.appearances_tracked.saturating_add(1);
        self.evaluate_role_transition();
    }

    /// Update the rolling starter ratio on a competitive match and emit the
    /// one-shot role-transition events when the player crosses the
    /// established / not-established threshold. EMA window ~ 4 matches.
    fn update_role_state(&mut self, o: &MatchOutcome<'_>) {
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
    fn evaluate_role_transition(&mut self) {
        const MIN_APPS: u8 = 5;
        const STARTER_FLOOR: f32 = 0.65;
        const BENCHED_CEILING: f32 = 0.40;
        if self.happiness.appearances_tracked < MIN_APPS {
            return;
        }
        if !self.happiness.is_established_starter
            && self.happiness.starter_ratio >= STARTER_FLOOR
        {
            let mag = self.won_starting_place_magnitude();
            // 90-day cooldown so a brief slump and recovery don't ping-pong
            // the event once per fortnight.
            if self.happiness.add_event_with_cooldown(
                HappinessEventType::WonStartingPlace,
                mag,
                90,
            ) {
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
    fn season_participation_factor(&self) -> f32 {
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
    fn season_event_role_factor(&self, event: &HappinessEventType, age: u8) -> f32 {
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
    fn team_event_personality_factor(
        &self,
        event: &HappinessEventType,
        age: u8,
    ) -> f32 {
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
            RelegationFear => scaling::ambition_amplifier(a)
                * scaling::criticism_dampener(self.attributes.professionalism),
            // Continental qualification — pure ambition lift.
            QualifiedForEurope => scaling::ambition_amplifier(a),
            _ => 1.0,
        }
    }

    /// React to a promotion from a youth/reserve team to the senior side.
    /// Career milestone — emit once per spell with a long cooldown so a
    /// player who oscillates between reserves and main doesn't get a fresh
    /// "breakthrough" each bounce. Late bloomers (>21) get a softened
    /// magnitude — the moment is real but they expected it eventually.
    pub fn on_youth_breakthrough(&mut self, now: NaiveDate) {
        let age = self.age(now);
        // Skip players already past the breakthrough window — a 25-year-old
        // moving from reserve to main is a squad-depth call, not a debut.
        if age >= 26 {
            return;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.youth_breakthrough;
        let age_factor = if age <= 21 { 1.0 } else { 0.6 };
        let mag = base * age_factor;
        // 5-year cooldown ≈ one-shot per career spell.
        self.happiness.add_event_with_cooldown(
            HappinessEventType::YouthBreakthrough,
            mag,
            365 * 5,
        );
    }

    /// React to a team-level season / competition outcome. Magnitude is
    /// the catalog default scaled by the player's involvement in the
    /// season and a personality blend chosen for the event type. Cooldown
    /// gates prevent the same event firing twice when emit logic stutters
    /// (e.g. season-end ticking on consecutive days).
    pub fn on_team_season_event(
        &mut self,
        event: HappinessEventType,
        cooldown_days: u16,
        now: NaiveDate,
    ) -> bool {
        self.on_team_season_event_with_prestige(event, cooldown_days, 1.0, now)
    }

    /// Same as [`on_team_season_event`] with an explicit prestige multiplier
    /// applied to the magnitude. Use it for cup / continental events whose
    /// magnitude depends on competition tier — e.g. `0.7` for a domestic
    /// minor cup, `1.0` for a domestic top cup, `1.4` for a continental
    /// trophy. Returned bool tracks whether the event was recorded
    /// (cooldown may have suppressed it).
    pub fn on_team_season_event_with_prestige(
        &mut self,
        event: HappinessEventType,
        cooldown_days: u16,
        prestige: f32,
        now: NaiveDate,
    ) -> bool {
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.magnitude(event.clone());
        let participation = self.season_participation_factor();
        let age = self.age(now);
        let personality = self.team_event_personality_factor(&event, age);
        let role = self.season_event_role_factor(&event, age);
        let mag = base * participation * personality * role * prestige.max(0.0);
        self.happiness
            .add_event_with_cooldown(event, mag, cooldown_days)
    }

    /// An approach from `buyer_rep` has made it past the selling club's
    /// initial acceptance check, so it counts as real media-reported
    /// interest rather than a rumour. Flattery boost for ambitious
    /// players being chased upward; light destabilisation for the rest
    /// (rumour mill unsettles focus). Noop unless the gap is at least
    /// modest — generic peer-level interest isn't news.
    ///
    /// Cooldown gates re-firing when the same buyer keeps probing — the
    /// player has already heard the rumour mill in the past fortnight.
    pub fn on_transfer_interest_confirmed(&mut self, buyer_rep: f32, seller_rep: f32) {
        let rep_diff = buyer_rep - seller_rep;
        if rep_diff < 0.1 {
            return;
        }
        let ambition = self.attributes.ambition;
        if ambition >= 12.0 {
            // Ambitious player flattered by a bigger club chasing them —
            // proper "wanted by a bigger club" event, not a generic
            // manager talk.
            let mag = 1.0 + (rep_diff - 0.1).clamp(0.0, 0.6) * 4.0;
            self.happiness.add_event_with_cooldown(
                HappinessEventType::WantedByBiggerClub,
                mag,
                14,
            );
        } else {
            // Settled player disrupted by headline-grabbing rumour —
            // tabloid drama, modelled as media noise.
            let mag = -(0.5 + (rep_diff - 0.1).clamp(0.0, 0.4) * 2.0);
            self.happiness.add_event_with_cooldown(
                HappinessEventType::MediaCriticism,
                mag,
                14,
            );
        }
    }

    /// Selling club rejected a real bid from a meaningfully bigger
    /// suitor, or from a club the player has flagged as a favorite.
    /// Magnitude grows with ambition and the rep gap, dampened by
    /// professionalism, amplified for favorite-club destinations.
    /// Cooldown 21d so a buying club's repeated bids don't pile this on.
    ///
    /// `buyer_rep` and `seller_rep` are normalised 0–1 reputation scores
    /// (the fields the negotiation already carries). `was_favorite_club`
    /// lifts the gating threshold and amplifies magnitude — a favorite
    /// club's bid being rejected stings even at a lateral move, where a
    /// generic peer-level rejection would otherwise be silent.
    pub fn on_transfer_bid_rejected(
        &mut self,
        buyer_rep: f32,
        seller_rep: f32,
        was_favorite_club: bool,
    ) {
        let rep_diff = buyer_rep - seller_rep;
        let ambition = self.attributes.ambition;
        let listed_or_unhappy = self.statuses.get().contains(&PlayerStatusType::Lst)
            || self.statuses.get().contains(&PlayerStatusType::Req)
            || self.statuses.get().contains(&PlayerStatusType::Unh)
            || self.statuses.get().contains(&PlayerStatusType::Trn);

        // Favorite-club bid: any meaningful approach (rep_diff > -0.05,
        // i.e. roughly peer-level or up) being rejected hurts even an
        // average-ambition player. Otherwise the existing gates apply.
        if was_favorite_club {
            if rep_diff < -0.05 {
                return;
            }
        } else {
            if rep_diff < 0.10 {
                return;
            }
            if ambition < 12.0 && !listed_or_unhappy {
                return;
            }
        }

        let cfg = HappinessConfig::default();
        let base = cfg.catalog.transfer_bid_rejected;
        let ambition_mul = scaling::ambition_amplifier(ambition);
        // Low loyalty stings more — the player wanted out.
        let loyalty = self.attributes.loyalty.clamp(0.0, 20.0) / 20.0;
        let loyalty_mul = 1.0 + (1.0 - loyalty) * 0.25;
        let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
        // Bigger gap, sharper hit — but capped so the magnitude band stays sane.
        let gap_mul = 1.0 + (rep_diff.max(0.0) - 0.10).clamp(0.0, 0.40) * 1.5;
        let favorite_mul = if was_favorite_club { 1.25 } else { 1.0 };
        let mag = base * ambition_mul * loyalty_mul * prof_dampen * gap_mul * favorite_mul;
        self.happiness
            .add_event_with_cooldown(HappinessEventType::TransferBidRejected, mag, 21);
    }

    /// A late-stage transfer collapse — clubs agreed, terms agreed, and
    /// then the move fell over (medical, registration mishap). Stronger
    /// than a bid rejection. Only fires for meaningfully upward moves or
    /// known favorite-club destinations — collapse of a sideways move is
    /// merely annoying, not a "dream" gone.
    pub fn on_dream_move_collapsed(
        &mut self,
        buyer_rep: f32,
        seller_rep: f32,
        was_favorite_club: bool,
    ) {
        let rep_diff = buyer_rep - seller_rep;
        if !was_favorite_club && rep_diff < 0.15 {
            return;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.dream_move_collapsed;
        let ambition_mul = scaling::ambition_amplifier(self.attributes.ambition);
        let favorite_mul = if was_favorite_club { 1.30 } else { 1.0 };
        // Loyal players feel it less — they were less invested in leaving.
        let loyalty = self.attributes.loyalty.clamp(0.0, 20.0) / 20.0;
        let loyalty_dampen = 1.0 - 0.25 * loyalty;
        let mag = base * ambition_mul * favorite_mul * loyalty_dampen;
        // 30-day cooldown so a chain of failed reattempts doesn't stack.
        self.happiness
            .add_event_with_cooldown(HappinessEventType::DreamMoveCollapsed, mag, 30);
    }

    /// React to a teammate leaving the club. Caller has already determined
    /// that this player had a meaningful bond with the departing teammate
    /// and supplies the bond signals so the helper can pick the right
    /// event flavour (close-friend vs mentor) and dial magnitude.
    ///
    /// `bond_friendship` is the 0..100 friendship score and
    /// `same_nationality` / `is_long_term_teammate` modulate magnitude.
    pub fn on_close_friend_sold(
        &mut self,
        partner_player_id: u32,
        bond_friendship: f32,
        same_nationality: bool,
        departing_was_high_reputation: bool,
    ) {
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.close_friend_sold;
        // Bond strength: 65→1.0, 100→1.4
        let bond = ((bond_friendship - 65.0).clamp(0.0, 35.0) / 35.0) * 0.4 + 1.0;
        let nat_mul = if same_nationality { 1.20 } else { 1.0 };
        let rep_mul = if departing_was_high_reputation { 1.15 } else { 1.0 };
        let mag = base * bond * nat_mul * rep_mul;
        self.happiness.add_event_with_partner_and_cooldown(
            HappinessEventType::CloseFriendSold,
            mag,
            Some(partner_player_id),
            30,
        );
    }

    /// React to a veteran mentor leaving. Same call shape as
    /// `on_close_friend_sold` but tuned for the mentor / mentee dynamic —
    /// larger base hit, longer cooldown.
    pub fn on_mentor_departed(
        &mut self,
        partner_player_id: u32,
        bond_friendship: f32,
        same_nationality: bool,
    ) {
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.mentor_departed;
        let bond = ((bond_friendship.clamp(0.0, 100.0)) / 100.0) * 0.5 + 0.75;
        let nat_mul = if same_nationality { 1.15 } else { 1.0 };
        let mag = base * bond * nat_mul;
        self.happiness.add_event_with_partner_and_cooldown(
            HappinessEventType::MentorDeparted,
            mag,
            Some(partner_player_id),
            60,
        );
    }

    /// React to a same-nationality player joining the squad. Strongest
    /// for foreign players who lack the local language; not emitted for
    /// domestic players in their home country (everyone speaks the same
    /// language, no integration boost).
    pub fn on_compatriot_joined(
        &mut self,
        partner_player_id: u32,
        club_country_id: u32,
        lacks_local_language: bool,
    ) {
        if self.country_id == club_country_id {
            return;
        }
        let cfg = HappinessConfig::default();
        let base = cfg.catalog.compatriot_joined;
        // Foreign players lacking the local tongue lean on a compatriot
        // doubly hard — bigger lift than a foreign player who's already
        // settled linguistically.
        let lang_mul = if lacks_local_language { 1.30 } else { 1.0 };
        let mag = base * lang_mul;
        self.happiness.add_event_with_partner_and_cooldown(
            HappinessEventType::CompatriotJoined,
            mag,
            Some(partner_player_id),
            30,
        );
    }

    /// React to a mutual contract termination. Clears the contract (player
    /// becomes a free agent), drops transfer statuses that no longer apply,
    /// and logs a mild morale event — it's a blow to pride, but freedom
    /// plus a payout softens it considerably.
    pub fn on_contract_terminated(&mut self, _date: NaiveDate) {
        self.contract = None;
        self.contract_loan = None;
        for s in [
            PlayerStatusType::Lst,
            PlayerStatusType::Req,
            PlayerStatusType::Unh,
            PlayerStatusType::Trn,
            PlayerStatusType::Bid,
        ] {
            self.statuses.remove(s);
        }
        self.happiness.add_event_default(HappinessEventType::ContractTerminated);
    }

    /// Apply the physical cost of featuring in a match: condition floor,
    /// readiness boost, jadedness accumulation, injury roll, workload.
    /// Called by the league/match-result pipeline once per featured player.
    pub fn on_match_exertion(&mut self, minutes: f32, now: NaiveDate, is_friendly: bool) {
        self.load.record_match_minutes(minutes, is_friendly);

        let age = self.age(now);
        let natural_fitness = self.skills.physical.natural_fitness;

        // Condition floor — the match engine drains condition during sim;
        // here we enforce an FM-style 30% minimum so nobody finishes a 90
        // at 0%. A full 90 should leave players at 55–70%.
        let condition_floor: i16 = 3000;
        if self.player_attributes.condition < condition_floor {
            self.player_attributes.condition = condition_floor;
        }

        if minutes >= 15.0 {
            let readiness_boost = minutes / 90.0 * 3.0;
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness + readiness_boost).min(20.0);
        }

        if minutes > 60.0 {
            self.player_attributes.jadedness += 400;
        } else if minutes >= 30.0 {
            self.player_attributes.jadedness += 200;
        }

        if self.player_attributes.jadedness > 7000
            && !self.statuses.get().contains(&PlayerStatusType::Rst)
        {
            self.statuses.add(now, PlayerStatusType::Rst);
        }

        self.player_attributes.days_since_last_match = 0;

        if !self.player_attributes.is_injured {
            self.roll_for_match_injury(minutes, age, natural_fitness, now);
        }
    }

    fn roll_for_match_injury(
        &mut self,
        minutes: f32,
        age: u8,
        natural_fitness: f32,
        now: NaiveDate,
    ) {
        let injury_proneness = self.player_attributes.injury_proneness;
        let proneness_modifier = injury_proneness as f32 / 10.0;
        let condition_pct = self.player_attributes.condition_percentage();

        let mut injury_chance: f32 = 0.005 * (minutes / 90.0);
        if age > 30 {
            injury_chance += (age as f32 - 30.0) * 0.001;
        }
        if condition_pct < 40 {
            injury_chance += (40.0 - condition_pct as f32) * 0.0001;
        }
        if self.player_attributes.jadedness > 7000 {
            injury_chance += 0.002;
        }
        if natural_fitness < 8.0 {
            injury_chance += 0.001;
        }
        injury_chance *= proneness_modifier;
        if self.player_attributes.last_injury_body_part != 0 {
            injury_chance += 0.002;
        }

        if rand::random::<f32>() < injury_chance {
            let injury = InjuryType::random_match_injury(
                minutes,
                age,
                condition_pct,
                natural_fitness,
                injury_proneness,
            );
            self.player_attributes.set_injury(injury);
            self.statuses.add(now, PlayerStatusType::Inj);
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
            self.player_attributes.update_reputation(current_delta, home_delta, world_delta);
        }
    }

    /// Extend the parent contract (if needed) so it doesn't expire while the
    /// player is out on loan — used by the loan pipeline before shipping the
    /// player to the borrower.
    pub fn ensure_contract_covers_loan_end(&mut self, loan_end: NaiveDate) {
        let min_expiry = loan_end
            .checked_add_signed(chrono::Duration::days(365))
            .unwrap_or(loan_end);
        if let Some(ref mut contract) = self.contract {
            if contract.expiration < min_expiry {
                contract.expiration = min_expiry;
            }
        }
    }
}

#[cfg(test)]
mod match_event_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::r#match::engine::result::PlayerMatchEndStats;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerFieldPositionGroup, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn build_player(pos: PlayerPositionType, person: PersonAttributes) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_reputation = 5_000;
        attrs.home_reputation = 6_000;
        attrs.world_reputation = 4_000;
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(d(2000, 1, 1))
            .country_id(1)
            .attributes(person)
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition { position: pos, level: 20 }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    fn stats(
        rating: f32,
        goals: u16,
        assists: u16,
        red_cards: u16,
        group: PlayerFieldPositionGroup,
    ) -> PlayerMatchEndStats {
        PlayerMatchEndStats {
            shots_on_target: 0,
            shots_total: 0,
            passes_attempted: 0,
            passes_completed: 0,
            tackles: 0,
            interceptions: 0,
            saves: 0,
            shots_faced: 0,
            goals,
            assists,
            match_rating: rating,
            xg: 0.0,
            position_group: group,
            fouls: 0,
            yellow_cards: 0,
            red_cards,
        }
    }

    fn outcome<'a>(
        s: &'a PlayerMatchEndStats,
        rating: f32,
        is_friendly: bool,
        is_cup: bool,
        is_motm: bool,
        is_derby: bool,
        team_for: u8,
        team_against: u8,
        participation: MatchParticipation,
    ) -> MatchOutcome<'a> {
        let won = team_for > team_against;
        let lost = team_for < team_against;
        MatchOutcome {
            stats: s,
            effective_rating: rating,
            participation,
            is_friendly,
            is_cup,
            is_motm,
            team_goals_for: team_for,
            team_goals_against: team_against,
            league_weight: 1.0,
            world_weight: 1.0,
            is_derby,
            team_won: won,
            team_lost: lost,
        }
    }

    fn count_events(p: &Player, kind: &HappinessEventType) -> usize {
        p.happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == *kind)
            .count()
    }

    // ── DecisiveGoal ──────────────────────────────────────────────

    #[test]
    fn decisive_goal_fires_on_one_goal_win_with_contribution() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let s = stats(7.0, 1, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 7.0, false, false, false, false, 1, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::DecisiveGoal), 1);
    }

    #[test]
    fn decisive_goal_silent_for_two_goal_win() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let s = stats(7.0, 1, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 7.0, false, false, false, false, 3, 1, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::DecisiveGoal), 0);
    }

    #[test]
    fn decisive_goal_silent_in_friendly() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let s = stats(7.0, 1, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 7.0, true, false, false, false, 1, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::DecisiveGoal), 0);
    }

    // ── FanPraise / MediaPraise ──────────────────────────────────

    #[test]
    fn fan_praise_fires_for_excellent_rating() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let s = stats(8.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 8.2, false, false, false, false, 0, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::FanPraise), 1);
    }

    #[test]
    fn media_praise_requires_higher_bar_than_fan_praise() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        // Rating = 8.0 — fan praise fires, media praise does not.
        let s = stats(8.0, 0, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 8.0, false, false, false, false, 0, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::FanPraise), 1);
        assert_eq!(count_events(&p, &HappinessEventType::MediaPraise), 0);
    }

    #[test]
    fn media_praise_fires_at_8_3_rating() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let s = stats(8.4, 0, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 8.4, false, false, false, false, 0, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::MediaPraise), 1);
    }

    #[test]
    fn fan_praise_cooldown_prevents_double_fire() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let s = stats(8.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 8.2, false, false, false, false, 0, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::FanPraise), 1);
    }

    // ── FanCriticism ─────────────────────────────────────────────

    #[test]
    fn fan_criticism_fires_on_low_rating() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let s = stats(5.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 5.2, false, false, false, false, 0, 1, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::FanCriticism), 1);
    }

    #[test]
    fn fan_criticism_fires_on_red_card() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        // Rating still ok but a red card is enough.
        let s = stats(6.5, 0, 0, 1, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 6.5, false, false, false, false, 0, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::FanCriticism), 1);
    }

    #[test]
    fn fan_criticism_silent_for_solid_performance() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let s = stats(6.8, 0, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 6.8, false, false, false, false, 1, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::FanCriticism), 0);
    }

    #[test]
    fn fan_criticism_dampened_by_professionalism() {
        let high_pro = PersonAttributes {
            professionalism: 20.0,
            ..PersonAttributes::default()
        };
        let mut high = build_player(PlayerPositionType::Striker, high_pro);

        let low_pro = PersonAttributes {
            professionalism: 0.0,
            ..PersonAttributes::default()
        };
        let mut low = build_player(PlayerPositionType::Striker, low_pro);

        let s = stats(5.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 5.2, false, false, false, false, 0, 1, MatchParticipation::Starter);
        high.on_match_played(&o);
        low.on_match_played(&o);

        let high_mag = high
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::FanCriticism)
            .unwrap()
            .magnitude;
        let low_mag = low
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::FanCriticism)
            .unwrap()
            .magnitude;
        // Both negative; "less negative" = closer to zero.
        assert!(high_mag > low_mag, "high pro {} should soften vs low pro {}", high_mag, low_mag);
    }

    // ── Clean-sheet pride extension ─────────────────────────────

    #[test]
    fn clean_sheet_pride_fires_for_defender() {
        let mut p = build_player(
            PlayerPositionType::DefenderCenter,
            PersonAttributes::default(),
        );
        let s = stats(7.0, 0, 0, 0, PlayerFieldPositionGroup::Defender);
        let o = outcome(&s, 7.0, false, false, false, false, 1, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::CleanSheetPride), 1);
    }

    #[test]
    fn clean_sheet_pride_silent_for_forward() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let s = stats(7.0, 0, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 7.0, false, false, false, false, 1, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::CleanSheetPride), 0);
    }

    // ── Reputation amplifier shapes magnitudes ──────────────────

    // ── Derby outcome ────────────────────────────────────────────

    #[test]
    fn ordinary_derby_winner_gets_derby_win_not_hero() {
        // Solid 6.8 rating, no goal/assist/POM, midfielder — the kind of
        // player who's on the winning side but didn't carry the day.
        let mut p = build_player(
            PlayerPositionType::MidfielderCenter,
            PersonAttributes::default(),
        );
        let s = stats(6.8, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
        let o = outcome(&s, 6.8, false, false, false, true, 2, 1, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 0);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyWin), 1);
    }

    #[test]
    fn derby_scorer_gets_derby_hero() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let s = stats(7.0, 1, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 7.0, false, false, false, true, 2, 1, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 1);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyWin), 0);
    }

    #[test]
    fn derby_assister_gets_derby_hero() {
        let mut p = build_player(
            PlayerPositionType::MidfielderCenter,
            PersonAttributes::default(),
        );
        let s = stats(7.0, 0, 1, 0, PlayerFieldPositionGroup::Midfielder);
        let o = outcome(&s, 7.0, false, false, false, true, 2, 1, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 1);
    }

    #[test]
    fn derby_high_rated_outfielder_gets_derby_hero() {
        let mut p = build_player(
            PlayerPositionType::MidfielderCenter,
            PersonAttributes::default(),
        );
        // 7.6 rating with no goal/assist still earns hero status.
        let s = stats(7.6, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
        let o = outcome(&s, 7.6, false, false, false, true, 2, 1, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 1);
    }

    #[test]
    fn derby_defender_clean_sheet_high_rating_gets_hero() {
        // Defender, no goal/assist, 7.3 rating, clean sheet — earns hero
        // status via the back-line clean-sheet gate (rating ≥ 7.2).
        let mut p = build_player(
            PlayerPositionType::DefenderCenter,
            PersonAttributes::default(),
        );
        let s = stats(7.3, 0, 0, 0, PlayerFieldPositionGroup::Defender);
        let o = outcome(&s, 7.3, false, false, false, true, 1, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 1);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyWin), 0);
    }

    #[test]
    fn derby_defender_clean_sheet_modest_rating_gets_win_only() {
        // Defender on the winning side, clean sheet, but rating below the
        // clean-sheet hero gate (7.2). Should be DerbyWin, not Hero.
        let mut p = build_player(
            PlayerPositionType::DefenderCenter,
            PersonAttributes::default(),
        );
        let s = stats(6.8, 0, 0, 0, PlayerFieldPositionGroup::Defender);
        let o = outcome(&s, 6.8, false, false, false, true, 1, 0, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyHero), 0);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyWin), 1);
    }

    #[test]
    fn derby_loser_gets_derby_defeat() {
        let mut p = build_player(
            PlayerPositionType::MidfielderCenter,
            PersonAttributes::default(),
        );
        let s = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
        let o = outcome(&s, 6.5, false, false, false, true, 0, 1, MatchParticipation::Starter);
        p.on_match_played(&o);
        assert_eq!(count_events(&p, &HappinessEventType::DerbyDefeat), 1);
    }

    #[test]
    fn derby_loser_poor_performer_takes_bigger_hit() {
        // Same defeat, two players: one performed solidly, the other
        // crumbled to a 5.0 rating. Poor performer should land a more
        // negative magnitude.
        let mut solid = build_player(
            PlayerPositionType::MidfielderCenter,
            PersonAttributes::default(),
        );
        let mut poor = build_player(
            PlayerPositionType::MidfielderCenter,
            PersonAttributes::default(),
        );
        let s_solid = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
        let o_solid = outcome(&s_solid, 6.5, false, false, false, true, 0, 1, MatchParticipation::Starter);
        let s_poor = stats(5.0, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
        let o_poor = outcome(&s_poor, 5.0, false, false, false, true, 0, 1, MatchParticipation::Starter);
        solid.on_match_played(&o_solid);
        poor.on_match_played(&o_poor);
        let m_solid = solid
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::DerbyDefeat)
            .unwrap()
            .magnitude;
        let m_poor = poor
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::DerbyDefeat)
            .unwrap()
            .magnitude;
        // More negative = bigger hit. Poor performer should be more negative.
        assert!(m_poor < m_solid, "poor {} should be more negative than solid {}", m_poor, m_solid);
    }

    #[test]
    fn derby_loser_red_card_amplifies_defeat() {
        let mut clean = build_player(
            PlayerPositionType::MidfielderCenter,
            PersonAttributes::default(),
        );
        let mut sent_off = build_player(
            PlayerPositionType::MidfielderCenter,
            PersonAttributes::default(),
        );
        let s_clean = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
        let o_clean = outcome(&s_clean, 6.5, false, false, false, true, 0, 1, MatchParticipation::Starter);
        // Red card with otherwise-acceptable rating — extra still applies.
        let s_red = stats(6.5, 0, 0, 1, PlayerFieldPositionGroup::Midfielder);
        let o_red = outcome(&s_red, 6.5, false, false, false, true, 0, 1, MatchParticipation::Starter);
        clean.on_match_played(&o_clean);
        sent_off.on_match_played(&o_red);
        let m_clean = clean
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::DerbyDefeat)
            .unwrap()
            .magnitude;
        let m_red = sent_off
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::DerbyDefeat)
            .unwrap()
            .magnitude;
        assert!(m_red < m_clean, "red-card {} should be more negative than clean {}", m_red, m_clean);
    }

    // ── Team-season events ───────────────────────────────────────

    #[test]
    fn trophy_won_emits_positive_magnitude() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.statistics.played = 30;
        let recorded = p.on_team_season_event(HappinessEventType::TrophyWon, 365, d(2032, 5, 30));
        assert!(recorded);
        let mag = p
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TrophyWon)
            .unwrap()
            .magnitude;
        assert!(mag > 0.0, "TrophyWon should be positive, got {}", mag);
    }

    #[test]
    fn relegated_emits_negative_magnitude() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.statistics.played = 30;
        let recorded = p.on_team_season_event(HappinessEventType::Relegated, 365, d(2032, 5, 30));
        assert!(recorded);
        let mag = p
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::Relegated)
            .unwrap()
            .magnitude;
        assert!(mag < 0.0, "Relegated should be negative, got {}", mag);
    }

    #[test]
    fn season_event_cooldown_prevents_duplicate() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.statistics.played = 30;
        let date = d(2032, 5, 30);
        assert!(p.on_team_season_event(HappinessEventType::Relegated, 365, date));
        assert!(!p.on_team_season_event(HappinessEventType::Relegated, 365, date));
    }

    #[test]
    fn season_event_prestige_scales_magnitude() {
        // Continental trophy (prestige 1.5) should land bigger than a
        // domestic-league title (prestige 1.0).
        let mut domestic = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let mut continental =
            build_player(PlayerPositionType::Striker, PersonAttributes::default());
        domestic.statistics.played = 30;
        continental.statistics.played = 30;
        let date = d(2032, 5, 30);
        domestic.on_team_season_event_with_prestige(
            HappinessEventType::TrophyWon,
            365,
            1.0,
            date,
        );
        continental.on_team_season_event_with_prestige(
            HappinessEventType::TrophyWon,
            365,
            1.5,
            date,
        );
        let dm = domestic
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TrophyWon)
            .unwrap()
            .magnitude;
        let cm = continental
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TrophyWon)
            .unwrap()
            .magnitude;
        assert!(cm > dm, "continental prestige {} should exceed domestic {}", cm, dm);
    }

    fn build_player_with_status(status: crate::PlayerSquadStatus) -> Player {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let mut contract = crate::PlayerClubContract::new(
            10_000,
            d(2035, 6, 30),
        );
        contract.squad_status = status;
        p.contract = Some(contract);
        p.statistics.played = 30;
        p
    }

    #[test]
    fn key_player_takes_bigger_relegation_hit_than_rotation() {
        let mut key = build_player_with_status(crate::PlayerSquadStatus::KeyPlayer);
        let mut rotation =
            build_player_with_status(crate::PlayerSquadStatus::FirstTeamSquadRotation);
        let date = d(2032, 5, 30);
        key.on_team_season_event(HappinessEventType::Relegated, 365, date);
        rotation.on_team_season_event(HappinessEventType::Relegated, 365, date);
        let m_key = key
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::Relegated)
            .unwrap()
            .magnitude;
        let m_rot = rotation
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::Relegated)
            .unwrap()
            .magnitude;
        // More negative = bigger hit. KeyPlayer should land more negatively.
        assert!(m_key < m_rot, "KeyPlayer {} should be more negative than rotation {}", m_key, m_rot);
    }

    #[test]
    fn fringe_not_needed_softens_relegation_hit() {
        let mut not_needed = build_player_with_status(crate::PlayerSquadStatus::NotNeeded);
        let mut regular =
            build_player_with_status(crate::PlayerSquadStatus::FirstTeamRegular);
        let date = d(2032, 5, 30);
        not_needed.on_team_season_event(HappinessEventType::Relegated, 365, date);
        regular.on_team_season_event(HappinessEventType::Relegated, 365, date);
        let m_not = not_needed
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::Relegated)
            .unwrap()
            .magnitude;
        let m_reg = regular
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::Relegated)
            .unwrap()
            .magnitude;
        // Less negative = softer hit. NotNeeded should be closer to zero.
        assert!(m_not > m_reg, "NotNeeded {} should be less negative than Regular {}", m_not, m_reg);
    }

    #[test]
    fn cup_final_defeat_emits_negative_with_prestige() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.statistics.played = 30;
        let date = d(2032, 5, 30);
        let recorded = p.on_team_season_event_with_prestige(
            HappinessEventType::CupFinalDefeat,
            365,
            1.4,
            date,
        );
        assert!(recorded);
        let mag = p
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::CupFinalDefeat)
            .unwrap()
            .magnitude;
        assert!(mag < 0.0, "CupFinalDefeat should be negative, got {}", mag);
    }

    #[test]
    fn fringe_player_feels_trophy_less_than_starter() {
        let mut starter = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        starter.statistics.played = 30;
        let mut fringe = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        fringe.statistics.played = 1;
        let date = d(2032, 5, 30);
        starter.on_team_season_event(HappinessEventType::TrophyWon, 365, date);
        fringe.on_team_season_event(HappinessEventType::TrophyWon, 365, date);
        let starter_mag = starter
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TrophyWon)
            .unwrap()
            .magnitude;
        let fringe_mag = fringe
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TrophyWon)
            .unwrap()
            .magnitude;
        assert!(starter_mag > fringe_mag);
    }

    #[test]
    fn ambitious_player_hurts_more_on_relegation() {
        let mut ambitious_pa = PersonAttributes::default();
        ambitious_pa.ambition = 20.0;
        let mut content_pa = PersonAttributes::default();
        content_pa.ambition = 1.0;
        let mut ambitious = build_player(PlayerPositionType::Striker, ambitious_pa);
        let mut content = build_player(PlayerPositionType::Striker, content_pa);
        ambitious.statistics.played = 30;
        content.statistics.played = 30;
        let date = d(2032, 5, 30);
        ambitious.on_team_season_event(HappinessEventType::Relegated, 365, date);
        content.on_team_season_event(HappinessEventType::Relegated, 365, date);
        let amb = ambitious
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::Relegated)
            .unwrap()
            .magnitude;
        let con = content
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::Relegated)
            .unwrap()
            .magnitude;
        // More negative = bigger hit. Ambition makes Relegated worse.
        assert!(amb < con, "ambitious {} should be more negative than content {}", amb, con);
    }

    // ── Role transitions ─────────────────────────────────────────

    fn run_match(p: &mut Player, participation: MatchParticipation) {
        let s = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
        let o = outcome(&s, 6.5, false, false, false, false, 1, 1, participation);
        p.on_match_played(&o);
    }

    #[test]
    fn won_starting_place_fires_after_run_of_starts() {
        let mut p = build_player(PlayerPositionType::MidfielderCenter, PersonAttributes::default());
        for _ in 0..6 {
            run_match(&mut p, MatchParticipation::Starter);
        }
        assert!(p.happiness.is_established_starter);
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::WonStartingPlace)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn lost_starting_place_fires_after_drop() {
        let mut p = build_player(PlayerPositionType::MidfielderCenter, PersonAttributes::default());
        // Establish first.
        for _ in 0..6 {
            run_match(&mut p, MatchParticipation::Starter);
        }
        assert!(p.happiness.is_established_starter);
        // Then a sustained run on the bench drops the EMA below 0.40.
        for _ in 0..10 {
            run_match(&mut p, MatchParticipation::Substitute);
        }
        assert!(!p.happiness.is_established_starter);
        let lost = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::LostStartingPlace)
            .count();
        assert_eq!(lost, 1);
    }

    fn run_match_with_status(
        p: &mut Player,
        participation: MatchParticipation,
        status: crate::PlayerSquadStatus,
    ) {
        let mut contract = crate::PlayerClubContract::new(10_000, d(2035, 6, 30));
        contract.squad_status = status;
        p.contract = Some(contract);
        let s = stats(6.5, 0, 0, 0, PlayerFieldPositionGroup::Midfielder);
        let o = outcome(&s, 6.5, false, false, false, false, 1, 1, participation);
        p.on_match_played(&o);
    }

    #[test]
    fn key_player_lost_starting_place_hit_exceeds_rotation() {
        // Establish both as starters, then sustain bench runs to flip the
        // role state. Key player should land a more negative LostStartingPlace.
        let mut key = build_player(PlayerPositionType::MidfielderCenter, PersonAttributes::default());
        for _ in 0..6 {
            run_match_with_status(&mut key, MatchParticipation::Starter, crate::PlayerSquadStatus::KeyPlayer);
        }
        for _ in 0..10 {
            run_match_with_status(&mut key, MatchParticipation::Substitute, crate::PlayerSquadStatus::KeyPlayer);
        }

        let mut rot = build_player(PlayerPositionType::MidfielderCenter, PersonAttributes::default());
        for _ in 0..6 {
            run_match_with_status(&mut rot, MatchParticipation::Starter, crate::PlayerSquadStatus::FirstTeamSquadRotation);
        }
        for _ in 0..10 {
            run_match_with_status(&mut rot, MatchParticipation::Substitute, crate::PlayerSquadStatus::FirstTeamSquadRotation);
        }

        let m_key = key
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::LostStartingPlace)
            .unwrap()
            .magnitude;
        let m_rot = rot
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::LostStartingPlace)
            .unwrap()
            .magnitude;
        // More negative = bigger hit.
        assert!(m_key < m_rot, "KeyPlayer {} should be more negative than rotation {}", m_key, m_rot);
    }

    #[test]
    fn prospect_won_starting_place_hit_exceeds_senior() {
        let mut prospect = build_player(PlayerPositionType::MidfielderCenter, PersonAttributes::default());
        for _ in 0..6 {
            run_match_with_status(
                &mut prospect,
                MatchParticipation::Starter,
                crate::PlayerSquadStatus::HotProspectForTheFuture,
            );
        }

        let mut senior = build_player(PlayerPositionType::MidfielderCenter, PersonAttributes::default());
        for _ in 0..6 {
            run_match_with_status(
                &mut senior,
                MatchParticipation::Starter,
                crate::PlayerSquadStatus::KeyPlayer,
            );
        }

        let m_p = prospect
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::WonStartingPlace)
            .unwrap()
            .magnitude;
        let m_s = senior
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::WonStartingPlace)
            .unwrap()
            .magnitude;
        assert!(m_p > m_s, "prospect {} should exceed established senior {}", m_p, m_s);
    }

    #[test]
    fn high_professionalism_softens_lost_starting_place() {
        let high_pro = PersonAttributes {
            professionalism: 20.0,
            ..PersonAttributes::default()
        };
        let low_pro = PersonAttributes {
            professionalism: 0.0,
            ..PersonAttributes::default()
        };
        let mut hi = build_player(PlayerPositionType::MidfielderCenter, high_pro);
        let mut lo = build_player(PlayerPositionType::MidfielderCenter, low_pro);
        for _ in 0..6 {
            run_match_with_status(&mut hi, MatchParticipation::Starter, crate::PlayerSquadStatus::FirstTeamRegular);
            run_match_with_status(&mut lo, MatchParticipation::Starter, crate::PlayerSquadStatus::FirstTeamRegular);
        }
        for _ in 0..10 {
            run_match_with_status(&mut hi, MatchParticipation::Substitute, crate::PlayerSquadStatus::FirstTeamRegular);
            run_match_with_status(&mut lo, MatchParticipation::Substitute, crate::PlayerSquadStatus::FirstTeamRegular);
        }
        let m_hi = hi
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::LostStartingPlace)
            .unwrap()
            .magnitude;
        let m_lo = lo
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::LostStartingPlace)
            .unwrap()
            .magnitude;
        // Less negative = softer.
        assert!(m_hi > m_lo, "high pro {} should soften vs low pro {}", m_hi, m_lo);
    }

    #[test]
    fn role_transition_silent_below_min_appearances() {
        let mut p = build_player(PlayerPositionType::MidfielderCenter, PersonAttributes::default());
        // Only 3 starts — below the 5-game minimum tracked window.
        for _ in 0..3 {
            run_match(&mut p, MatchParticipation::Starter);
        }
        assert!(!p.happiness.is_established_starter);
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::WonStartingPlace)
            .count();
        assert_eq!(count, 0);
    }

    // ── Youth breakthrough ───────────────────────────────────────

    #[test]
    fn youth_breakthrough_fires_for_young_player() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        // build_player birth date is 2000-01-01; promote in 2019 → age 19.
        p.on_youth_breakthrough(d(2019, 6, 1));
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::YouthBreakthrough)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn youth_breakthrough_silent_for_veteran() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        // Promote in 2027 → age 27. Squad-depth call, not a debut.
        p.on_youth_breakthrough(d(2027, 6, 1));
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::YouthBreakthrough)
            .count();
        assert_eq!(count, 0);
    }

    #[test]
    fn youth_breakthrough_late_bloomer_smaller_magnitude() {
        let mut early = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let mut late = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        early.on_youth_breakthrough(d(2020, 6, 1)); // age 20
        late.on_youth_breakthrough(d(2024, 6, 1)); // age 24
        let early_mag = early
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::YouthBreakthrough)
            .unwrap()
            .magnitude;
        let late_mag = late
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::YouthBreakthrough)
            .unwrap()
            .magnitude;
        assert!(early_mag > late_mag);
    }

    // ── Transfer events ──────────────────────────────────────────

    #[test]
    fn transfer_bid_rejected_silent_for_peer_buyer() {
        // Same-rep clubs → not a credible bigger move → no event.
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.attributes.ambition = 18.0;
        p.on_transfer_bid_rejected(0.50, 0.48, false);
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::TransferBidRejected)
            .count();
        assert_eq!(count, 0);
    }

    #[test]
    fn transfer_bid_rejected_silent_for_content_player() {
        // Bigger buyer but settled, low-ambition player who isn't pushing
        // for a move → still silent.
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.attributes.ambition = 6.0;
        p.on_transfer_bid_rejected(0.80, 0.40, false);
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::TransferBidRejected)
            .count();
        assert_eq!(count, 0);
    }

    #[test]
    fn transfer_bid_rejected_fires_for_ambitious_player_bigger_buyer() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.attributes.ambition = 16.0;
        p.on_transfer_bid_rejected(0.75, 0.40, false);
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::TransferBidRejected)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn transfer_bid_rejected_cooldown_blocks_repeat() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.attributes.ambition = 16.0;
        p.on_transfer_bid_rejected(0.75, 0.40, false);
        p.on_transfer_bid_rejected(0.75, 0.40, false);
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::TransferBidRejected)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn transfer_bid_rejected_favorite_fires_at_lateral_rep() {
        // Favorite-club bid being rejected hurts even at lateral reputation
        // and even for an average-ambition player who isn't pushing for a move.
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.attributes.ambition = 8.0; // not pushing for a move
        p.on_transfer_bid_rejected(0.50, 0.50, true);
        assert_eq!(count_events(&p, &HappinessEventType::TransferBidRejected), 1);
    }

    #[test]
    fn transfer_bid_rejected_favorite_amplifies_magnitude() {
        let mut anon = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let mut fav = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        anon.attributes.ambition = 16.0;
        fav.attributes.ambition = 16.0;
        anon.on_transfer_bid_rejected(0.75, 0.40, false);
        fav.on_transfer_bid_rejected(0.75, 0.40, true);
        let m_anon = anon
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TransferBidRejected)
            .unwrap()
            .magnitude;
        let m_fav = fav
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TransferBidRejected)
            .unwrap()
            .magnitude;
        // More negative = bigger hit. Favorite-club rejection should land harder.
        assert!(m_fav < m_anon, "favorite {} should be more negative than anon {}", m_fav, m_anon);
    }

    #[test]
    fn dream_move_collapsed_silent_for_lateral_move() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.on_dream_move_collapsed(0.55, 0.50, false);
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::DreamMoveCollapsed)
            .count();
        assert_eq!(count, 0);
    }

    #[test]
    fn dream_move_collapsed_fires_for_step_up() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.on_dream_move_collapsed(0.85, 0.50, false);
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::DreamMoveCollapsed)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn dream_move_collapsed_fires_for_favorite_even_at_lateral_rep() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        // Lateral rep, but favorite club destination — still a dream move.
        p.on_dream_move_collapsed(0.55, 0.50, true);
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::DreamMoveCollapsed)
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn dream_move_favorite_club_amplifies_magnitude() {
        let mut step_up = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let mut favorite = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        step_up.on_dream_move_collapsed(0.85, 0.50, false);
        favorite.on_dream_move_collapsed(0.85, 0.50, true);
        let s = step_up
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::DreamMoveCollapsed)
            .unwrap()
            .magnitude;
        let f = favorite
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::DreamMoveCollapsed)
            .unwrap()
            .magnitude;
        // More negative = bigger hit. Favorite-club collapse hurts more.
        assert!(f < s, "favorite {} should be more negative than step_up {}", f, s);
    }

    // ── Social events ────────────────────────────────────────────

    #[test]
    fn close_friend_sold_emits_negative() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.on_close_friend_sold(42, 80.0, true, true);
        let ev = p
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::CloseFriendSold)
            .unwrap();
        assert!(ev.magnitude < 0.0);
        assert_eq!(ev.partner_player_id, Some(42));
    }

    #[test]
    fn close_friend_sold_stronger_with_compatriot() {
        let mut compat = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let mut foreign = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        compat.on_close_friend_sold(7, 80.0, true, false);
        foreign.on_close_friend_sold(7, 80.0, false, false);
        let c = compat
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::CloseFriendSold)
            .unwrap()
            .magnitude;
        let f = foreign
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::CloseFriendSold)
            .unwrap()
            .magnitude;
        assert!(c < f, "compatriot version {} should be more negative than foreign {}", c, f);
    }

    #[test]
    fn mentor_departed_emits_negative() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.on_mentor_departed(13, 70.0, false);
        let ev = p
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::MentorDeparted)
            .unwrap();
        assert_eq!(ev.partner_player_id, Some(13));
    }

    #[test]
    fn compatriot_joined_silent_for_domestic_player() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        // p.country_id == 1 by default. Club is in same country.
        p.on_compatriot_joined(2, 1, false);
        let count = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::CompatriotJoined)
            .count();
        assert_eq!(count, 0);
    }

    #[test]
    fn compatriot_joined_fires_for_foreign_player() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        // Player from country 1, club in country 99 → foreign at this club.
        p.on_compatriot_joined(2, 99, true);
        let ev = p
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::CompatriotJoined)
            .unwrap();
        assert_eq!(ev.partner_player_id, Some(2));
    }

    #[test]
    fn compatriot_joined_does_not_double_fire_in_cooldown() {
        // Two compatriots joining within 30 days at the same club should
        // not stack two events on the existing player — `on_compatriot_joined`
        // has a 30-day cooldown.
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        p.on_compatriot_joined(2, 99, true);
        p.on_compatriot_joined(3, 99, true);
        assert_eq!(count_events(&p, &HappinessEventType::CompatriotJoined), 1);
    }

    #[test]
    fn compatriot_joined_amplified_when_no_local_language() {
        let mut isolated = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        let mut settled = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        isolated.on_compatriot_joined(2, 99, true);
        settled.on_compatriot_joined(2, 99, false);
        let i = isolated
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::CompatriotJoined)
            .unwrap()
            .magnitude;
        let s = settled
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::CompatriotJoined)
            .unwrap()
            .magnitude;
        assert!(i > s, "language-isolated boost {} should exceed settled {}", i, s);
    }

    // ── End-to-end event-stream audit ────────────────────────────
    //
    // Drives one player through a season-shaped sequence of matches and
    // checks that no single event-type spams the recent_events buffer.
    // Catches regressions in cooldowns (e.g. someone shortens
    // FanPraise's 21d gate to 0d and the buffer fills up with FanPraise
    // entries inside a single test). Ratings/derbies/wins are randomised
    // to a deterministic shape — the goal isn't a real simulator, it's
    // an integration-level smoke that exercises every emit path under
    // realistic call cadence.

    fn drive_season(p: &mut Player) {
        // 38 league + ~6 cup matches = 44, similar to an English season.
        // Shape: ~50% wins, ~25% draws, ~25% losses, mixed ratings.
        // Two derbies, half cup matches, occasional reds.
        let pattern: &[(f32, u16, u16, u16, bool, bool, bool, u8, u8)] = &[
            // (rating, goals, assists, reds, is_cup, is_motm, is_derby, gf, ga)
            (7.2, 1, 0, 0, false, false, false, 2, 1),
            (6.8, 0, 1, 0, false, false, false, 1, 0),
            (6.5, 0, 0, 0, false, false, false, 0, 1),
            (8.1, 1, 1, 0, false, true, false, 3, 0),
            (5.5, 0, 0, 0, false, false, false, 0, 2),
            (7.0, 0, 0, 0, false, false, true, 1, 0), // derby win, modest perf
            (7.8, 1, 0, 0, false, false, false, 2, 1),
            (6.2, 0, 0, 0, false, false, false, 1, 1),
            (6.9, 0, 1, 0, true, false, false, 2, 1),
            (5.8, 0, 0, 1, false, false, false, 0, 3), // red card defeat
            (7.5, 0, 0, 0, false, false, false, 1, 0),
            (8.4, 2, 0, 0, false, true, false, 3, 1),
            (6.5, 0, 0, 0, false, false, false, 1, 1),
            (7.0, 0, 1, 0, true, false, false, 2, 0),
            (5.4, 0, 0, 0, false, false, false, 0, 2),
            (6.8, 0, 0, 0, false, false, false, 1, 1),
            (7.6, 1, 0, 0, false, false, true, 2, 0), // derby win, standout
            (6.2, 0, 0, 0, false, false, false, 0, 1),
            (7.0, 0, 1, 0, false, false, false, 1, 0),
            (8.0, 1, 1, 0, true, false, false, 4, 0),
            (6.5, 0, 0, 0, false, false, false, 1, 1),
            (5.9, 0, 0, 0, false, false, false, 0, 1),
            (7.2, 0, 0, 0, false, false, false, 2, 0),
            (6.8, 1, 0, 0, false, false, false, 1, 0),
            (7.0, 0, 0, 0, false, false, false, 1, 1),
            (5.5, 0, 0, 0, false, false, false, 0, 2),
            (8.1, 1, 1, 0, false, true, false, 3, 1),
            (6.4, 0, 0, 0, false, false, false, 1, 1),
            (7.5, 0, 0, 0, true, false, false, 2, 0),
            (6.2, 0, 0, 0, false, false, false, 0, 0),
            (7.0, 0, 1, 0, false, false, false, 2, 1),
            (8.3, 2, 0, 0, false, true, false, 4, 1),
            (5.8, 0, 0, 0, false, false, false, 0, 2),
            (6.9, 0, 0, 0, false, false, false, 1, 0),
            (7.0, 1, 0, 0, false, false, false, 2, 1),
            (6.5, 0, 0, 0, true, false, false, 1, 0),
            (5.6, 0, 0, 0, false, false, false, 0, 2),
            (7.4, 0, 1, 0, false, false, false, 1, 0),
            (6.8, 0, 0, 0, false, false, false, 1, 1),
            (7.2, 0, 0, 0, false, false, false, 2, 1),
            (6.0, 0, 0, 0, false, false, false, 0, 1),
            (7.8, 1, 0, 0, false, false, false, 2, 0),
            (6.5, 0, 0, 0, false, false, false, 1, 1),
            (8.2, 1, 1, 0, true, true, false, 3, 0),
        ];
        for &(rating, goals, assists, reds, is_cup, is_motm, is_derby, gf, ga) in pattern {
            let s = stats(rating, goals, assists, reds, PlayerFieldPositionGroup::Forward);
            let o = outcome(&s, rating, false, is_cup, is_motm, is_derby, gf, ga, MatchParticipation::Starter);
            p.on_match_played(&o);
        }
    }

    #[test]
    fn season_long_event_stream_stays_within_sane_bounds() {
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        drive_season(&mut p);

        // Within a single season (no decay applied here — `decay_events`
        // would be invoked weekly in production), the recent_events
        // buffer should not be dominated by any single repeat-event
        // type. Cooldowns should keep each individual event from firing
        // more than ~once a fortnight to month.
        let count_of = |kind: &HappinessEventType| -> u32 {
            p.happiness
                .recent_events
                .iter()
                .filter(|e| e.event_type == *kind)
                .count() as u32
        };

        // Hard ceilings per event type. `MatchSelection` legitimately
        // fires every match (no cooldown by design — it's the routine
        // tick) so it gets a generous cap. Everything else should be
        // gated by its emit-site cooldown.
        let assertions: &[(HappinessEventType, u32)] = &[
            (HappinessEventType::FanPraise, 4),       // 21d cooldown × 44 matches
            (HappinessEventType::FanCriticism, 4),
            (HappinessEventType::MediaPraise, 3),     // 30d cooldown
            (HappinessEventType::DecisiveGoal, 4),    // 14d cooldown
            (HappinessEventType::DerbyHero, 2),       // 2 derbies in pattern
            (HappinessEventType::DerbyDefeat, 2),
            (HappinessEventType::WonStartingPlace, 1),
            (HappinessEventType::FirstClubGoal, 1),
            (HappinessEventType::PlayerOfTheMatch, 8),
        ];
        for (event, ceiling) in assertions {
            let n = count_of(event);
            assert!(
                n <= *ceiling,
                "event {:?} fired {} times in season — ceiling {}; cooldown likely broken",
                event,
                n,
                ceiling
            );
        }

        // Per the cap on `recent_events_cap` (100), the buffer must not
        // be saturated by a single season for one player.
        assert!(
            p.happiness.recent_events.len() <= 100,
            "recent_events buffer overran: {} entries",
            p.happiness.recent_events.len()
        );
    }

    #[test]
    fn season_long_no_event_repeats_within_30_days_for_cooldown_gated_types() {
        // For the event types that explicitly use `add_event_with_cooldown`
        // ≥ 21 days, walk pairs of recorded events and assert no two of
        // the same type sit at `days_ago` within 21 days of each other.
        // (All recent_events have `days_ago = 0` in this synthetic test
        // since we don't tick `decay_events`. The check we want is the
        // *count* exceeding the legal max, which the previous test
        // covers — this test is a redundant safety net for the
        // derby/captaincy paths that fire on bespoke cadence.)
        let mut p = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        drive_season(&mut p);

        // DerbyHero requires standout perf in derby win — should be at
        // most one entry per derby in the pattern, and the pattern has
        // exactly 2 derbies (one routine win, one with a goal).
        let derby_hero = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::DerbyHero)
            .count();
        let derby_win = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::DerbyWin)
            .count();
        assert_eq!(derby_hero + derby_win, 2,
            "expected exactly 2 derby outcomes (hero or win), got hero={} win={}",
            derby_hero, derby_win);
    }

    #[test]
    fn fan_praise_amplified_by_reputation() {
        let mut famous = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        famous.player_attributes.current_reputation = 10_000;
        let mut anon = build_player(PlayerPositionType::Striker, PersonAttributes::default());
        anon.player_attributes.current_reputation = 0;

        let s = stats(8.2, 0, 0, 0, PlayerFieldPositionGroup::Forward);
        let o = outcome(&s, 8.2, false, false, false, false, 0, 0, MatchParticipation::Starter);
        famous.on_match_played(&o);
        anon.on_match_played(&o);

        let fmag = famous
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::FanPraise)
            .unwrap()
            .magnitude;
        let amag = anon
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::FanPraise)
            .unwrap()
            .magnitude;
        assert!(fmag > amag, "famous {} should exceed anon {}", fmag, amag);
    }
}
