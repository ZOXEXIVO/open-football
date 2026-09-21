//! A loan is three parties agreeing, not a player passing twelve gates.
//!
//! ```text
//! score = willingness × appetite × consent × affordability
//! ```
//!
//! Each term is continuous in 0..1 and each is somebody's actual
//! position: whether the parent would send him, whether the borrower
//! wants him, whether he would go, and whether the money works.
//! [`InterestDraw`] then draws from the candidates weighted by the
//! product, so a thin agreement is rare rather than forbidden.
//!
//! [`InterestDraw`]: super::interest::InterestDraw
//!
//! **Only physical constraints are hard**, and they live at the call
//! sites where they belong: the window is open, he has a contract and is
//! not already on loan, the clubs are not rivals, the route policy allows
//! it, he is not away with his country, and no negotiation is live for
//! the pair.

use chrono::NaiveDate;

use crate::club::Club;
use crate::club::player::mind::{CareerPlanView, MindClock};
use crate::club::player::player::Player;
use crate::transfers::loan::guard::LoanAssetGuard;
use crate::transfers::market::knowledge::ClubMarketKnowledge;
use crate::transfers::pipeline::LoanOutReason;
use crate::transfers::pipeline::trace::MarketSwitches;
use crate::transfers::squad::LevelBand;
use crate::{ClubPhilosophy, PathwayStage, PlayerFieldPositionGroup, ReputationLevel};

/// What the parent club can see about lending one of its own out.
///
/// Every field is a reading the caller already holds. Nothing here walks
/// the world, and nothing here is a verdict.
#[derive(Debug, Clone, Copy)]
pub struct ParentReading {
    /// He is inside the slots the formation starts at his position AND
    /// at the club's own key-player level —
    /// [`super::LoanAssetGuard::first_choice`]. A reading, never a veto.
    pub first_choice: bool,
    /// Share of the side's matches he has started, 0..1. The neutral 0.5
    /// stands for "not enough football to say".
    pub starter_share: f32,
    /// Years of prime left, 0..1.
    pub runway: f32,
    /// Spells he has already been sent on.
    pub loans_used: u8,
    /// Bodies the club has in his position group, and the fewest it can
    /// field with.
    pub group_count: usize,
    pub group_min_needed: usize,
    /// Where he sits in that queue, 0-based.
    pub rank: u8,
    /// What the club's own pathway says about him.
    pub stage: PathwayStage,
    pub philosophy: ClubPhilosophy,
    /// How hard his own plan is pushing for a season away, 0..1.
    pub plan_push: f32,
    /// He has formally asked to go.
    pub requested: bool,
    /// The club has advertised him.
    pub advertised: bool,
}

/// The three readings every loan caller needs about one man at one
/// moment: what his parent thinks, what he has decided, and how much of
/// his wage the parent means to keep paying.
///
/// Assembled here so that nobody outside his own module reads his plan
/// to work out the third one.
#[derive(Debug, Clone, Copy)]
pub struct LoanTerms {
    pub willingness: ParentWillingness,
    pub plan: CareerPlanView,
    pub parent_subsidy: f32,
}

impl LoanTerms {
    pub fn of(parent: &Club, player: &Player, date: NaiveDate) -> Self {
        LoanTerms {
            willingness: LoanAssetGuard::willingness_for(parent, player, date),
            plan: player.mind.career.plan_view(MindClock::day(date)),
            parent_subsidy: LoanMoney::parent_desire(player.pathway_stage(), player.loan_purpose()),
        }
    }
}

/// How willing the parent is to let him go, 0..1.
#[derive(Debug, Clone, Copy)]
pub struct ParentWillingness {
    pub score: f32,
    /// The first-choice term on its own, for the trace: the first thing
    /// a surprising score needs explaining by.
    pub starter_hold: f32,
    pub minutes: f32,
    pub depth_room: f32,
    /// What the DESTINATION did to the score — 1.0 until the pair is
    /// priced, because willingness is staged once per player and a
    /// destination does not exist yet when it is.
    pub placement_trust: f32,
}

impl ParentWillingness {
    /// How much of a club's willingness survives it being about its own
    /// first choice. Not zero: a first-choice full-back at a giant who
    /// has asked to go out and play is an ordinary loan.
    const STARTER_HOLD: f32 = 0.15;
    /// …opened this far by the man's own plan …
    const STARTER_HOLD_WANTED: f32 = 0.6;
    /// … and all the way by a formal request, which is his decision
    /// rather than the club's.
    const STARTER_HOLD_REQUESTED: f32 = 1.0;
    /// Start share at which a club stops reading him as a man who needs
    /// football elsewhere.
    const MINUTES_SPAN: f32 = 0.45;
    /// Runway at or above which age says nothing …
    const AGE_FIT_RUNWAY: f32 = 0.6;
    /// … and the willingness left at the very end of a career: a
    /// thirty-one-year-old squad player going out for a season is
    /// ordinary football.
    const AGE_FIT_FLOOR: f32 = 0.25;
    /// Spells already used, as a damper.
    const LOAN_FATIGUE: [f32; 4] = [1.0, 0.75, 0.45, 0.25];
    /// Bodies over the fielding minimum at which depth stops being a
    /// worry at all.
    const DEPTH_SPAN: f32 = 3.0;
    /// How much the plan has to be pushing before it opens the hold.
    pub const PLAN_OPENS_AT: f32 = 0.5;

    /// No objection at all — what a caller that cannot read the parent
    /// side gets, and what the `OF_LOAN_GUARD_OFF` arm gets everywhere.
    pub fn open() -> Self {
        ParentWillingness {
            score: 1.0,
            starter_hold: 1.0,
            minutes: 1.0,
            depth_room: 1.0,
            placement_trust: 1.0,
        }
    }

