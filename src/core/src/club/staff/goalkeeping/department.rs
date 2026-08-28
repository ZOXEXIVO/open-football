//! The goalkeeping department: the coach, his standing with the manager,
//! and the review that produces the plan.
//!
//! `StaffPosition::GoalkeeperCoach` has existed since the staff model was
//! written. It had a relevance formula, an i18n key, a place on every
//! well-funded club's books — and not one line of simulation read it. This
//! is the role becoming real.
//!
//! The department does three things a real one does. It keeps a declared
//! pecking order instead of re-ranking the keepers every Saturday. It runs
//! a succession clock on the number one, because the replacement for a
//! thirty-four-year-old has to be in the building at thirty-one. And it
//! watches the academy on the first team's behalf, and says so when a boy
//! is ready — which at a real club is the single most common thing a
//! goalkeeping coach says out loud.

use chrono::NaiveDate;

use crate::club::StaffPosition;
use crate::club::staff::perception::CoachProfile;
use crate::{Staff, StaffCollection};

use super::advice::{
    KeeperAdvice, KeeperRecommendation, KeeperSuccession, KeeperTier, KeeperUrgency,
};
use super::plan::{KeeperReviewOutcome, KeeperRoomPlan};
use super::room::{KeeperAgeCurve, KeeperRoom, RoomKeeper};

/// How far the manager acts on the goalkeeping coach's word.
///
/// Nobody defers completely and nobody ignores the specialist entirely.
/// The weight is the specialist's own quality, his eye for a player, the
/// manager's willingness to be advised, and the department's track record
/// — and a club with no goalkeeping coach at all still lands on the floor
/// rather than on zero, because the manager then simply holds the opinion
/// himself.
#[derive(Debug, Clone, Copy)]
pub struct KeeperCoachAuthority {
    /// 0..1 — the scale every consumer applies to the department's word.
    pub weight: f32,
    /// 0..1 — how much specialist attention the academy keepers get. Zero
    /// without a goalkeeping coach: nobody is at the under-eighteens on
    /// Tuesday morning making a case for a sixteen-year-old.
    pub specialist_focus: f32,
}

impl KeeperCoachAuthority {
    /// A manager with no specialist to consult still has a view of his own
    /// keepers — he just has less reason to change it.
    const FLOOR: f32 = 0.20;
    const CEILING: f32 = 0.95;
    /// Best plausible `relevance_score_for(GoalkeeperCoach)`: the weights
    /// there are 8+8+6+3 against a 20-point attribute scale.
    const MAX_RELEVANCE: f32 = 500.0;

    /// Read the department's standing from the two men involved.
    pub fn read(gk_coach: Option<&Staff>, head_coach: &Staff, credibility: f32) -> Self {
        let specialist = gk_coach
            .map(|s| {
                (s.relevance_score_for(&StaffPosition::GoalkeeperCoach) as f32
                    / Self::MAX_RELEVANCE)
                    .clamp(0.0, 1.0)
            })
            .unwrap_or(0.0);

        // Whose eye is being trusted — the specialist's when there is one,
        // the manager's own otherwise.
        let judge = gk_coach.unwrap_or(head_coach);
        let knowledge = &judge.staff_attributes.knowledge;
        let judgement = ((knowledge.judging_player_ability as f32
            + knowledge.judging_player_potential as f32)
            / 40.0)
            .clamp(0.0, 1.0);

        // A manager who manages people and adapts takes advice; a stubborn
        // one keeps his own counsel whoever is telling him otherwise.
        let profile = CoachProfile::from_staff(head_coach);
        let mental = &head_coach.staff_attributes.mental;
        let openness =
            ((mental.man_management as f32 + mental.adaptability as f32) / 40.0).clamp(0.0, 1.0);
        let delegation = (openness - profile.stubbornness * 0.35).clamp(0.0, 1.0);

        let weight = (specialist * 0.30
            + judgement * 0.20
            + delegation * 0.25
            + credibility.clamp(0.0, 1.0) * 0.25)
            .clamp(Self::FLOOR, Self::CEILING);

        KeeperCoachAuthority {
            weight,
            specialist_focus: specialist,
        }
    }

    /// A department that does not exist — used where no staff are known.
    pub fn absent() -> Self {
        KeeperCoachAuthority {
            weight: Self::FLOOR,
            specialist_focus: 0.0,
        }
    }
}

