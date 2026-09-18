use crate::club::player::happiness::LoanSpellVerdict;
use crate::club::player::mind::{ActorRef, EpisodeKind, MindClock};
use crate::transfers::deal::offer::PromisedSquadStatus;
use crate::transfers::pipeline::{LoanDestinationPreference, LoanOutReason};
use crate::{Person, Player, PlayerStatusType};
use chrono::{Duration, NaiveDate};

/// Where the club currently has this player on its development pathway.
///
/// One ladder for every contracted player, academy graduate to veteran, so
/// "what is he here for" is a stored decision rather than a thing every
/// sweep re-derives from his birth year. The stages are ordered by how
/// close he is to the first team; [`PathwayStage::is_terminal`] separates
/// the two that end the pathway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathwayStage {
    /// Still in the academy or a youth side, not yet a senior question.
    Academy,
    /// A senior contract and a future here, not yet in the reckoning.
    Prospect,
    /// Out, or going out, to play somewhere else.
    LoanOut,
    /// Back from a spell, or between them, with the club deciding.
    Reassess,
    /// Squad depth that plays.
    Rotation,
    /// In the side.
    Starter,
    /// One of the men the team is built around.
    Core,
    /// The club means to cash him in while he is worth it.
    SellAtPeak,
    /// The club is finished with him.
    MoveOn,
}

impl PathwayStage {
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            PathwayStage::Academy => "pathway_stage_academy",
            PathwayStage::Prospect => "pathway_stage_prospect",
            PathwayStage::LoanOut => "pathway_stage_loan_out",
            PathwayStage::Reassess => "pathway_stage_reassess",
            PathwayStage::Rotation => "pathway_stage_rotation",
            PathwayStage::Starter => "pathway_stage_starter",
            PathwayStage::Core => "pathway_stage_core",
            PathwayStage::SellAtPeak => "pathway_stage_sell_at_peak",
            PathwayStage::MoveOn => "pathway_stage_move_on",
        }
    }

    pub fn as_token(self) -> &'static str {
        match self {
            PathwayStage::Academy => "academy",
            PathwayStage::Prospect => "prospect",
            PathwayStage::LoanOut => "loan_out",
            PathwayStage::Reassess => "reassess",
            PathwayStage::Rotation => "rotation",
            PathwayStage::Starter => "starter",
            PathwayStage::Core => "core",
            PathwayStage::SellAtPeak => "sell_at_peak",
            PathwayStage::MoveOn => "move_on",
        }
    }

    /// The pathway has run out: the club is selling or releasing him.
    #[inline]
    pub fn is_terminal(self) -> bool {
        matches!(self, PathwayStage::SellAtPeak | PathwayStage::MoveOn)
    }

    /// He is in the first-team reckoning today.
    #[inline]
    pub fn is_first_team(self) -> bool {
        matches!(
            self,
            PathwayStage::Rotation | PathwayStage::Starter | PathwayStage::Core
        )
    }

    /// One rung closer to the side — what a successor moves to when the
    /// man ahead of him is sold.
    pub fn promoted(self) -> Self {
        match self {
            PathwayStage::Academy => PathwayStage::Prospect,
            PathwayStage::Prospect | PathwayStage::LoanOut | PathwayStage::Reassess => {
                PathwayStage::Rotation
            }
            PathwayStage::Rotation => PathwayStage::Starter,
            PathwayStage::Starter => PathwayStage::Core,
            other => other,
        }
    }

    /// How hard the club pushes a loan for a man at this stage, 0..1 —
    /// the `pathway_push` term of [`crate::transfers::loan::ParentWillingness`].
    pub fn loan_push(self) -> f32 {
        match self {
            PathwayStage::LoanOut => 1.0,
            PathwayStage::Prospect | PathwayStage::Reassess => 0.7,
            PathwayStage::Rotation => 0.4,
            PathwayStage::Starter | PathwayStage::Core => 0.15,
            _ => 0.5,
        }
    }

    /// The band the club means a player at this stage to be playing at,
    /// as a [`crate::transfers::loan::LevelBand`] reading of its own squad.
    pub fn band_target(self) -> f32 {
        match self {
            PathwayStage::Academy => 0.4,
            PathwayStage::Prospect | PathwayStage::LoanOut | PathwayStage::Reassess => 0.7,
            PathwayStage::Rotation => 0.8,
            PathwayStage::Starter => 1.0,
            PathwayStage::Core => 1.15,
            PathwayStage::SellAtPeak | PathwayStage::MoveOn => 0.9,
        }
    }
}