    pub fn of(reading: &ParentReading) -> Self {
        let starter_hold = if !reading.first_choice {
            1.0
        } else if reading.requested || reading.advertised {
            Self::STARTER_HOLD_REQUESTED
        } else if reading.plan_push >= Self::PLAN_OPENS_AT {
            Self::STARTER_HOLD_WANTED
        } else {
            Self::STARTER_HOLD
        };

        let minutes = (1.0 - reading.starter_share / Self::MINUTES_SPAN).clamp(0.0, 1.0);
        let age_fit = if reading.runway >= Self::AGE_FIT_RUNWAY {
            1.0
        } else {
            Self::AGE_FIT_FLOOR
                + (1.0 - Self::AGE_FIT_FLOOR) * (reading.runway / Self::AGE_FIT_RUNWAY)
        };
        let loan_fatigue = Self::LOAN_FATIGUE[(reading.loans_used as usize).min(3)];
        // Zero is the caller saying it cannot see the club's roster at
        // all, which is no view rather than an empty position group.
        let depth_room = if reading.group_count == 0 {
            1.0
        } else {
            ((reading.group_count as f32 - reading.group_min_needed as f32 + 1.0)
                / Self::DEPTH_SPAN)
                .clamp(0.0, 1.0)
        };
        let philosophy = match reading.philosophy {
            ClubPhilosophy::DevelopAndSell => 1.0,
            ClubPhilosophy::LoanFocused => 1.1,
            ClubPhilosophy::Balanced => 0.9,
            ClubPhilosophy::SignToCompete => 0.8,
        };
        // The pathway and his own minutes are two answers to the same
        // question, so the louder one speaks: a club that has already
        // decided to send him does not need him to be out of the side
        // first.
        let purpose = minutes.max(reading.stage.loan_push());

        let score = (starter_hold * purpose * age_fit * loan_fatigue * depth_room * philosophy)
            .clamp(0.0, 1.0);
        ParentWillingness {
            score,
            starter_hold,
            minutes,
            depth_room,
            placement_trust: 1.0,
        }
    }

    /// Willingness at or above which the club will entertain the idea at
    /// all — the one place the parent side is still a yes/no, and it sits
    /// low on purpose. Below it the club is not refusing a destination,
    /// it is refusing the conversation.
    pub const ENTERTAINS: f32 = 0.30;

    /// Willingness at which the whole market below the parent is fair
    /// game. A listing is consent to a loan, not consent to any
    /// destination: a club that merely tolerates the idea keeps him near
    /// its own level, and one that wants him gone opens everything.
    const CASCADE_OPEN: f32 = 0.6;
    /// Rungs between the top of the reputation ladder and the bottom.
    const CASCADE_LADDER: f32 = 5.0;

    /// Lowest tier a seller-side broadcast may walk down to.
    pub fn cascade_floor(score: f32, parent_tier: ReputationLevel) -> ReputationLevel {
        let steps = (score.clamp(0.0, 1.0) / Self::CASCADE_OPEN) * Self::CASCADE_LADDER;
        parent_tier.step_down(steps.round() as u32)
    }

    /// The club's own squad-asset classifier, folded into a willingness
    /// as the same fact [`Self::of`] prices from the other side.
    ///
    /// `first_choice` is read off the depth chart and the club's level
    /// bands; the asset class is read off his label, his record and his
    /// standing. They are two views of "is he one of ours", and a caller
    /// holding both takes whichever says he is more clearly first-team.
    /// `opened` is his own side of it — a request, a listing, or a plan
    /// pushing for a season away — and it lifts the hold exactly as it
    /// does inside [`Self::of`].
    pub fn held_as_first_team(score: f32, first_team: bool, opened: bool) -> f32 {
        if !first_team || opened {
            score
        } else {
            score.min(Self::STARTER_HOLD)
        }
    }

    /// Placement knowledge at or above which the parent is doing
    /// business it already does.
    const PLACEMENT_WORKING: f32 = ClubMarketKnowledge::WORKING_KNOWLEDGE;
    /// Share of that bar the ramp runs over.
    const PLACEMENT_RAMP: f32 = 0.5;
    /// …and what a destination it knows nothing about costs. Not all of
    /// it: a club will send a boy somewhere it has never sent one, rarely.
    const PLACEMENT_COST: f32 = 0.8;

    /// The same willingness, answered about ONE destination.
    ///
    /// Staged willingness is a fact about the club and the man and knows
    /// nothing about where he would go; this is the half that needs a
    /// borrower, applied where every other pair term is applied. A ramp
    /// to a floor rather than a gate — [`BorrowerAppetite::floor_term`]
    /// is the same shape on the other side of the deal.
    pub fn placed_into(self, trust: f32) -> Self {
        let below = ((Self::PLACEMENT_WORKING - trust.clamp(0.0, 1.0))
            / (Self::PLACEMENT_RAMP * Self::PLACEMENT_WORKING))
            .clamp(0.0, 1.0);
        let term = 1.0 - Self::PLACEMENT_COST * below;
        ParentWillingness {
            score: (self.score * term).clamp(0.0, 1.0),
            placement_trust: term,
            ..self
        }
    }

    /// Extra level-below-group-average a man needs before a surplus
    /// trigger fires, by where he sits in the queue. Modest, because the
    /// hold itself is priced in [`Self::of`] and the cushion must not do
    /// the same job a second time.
    pub fn depth_cushion(rank: usize) -> i16 {
        match rank {
            0 => 12,
            1 => 6,
            2 => 2,
            _ => 0,
        }
    }
}