/// The review that turns a keeper room into a plan.
pub struct GoalkeepingDepartment;

impl GoalkeepingDepartment {
    /// How far a challenger must be clear of the incumbent number one before
    /// the department changes the declared order. Assessed levels run 1..200,
    /// so this is a real gap and not a week's form.
    const USURP_MARGIN: i16 = 6;
    /// A number one having a genuinely bad season is easier to displace.
    const POOR_FORM_RATING: f32 = 6.25;
    const POOR_FORM_MIN_GAMES: u16 = 8;

    /// The deputy should be able to go in without the team dropping. Past
    /// this gap he cannot, and the department says so.
    const DEPUTY_GAP_MAX: i16 = 22;

    /// Oldest a keeper can be and still be groomed as the heir.
    const HEIR_MAX_AGE: u8 = 25;
    /// How much younger than the incumbent the heir has to be for grooming
    /// him to mean anything.
    const HEIR_AGE_GAP: u8 = 4;
    /// Fraction of the incumbent's current level the heir has to be judged
    /// capable of reaching before the department calls him a successor
    /// rather than a hopeful.
    const HEIR_REACH: f32 = 0.90;
    /// How far ahead the heir is projected — three seasons is the horizon a
    /// club actually plans a keeper over.
    const HEIR_HORIZON_YEARS: u8 = 3;

    /// Youngest a keeper travels with the senior squad.
    const BENCH_AGE: u8 = 17;
    /// Youngest the department will ask for a senior start.
    const START_AGE: u8 = 18;
    /// Age from which a blocked prospect should be out on loan rather than
    /// carrying the drinks.
    const LOAN_AGE: u8 = 19;

    /// How close to the senior room's last man an academy keeper must be
    /// before he is put on the pathway. The band is the specialist's
    /// contribution: without a goalkeeping coach only the obvious cases get
    /// through, because nobody is arguing the marginal ones.
    const READY_BAND_BASE: f32 = 8.0;
    const READY_BAND_SPECIALIST: f32 = 16.0;

    /// A breakout academy season: a real sample at a rating clearly above
    /// the positional neutral.
    const BREAKOUT_GAMES: u16 = 8;
    const BREAKOUT_RATING: f32 = 7.05;

    /// Senior appearances below which a pathway keeper counts as starved of
    /// first-team football.
    const PATHWAY_SEASON_TARGET: u16 = 2;

    /// At most this many keepers are carried on the senior pathway at once.
    /// A club grooms one, occasionally two; beyond that nobody plays.
    const MAX_PATHWAY: usize = 2;

    /// A nomination the manager acted on is worth this much to the
    /// department's standing; one he sat on for a full window costs it a
    /// little less, because ignoring advice is not the same as being wrong.
    const CREDIT_LISTENED: f32 = 0.06;
    const CREDIT_IGNORED: f32 = -0.05;

    /// How the department's standing has moved since the last review.
    ///
    /// This is the whole track record: did the boy he asked for actually
    /// play. A specialist whose recommendations turn into minutes gets
    /// listened to more next time, one whose requests die on the manager's
    /// desk gets listened to less — which is how professional credibility
    /// works, and it is the reason the authority weight is not simply a
    /// function of attributes.
    ///
    /// Read against the room rather than against a match feed, so it costs
    /// nothing and cannot drift out of step with what actually happened.
    pub fn credibility_delta(
        previous: &KeeperRoomPlan,
        room: &KeeperRoom,
        today: NaiveDate,
    ) -> f32 {
        let Some(nomination) = previous.nomination() else {
            return 0.0;
        };
        let Some(keeper) = room.get(nomination.player_id) else {
            return 0.0;
        };
        if keeper.senior_apps > 0 {
            return Self::CREDIT_LISTENED;
        }
        // Still nothing, and the request has stood as long as it is allowed
        // to. He was not persuasive.
        if !nomination.is_live(today) {
            return Self::CREDIT_IGNORED;
        }
        0.0
    }