/// The club's development pathway for one contracted player.
///
/// Two things at once, and deliberately so. It is still the listing
/// protection it always was — `role` / `min_games` / `evaluation_months`
/// are the commitment the club made when it signed him, and every
/// automatic surplus sweep respects it. And it is now the club's held
/// intention: what stage he is at, when it is next looked at, what a loan
/// would be FOR, and how the last spell went.
///
/// Set for every contracted player — at signing, at academy graduation, at
/// a youth-contract upgrade, and once for an existing squad the first time
/// the monthly review sees him.
#[derive(Debug, Clone)]
pub struct PlayerPlan {
    /// What role the club envisioned when signing this player.
    pub role: PlayerPlanRole,
    /// When the plan started (transfer date).
    pub started: NaiveDate,
    /// Minimum appearances before the club can fairly judge the player.
    pub min_games: u8,
    /// Months from `started` before the evaluation period ends.
    pub evaluation_months: u8,

    pub stage: PathwayStage,
    pub stage_since: NaiveDate,
    /// When the club next looks at him on purpose.
    pub review_on: NaiveDate,
    /// What the loan the club has staged for him is FOR. Carried the whole
    /// way to the borrower, never collapsed — the purpose decides the
    /// minutes bar, the reach and the subsidy.
    pub loan_purpose: Option<LoanOutReason>,
    pub loans_used: u8,
    pub last_verdict: Option<LoanSpellVerdict>,
    /// The band the club means him to reach HERE.
    pub band_target: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerPlanRole {
    /// Signed to be a first-team starter right away (experienced, high fee).
    ImmediateStarter,
    /// Signed to compete for a starting spot — needs integration time.
    CompeteForStarting,
    /// Backup / rotation signing — depth for the squad.
    DepthRotation,
    /// Young player signed for long-term development.
    Development,
}

impl PlayerPlan {
    /// Days a freshly-set pathway runs before the club looks again. A
    /// season's third: long enough for a window and a run of games.
    pub const REVIEW_DAYS: i64 = 120;
    /// Days a stage the club has just decided on runs before the next
    /// look, when the decision itself carried no deadline.
    pub const SHORT_REVIEW_DAYS: i64 = 60;
    /// Days a promoted returnee is given to hold the shirt he was promised.
    pub const PROMISE_REVIEW_DAYS: i64 = 180;

    /// Create a plan based on who the player is and what the club paid.
    ///
    /// Real clubs decide the role based on fee, age, and ability:
    /// - A 19yo for 8M → development project, give 2 years
    /// - A 26yo for 20M → compete for starting spot, give 1 year
    /// - A 31yo for 5M → experienced depth, evaluate in 6 months
    /// - A free agent → short trial period
    pub fn from_signing(age: u8, fee: f64, date: NaiveDate) -> Self {
        let (role, min_games, evaluation_months) = if age <= 21 {
            // Young player: long development runway
            (PlayerPlanRole::Development, 10, 18)
        } else if age <= 23 && fee > 0.0 {
            // Young-ish paid signing: still developing but expected to contribute
            (PlayerPlanRole::CompeteForStarting, 12, 12)
        } else if age <= 29 && fee > 0.0 {
            // Prime age paid signing: expected to compete for the team
            (PlayerPlanRole::CompeteForStarting, 15, 12)
        } else if age >= 30 && fee > 0.0 {
            // Experienced paid signing: should contribute quickly
            (PlayerPlanRole::ImmediateStarter, 10, 6)
        } else {
            // Free transfer / low investment: shorter evaluation
            (PlayerPlanRole::DepthRotation, 5, 6)
        };

        let stage = match role {
            PlayerPlanRole::Development => PathwayStage::Prospect,
            PlayerPlanRole::DepthRotation => PathwayStage::Rotation,
            PlayerPlanRole::CompeteForStarting => PathwayStage::Rotation,
            PlayerPlanRole::ImmediateStarter => PathwayStage::Starter,
        };

        PlayerPlan::at_stage(role, min_games, evaluation_months, stage, date)
    }

    /// The pathway an academy graduate starts on: a boy with a senior
    /// contract and no senior football yet.
    pub fn from_graduation(date: NaiveDate) -> Self {
        PlayerPlan::at_stage(
            PlayerPlanRole::Development,
            10,
            18,
            PathwayStage::Academy,
            date,
        )
    }