/// What the borrowing club can see about taking him.
#[derive(Debug, Clone, Copy)]
pub struct BorrowerReading {
    /// Reputation tier of the borrower, as the base appetite scale.
    pub base_by_tier: f32,
    /// 1.0 in the mid-season window, less in the summer one.
    pub season_phase: f32,
    pub group: PlayerFieldPositionGroup,
    /// Bodies it has in the group, and how many it likes to carry.
    pub count: usize,
    pub ideal_depth: usize,
    /// Its own best in the group, on the observable scale, and the
    /// candidate's.
    pub best_here: u8,
    pub candidate: u8,
    /// Men clearly better than him in that group, and how many of them
    /// the purpose of the loan tolerates.
    pub clearly_better_ahead: usize,
    pub allowed_ahead: usize,
    /// What he would BE here, and what the move is supposed to make him.
    pub band_here: f32,
    pub band_target: f32,
    /// How ready he already is for his parent's first team, 0..1, and
    /// how far this club falls short of its standing / its division.
    pub readiness: f32,
    pub standing_ratio: f32,
    pub league_ratio: f32,
    /// How badly this club wants a body in that shirt, 0..1 —
    /// [`BorrowerNeed`].
    pub need: f32,
    /// Registration room this candidate's passport has here, 0..1 —
    /// [`crate::transfers::gate::fit::ForeignSlotCount::room_for`]. 1.0
    /// for a domestic passport and for a league that runs no quota.
    pub slot_room: f32,
}

/// How badly the borrowing club wants somebody in that shirt, 0..1.
///
/// One reading, read by the domestic scan, the seller broadcast and the
/// cross-border scan alike, so the same borrower cannot want a man
/// badly, mildly or not at all depending on who asks.
#[derive(Debug, Clone, Copy)]
pub struct BorrowerNeed {
    /// It has an open request at the group.
    pub requested: bool,
    /// Observable points the request is short of, and years it is over
    /// the age it asked for. Both zero for a candidate inside the band.
    pub level_shortfall: i16,
    pub age_excess: i16,
    /// How short the group is of the bodies it likes to carry, 0..1.
    pub vacancy: f32,
}

impl BorrowerNeed {
    /// What a club with no request and a full group still gives a
    /// candidate. Not zero: somebody is always worth a look.
    const BASE: f32 = 0.35;
    /// … and how much a request that matches him adds on top.
    const REQUEST_SPAN: f32 = 0.65;
    /// Observable points below the request at which it stops matching
    /// him at all …
    const LEVEL_TAPER: f32 = 10.0;
    /// … and years over the age band it asked for.
    const AGE_TAPER: f32 = 6.0;
    /// What an empty shirt adds when nobody asked for one.
    const VACANCY_LIFT: f32 = 0.25;

    /// No request, no vacancy — what a club that has not said anything
    /// gives a name it is shown.
    pub fn none() -> Self {
        BorrowerNeed {
            requested: false,
            level_shortfall: 0,
            age_excess: 0,
            vacancy: 0.0,
        }
    }

    pub fn score(&self) -> f32 {
        let level = 1.0 - (self.level_shortfall.max(0) as f32 / Self::LEVEL_TAPER).clamp(0.0, 1.0);
        let age = 1.0 - (self.age_excess.max(0) as f32 / Self::AGE_TAPER).clamp(0.0, 1.0);
        let request_match = if self.requested { level * age } else { 0.0 };
        (Self::BASE
            + Self::REQUEST_SPAN * request_match
            + Self::VACANCY_LIFT * self.vacancy.clamp(0.0, 1.0))
        .clamp(0.0, 1.0)
    }
}

/// How badly the borrower wants him, 0..1.
#[derive(Debug, Clone, Copy)]
pub struct BorrowerAppetite {
    pub score: f32,
    pub room: f32,
    /// The registration quota's own term, for the trace: a full appetite
    /// killed by a full quota is a different story from one killed by a
    /// full position group.
    pub slot_room: f32,
    pub minutes_here: f32,
    pub fit_band: f32,
    pub level_floor: f32,
}

impl BorrowerAppetite {
    /// How far a group short of its ideal depth lifts the appetite.
    const SHORTAGE_LIFT: f32 = 0.6;
    /// Observable points over the borrower's best at which a full line
    /// is fully open to him again.
    const UPGRADE_SPAN: f32 = 10.0;
    /// How wide a band mismatch has to be before the fit term is at its
    /// worst …
    const FIT_SPAN: f32 = LevelBand::ONE_LEVEL;
    /// … and how much of the appetite that worst case costs.
    ///
    /// Deliberately not all of it. The band is the club's own nominal
    /// standard, and a side can be nominally National with an Amateur
    /// keeper room — a raw boy who walks into THAT team is the
    /// development loan. What says he would play is `minutes_here`, and
    /// that one is allowed to kill a minutes loan outright.
    const FIT_COST: f32 = 0.6;
    /// How far below the floor a destination has to sit before the term
    /// is at its worst — half of it …
    const FLOOR_RAMP: f32 = 0.5;
    /// … and what that worst case costs. A destination below the floor
    /// the player's readiness sets is discounted, never refused.
    const FLOOR_COST: f32 = 0.85;
    /// Club-standing floor a raw loanee is held to, and the extra a
    /// fully first-team-ready one adds. Both are soft now.
    const STANDING_FLOOR_RAW: f32 = 0.12;
    const STANDING_FLOOR_SPAN: f32 = 0.43;
    /// The same pair for the division he would play in.
    const LEAGUE_FLOOR_RAW: f32 = 0.40;
    const LEAGUE_FLOOR_SPAN: f32 = 0.30;