    /// Review the room and produce the department's plan for it.
    ///
    /// `previous` is read for the declared order only — the point of the
    /// pecking order is that it persists.
    pub fn review(
        room: &KeeperRoom,
        previous: &KeeperRoomPlan,
        authority: KeeperCoachAuthority,
        today: NaiveDate,
    ) -> Option<KeeperReviewOutcome> {
        if room.is_empty() {
            return None;
        }

        let seniors: Vec<&RoomKeeper> = room.seniors().collect();
        let pathway: Vec<&RoomKeeper> = room.pathway().collect();

        // ── The number one ──
        let number_one = Self::declare_number_one(&seniors, &pathway, previous);

        // ── The senior order behind him ──
        // Drawn from the senior squads first; a club carrying only two
        // keepers finds its third in the academy, which is what actually
        // happens.
        let mut order: Vec<&RoomKeeper> = Vec::with_capacity(3);
        if let Some(one) = number_one {
            order.push(one);
        }
        for k in seniors.iter().chain(pathway.iter()) {
            if order.len() >= 3 {
                break;
            }
            if order.iter().any(|o| o.player_id == k.player_id) {
                continue;
            }
            order.push(k);
        }
        let deputy = order.get(1).copied();
        let third = order.get(2).copied();

        // ── The heir, and the clock behind him ──
        let heir = number_one.and_then(|one| Self::find_heir(room, one));
        let succession = Self::succession_clock(number_one, heir.is_some());

        // ── The pathway ──
        let ready_band =
            Self::READY_BAND_BASE + Self::READY_BAND_SPECIALIST * authority.specialist_focus;
        let bar = order
            .last()
            .map(|k| k.level)
            .or_else(|| room.best().map(|k| k.level))
            .unwrap_or(1);
        let mut pathway_picks: Vec<&RoomKeeper> = pathway
            .iter()
            .copied()
            .filter(|k| !order.iter().any(|o| o.player_id == k.player_id))
            .filter(|k| k.age >= Self::BENCH_AGE)
            .filter(|k| Self::is_senior_ready(k, bar, ready_band))
            .collect();
        // The heir leads the pathway even when a bigger academy keeper is
        // rated slightly higher today — he is the one the club is building.
        pathway_picks.sort_by(|a, b| {
            let key = |k: &RoomKeeper| {
                (
                    heir == Some(k.player_id),
                    (Self::pathway_priority(k) * 1000.0) as i32,
                )
            };
            key(b).cmp(&key(a))
        });
        pathway_picks.truncate(Self::MAX_PATHWAY);

        // ── Tiers ──
        let mut tiers: Vec<(u32, KeeperTier, Option<u32>)> = Vec::with_capacity(room.len());
        for (index, k) in order.iter().enumerate() {
            let tier = match index {
                0 => KeeperTier::NumberOne,
                1 => KeeperTier::Deputy,
                _ => KeeperTier::Third,
            };
            tiers.push((k.player_id, tier, None));
        }
        for k in &pathway_picks {
            tiers.push((
                k.player_id,
                KeeperTier::Pathway,
                number_one.map(|o| o.player_id),
            ));
        }
        for k in room.iter() {
            if tiers.iter().any(|(id, _, _)| *id == k.player_id) {
                continue;
            }
            // Everyone else: still developing if he is young enough for that
            // to be the honest answer, surplus to the plan if he is not.
            let tier = if k.team_type.is_youth() || k.is_pathway_age() {
                KeeperTier::Academy
            } else {
                KeeperTier::Surplus
            };
            tiers.push((k.player_id, tier, None));
        }

        // ── The nomination ──
        let nominated = Self::nominate(&pathway_picks, &order, today);

        // ── What he tells the manager ──
        let recommendations = Self::advise(
            room,
            &order,
            &pathway_picks,
            heir,
            succession,
            nominated,
            previous,
        );

        Some(KeeperReviewOutcome {
            tiers,
            number_one: number_one.map(|k| k.player_id),
            deputy: deputy.map(|k| k.player_id),
            third: third.map(|k| k.player_id),
            heir,
            succession,
            nominated,
            recommendations,
            authority: authority.weight,
        })
    }

