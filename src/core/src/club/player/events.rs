use crate::club::player::adaptation::PendingSigning;
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
    /// Goals conceded by this player's team — used for GK stats. `None`
    /// when this player isn't a starting goalkeeper.
    pub team_goals_against: Option<u8>,
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

pub struct TransferCompletion<'a> {
    pub from: &'a TeamInfo,
    pub to: &'a TeamInfo,
    pub fee: f64,
    pub date: NaiveDate,
    pub selling_club_id: u32,
}

pub struct LoanCompletion<'a> {
    pub from: &'a TeamInfo,
    pub to: &'a TeamInfo,
    pub loan_fee: f64,
    pub date: NaiveDate,
    pub loan_contract: PlayerClubContract,
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
        self.install_permanent_contract(t.fee, t.date);
        self.plan = Some(PlayerPlan::from_signing(self.age(t.date), t.fee, t.date));
        self.pending_signing = Some(PendingSigning {
            previous_salary,
            fee: t.fee,
            is_loan: false,
        });
    }

    /// React to a completed loan. The parent contract is preserved; the
    /// borrowing club's contract is installed as `contract_loan`.
    pub fn complete_loan(&mut self, l: LoanCompletion<'_>) {
        self.on_loan(l.from, l.to, l.loan_fee, l.date);
        self.reset_on_club_change();
        self.contract_loan = Some(l.loan_contract);
        self.pending_signing = Some(PendingSigning {
            previous_salary: None,
            fee: l.loan_fee,
            is_loan: true,
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
    fn install_permanent_contract(&mut self, fee: f64, date: NaiveDate) {
        let age = self.age(date);
        let years = if age < 24 { 5 } else if age < 28 { 4 } else if age < 32 { 3 } else { 2 };
        let expiry = date
            .checked_add_signed(chrono::Duration::days(years * 365))
            .unwrap_or(date);
        let salary = (fee * 0.05).max(500.0) as u32;
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

        if let Some(conceded) = o.team_goals_against {
            if self.position().is_goalkeeper() {
                let s = stats_bucket_mut(self, o.is_cup, o.is_friendly);
                s.conceded += conceded as u16;
                if conceded == 0 {
                    s.clean_sheets += 1;
                }
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
        }

        if o.is_motm {
            self.happiness.add_event(HappinessEventType::PlayerOfTheMatch, 4.0);
        }

        // Debriefs are only meaningful for competitive matches where the
        // player actually had stats recorded. Derbies double both directions —
        // a derby day lost is a wound; a derby day won lifts everyone.
        if !o.is_friendly && o.stats.match_rating >= 1.0 {
            let derby_mult = if o.is_derby { 2.0 } else { 1.0 };
            if o.effective_rating < 6.3 {
                let mag = -(2.0 + (6.3 - o.effective_rating).clamp(0.0, 3.0)) * derby_mult;
                self.happiness.add_event(HappinessEventType::ManagerCriticism, mag);
            } else if o.effective_rating >= 7.5 {
                let mag = (1.5 + (o.effective_rating - 7.5).clamp(0.0, 2.5)) * derby_mult;
                self.happiness.add_event(HappinessEventType::ManagerEncouragement, mag);
            }
        }

        // Team result morale: derby wins and losses swing morale on top of
        // individual-performance effects. Keeps losses from a rival stinging
        // even when the player personally played fine.
        if !o.is_friendly && o.is_derby {
            if o.team_won {
                self.happiness.add_event(HappinessEventType::ManagerEncouragement, 2.5);
            } else if o.team_lost {
                self.happiness.add_event(HappinessEventType::ManagerCriticism, -3.0);
            }
        }
    }

    /// Named to a squad but never got off the bench. Small morale hit.
    pub fn on_match_dropped(&mut self) {
        self.happiness.add_event(HappinessEventType::MatchDropped, -1.5);
    }

    /// An approach from `buyer_rep` has made it past the selling club's
    /// initial acceptance check, so it counts as real media-reported
    /// interest rather than a rumour. Flattery boost for ambitious
    /// players being chased upward; light destabilisation for the rest
    /// (rumour mill unsettles focus). Noop unless the gap is at least
    /// modest — generic peer-level interest isn't news.
    pub fn on_transfer_interest_confirmed(&mut self, buyer_rep: f32, seller_rep: f32) {
        let rep_diff = buyer_rep - seller_rep;
        if rep_diff < 0.1 {
            return;
        }
        let ambition = self.attributes.ambition;
        if ambition >= 12.0 {
            // Ambitious player flattered by a bigger club's interest.
            let mag = 1.0 + (rep_diff - 0.1).clamp(0.0, 0.6) * 4.0;
            self.happiness.add_event(HappinessEventType::ManagerEncouragement, mag);
        } else {
            // Settled player disrupted by headline-grabbing rumour.
            let mag = -(0.5 + (rep_diff - 0.1).clamp(0.0, 0.4) * 2.0);
            self.happiness.add_event(HappinessEventType::ManagerCriticism, mag);
        }
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
        self.happiness.add_event(HappinessEventType::ContractTerminated, -3.0);
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