    pub fn of(reading: &BorrowerReading) -> Self {
        let shortage = 1.0
            + Self::SHORTAGE_LIFT
                * ((reading.ideal_depth as f32 - reading.count as f32)
                    / reading.ideal_depth.max(1) as f32)
                    .clamp(0.0, 1.0);

        // A full line is not shut; it is open in proportion to how much
        // of an upgrade he is.
        let room = if reading.count >= reading.ideal_depth {
            ((reading.candidate as f32 - reading.best_here as f32) / Self::UPGRADE_SPAN)
                .clamp(0.0, 1.0)
        } else {
            1.0
        };

        let minutes_here = (1.0
            - reading.clearly_better_ahead as f32 / reading.allowed_ahead.max(1) as f32)
            .clamp(0.0, 1.0);

        let fit_band = 1.0
            - Self::FIT_COST
                * ((reading.band_here - reading.band_target).abs() / Self::FIT_SPAN)
                    .clamp(0.0, 1.0);

        let standing_floor =
            (Self::STANDING_FLOOR_RAW + Self::STANDING_FLOOR_SPAN * reading.readiness).min(0.55);
        let league_floor =
            (Self::LEAGUE_FLOOR_RAW + Self::LEAGUE_FLOOR_SPAN * reading.readiness).min(0.70);
        let level_floor = Self::floor_term(reading.standing_ratio, standing_floor)
            * Self::floor_term(reading.league_ratio, league_floor);

        let score = (reading.base_by_tier
            * reading.season_phase
            * shortage
            * room
            * reading.slot_room.clamp(0.0, 1.0)
            * minutes_here
            * fit_band
            * level_floor
            * reading.need)
            .clamp(0.0, 1.0);
        BorrowerAppetite {
            score,
            room,
            slot_room: reading.slot_room.clamp(0.0, 1.0),
            minutes_here,
            fit_band,
            level_floor,
        }
    }

    /// Base appetite by reputation tier. Elite and Continental clubs do
    /// take loans — cover, a compatriot, a prospect — they simply take
    /// fewer of them, which is a smaller number rather than a closed
    /// door.
    pub fn base_for_tier(tier: ReputationLevel) -> f32 {
        match tier {
            ReputationLevel::Elite => 0.25,
            ReputationLevel::Continental => 0.50,
            ReputationLevel::National => 0.80,
            _ => 1.0,
        }
    }

    /// Men clearly better than him this purpose tolerates ahead of him.
    /// A keeper needs the shirt; a development loan tolerates one; cover
    /// tolerates two.
    pub fn allowed_ahead(group: PlayerFieldPositionGroup, development: bool) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 1,
            _ if development => 2,
            _ => 3,
        }
    }

    /// Observable points ahead at which a man counts as clearly better.
    /// Both sides of the comparison are
    /// [`AbilityEstimator::observable_level`](crate::club::staff::perception::AbilityEstimator::observable_level):
    /// a band is a statement about the player football can see.
    pub const CLEARLY_BETTER: u8 = 6;

    /// A ratio against a floor, as a factor rather than a verdict. An
    /// unknown ratio (zero on either side) stands the term down, exactly
    /// as the hard gates did.
    ///
    /// A ramp rather than three steps: cubed by the draw alongside a
    /// three-valued `need`, two step functions dominated every continuous
    /// term in the product and the whole agreement sat within a couple of
    /// multiples of the floor for an ordinary pair.
    fn floor_term(ratio: f32, floor: f32) -> f32 {
        if ratio <= 0.0 || floor <= 0.0 {
            return 1.0;
        }
        let below = ((floor - ratio) / (Self::FLOOR_RAMP * floor)).clamp(0.0, 1.0);
        1.0 - Self::FLOOR_COST * below
    }
}

/// What the player can see about going.
#[derive(Debug, Clone, Copy)]
pub struct ConsentReading {
    /// The arc he is living out — the term that tells a step down he
    /// meant to take from one he is being talked into.
    pub plan: CareerPlanView,
    /// What he would BE at the borrower.
    pub band_here: f32,
    /// How far the destination falls below his own renown, and the band
    /// he tolerates.
    pub renown_gap: f32,
    pub renown_band: f32,
    /// The move is toward his own country.
    pub going_home: bool,
    /// How far he has lowered his sights, 0..1.
    pub resignation: f32,
    /// How familiar the destination is to HIM — his country's export
    /// corridor, his diaspora, his language, whichever speaks loudest
    /// ([`MarketAffinity::player_affinity`]). 1.0 for a domestic pair and
    /// for a man going home.
    ///
    /// [`MarketAffinity::player_affinity`]:
    ///     crate::transfers::MarketAffinity::player_affinity
    pub familiarity: f32,
}

/// Whether he would go, 0..1.
#[derive(Debug, Clone, Copy)]
pub struct PlayerConsent {
    pub score: f32,
    pub plan_fit: f32,
    /// What the place cost him, for the trace.
    pub familiarity_cost: f32,
}

