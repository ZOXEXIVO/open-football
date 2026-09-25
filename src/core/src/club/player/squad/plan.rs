use crate::club::board::mandate::{MandateAuthor, MandatePurpose, MinutesCurve, SigningMandate};
use crate::club::player::happiness::LoanSpellVerdict;
use crate::club::player::mind::MindSituation;
use crate::club::player::mind::{ActorRef, EpisodeKind, MindClock};
use crate::transfers::deal::offer::PromisedSquadStatus;
use crate::transfers::loan::agreement::LoanMoney;
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
    /// What the board signed him for: the purpose, the minutes it promised
    /// him season by season, the money it approved and the years it writes
    /// that money off over. Every question the plan used to answer from
    /// (age, fee > 0) is answered from here instead.
    pub mandate: SigningMandate,

    pub stage: PathwayStage,
    pub stage_since: NaiveDate,
    /// When the club next looks at him on purpose.
    pub review_on: NaiveDate,
    /// What the loan the club has staged for him is FOR. Carried the whole
    /// way to the borrower, never collapsed — the purpose decides the
    /// minutes bar, the reach and the subsidy.
    pub loan_purpose: Option<LoanOutReason>,
    /// Share of his wage the board agreed to keep paying on the loan it
    /// staged to clear him — its own number, in place of the one the
    /// purpose implies. Gone with the staged loan.
    pub loan_subsidy: Option<f32>,
    pub loans_used: u8,
    pub last_verdict: Option<LoanSpellVerdict>,
    /// What the club would take for him, as a multiple of his value —
    /// the number a return verdict named. Read by the asset ledger,
    /// which is the one place a seller's ask is built.
    pub asking_multiple: Option<f32>,
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

    /// The pathway a mandate implies.
    ///
    /// Role and stage are the curve read at two points — what he is
    /// promised on day one, and whether that promise is a shirt, a share
    /// of one, or a place to grow. Nothing here reads his age or his fee:
    /// those decided the purpose, and the purpose decided the minutes.
    pub fn from_mandate(mandate: SigningMandate, date: NaiveDate) -> Self {
        let role = match mandate.purpose {
            MandatePurpose::Starter => PlayerPlanRole::ImmediateStarter,
            MandatePurpose::Asset | MandatePurpose::Heir { .. } => {
                PlayerPlanRole::CompeteForStarting
            }
            MandatePurpose::Rotation | MandatePurpose::Cover => PlayerPlanRole::DepthRotation,
            MandatePurpose::Prospect => PlayerPlanRole::Development,
        };
        let stage = match mandate.purpose {
            MandatePurpose::Starter => PathwayStage::Starter,
            MandatePurpose::Asset
            | MandatePurpose::Heir { .. }
            | MandatePurpose::Rotation
            | MandatePurpose::Cover => PathwayStage::Rotation,
            MandatePurpose::Prospect => PathwayStage::Prospect,
        };
        PlayerPlan::at_stage(role, mandate, stage, date)
    }

    /// The pathway an academy graduate starts on: a boy with a senior
    /// contract and no senior football yet.
    pub fn from_graduation(group: crate::PlayerFieldPositionGroup, date: NaiveDate) -> Self {
        PlayerPlan::at_stage(
            PlayerPlanRole::Development,
            SigningMandate::new(
                MandatePurpose::Prospect,
                group,
                18,
                date,
                MandateAuthor::Board,
            ),
            PathwayStage::Academy,
            date,
        )
    }

    /// The pathway derived once for a squad that existed before the club
    /// held pathways at all. The stage comes from what the club already
    /// believes about him, so nothing is invented — and the mandate it
    /// derives carries no approved fee, because none was ever signed.
    pub fn from_existing(
        role: PlayerPlanRole,
        stage: PathwayStage,
        group: crate::PlayerFieldPositionGroup,
        age: u8,
        date: NaiveDate,
    ) -> Self {
        let mandate = SigningMandate::new(
            MandatePurpose::from_stage(stage),
            group,
            age,
            // An existing squad member has already served whatever
            // commitment the club made to him, so the window opens behind
            // him rather than in front.
            date - Duration::days(MinutesCurve::MAX_HORIZON as i64 * 365),
            MandateAuthor::Board,
        );
        PlayerPlan {
            role,
            started: mandate.issued,
            mandate,
            stage,
            stage_since: date,
            review_on: date + Duration::days(Self::REVIEW_DAYS),
            loan_purpose: None,
            loan_subsidy: None,
            loans_used: 0,
            last_verdict: None,
            asking_multiple: None,
            band_target: stage.band_target(),
        }
    }

    fn at_stage(
        role: PlayerPlanRole,
        mandate: SigningMandate,
        stage: PathwayStage,
        date: NaiveDate,
    ) -> Self {
        PlayerPlan {
            role,
            started: date,
            mandate,
            stage,
            stage_since: date,
            review_on: date + Duration::days(Self::REVIEW_DAYS),
            loan_purpose: None,
            loan_subsidy: None,
            loans_used: 0,
            last_verdict: None,
            asking_multiple: None,
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
            self.loan_subsidy = None;
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

    /// Whole seasons the mandate has been running.
    #[inline]
    pub fn season_index(&self, date: NaiveDate) -> u8 {
        self.mandate.season_index(date)
    }

    /// Is he getting what the club promised him?
    ///
    /// One tolerance against the curve, at whatever season the mandate is
    /// in. This is the question every sweep that used to count idle days
    /// or appearances is really asking.
    pub fn on_schedule(&self, date: NaiveDate, delivered_share: f32, tracked: u8) -> bool {
        !Self::has_playing_view(tracked) || self.mandate.on_schedule(date, delivered_share)
    }

    /// Has the club seen enough of him to judge?
    ///
    /// It has when a season has gone by and it gave him the minutes it
    /// said it would. A man who never got them has not been evaluated —
    /// that is the club's own failure to integrate him, and it is why the
    /// automatic surplus sweeps leave him alone until the mandate's own
    /// horizon runs out.
    pub fn is_evaluated(&self, date: NaiveDate, delivered_share: f32, tracked: u8) -> bool {
        self.season_index(date) >= 1
            && Self::has_playing_view(tracked)
            && self.mandate.on_schedule(date, delivered_share)
    }

    /// Has the mandate run its course, whatever the minutes said?
    pub fn is_expired(&self, date: NaiveDate) -> bool {
        self.season_index(date) >= self.mandate.minutes.review_horizon()
    }

    /// Has he played enough for a share of the season to mean anything?
    #[inline]
    fn has_playing_view(tracked: u8) -> bool {
        tracked >= MindSituation::TRACKED_APPS
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
                !plan.is_evaluated(
                    date,
                    self.happiness.starter_ratio,
                    self.happiness.appearances_tracked,
                ) && !plan.is_expired(date)
            }
            None => false,
        }
    }

    /// The share of his club's matches he has actually started, and the
    /// matches behind it. The one reading every mandate-aware pass
    /// compares against what he was promised.
    #[inline]
    pub fn delivered_minutes(&self) -> (f32, u8) {
        (
            self.happiness.starter_ratio,
            self.happiness.appearances_tracked,
        )
    }

    /// He is getting what the club said he would when it signed him.
    /// `true` for a player nobody has written a plan for — a man the club
    /// has made no promise to cannot be behind on one.
    pub fn mandate_on_schedule(&self, date: NaiveDate) -> bool {
        let (share, tracked) = self.delivered_minutes();
        self.plan
            .as_ref()
            .map(|p| p.on_schedule(date, share, tracked))
            .unwrap_or(true)
    }

    /// What is left of the fee the club paid for him on its own books.
    /// Zero for everyone it did not buy.
    pub fn book_value(&self, date: NaiveDate) -> f64 {
        self.plan
            .as_ref()
            .map(|p| p.mandate.book_value(date))
            .unwrap_or(0.0)
    }

    /// The purpose the club bought him for, when it bought him.
    #[inline]
    pub fn mandate(&self) -> Option<&SigningMandate> {
        self.plan.as_ref().map(|p| &p.mandate)
    }

    /// The pathway a club writes for a man it has never written one for.
    /// His own position and age are the only things it needs that the
    /// caller does not already hold.
    pub fn default_plan(
        &self,
        role: PlayerPlanRole,
        stage: PathwayStage,
        date: NaiveDate,
    ) -> PlayerPlan {
        PlayerPlan::from_existing(
            role,
            stage,
            self.position().position_group(),
            self.age(date),
            date,
        )
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

    /// Why the club would send him out, when it has decided. Nobody
    /// outside reads his plan for it.
    #[inline]
    pub fn loan_purpose(&self) -> Option<LoanOutReason> {
        self.plan.as_ref().and_then(|p| p.loan_purpose)
    }

    /// How much of his wage his club means to keep paying if he is lent
    /// out: what its board agreed for this loan, else what the loan's
    /// purpose implies.
    pub fn loan_subsidy(&self) -> f32 {
        self.plan
            .as_ref()
            .and_then(|p| p.loan_subsidy)
            .unwrap_or_else(|| LoanMoney::parent_desire(self.pathway_stage(), self.loan_purpose()))
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

    /// The club has moved him a rung along its pathway, and says which.
    /// The caller states the fact; what it means to him is his own
    /// business.
    pub fn on_pathway_advanced(
        &mut self,
        club_id: u32,
        stage: PathwayStage,
        review_in_days: i64,
        date: NaiveDate,
    ) {
        let from = self.plan.as_ref().map(|p| p.stage);
        match self.plan.as_mut() {
            Some(plan) => {
                plan.move_to(stage, date, review_in_days);
            }
            None => {
                self.plan = Some(PlayerPlan::from_existing(
                    PlayerPlanRole::CompeteForStarting,
                    stage,
                    self.position().position_group(),
                    self.age(date),
                    date,
                ));
            }
        }
        self.on_pathway_stage_changed(club_id, from, stage, date);
    }

    /// The window shut and nobody took him. The intention lapses with it
    /// — the badge that said he was on his way out comes off, and the
    /// club owes him another look.
    pub fn on_pathway_loan_lapsed(&mut self, club_id: u32, date: NaiveDate) {
        self.statuses.remove(PlayerStatusType::Loa);
        self.decision_history.add(
            date,
            "dec_loan_withdrawn".to_string(),
            "dec_reason_no_borrower".to_string(),
            "dec_decided_board".to_string(),
        );
        self.on_pathway_advanced(
            club_id,
            PathwayStage::Reassess,
            PlayerPlan::SHORT_REVIEW_DAYS,
            date,
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
        loan_band: f32,
        date: NaiveDate,
    ) {
        let runway = self.career_runway(date);
        self.mind
            .career
            .on_loan_spell_reviewed(verdict, runway, loan_band, MindClock::day(date));
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

#[cfg(test)]
mod loan_subsidy_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::calculators::WageCalculator;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerFieldPositionGroup, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2027, 1, 31).unwrap()
        }

        /// A man his club has staged for a loan, for `purpose`.
        fn staged(purpose: LoanOutReason, board_subsidy: Option<f32>) -> Player {
            let mut player = PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("Test".to_string(), "Player".to_string()))
                .birth_date(NaiveDate::from_ymd_opt(2003, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap();
            let mut plan = PlayerPlan::from_existing(
                PlayerPlanRole::DepthRotation,
                PathwayStage::LoanOut,
                PlayerFieldPositionGroup::Midfielder,
                24,
                Self::date(),
            );
            plan.loan_purpose = Some(purpose);
            plan.loan_subsidy = board_subsidy;
            player.plan = Some(plan);
            player
        }
    }

    #[test]
    fn a_more_resolved_board_pays_more_of_the_wage() {
        let low = Fx::staged(LoanOutReason::FinancialRelief, Some(0.2));
        let high = Fx::staged(LoanOutReason::FinancialRelief, Some(0.9));
        let (low_borrower, _) =
            WageCalculator::loan_wage_split_v2(1_000_000, 0.6, low.loan_subsidy());
        let (high_borrower, _) =
            WageCalculator::loan_wage_split_v2(1_000_000, 0.6, high.loan_subsidy());
        assert!(
            high_borrower < low_borrower,
            "{high_borrower} vs {low_borrower}"
        );
    }

    #[test]
    fn other_loans_keep_the_subsidy_their_purpose_implies() {
        let development = Fx::staged(LoanOutReason::NeedsGameTime, None);
        assert_eq!(
            development.loan_subsidy(),
            LoanMoney::parent_desire(PathwayStage::LoanOut, Some(LoanOutReason::NeedsGameTime))
        );
    }

    #[test]
    fn the_board_subsidy_goes_with_the_staged_loan() {
        let mut player = Fx::staged(LoanOutReason::FinancialRelief, Some(0.9));
        let plan = player.plan.as_mut().unwrap();
        plan.move_to(
            PathwayStage::Reassess,
            Fx::date(),
            PlayerPlan::SHORT_REVIEW_DAYS,
        );
        assert_eq!(plan.loan_subsidy, None);
        assert_eq!(player.loan_subsidy(), 0.0);
    }
}