    /// The pathway derived once for a squad that existed before the club
    /// held pathways at all. The stage comes from what the club already
    /// believes about him, so nothing is invented.
    pub fn from_existing(role: PlayerPlanRole, stage: PathwayStage, date: NaiveDate) -> Self {
        let (_, evaluation_months) = match role {
            PlayerPlanRole::Development => (10, 18),
            PlayerPlanRole::CompeteForStarting => (12, 12),
            PlayerPlanRole::ImmediateStarter => (10, 6),
            PlayerPlanRole::DepthRotation => (5, 6),
        };
        // An existing squad member has already served whatever commitment
        // the club made to him, so the protection window opens behind him.
        let started = date - Duration::days(evaluation_months as i64 * 31);
        PlayerPlan {
            role,
            started,
            min_games: 0,
            evaluation_months,
            stage,
            stage_since: date,
            review_on: date + Duration::days(Self::REVIEW_DAYS),
            loan_purpose: None,
            loans_used: 0,
            last_verdict: None,
            band_target: stage.band_target(),
        }
    }

    fn at_stage(
        role: PlayerPlanRole,
        min_games: u8,
        evaluation_months: u8,
        stage: PathwayStage,
        date: NaiveDate,
    ) -> Self {
        PlayerPlan {
            role,
            started: date,
            min_games,
            evaluation_months,
            stage,
            stage_since: date,
            review_on: date + Duration::days(Self::REVIEW_DAYS),
            loan_purpose: None,
            loans_used: 0,
            last_verdict: None,
            band_target: stage.band_target(),
        }
    }

    /// Move him to a new stage, restating the band and the clock. Returns
    /// the stage he was on, so the caller can tell the player what changed.
    pub fn move_to(
        &mut self,
        stage: PathwayStage,
        date: NaiveDate,
        review_in_days: i64,
    ) -> PathwayStage {
        let from = self.stage;
        self.stage = stage;
        self.stage_since = date;
        self.review_on = date + Duration::days(review_in_days);
        self.band_target = stage.band_target();
        if stage != PathwayStage::LoanOut {
            self.loan_purpose = None;
        }
        from
    }

    /// Is the club's own review due?
    #[inline]
    pub fn review_due(&self, date: NaiveDate) -> bool {
        date >= self.review_on
    }

    /// Push the next look back without changing the stage.
    pub fn defer_review(&mut self, date: NaiveDate, days: i64) {
        self.review_on = date + Duration::days(days);
    }

    /// Has the plan's evaluation period concluded?
    ///
    /// A plan is "evaluated" only when BOTH conditions are met:
    /// 1. Enough time has passed (the club gave the player a fair window)
    /// 2. The player had enough appearances (they got a real chance)
    ///
    /// If either condition isn't met, the plan is still active and the player
    /// should not be listed for sale.
    pub fn is_evaluated(&self, current_date: NaiveDate, appearances: u16) -> bool {
        let months_elapsed = (current_date - self.started).num_days() / 30;
        let time_served = months_elapsed >= self.evaluation_months as i64;
        let games_played = appearances >= self.min_games as u16;

        time_served && games_played
    }

    /// Has enough time passed, regardless of appearances?
    /// Used as a fallback — even if a player never played, after a very long
    /// time the club should be allowed to move on.
    pub fn is_expired(&self, current_date: NaiveDate) -> bool {
        let months_elapsed = (current_date - self.started).num_days() / 30;
        // Double the evaluation period as absolute maximum
        months_elapsed >= (self.evaluation_months as i64) * 2
    }
}

impl Player {
    /// True while the club is still inside the evaluation commitment it made
    /// when signing this player — the plan window is neither served (time +
    /// appearances) nor expired. Every automatic surplus mechanism (weekly
    /// rebalance demotion, season-start positional trim, idle-days audit,
    /// the country listing sweep) must leave a protected signing alone: a
    /// club that just bought a player does not turn around and list him
    /// weeks later because a depth cap or a squad average says so.
    ///
    /// Player-initiated exits (formal transfer request, hardened
    /// unhappiness) and explicit manager decisions are NOT gated here —
    /// this only restrains the automatic numeric sweeps.
    pub fn signing_protection_active(&self, date: NaiveDate) -> bool {
        match &self.plan {
            Some(plan) => {
                let appearances = self.statistics.played + self.statistics.played_subs;
                !plan.is_evaluated(date, appearances) && !plan.is_expired(date)
            }
            None => false,
        }
    }

    /// Where the club has him on its pathway. `Prospect` when it has not
    /// formed one yet — a player nobody has decided about is not thereby
    /// surplus.
    #[inline]
    pub fn pathway_stage(&self) -> PathwayStage {
        self.plan
            .as_ref()
            .map(|p| p.stage)
            .unwrap_or(PathwayStage::Prospect)
    }