impl PlayerConsent {
    /// What a man with no plan and nothing to object to gives a
    /// destination. Not 1.0: a loan is somebody else's idea until he has
    /// made it his own.
    const BASE: f32 = 0.6;
    /// How much the arc he is living out moves it, either way.
    const PLAN_WEIGHT: f32 = 0.35;
    /// A move toward his own country.
    const HOME: f32 = 0.2;
    /// How far a destination below his own renown costs him, at a full
    /// band of daylight.
    const RENOWN_COST: f32 = 0.45;
    /// …and how much of that months on the market take back.
    const RESIGNATION_RELIEF: f32 = 0.6;
    /// Reading of the place at or above which it is simply somewhere he
    /// could live — his compatriots go there, or he speaks it.
    const FAMILIAR: f32 = 0.35;
    /// What a year somewhere he knows nothing about costs him. Below the
    /// renown cost on purpose: a strange country is a reason to say no,
    /// and a smaller one than dropping two divisions.
    const FAMILIARITY_COST: f32 = 0.30;

    pub fn of(reading: &ConsentReading) -> Self {
        let plan_fit = reading.plan.fit_for(reading.band_here, reading.going_home);
        let renown = if reading.renown_band > 0.0 {
            ((reading.renown_gap - reading.renown_band) / reading.renown_band).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let renown_cost =
            Self::RENOWN_COST * renown * (1.0 - Self::RESIGNATION_RELIEF * reading.resignation);
        // The same shape the renown cost uses, and relieved by the same
        // two things: a man who has lowered his sights, and a man whose
        // own arc is pushing him out of where he is, will go where the
        // football is. A strange place is a cost, never a wall.
        let strangeness = ((Self::FAMILIAR - reading.familiarity) / Self::FAMILIAR).clamp(0.0, 1.0);
        let relief = (Self::RESIGNATION_RELIEF * reading.resignation).max(plan_fit.max(0.0));
        let familiarity_cost =
            Self::FAMILIARITY_COST * strangeness * (1.0 - relief.clamp(0.0, 1.0));
        let score = (Self::BASE
            + Self::PLAN_WEIGHT * plan_fit
            + Self::HOME * f32::from(reading.going_home)
            - renown_cost
            - familiarity_cost)
            .clamp(0.0, 1.0);
        PlayerConsent {
            score,
            plan_fit,
            familiarity_cost,
        }
    }
}

/// What the deal costs, and who pays it.
#[derive(Debug, Clone, Copy)]
pub struct MoneyReading {
    /// Player value ÷ the borrower's annual income.
    pub weight: f64,
    /// The borrower's share of the wage ÷ what it can actually pay,
    /// computed AFTER any parent subsidy.
    pub carry: f64,
    /// What the parent is asking, and what the borrower can spend.
    pub asking: f64,
    pub max_loan_fee: f64,
    /// The loan is one the parent wants for its own reasons, so it is
    /// willing to keep paying part of the wage.
    pub development: bool,
}

/// Whether the money works, 0..1.
#[derive(Debug, Clone, Copy)]
pub struct LoanMoney {
    pub affordability: f32,
}

impl LoanMoney {
    /// Value ÷ borrower income at which the capacity penalty starts …
    pub const W_SOFT: f64 = 0.35;
    /// … and the ceiling for an ordinary cover loan …
    pub const W_MAX: f64 = 1.0;
    /// … which a loan the parent wants stretches to, because a parent
    /// paying most of the wage has changed what the borrower is
    /// carrying.
    pub const W_MAX_DEVELOPMENT: f64 = 2.5;
    /// Wage share ÷ what the borrower can pay, above which it is
    /// borrowing a wage it cannot carry whatever the fee is. The one
    /// triple, read by the draw and by the negotiation room alike.
    pub const CARRY_MAX: f64 = 1.0;
    /// How much of the affordability the weight term can take.
    const WEIGHT_PENALTY: f32 = 0.6;
    /// The share of the asking price a borrower typically tables.
    pub const OFFER_SHARE: f64 = 0.8;
    /// Floor on the fee term, so an expensive loan is dear rather than
    /// impossible.
    const FEE_FLOOR: f32 = 0.2;

    pub fn of(reading: &MoneyReading) -> Self {
        let w_max = if reading.development {
            Self::W_MAX_DEVELOPMENT
        } else {
            Self::W_MAX
        };
        let weight_term = 1.0
            - Self::WEIGHT_PENALTY
                * (((reading.weight - Self::W_SOFT) / (w_max - Self::W_SOFT)) as f32)
                    .clamp(0.0, 1.0);
        let carry_term = (1.0 - reading.carry as f32).clamp(0.0, 1.0);
        let offer = reading.asking * Self::OFFER_SHARE;
        let fee_term = if offer <= reading.max_loan_fee || offer <= 0.0 {
            1.0
        } else {
            ((reading.max_loan_fee / offer) as f32).clamp(Self::FEE_FLOOR, 1.0)
        };
        LoanMoney {
            affordability: (carry_term * weight_term * fee_term).clamp(0.0, 1.0),
        }
    }

    /// How much of the wage the parent means to keep paying, 0..1 — the
    /// `parent_desire_to_develop` the wage split takes. A loan the
    /// parent arranged for its own reasons is one it subsidises.
    pub fn parent_desire(stage: PathwayStage, purpose: Option<LoanOutReason>) -> f32 {
        if stage != PathwayStage::LoanOut {
            return 0.0;
        }
        match purpose {
            Some(LoanOutReason::DevelopmentPathway)
            | Some(LoanOutReason::NeedsGameTime)
            | Some(LoanOutReason::NeedsFirstTeamMinutes)
            | Some(LoanOutReason::BlockedByDepth)
            | Some(LoanOutReason::AssetValueProtection) => 1.0,
            Some(_) => 0.3,
            None => 0.0,
        }
    }
}

/// The three parties and the money, folded into one number.
pub struct LoanAgreement;

impl LoanAgreement {
    /// Below this nobody is agreeing to anything. It exists so a
    /// candidate list stays finite, not to express a judgement: every
    /// real refusal is priced in one of the four terms.
    pub const FLOOR: f32 = 0.01;