    /// The declared number one, with the incumbent's standing respected.
    ///
    /// This is the whole difference between a keeper room and an argmax. A
    /// manager names his number one and then plays him: a challenger takes
    /// the shirt by being clearly better over a period, or by the incumbent
    /// having a bad season, not by turning up a point fresher on Saturday.
    fn declare_number_one<'k>(
        seniors: &[&'k RoomKeeper],
        pathway: &[&'k RoomKeeper],
        previous: &KeeperRoomPlan,
    ) -> Option<&'k RoomKeeper> {
        // A manager-pinned keeper is the number one by definition.
        if let Some(pinned) = seniors.iter().find(|k| k.is_pinned) {
            return Some(pinned);
        }

        let best = seniors
            .first()
            .copied()
            .or_else(|| pathway.first().copied())?;

        let Some(incumbent) = previous
            .number_one()
            .and_then(|id| seniors.iter().find(|k| k.player_id == id).copied())
        else {
            return Some(best);
        };
        if incumbent.player_id == best.player_id {
            return Some(incumbent);
        }

        // A number one having a genuinely poor season is easier to move.
        let struggling = incumbent.development_apps >= Self::POOR_FORM_MIN_GAMES
            && incumbent.form < Self::POOR_FORM_RATING;
        let margin = if struggling {
            Self::USURP_MARGIN / 2
        } else {
            Self::USURP_MARGIN
        };

        if best.level as i16 - incumbent.level as i16 >= margin {
            Some(best)
        } else {
            Some(incumbent)
        }
    }