    /// The club states the pathway; the player is told.
    pub fn assign_pathway(&mut self, club_id: u32, plan: PlayerPlan, date: NaiveDate) {
        let from = self.plan.as_ref().map(|p| p.stage);
        let to = plan.stage;
        self.plan = Some(plan);
        if from != Some(to) {
            self.on_pathway_stage_changed(club_id, from, to, date);
        }
    }

    /// The club has moved him along its pathway. The club states the
    /// fact; what it means to him is his own business — he remembers it,
    /// and it goes on his record.
    ///
    /// A first pathway (`from` is `None`) is bookkeeping, not news: the
    /// club is writing down what it already thought of him.
    pub fn on_pathway_stage_changed(
        &mut self,
        club_id: u32,
        from: Option<PathwayStage>,
        to: PathwayStage,
        date: NaiveDate,
    ) {
        let Some(from) = from else {
            return;
        };
        if from == to {
            return;
        }

        let ctx = self.mind_context(date, Some(club_id));
        let episode = match (from.is_first_team(), to) {
            (_, PathwayStage::MoveOn) => Some(EpisodeKind::RoleDowngraded),
            (true, PathwayStage::LoanOut) | (true, PathwayStage::Reassess) => {
                Some(EpisodeKind::RoleDowngraded)
            }
            (false, stage) if stage.is_first_team() => Some(EpisodeKind::RoleUpgraded),
            _ => None,
        };
        if let Some(kind) = episode {
            self.mind.remember(kind, ActorRef::club(ctx.club_id), &ctx);
        }

        self.decision_history.add(
            date,
            "dec_pathway_stage_changed".to_string(),
            to.as_i18n_key().to_string(),
            "dec_decided_board".to_string(),
        );
    }

    /// The club has told him what he is for the next stretch. Written onto
    /// the contract as a bound promise, exactly as a signing promise is,
    /// so breaking it surfaces as playing-time unhappiness rather than
    /// being absorbed by the next rank recompute.
    pub fn on_squad_role_promised(
        &mut self,
        promise: PromisedSquadStatus,
        date: NaiveDate,
        days: i64,
    ) {
        let status = promise.as_squad_status();
        let Some(contract) = self.contract.as_mut() else {
            return;
        };
        if status.seniority_rank() > contract.squad_status.seniority_rank() {
            contract.squad_status = status.clone();
        }
        let until = date + Duration::days(days);
        contract.promised_squad_status = Some((status, until));
    }

    /// A finished spell, and what the parent made of it. The club reads
    /// the record and decides what he is; he reads the same record and
    /// decides what he is going to do about it — which is his own
    /// business, and is why the two are separate methods on separate
    /// owners.
    pub fn on_loan_spell_reviewed(
        &mut self,
        verdict: LoanSpellVerdict,
        parent_country_id: u32,
        date: NaiveDate,
    ) {
        let runway = ((34.0_f32 - self.age(date) as f32) / 12.0).clamp(0.0, 1.0);
        let at_home = parent_country_id == 0 || parent_country_id == self.country_id;
        self.mind
            .career
            .on_loan_spell_reviewed(verdict, runway, at_home, MindClock::day(date));
    }

    /// The club has put him on the market, and why.
    ///
    /// The badge is the visible half of the decision; the contract flag
    /// is the durable half. The flag survives the listing pass's badge
    /// reconciliation, blocks renewal offers, and tells that pass this
    /// is a club decision to materialise without a second history row.
    /// Without it the badge was stripped the first time the pass ran,
    /// the renewal manager saw a clean player and re-signed him, and the
    /// audit listed him again every window.
    pub fn on_listed_by_club(&mut self, reason: &str, date: NaiveDate) {
        self.statuses.add(date, PlayerStatusType::Lst);
        if let Some(contract) = self.contract.as_mut() {
            contract.is_transfer_listed = true;
        }
        self.decision_history.add(
            date,
            "dec_board_transfer_listed".to_string(),
            reason.to_string(),
            "dec_decided_board".to_string(),
        );
    }

    /// Where he would rather go if he is lent out — his own wants, read
    /// off the pull the weekly tick already computed. A ranking term on
    /// the broadcast, never a gate.
    pub fn loan_destination_preference(&self) -> LoanDestinationPreference {
        if self.home_pull.wanted {
            LoanDestinationPreference::HomeCountry
        } else {
            LoanDestinationPreference::Any
        }
    }
}