    /// `willingness × appetite × consent × affordability`.
    pub fn score(
        parent: &ParentWillingness,
        borrower: &BorrowerAppetite,
        player: &PlayerConsent,
        money: &LoanMoney,
    ) -> f32 {
        if Self::disarmed() {
            return 1.0;
        }
        (parent.score * borrower.score * player.score * money.affordability).clamp(0.0, 1.0)
    }

    /// The A/B arm that runs the conjunctive gate stack instead — see
    /// [`crate::transfers::loan::legacy`].
    pub fn disarmed() -> bool {
        MarketSwitches::loan_agreement_off()
    }

    /// One grepable line per priced pair, for `OF_TRACE_PLAYER`.
    pub fn explain(
        parent: &ParentWillingness,
        borrower: &BorrowerAppetite,
        player: &PlayerConsent,
        money: &LoanMoney,
    ) -> String {
        format!(
            "agreement={:.3} willingness={:.2} (hold={:.2} minutes={:.2} depth={:.2} \
             placement={:.2}) \
             appetite={:.2} (room={:.2} slots={:.2} minutes={:.2} band={:.2} floor={:.2}) \
             consent={:.2} (plan={:+.2} strange={:.2}) affordability={:.2}",
            Self::score(parent, borrower, player, money),
            parent.score,
            parent.starter_hold,
            parent.minutes,
            parent.depth_room,
            parent.placement_trust,
            borrower.score,
            borrower.room,
            borrower.slot_room,
            borrower.minutes_here,
            borrower.fit_band,
            borrower.level_floor,
            player.score,
            player.plan_fit,
            player.familiarity_cost,
            money.affordability,
        )
    }
}

/// Everything the three sides know about one (player, borrower) pair,
/// flat, so the domestic scan, the seller broadcast and the foreign scan
/// price a loan through the same call instead of each repeating the
/// gate cluster in its own filter chain.
#[derive(Debug, Clone, Copy)]
pub struct AgreementInputs {
    // ── the parent ──────────────────────────────────────────────
    /// What the parent's own reading of him came to — carried whole so
    /// the trace describes the deal the score priced.
    pub parent: ParentWillingness,
    pub parent_rep: u16,
    pub parent_league_rep: u16,
    pub parent_best_in_group: u8,
    /// What the parent will keep paying of his wage, 0..1.
    pub parent_subsidy: f32,
    /// How far the parent's own placement network reaches into the
    /// borrower's country, 0..1 — [`LoanPlacementKnowledge`]. 1.0 for a
    /// domestic pair.
    ///
    /// [`LoanPlacementKnowledge`]:
    ///     crate::transfers::market::knowledge::LoanPlacementKnowledge
    pub placement_trust: f32,

    // ── the borrower ────────────────────────────────────────────
    pub borrower_tier: ReputationLevel,
    pub borrower_rep: u16,
    pub borrower_league_rep: u16,
    pub group: PlayerFieldPositionGroup,
    pub count: usize,
    pub best_here: u8,
    pub clearly_better_ahead: usize,
    /// 1.0 an open request at the group, 0.6 a vacancy, 0.35 neither.
    pub need: f32,
    /// Registration room this passport has at the borrower, 0..1 —
    /// [`crate::transfers::gate::fit::ForeignSlotCount::room_for`].
    pub slot_room: f32,
    /// The mid-season window is open where the borrower plays.
    pub mid_season_window: bool,

    // ── the player ──────────────────────────────────────────────
    pub candidate: u8,
    pub is_development: bool,
    /// Where the parent's pathway has him. Read when he holds no plan of
    /// his own: the club's rung says what the move is supposed to make
    /// him just as his arc would.
    pub stage: PathwayStage,
    /// The band the parent's own loan-out row names, when it named one.
    /// Outranks the stage's default: it is a decision about this spell
    /// rather than about his rung.
    pub club_band_target: Option<f32>,
    pub plan: CareerPlanView,
    pub renown_gap: f32,
    pub renown_band: f32,
    pub resignation: f32,
    pub going_home: bool,
    /// How familiar the borrower's country is to him, 0..1.
    pub familiarity: f32,

    // ── the money ───────────────────────────────────────────────
    /// Value ÷ the borrower's year, and the share of his wage the
    /// borrower is left carrying once the parent's subsidy is written
    /// into the split. Both come from [`super::LoanAssetGuard::assess`],
    /// which prices them once for the pair.
    pub weight: f64,
    pub carry: f64,
    pub asking: f64,
    pub max_loan_fee: f64,
}

impl AgreementInputs {
    /// The mid-season window is when a borrower most wants a body …
    const MID_SEASON_PHASE: f32 = 1.0;
    /// … and the summer one is when it would rather sign somebody.
    const SUMMER_PHASE: f32 = 0.85;

    /// What he would BE at the borrower.
    pub fn band_here(&self) -> f32 {
        LevelBand::at_reputation(
            self.candidate,
            self.group,
            (self.borrower_rep as f32 / 10_000.0).clamp(0.0, 1.0),
        )
    }

    /// What the move is supposed to make him. His own plan states it
    /// when he has one; failing that his club's own pathway does, which
    /// is the same sentence from the other side of the desk.
    pub fn band_target(&self) -> f32 {
        if self.plan.arc.is_some() {
            self.plan.band_target
        } else {
            self.club_band_target
                .unwrap_or_else(|| self.stage.band_target())
        }
    }
}

impl LoanAgreement {
    /// Price one pair: the four terms, multiplied, or `None` when
    /// nobody would agree to anything.
    pub fn price(inputs: &AgreementInputs) -> Option<f32> {
        if Self::disarmed() {
            return Some(1.0);
        }
        let (parent, appetite, consent, money) = Self::terms(inputs);
        let score = Self::score(&parent, &appetite, &consent, &money);
        Some(score).filter(|s| *s >= Self::FLOOR)
    }