    /// The young keeper the club is building toward the shirt: meaningfully
    /// younger than the incumbent, and judged capable of reaching his level.
    ///
    /// Two ways of clearing the bar, because a coach uses both. His assessed
    /// ceiling is the direct answer to "could he get there"; the three-year
    /// projection catches the keeper already close enough that the age curve
    /// alone will carry him. Requiring the projection on its own would name
    /// almost nobody — the observable ceiling is deliberately conservative,
    /// and a twenty-two-year-old is credited with very little growth room.
    fn find_heir(room: &KeeperRoom, number_one: &RoomKeeper) -> Option<u32> {
        let bar = number_one.level as f32 * Self::HEIR_REACH;
        room.iter()
            .filter(|k| k.player_id != number_one.player_id)
            .filter(|k| k.age <= Self::HEIR_MAX_AGE)
            .filter(|k| number_one.age.saturating_sub(k.age) >= Self::HEIR_AGE_GAP)
            .filter(|k| {
                k.ceiling as f32 >= bar || k.projected_level(Self::HEIR_HORIZON_YEARS) >= bar
            })
            .max_by(|a, b| {
                a.projected_level(Self::HEIR_HORIZON_YEARS)
                    .partial_cmp(&b.projected_level(Self::HEIR_HORIZON_YEARS))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|k| k.player_id)
    }

    /// How pressing the succession is. Age against the keeper curve, one
    /// step worse when there is nobody in the building to take over.
    fn succession_clock(number_one: Option<&RoomKeeper>, has_heir: bool) -> KeeperSuccession {
        let Some(one) = number_one else {
            return KeeperSuccession::Critical;
        };
        let base = if one.age >= KeeperAgeCurve::LATE_CAREER {
            KeeperSuccession::Critical
        } else if one.age > KeeperAgeCurve::PEAK_UNTIL {
            KeeperSuccession::Pressing
        } else if one.age >= KeeperAgeCurve::SUCCESSION_FROM {
            KeeperSuccession::Watch
        } else {
            KeeperSuccession::Settled
        };
        if has_heir { base } else { base.escalated() }
    }

    /// Whether an academy keeper is close enough to the senior room to train
    /// and travel with it. Two ways in, as in real life: he holds his own
    /// against the club's last senior keeper, or he is tearing up his own
    /// age group with a ceiling well past the bar.
    fn is_senior_ready(keeper: &RoomKeeper, bar_level: u8, band: f32) -> bool {
        if keeper.level as f32 + band >= bar_level as f32 {
            return true;
        }
        keeper.development_apps >= Self::BREAKOUT_GAMES
            && keeper.form >= Self::BREAKOUT_RATING
            && keeper.ceiling as f32 >= bar_level as f32 + band
    }

    /// How strongly the department wants this academy keeper in the senior
    /// picture — where he is going, not only where he is.
    fn pathway_priority(keeper: &RoomKeeper) -> f32 {
        let ceiling_pull = (keeper.ceiling as f32 - keeper.level as f32).clamp(0.0, 60.0) / 60.0;
        let projection = keeper.projected_level(Self::HEIR_HORIZON_YEARS) / 200.0;
        let form_pull = ((keeper.form - 6.6) / 1.4).clamp(-0.5, 1.0);
        let playing = (keeper.development_apps as f32 / 15.0).clamp(0.0, 1.0);
        projection * 0.45 + ceiling_pull * 0.25 + form_pull * 0.20 + playing * 0.10
    }

    /// The keeper the department asks the manager to start.
    ///
    /// The whole point of this request is that it is made for a keeper who
    /// is NOT the best keeper at the club — a boy the department believes
    /// in, who will not become a first-team keeper by watching. So the bar
    /// is age and starvation, not ability: he must be old enough for a
    /// senior pitch, and he must not already be getting the games.
    fn nominate(pathway: &[&RoomKeeper], order: &[&RoomKeeper], _today: NaiveDate) -> Option<u32> {
        // Never at the cost of a keeper room too thin to field a team.
        if order.len() < 2 {
            return None;
        }
        pathway
            .iter()
            .filter(|k| k.age >= Self::START_AGE)
            .filter(|k| !k.is_injured)
            .filter(|k| k.senior_apps < Self::PATHWAY_SEASON_TARGET)
            .max_by(|a, b| {
                Self::pathway_priority(a)
                    .partial_cmp(&Self::pathway_priority(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|k| k.player_id)
    }

    /// Everything the goalkeeping coach has to say this month.
    fn advise(
        room: &KeeperRoom,
        order: &[&RoomKeeper],
        pathway: &[&RoomKeeper],
        heir: Option<u32>,
        succession: KeeperSuccession,
        nominated: Option<u32>,
        _previous: &KeeperRoomPlan,
    ) -> Vec<KeeperRecommendation> {
        let mut out: Vec<KeeperRecommendation> = Vec::new();

        let number_one = order.first().copied();
        let deputy = order.get(1).copied();
        let third = order.get(2).copied();

        // The deputy owns the cup. Said every season, because it is a
        // decision taken before the season and not a weekly preference.
        if let Some(d) = deputy {
            out.push(KeeperRecommendation::about(
                KeeperAdvice::HandHimTheCup,
                d.player_id,
                KeeperUrgency::Noted,
            ));
        }

        // Is there actually a deputy, or only a body?
        match (number_one, deputy) {
            (Some(one), Some(d)) if one.level as i16 - d.level as i16 > Self::DEPUTY_GAP_MAX => {
                out.push(KeeperRecommendation::club(
                    KeeperAdvice::SignACredibleDeputy,
                    KeeperUrgency::Pressing,
                ));
            }
            (Some(_), None) => {
                out.push(KeeperRecommendation::club(
                    KeeperAdvice::SignACredibleDeputy,
                    KeeperUrgency::Urgent,
                ));
            }
            _ => {}
        }

        // The third keeper. At a real club he is either the old head who
        // runs the goalkeeping group or a homegrown boy learning it — and
        // a twenty-seven-year-old who will never play is neither.
        match third {
            Some(t) if t.age >= KeeperAgeCurve::PEAK_UNTIL => {
                out.push(KeeperRecommendation::about(
                    KeeperAdvice::KeepHimAsTheSeniorVoice,
                    t.player_id,
                    KeeperUrgency::Noted,
                ));
            }
            Some(t) if t.age > RoomKeeper::DEVELOPMENT_AGE && t.senior_apps == 0 => {
                out.push(KeeperRecommendation::about(
                    KeeperAdvice::TimeToMoveHimOn,
                    t.player_id,
                    KeeperUrgency::Noted,
                ));
                if !room.has_senior_aged(KeeperAgeCurve::PEAK_UNTIL, 42) {
                    out.push(KeeperRecommendation::club(
                        KeeperAdvice::SignAnExperiencedThird,
                        KeeperUrgency::Noted,
                    ));
                }
            }
            _ => {}
        }

        // The succession.
        if succession >= KeeperSuccession::Watch {
            if let Some(one) = number_one {
                out.push(KeeperRecommendation::about(
                    KeeperAdvice::OpenTheSuccession,
                    one.player_id,
                    match succession {
                        KeeperSuccession::Critical => KeeperUrgency::Urgent,
                        KeeperSuccession::Pressing => KeeperUrgency::Pressing,
                        _ => KeeperUrgency::Noted,
                    },
                ));
            }
        }
        if let Some(heir_id) = heir {
            // The heir's minutes are the succession. Saying it is the job.
            out.push(KeeperRecommendation::about(
                KeeperAdvice::MakeHimNumberOne,
                heir_id,
                if succession >= KeeperSuccession::Pressing {
                    KeeperUrgency::Pressing
                } else {
                    KeeperUrgency::Noted
                },
            ));
        } else if succession >= KeeperSuccession::Pressing {
            out.push(KeeperRecommendation::club(
                KeeperAdvice::SignAKeeperForTheFuture,
                KeeperUrgency::Urgent,
            ));
        }

        // Nobody coming at all — said even when the number one is young,
        // because a room with no age below him is a room that will have to
        // buy one in a hurry.
        let has_future = room
            .iter()
            .any(|k| k.is_pathway_age() && number_one.is_some_and(|o| k.ceiling >= o.level));
        if !has_future
            && !out
                .iter()
                .any(|r| r.advice == KeeperAdvice::SignAKeeperForTheFuture)
        {
            out.push(KeeperRecommendation::club(
                KeeperAdvice::SignAKeeperForTheFuture,
                KeeperUrgency::Noted,
            ));
        }

        // The academy.
        for k in pathway {
            out.push(KeeperRecommendation::about(
                KeeperAdvice::NameHimOnTheBench,
                k.player_id,
                KeeperUrgency::Noted,
            ));
        }
        if let Some(id) = nominated {
            out.push(KeeperRecommendation::about(
                KeeperAdvice::GiveHimASeniorStart,
                id,
                KeeperUrgency::Pressing,
            ));
        }

        // Blocked prospects. A keeper of developing age who is behind two or
        // more men here and is getting no senior football needs a season
        // somewhere he will play — the standard route in real football, and
        // the one the club had no way of choosing deliberately.
        //
        // Deliberately not limited to the academy squads: the case that
        // matters most is the twenty-one-year-old already rostered with the
        // reserves, who is not positional surplus, is not a youth-team
        // prospect, plays enough reserve football never to look idle, and is
        // nonetheless third or fourth in a queue of one shirt.
        let blocked_by = order.len();
        for k in room.iter() {
            if order.iter().any(|o| o.player_id == k.player_id) {
                continue;
            }
            if k.age < Self::LOAN_AGE || k.age > RoomKeeper::DEVELOPMENT_AGE {
                continue;
            }
            if k.senior_apps >= Self::PATHWAY_SEASON_TARGET || nominated == Some(k.player_id) {
                continue;
            }
            if blocked_by < 2 {
                continue;
            }
            // A keeper already on the senior pathway is getting his run here;
            // he only goes out when the queue in front of him is a full room.
            let on_pathway = pathway.iter().any(|p| p.player_id == k.player_id);
            if on_pathway && blocked_by < 3 {
                continue;
            }
            out.push(KeeperRecommendation::about(
                KeeperAdvice::LoanHimOutForMinutes,
                k.player_id,
                if k.age >= 21 {
                    KeeperUrgency::Pressing
                } else {
                    KeeperUrgency::Noted
                },
            ));
        }
        // The youngest are simply left alone.
        for k in room.pathway() {
            if k.age >= Self::LOAN_AGE {
                continue;
            }
            if out.iter().any(|r| r.player_id == Some(k.player_id)) {
                continue;
            }
            out.push(KeeperRecommendation::about(
                KeeperAdvice::KeepHimDeveloping,
                k.player_id,
                KeeperUrgency::Noted,
            ));
        }

        out
    }
}

/// Locating the department inside a club's backroom.
pub struct GoalkeepingStaff;

impl GoalkeepingStaff {
    /// The specialist, when the club employs one. Deliberately strict: a
    /// club with no goalkeeping coach genuinely has no goalkeeping
    /// department, and that difference between a well-funded club and a
    /// small one is worth keeping.
    pub fn specialist(staffs: &StaffCollection) -> Option<&Staff> {
        staffs.find_by_any_position(&[StaffPosition::GoalkeeperCoach])
    }

    /// Whoever holds the department's plan. The specialist when there is
    /// one, otherwise the manager — somebody always has a view of who the
    /// number one is, even if nobody is employed to.
    pub fn lead(staffs: &StaffCollection) -> Option<&Staff> {
        Self::specialist(staffs).or_else(|| staffs.social_head_coach())
    }
}