    /// The four terms themselves. One place builds them, so the score
    /// and the trace can never describe different deals.
    pub fn terms(
        inputs: &AgreementInputs,
    ) -> (
        ParentWillingness,
        BorrowerAppetite,
        PlayerConsent,
        LoanMoney,
    ) {
        let parent = inputs.parent.placed_into(inputs.placement_trust);
        let band_here = inputs.band_here();
        let readiness = LevelBand::readiness_of(inputs.candidate, inputs.parent_best_in_group);
        let appetite = BorrowerAppetite::of(&BorrowerReading {
            base_by_tier: BorrowerAppetite::base_for_tier(inputs.borrower_tier),
            season_phase: if inputs.mid_season_window {
                AgreementInputs::MID_SEASON_PHASE
            } else {
                AgreementInputs::SUMMER_PHASE
            },
            group: inputs.group,
            count: inputs.count,
            ideal_depth: inputs.group.ideal_squad_depth(),
            best_here: inputs.best_here,
            candidate: inputs.candidate,
            clearly_better_ahead: inputs.clearly_better_ahead,
            allowed_ahead: BorrowerAppetite::allowed_ahead(inputs.group, inputs.is_development),
            band_here,
            band_target: inputs.band_target(),
            readiness,
            standing_ratio: Self::ratio(inputs.borrower_rep, inputs.parent_rep),
            league_ratio: Self::ratio(inputs.borrower_league_rep, inputs.parent_league_rep),
            need: inputs.need,
            slot_room: inputs.slot_room,
        });
        let consent = PlayerConsent::of(&ConsentReading {
            plan: inputs.plan,
            band_here,
            renown_gap: inputs.renown_gap,
            renown_band: inputs.renown_band,
            going_home: inputs.going_home,
            resignation: inputs.resignation,
            familiarity: inputs.familiarity,
        });
        let money = LoanMoney::of(&MoneyReading {
            weight: inputs.weight,
            carry: inputs.carry,
            asking: inputs.asking,
            max_loan_fee: inputs.max_loan_fee,
            development: inputs.is_development,
        });
        (parent, appetite, consent, money)
    }

    /// The same pair, explained: one line carrying every term, so a
    /// stuck case can be read off the funnel rather than re-derived
    /// from file:line. `OF_TRACE_PLAYER=<id>`.
    pub fn explain_inputs(inputs: &AgreementInputs) -> String {
        let (parent, appetite, consent, money) = Self::terms(inputs);
        format!(
            "{} band={:.2}->{:.2} tier={:?} subsidy={:.2}",
            Self::explain(&parent, &appetite, &consent, &money),
            inputs.band_here(),
            inputs.band_target(),
            inputs.borrower_tier,
            inputs.parent_subsidy,
        )
    }

    /// A reputation ratio, with an unknown side standing the term down
    /// exactly as the hard gates did.
    fn ratio(borrower: u16, parent: u16) -> f32 {
        if parent == 0 || borrower == 0 {
            0.0
        } else {
            borrower as f32 / parent as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::mind::{CareerArc, PlanStage};

    struct Fx;

    impl Fx {
        /// A squad player at a big club: out of the side, plenty of
        /// runway, one spell behind him, a group with bodies to spare.
        fn fringe() -> ParentReading {
            ParentReading {
                first_choice: false,
                starter_share: 0.1,
                runway: 0.8,
                loans_used: 0,
                group_count: 7,
                group_min_needed: 4,
                rank: 4,
                stage: PathwayStage::Prospect,
                philosophy: ClubPhilosophy::Balanced,
                plan_push: 0.0,
                requested: false,
                advertised: false,
            }
        }

        fn borrower() -> BorrowerReading {
            BorrowerReading {
                base_by_tier: 1.0,
                season_phase: 1.0,
                group: PlayerFieldPositionGroup::Defender,
                count: 6,
                ideal_depth: 8,
                best_here: 100,
                candidate: 105,
                clearly_better_ahead: 0,
                allowed_ahead: 2,
                band_here: 0.9,
                band_target: 0.9,
                readiness: 0.5,
                standing_ratio: 0.5,
                league_ratio: 0.7,
                need: 0.6,
                slot_room: 1.0,
            }
        }

        fn consent() -> ConsentReading {
            ConsentReading {
                plan: CareerPlanView::none(),
                band_here: 0.9,
                renown_gap: 0.0,
                renown_band: 2000.0,
                going_home: false,
                resignation: 0.0,
                familiarity: 1.0,
            }
        }

        fn money() -> MoneyReading {
            MoneyReading {
                weight: 0.2,
                carry: 0.3,
                asking: 0.0,
                max_loan_fee: 1_000_000.0,
                development: false,
            }
        }

        fn plan(arc: CareerArc, stage: PlanStage) -> CareerPlanView {
            CareerPlanView {
                arc: Some(arc),
                stage: Some(stage),
                band_floor: -0.2,
                band_floor_home: -0.2,
                band_target: 0.9,
                deadline_pressure: 0.3,
                attempts: 0,
                strength: 0.7,
            }
        }
    }

    #[test]
    fn a_first_choice_is_held_back_and_never_barred() {
        let held = ParentWillingness::of(&ParentReading {
            first_choice: true,
            starter_share: 0.9,
            ..Fx::fringe()
        });
        assert!(held.score > 0.0, "a veto is not a price");
        assert!(held.score < ParentWillingness::ENTERTAINS);
    }

    #[test]
    fn his_own_plan_opens_the_door_his_clubs_first_choice_would_close() {
        let wanted = ParentWillingness::of(&ParentReading {
            first_choice: true,
            starter_share: 0.9,
            plan_push: 0.8,
            stage: PathwayStage::Prospect,
            ..Fx::fringe()
        });
        assert!(
            wanted.score >= ParentWillingness::ENTERTAINS,
            "a first-choice full-back who has asked to go and play is loanable: {:.3}",
            wanted.score
        );
    }

    #[test]
    fn a_thirty_one_year_old_fringe_player_is_still_loanable() {
        let veteran = ParentWillingness::of(&ParentReading {
            runway: 0.25,
            ..Fx::fringe()
        });
        assert!(
            veteran.score >= ParentWillingness::ENTERTAINS,
            "age dampens the willingness, it does not bar the loan: {:.3}",
            veteran.score
        );
    }

    #[test]
    fn a_third_spell_is_priced_rather_than_blocked() {
        let third = ParentWillingness::of(&ParentReading {
            loans_used: 2,
            ..Fx::fringe()
        });
        let first = ParentWillingness::of(&Fx::fringe());
        assert!(third.score > 0.0);
        assert!(third.score < first.score);
    }

    #[test]
    fn a_club_at_its_fielding_minimum_lends_nobody() {
        let stripped = ParentWillingness::of(&ParentReading {
            group_count: 4,
            group_min_needed: 4,
            ..Fx::fringe()
        });
        assert!(stripped.score < ParentWillingness::ENTERTAINS);
    }

    #[test]
    fn an_elite_club_still_takes_a_cover_loan() {
        let elite = BorrowerAppetite::of(&BorrowerReading {
            base_by_tier: BorrowerAppetite::base_for_tier(ReputationLevel::Elite),
            need: 1.0,
            ..Fx::borrower()
        });
        assert!(
            elite.score > 0.0,
            "fewer loans is a smaller number, not a closed door"
        );
    }

    #[test]
    fn a_full_line_opens_in_proportion_to_the_upgrade() {
        let marginal = BorrowerAppetite::of(&BorrowerReading {
            count: 8,
            candidate: 102,
            best_here: 100,
            ..Fx::borrower()
        });
        let clear = BorrowerAppetite::of(&BorrowerReading {
            count: 8,
            candidate: 115,
            best_here: 100,
            ..Fx::borrower()
        });
        assert!(marginal.score > 0.0);
        assert!(clear.score > marginal.score);
    }

    #[test]
    fn a_club_two_levels_down_is_discounted_not_refused() {
        let deep = BorrowerAppetite::of(&BorrowerReading {
            standing_ratio: 0.1,
            league_ratio: 0.2,
            readiness: 1.0,
            ..Fx::borrower()
        });
        assert!(deep.score > 0.0, "a deep drop is a discount, not a veto");
        assert!(deep.score < BorrowerAppetite::of(&Fx::borrower()).score);
    }

    #[test]
    fn a_plan_written_for_the_drop_carries_the_consent() {
        let planned = PlayerConsent::of(&ConsentReading {
            plan: Fx::plan(CareerArc::ProveOnLoan, PlanStage::Asking),
            band_here: 0.9,
            ..Fx::consent()
        });
        let unplanned = PlayerConsent::of(&Fx::consent());
        assert!(planned.score > unplanned.score);
    }

    #[test]
    fn a_destination_under_his_floor_is_argued_against() {
        let too_low = PlayerConsent::of(&ConsentReading {
            plan: Fx::plan(CareerArc::ClaimMyPlace, PlanStage::Committed),
            band_here: -0.5,
            ..Fx::consent()
        });
        assert!(too_low.plan_fit < 0.0);
        assert!(too_low.score < PlayerConsent::of(&Fx::consent()).score);
    }

    #[test]
    fn a_willing_parent_is_not_blocked_by_carry() {
        // The borrower can only cover a fraction of the wage, which is
        // exactly the case the old `Untouchable` verdict killed. With
        // the parent paying, the carry it is left with is small.
        let subsidised = LoanMoney::of(&MoneyReading {
            carry: 0.2,
            weight: 1.8,
            development: true,
            ..Fx::money()
        });
        assert!(
            subsidised.affordability > 0.3,
            "a parent that wants him developed makes the deal affordable: {:.3}",
            subsidised.affordability
        );
    }

    #[test]
    fn the_parent_pays_only_for_a_loan_it_arranged() {
        assert_eq!(
            LoanMoney::parent_desire(PathwayStage::LoanOut, Some(LoanOutReason::NeedsGameTime)),
            1.0
        );
        assert_eq!(
            LoanMoney::parent_desire(PathwayStage::LoanOut, Some(LoanOutReason::Surplus)),
            0.3
        );
        assert_eq!(
            LoanMoney::parent_desire(PathwayStage::Rotation, Some(LoanOutReason::NeedsGameTime)),
            0.0
        );
    }

    #[test]
    fn the_score_is_the_product_of_the_four() {
        let parent = ParentWillingness::of(&Fx::fringe());
        let borrower = BorrowerAppetite::of(&Fx::borrower());
        let consent = PlayerConsent::of(&Fx::consent());
        let money = LoanMoney::of(&Fx::money());
        let expected = parent.score * borrower.score * consent.score * money.affordability;
        assert!(
            (LoanAgreement::score(&parent, &borrower, &consent, &money) - expected).abs() < 1e-6
        );
    }
}
