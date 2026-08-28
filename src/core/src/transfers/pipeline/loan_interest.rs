//! Who wants a loan player, and where his parent club sends him.
//!
//! The loan pipeline used to settle both questions with a bare argmax over a
//! single near-constant integer. A borrowing club took the highest
//! `current_ability` listing that cleared its gates; a parent placed a
//! prospect at the highest `reputation.world` club that cleared the same
//! gates. Neither number moves much across a season, both are read off the
//! ground truth rather than off anybody's *opinion* of it, and both scans walk
//! `country.clubs` in registration order — so the argmax resolved to the same
//! pair every cycle, a failed bid re-ran the identical decision the following
//! Monday, and the club sitting earliest in the registration order got first
//! refusal on everything. The market looked like a fixed permutation because
//! it was one.
//!
//! Nothing here changes what a loan is *allowed* to be. Every eligibility gate
//! — depth, minutes, reputation drop, division floor, appetite, plausibility,
//! affordability — runs first and runs unchanged, on the same true values it
//! always read. This module only decides who wins among the options those
//! gates already approved, and it decides with three things the old argmax had
//! none of:
//!
//! * **Opinion.** [`ClubOpinion`] gives every club a standing, private read of
//!   every player, sharp or blurry according to the eyes it employs. Two clubs
//!   looking at the same prospect no longer agree to the decimal, so they no
//!   longer converge on the same name. The read is *stable* — hashed from the
//!   pair, not rolled per tick — because a scouting department that rated a
//!   player highly in August still rates him highly in September. A random
//!   re-roll each week would be a lottery, not a difference of opinion.
//! * **Taste.** [`BorrowerTaste`] and [`DestinationAppeal`] score a candidate
//!   against what this particular club actually wants — its philosophy, its
//!   board's age and youth policy, its financial stance, how thin the position
//!   really is, how much of the fee it can comfortably carry, who it has dealt
//!   with before. That club-policy layer already existed; the loan market
//!   simply never read it.
//! * **A draw, not a maximum.** [`InterestDraw`] picks proportionally to
//!   interest instead of taking the top row. The strongest option still wins
//!   most of the time — that is what makes it a preference — but the second
//!   and third get a real share, so identical world states stop producing
//!   identical assignments.

use crate::club::board::{FinancialStance, SigningPreference, VisionYouthFocus};
use crate::utils::random::engine::RandomEngine;
use crate::utils::{FloatUtils, IntegerUtils};
use crate::{Club, ClubPhilosophy, PlayerFieldPositionGroup};
use chrono::NaiveDate;

/// A club's standing private read of a player, and how sharp it is.
///
/// Real recruitment departments disagree. One club's chief scout is convinced
/// by a player another club's has written off, and that disagreement persists
/// — it is a judgement, not a coin flip. So the offset is derived by hashing
/// the (club, player) pair rather than drawn from the RNG: the same club reads
/// the same player the same way every week, while two different clubs read him
/// differently. The spread of that disagreement scales with how good the eyes
/// are — a department with a strong `judging_player_ability` sits close to the
/// truth, one running on a part-time manager is off by a wide margin.
///
/// This is the read the loan market *ranks* on. The eligibility gates keep
/// reading true ability, because whether a move actually works out is a fact
/// about the world, not about who is looking at it.
pub(in crate::transfers::pipeline) struct ClubOpinion {
    /// Signed ability offset in CA points, already scaled by judgement.
    offset: f32,
}

impl ClubOpinion {
    /// Widest misjudgement, in CA points, carried by a club with no scouting
    /// to speak of. Scales down continuously toward [`Self::SHARPEST_SPREAD`]
    /// as the department's judgement improves — no tier line.
    const BLURRIEST_SPREAD: f32 = 14.0;
    /// Residual spread left even to the best recruitment department in the
    /// world. Never zero: nobody reads a loan prospect exactly.
    const SHARPEST_SPREAD: f32 = 3.0;

    /// How this club reads `player_id`. `judging` is the department's best
    /// `judging_player_ability` (1..20) — the scout's, or the manager's at a
    /// club that employs none.
    pub(in crate::transfers::pipeline) fn of(club_id: u32, player_id: u32, judging: u8) -> Self {
        let sharpness = (judging.clamp(1, 20) as f32 - 1.0) / 19.0;
        let spread =
            Self::BLURRIEST_SPREAD - (Self::BLURRIEST_SPREAD - Self::SHARPEST_SPREAD) * sharpness;
        ClubOpinion {
            offset: Self::unit_offset(club_id, player_id) * spread,
        }
    }

    /// The club's believed ability for a player whose true ability is `truth`.
    pub(in crate::transfers::pipeline) fn believed_ability(&self, truth: u8) -> f32 {
        (truth as f32 + self.offset).clamp(1.0, 200.0)
    }

    /// Stable [-1.0, 1.0] residual for one pair. Mixes the pinned sim seed so
    /// a seeded run reproduces its opinion field exactly, and two worlds run
    /// under different seeds disagree about who rates whom.
    fn unit_offset(club_id: u32, player_id: u32) -> f32 {
        let mut h = ((club_id as u64) << 32) | player_id as u64;
        h ^= RandomEngine::current_seed().wrapping_mul(0x9E37_79B9_7F4A_7C15);
        // splitmix64 finalizer — cheap, well-distributed, no allocation.
        h = h.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = h;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // Top 24 bits → [0, 1) → [-1, 1].
        ((z >> 40) as f32 / 16_777_216.0) * 2.0 - 1.0
    }
}

/// What a borrowing club is actually in the market for.
///
/// Built once per club per scan, then asked about each candidate the gates
/// have already cleared. Every input already existed on the club — the loan
/// market ranked on ability and read none of it, which is why a
/// youth-development side and a survival-minded veteran side chased the same
/// name.
pub(in crate::transfers::pipeline) struct BorrowerTaste {
    club_id: u32,
    /// Best `judging_player_ability` at the club — drives opinion spread.
    judging: u8,
    /// Squad average CA, the yardstick a candidate is read against.
    squad_average: u8,
    philosophy: ClubPhilosophy,
    youth_focus: VisionYouthFocus,
    financial_stance: FinancialStance,
    signing_preference: SigningPreference,
    /// Loan-fee ceiling the scan computed. Used for comfort, not as a gate —
    /// the gate already ran.
    max_loan_fee: f64,
}

impl BorrowerTaste {
    /// Interest below this reads as "not really wanted": the option is dropped
    /// from the draw rather than given a vanishing share of it.
    const INDIFFERENCE_FLOOR: f32 = 0.05;

    pub(in crate::transfers::pipeline) fn of(
        club: &Club,
        judging: u8,
        squad_average: u8,
        max_loan_fee: f64,
    ) -> Self {
        BorrowerTaste {
            club_id: club.id,
            judging,
            squad_average,
            philosophy: club.philosophy.clone(),
            youth_focus: club.board.vision.youth_focus,
            financial_stance: club.board.vision.financial_stance,
            signing_preference: club.board.vision.signing_preference,
            max_loan_fee,
        }
    }

    /// How much this club wants this candidate, on an open positive scale
    /// where 1.0 is "a good ordinary fit". `None` when the club is indifferent
    /// enough that entering him in the draw would be noise.
    ///
    /// The candidate has already cleared every eligibility gate; this is
    /// preference only.
    pub(in crate::transfers::pipeline) fn interest_in(
        &self,
        candidate: &LoanCandidateProfile,
    ) -> Option<f32> {
        let believed = ClubOpinion::of(self.club_id, candidate.player_id, self.judging)
            .believed_ability(candidate.true_ability);

        // Quality, read as a gain over what the club already fields. A
        // continuous curve, not a rank: +15 CA over the squad average is
        // roughly twice as interesting as parity, and a candidate below the
        // average still counts for something — he may be a prospect, and the
        // minutes gate has already said he would play.
        let gain = (believed - self.squad_average as f32) / 15.0;
        let quality = (1.0 + gain).clamp(0.25, 2.5);

        let interest = quality
            * self.age_fit(candidate.age, candidate.is_development)
            * self.profile_fit(candidate.is_development)
            * self.fee_comfort(candidate.fee)
            * (0.55 + candidate.group_thinness * 0.9)
            * if candidate.answers_open_request {
                1.35
            } else {
                1.0
            };

        (interest > Self::INDIFFERENCE_FLOOR).then_some(interest)
    }

    /// Board age policy as a preference curve rather than a band. A
    /// youth-focused side is drawn to a 19-year-old and lukewarm about a
    /// 27-year-old; a side told to sign experience reads it the other way.
    /// Nobody is excluded — the age gates already ran.
    fn age_fit(&self, age: u8, is_development: bool) -> f32 {
        let youthfulness = ((26.0 - age as f32) / 8.0).clamp(-1.0, 1.0);
        let tilt = match (&self.philosophy, self.youth_focus) {
            (ClubPhilosophy::DevelopAndSell, _) => 0.45,
            (_, VisionYouthFocus::DevelopYouth) => 0.35,
            (_, VisionYouthFocus::SignExperienced) => -0.30,
            (ClubPhilosophy::SignToCompete, _) => -0.25,
            _ => 0.0,
        };
        // A development loan is a youth-policy decision whichever way the board
        // leans, so the tilt bites harder on one.
        let weight = if is_development { 1.0 } else { 0.6 };
        (1.0 + youthfulness * tilt * weight).clamp(0.4, 1.6)
    }

    /// How well the *kind* of loan matches how the club does business.
    fn profile_fit(&self, is_development: bool) -> f32 {
        let philosophy = match (&self.philosophy, is_development) {
            (ClubPhilosophy::LoanFocused, _) => 1.30,
            (ClubPhilosophy::DevelopAndSell, true) => 1.20,
            (ClubPhilosophy::DevelopAndSell, false) => 0.90,
            (ClubPhilosophy::SignToCompete, true) => 0.70,
            (ClubPhilosophy::SignToCompete, false) => 0.95,
            (ClubPhilosophy::Balanced, _) => 1.0,
        };
        // A value-hunting or domestically-minded board is more comfortable in
        // the loan market than one shopping for marquee names.
        let preference = match self.signing_preference {
            SigningPreference::ValueHunter => 1.15,
            SigningPreference::Domestic => 1.05,
            SigningPreference::Marquee => 0.85,
            SigningPreference::Anyone => 1.0,
        };
        philosophy * preference
    }

    /// Comfort with the fee — an austere board dislikes paying for a loan even
    /// when it can. Continuous in the fraction of the ceiling consumed, so a
    /// free development loan is maximally comfortable for everyone.
    fn fee_comfort(&self, fee: f64) -> f32 {
        if self.max_loan_fee <= 0.0 {
            return 1.0;
        }
        let load = (fee / self.max_loan_fee).clamp(0.0, 1.0) as f32;
        let aversion = match self.financial_stance {
            FinancialStance::Austerity => 0.55,
            FinancialStance::Conservative => 0.35,
            FinancialStance::Balanced => 0.20,
            FinancialStance::Ambitious => 0.08,
        };
        1.0 - load * aversion
    }
}

/// One already-eligible loan candidate, as the borrowing club sees him.
/// Bundled so the scan's several branches ask the same question the same way.
pub(in crate::transfers::pipeline) struct LoanCandidateProfile {
    pub player_id: u32,
    /// True current ability. The *gates* read this; [`BorrowerTaste`] converts
    /// it into a believed figure before ranking on it.
    pub true_ability: u8,
    pub age: u8,
    pub is_development: bool,
    /// Loan fee the borrower would actually pay.
    pub fee: f64,
    /// 0..1 scarcity at the candidate's position group — see [`GroupPressure`].
    pub group_thinness: f32,
    /// The candidate answers a transfer request the club has open.
    pub answers_open_request: bool,
}

/// How attractive one destination looks to the parent club placing a loanee.
///
/// The push used to rank destinations on `reputation.world` alone, which is
/// both the most static number available and, on its own, the wrong one: a
/// parent sending a teenager out picks the place he will *develop*, and
/// reputation is only part of that. Standing still leads — it is why a club is
/// the most attractive name on the list — but it now competes with how clearly
/// he would start, the standard of football he would be playing, the coaching
/// and training he would get, and how many of the parent's players are already
/// parked there.
pub(in crate::transfers::pipeline) struct DestinationAppeal {
    /// Borrower main-team world reputation.
    pub borrower_rep: u16,
    /// Reputation of the competition the borrower plays in.
    pub borrower_league_rep: u16,
    /// Reputation of the competition the loanee is leaving.
    pub parent_league_rep: u16,
    /// 0..1 — how clearly the loanee would be first choice here.
    pub minutes_headroom: f32,
    /// Borrower training-facility rating, 1..20.
    pub training_rating: u8,
    /// Loanees this parent has already placed at this borrower recently.
    pub existing_placements: u8,
    /// Development loan (the parent is buying him minutes) vs cover loan.
    pub is_development: bool,
}

impl DestinationAppeal {
    /// World reputation at which the standing term saturates. Above this the
    /// curve flattens, so the two biggest names in a country are near enough
    /// interchangeable to the parent — which is true, and is what stops one
    /// giant hoovering up every loanee in the division.
    const REPUTATION_SCALE: f32 = 6_000.0;

    /// Score on an open positive scale, 1.0 being an unremarkable destination.
    pub(in crate::transfers::pipeline) fn score(&self) -> f32 {
        // Standing, on a saturating curve rather than a raw comparison. The old
        // `max_by_key` treated a one-point reputation edge as decisive; here two
        // comparable clubs come out comparable and the rest of the model gets to
        // break the tie.
        let standing = 1.0 - (-(self.borrower_rep as f32) / Self::REPUTATION_SCALE).exp();

        // Standard of football on offer, against what he is leaving. A parent
        // placing a prospect cares far more about this than one parking a
        // surplus earner, so the weight follows the loan's purpose.
        let step = if self.parent_league_rep > 0 {
            (self.borrower_league_rep as f32 / self.parent_league_rep as f32).clamp(0.0, 1.5)
        } else {
            0.75
        };
        let stage_weight = if self.is_development { 0.55 } else { 0.30 };
        let stage = 1.0 - stage_weight + step * stage_weight;

        // Will he actually play, and how clearly? The gate answered yes or no;
        // this reads the margin, so "walks into the side" beats "scrapes in".
        let minutes_weight = if self.is_development { 0.70 } else { 0.35 };
        let minutes = 1.0 - minutes_weight + self.minutes_headroom * minutes_weight;

        // Who will coach him. Only a development loan cares.
        let coaching = if self.is_development {
            0.80 + (self.training_rating.clamp(1, 20) as f32 / 20.0) * 0.40
        } else {
            1.0
        };

        // Don't turn one borrower into a farm team. Each loanee already there
        // costs the next one about a third of the parent's enthusiasm — the
        // per-tick `claimed_loans` guard only ever saw a single pass.
        let crowding = 1.0 / (1.0 + self.existing_placements as f32 * 0.45);

        (0.35 + standing) * stage * minutes * coaching * crowding
    }
}

/// Picks one option in proportion to interest.
///
/// An argmax over a stable score is a fixed assignment; a uniform pick is a
/// lottery. A weighted draw is a preference — the club that wants a player
/// most usually gets him, and the one who wants him nearly as much is not shut
/// out forever.
pub(in crate::transfers::pipeline) struct InterestDraw;

impl InterestDraw {
    /// How decisively interest converts into odds. Weight is
    /// `score^SHARPNESS`, so at 3.0 an option scoring 25% higher than its
    /// rival is picked about twice as often — a clear favourite, not a
    /// foregone conclusion.
    pub(in crate::transfers::pipeline) const SHARPNESS: f32 = 3.0;

    /// Draw one `(id, interest)` pair. `None` for an empty slate.
    pub(in crate::transfers::pipeline) fn pick(options: &[(u32, f32)]) -> Option<u32> {
        Self::pick_index(options).map(|i| options[i].0)
    }

    /// Draw one option, returning its index — for callers whose payload is
    /// richer than an id.
    pub(in crate::transfers::pipeline) fn pick_index(options: &[(u32, f32)]) -> Option<usize> {
        match options.len() {
            0 => return None,
            1 => return Some(0),
            _ => {}
        }

        let weights: Vec<f32> = options
            .iter()
            .map(|(_, score)| score.max(0.0).powf(Self::SHARPNESS))
            .collect();
        let total: f32 = weights.iter().sum();
        if total <= 0.0 || !total.is_finite() {
            return Some(0);
        }

        let mut roll = FloatUtils::random(0.0, total);
        for (i, w) in weights.iter().enumerate() {
            roll -= w;
            if roll <= 0.0 {
                return Some(i);
            }
        }
        Some(weights.len() - 1)
    }

    /// Draw up to `count` distinct options, in draw order. Successive picks
    /// re-normalise over what is left, so this is a weighted sample without
    /// replacement rather than "take the top N".
    ///
    /// Weights are raised to [`Self::SHARPNESS`] once and then carried, not
    /// recomputed per pick: the opportunistic scan draws a handful of names
    /// out of every loan listing in the country, and re-running `powf` over
    /// the whole slate on each draw is the one part of this that would show
    /// up in a world tick.
    pub(in crate::transfers::pipeline) fn pick_several(
        options: &[(u32, f32)],
        count: usize,
    ) -> Vec<u32> {
        let mut ids: Vec<u32> = options.iter().map(|(id, _)| *id).collect();
        let mut weights: Vec<f32> = options
            .iter()
            .map(|(_, score)| score.max(0.0).powf(Self::SHARPNESS))
            .collect();

        let mut taken = Vec::with_capacity(count.min(ids.len()));
        while taken.len() < count && !ids.is_empty() {
            // Summed fresh each round rather than decremented, so a long draw
            // can't accumulate float drift into a negative total.
            let total: f32 = weights.iter().sum();
            let chosen = if total > 0.0 && total.is_finite() {
                let mut roll = FloatUtils::random(0.0, total);
                let mut pick = ids.len() - 1;
                for (i, w) in weights.iter().enumerate() {
                    roll -= w;
                    if roll <= 0.0 {
                        pick = i;
                        break;
                    }
                }
                pick
            } else {
                0
            };
            taken.push(ids.swap_remove(chosen));
            weights.swap_remove(chosen);
        }
        taken
    }

    /// Visit order for a scan that hands out first refusal.
    ///
    /// Walking `country.clubs` in registration order gave the club with the
    /// lowest id first pick of the whole market every single tick, because the
    /// dedup guard is "has anyone claimed him yet". The order is now rotated by
    /// an offset that moves each tick, so the privilege travels around the
    /// division instead of sitting on one club forever. Rotation rather than a
    /// shuffle: it is cheap, it allocates only the index vector, and every club
    /// is guaranteed to lead the queue eventually.
    pub(in crate::transfers::pipeline) fn visit_order(len: usize) -> Vec<usize> {
        if len <= 1 {
            return (0..len).collect();
        }
        let start = IntegerUtils::random(0, len as i32).clamp(0, len as i32 - 1) as usize;
        (0..len).map(|i| (start + i) % len).collect()
    }
}

/// A club's memory of loan approaches that came to nothing, and of where it
/// has already sent players.
///
/// Neither existed. A rejected loan bid left no trace at all, so the next
/// Monday's scan re-ran the same decision against the same market and made the
/// same approach — the "keeps chasing the same player" half of the symptom,
/// entirely separate from the argmax. Real recruitment moves on for a while
/// after a knock-back, and moves on for longer the more often it is knocked
/// back.
pub(in crate::transfers::pipeline) struct LoanApproachMemory;

impl LoanApproachMemory {
    /// Days a club leaves a target alone after moving for him once.
    const FIRST_REBUFF_DAYS: i64 = 24;
    /// Each further approach for the same player adds this much again, so a
    /// club that keeps being told no gives up on him for the window.
    const REPEAT_REBUFF_DAYS: i64 = 30;
    /// Ceiling, so a target is never blacklisted permanently.
    const MAX_REBUFF_DAYS: i64 = 150;
    /// How long a standoff row survives past its own deadline, carrying the
    /// approach tally so the next look escalates rather than restarting at the
    /// first-rebuff length. Past this the club has simply forgotten the
    /// pursuit and may come at him fresh.
    pub(in crate::transfers::pipeline) const FORGET_DAYS: i64 = 120;
    /// Placements older than this stop counting toward destination crowding —
    /// last season's loanee is not this season's blocked pathway.
    pub(in crate::transfers::pipeline) const PLACEMENT_MEMORY_DAYS: i64 = 300;

    /// How long to stand off, given how many times this club has now gone in
    /// for this player. Jittered so two clubs rebuffed on the same day don't
    /// come back on the same day.
    pub(in crate::transfers::pipeline) fn rebuff_days(approaches: u8) -> i64 {
        let repeats = approaches.saturating_sub(1) as i64;
        let base = Self::FIRST_REBUFF_DAYS + repeats * Self::REPEAT_REBUFF_DAYS;
        (base + IntegerUtils::random(-4, 8) as i64).clamp(7, Self::MAX_REBUFF_DAYS)
    }

    /// Placements at `borrower_id` recent enough to still crowd the door.
    pub(in crate::transfers::pipeline) fn crowding_at(
        placements: &[(u32, NaiveDate)],
        borrower_id: u32,
        date: NaiveDate,
    ) -> u8 {
        placements
            .iter()
            .filter(|(club, when)| {
                *club == borrower_id && (date - *when).num_days() <= Self::PLACEMENT_MEMORY_DAYS
            })
            .count()
            .min(u8::MAX as usize) as u8
    }
}

/// Position-group scarcity as a continuous 0..1 pressure, alongside the
/// pass/fail depth gate. Feeds [`BorrowerTaste::interest_in`] so a club one
/// injury from trouble at centre-half wants a centre-half more than a club
/// merely tidying up its bench — a distinction a flat ability ranking cannot
/// make.
pub(in crate::transfers::pipeline) struct GroupPressure;

impl GroupPressure {
    pub(in crate::transfers::pipeline) fn thinness(
        headcount: usize,
        group: PlayerFieldPositionGroup,
    ) -> f32 {
        let ideal = group.ideal_squad_depth().max(1);
        (1.0 - headcount as f32 / ideal as f32).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Taste fixtures. `BorrowerTaste::of` wants a whole `Club`; the model it
    /// distils is these six fields, so the tests build it directly.
    struct TasteFixtures;

    impl TasteFixtures {
        fn taste(
            club_id: u32,
            philosophy: ClubPhilosophy,
            youth_focus: VisionYouthFocus,
            financial_stance: FinancialStance,
        ) -> BorrowerTaste {
            BorrowerTaste {
                club_id,
                judging: 12,
                squad_average: 100,
                philosophy,
                youth_focus,
                financial_stance,
                signing_preference: SigningPreference::Anyone,
                max_loan_fee: 1_000_000.0,
            }
        }

        fn balanced(club_id: u32) -> BorrowerTaste {
            Self::taste(
                club_id,
                ClubPhilosophy::Balanced,
                VisionYouthFocus::Balanced,
                FinancialStance::Balanced,
            )
        }

        fn candidate(player_id: u32, age: u8, ability: u8) -> LoanCandidateProfile {
            LoanCandidateProfile {
                player_id,
                true_ability: ability,
                age,
                is_development: age <= 23,
                fee: 0.0,
                group_thinness: 0.5,
                answers_open_request: false,
            }
        }
    }

    /// Two boards with opposite youth policies must not want the same player.
    /// The loan market read none of this and so ranked every club's market
    /// identically.
    #[test]
    fn youth_and_experience_boards_want_different_players() {
        let developer = TasteFixtures::taste(
            1,
            ClubPhilosophy::DevelopAndSell,
            VisionYouthFocus::DevelopYouth,
            FinancialStance::Balanced,
        );
        let win_now = TasteFixtures::taste(
            1,
            ClubPhilosophy::SignToCompete,
            VisionYouthFocus::SignExperienced,
            FinancialStance::Balanced,
        );
        let kid = TasteFixtures::candidate(10, 19, 100);
        let senior = TasteFixtures::candidate(11, 29, 100);

        let dev_kid = developer.interest_in(&kid).unwrap();
        let dev_senior = developer.interest_in(&senior).unwrap();
        let now_kid = win_now.interest_in(&kid).unwrap();
        let now_senior = win_now.interest_in(&senior).unwrap();

        assert!(
            dev_kid > dev_senior,
            "a develop-and-sell side should prefer the teenager: {dev_kid} vs {dev_senior}"
        );
        assert!(
            now_senior > now_kid,
            "a sign-to-compete side should prefer the senior: {now_senior} vs {now_kid}"
        );
    }

    /// A club genuinely short at a position wants a body there more than a club
    /// merely tidying its bench — the flat ability ranking could not tell them
    /// apart.
    #[test]
    fn scarcity_raises_interest() {
        let taste = TasteFixtures::balanced(1);
        let mut thin = TasteFixtures::candidate(10, 24, 100);
        thin.group_thinness = 1.0;
        let mut stocked = TasteFixtures::candidate(10, 24, 100);
        stocked.group_thinness = 0.0;
        assert!(taste.interest_in(&thin).unwrap() > taste.interest_in(&stocked).unwrap());
    }

    /// An austere board dislikes paying for a loan even when it can afford to;
    /// an ambitious one barely notices.
    #[test]
    fn fee_bites_harder_on_an_austere_board() {
        let austere = TasteFixtures::taste(
            1,
            ClubPhilosophy::Balanced,
            VisionYouthFocus::Balanced,
            FinancialStance::Austerity,
        );
        let ambitious = TasteFixtures::taste(
            1,
            ClubPhilosophy::Balanced,
            VisionYouthFocus::Balanced,
            FinancialStance::Ambitious,
        );
        let mut pricey = TasteFixtures::candidate(10, 24, 100);
        pricey.fee = 1_000_000.0;
        let free = TasteFixtures::candidate(10, 24, 100);

        let austere_drop =
            austere.interest_in(&free).unwrap() - austere.interest_in(&pricey).unwrap();
        let ambitious_drop =
            ambitious.interest_in(&free).unwrap() - ambitious.interest_in(&pricey).unwrap();
        assert!(austere_drop > ambitious_drop);
    }

    /// Quality still leads: at equal everything else, the better player is
    /// wanted more. The draw is a preference, not a coin flip.
    #[test]
    fn quality_still_leads() {
        let taste = TasteFixtures::balanced(1);
        let good = TasteFixtures::candidate(10, 24, 130);
        let modest = TasteFixtures::candidate(10, 24, 90);
        assert!(taste.interest_in(&good).unwrap() > taste.interest_in(&modest).unwrap());
    }

    /// The whole point: identical clubs facing an identical market must not
    /// produce an identical ranking, because their departments read the players
    /// differently.
    #[test]
    fn identical_clubs_rank_the_same_market_differently() {
        let market: Vec<LoanCandidateProfile> = (0..12)
            .map(|i| TasteFixtures::candidate(100 + i, 22, 100))
            .collect();
        let favourite_of = |club_id: u32| -> u32 {
            let taste = TasteFixtures::balanced(club_id);
            market
                .iter()
                .max_by(|a, b| {
                    taste
                        .interest_in(a)
                        .unwrap()
                        .partial_cmp(&taste.interest_in(b).unwrap())
                        .unwrap()
                })
                .map(|c| c.player_id)
                .unwrap()
        };
        let picks: std::collections::HashSet<u32> = (1u32..=25).map(favourite_of).collect();
        assert!(
            picks.len() > 3,
            "25 clubs on one market converged on {} names",
            picks.len()
        );
    }

    /// A club's read of a player must not change between two lookups —
    /// otherwise the "preference" is a weekly lottery, which is exactly the
    /// failure mode the draw exists to avoid.
    #[test]
    fn opinion_is_stable_for_a_pair() {
        let a = ClubOpinion::of(17, 4242, 12).believed_ability(120);
        let b = ClubOpinion::of(17, 4242, 12).believed_ability(120);
        assert_eq!(a, b);
    }

    /// …but two clubs must genuinely disagree, or the ranking collapses back
    /// onto true ability and every club converges on the same name.
    #[test]
    fn different_clubs_read_the_same_player_differently() {
        let reads: Vec<f32> = (1u32..=40)
            .map(|club| ClubOpinion::of(club, 4242, 8).believed_ability(120))
            .collect();
        let spread = reads.iter().cloned().fold(f32::MIN, f32::max)
            - reads.iter().cloned().fold(f32::MAX, f32::min);
        assert!(
            spread > 8.0,
            "40 clubs with mid judgement should span a real range, got {spread}"
        );
    }

    /// Better eyes, tighter read. The relationship has to be monotone or a
    /// club gains nothing by employing scouts.
    #[test]
    fn sharper_judgement_narrows_the_spread() {
        let spread_at = |judging: u8| {
            let reads: Vec<f32> = (1u32..=200)
                .map(|club| ClubOpinion::of(club, 77, judging).believed_ability(120))
                .collect();
            reads.iter().cloned().fold(f32::MIN, f32::max)
                - reads.iter().cloned().fold(f32::MAX, f32::min)
        };
        assert!(spread_at(3) > spread_at(19));
    }

    /// The draw must favour the strongest option without owning it.
    #[test]
    fn draw_favours_the_leader_but_shares() {
        let options = [(1u32, 1.0f32), (2, 0.8), (3, 0.6)];
        let mut wins = [0u32; 3];
        for _ in 0..4_000 {
            match InterestDraw::pick(&options) {
                Some(1) => wins[0] += 1,
                Some(2) => wins[1] += 1,
                Some(3) => wins[2] += 1,
                other => panic!("draw returned {other:?}"),
            }
        }
        assert!(wins[0] > wins[1], "leader should win most often");
        assert!(wins[1] > wins[2], "odds should follow interest order");
        assert!(
            wins[1] > 400,
            "runner-up must get a real share, got {}",
            wins[1]
        );
        assert!(
            wins[0] < 3_200,
            "leader must not monopolise the market, got {}",
            wins[0]
        );
    }

    /// Sampling without replacement — the old code took the top N, which meant
    /// the same N every tick.
    #[test]
    fn several_picks_are_distinct() {
        let options: Vec<(u32, f32)> = (1..=6).map(|i| (i, i as f32 * 0.2)).collect();
        let picked = InterestDraw::pick_several(&options, 3);
        assert_eq!(picked.len(), 3);
        let mut sorted = picked.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 3, "picked {picked:?} with a duplicate");
    }

    /// Every club must eventually lead the queue, or the rotation has merely
    /// moved the privilege rather than removed it.
    #[test]
    fn visit_order_rotates() {
        let mut leaders = std::collections::HashSet::new();
        for _ in 0..400 {
            leaders.insert(InterestDraw::visit_order(8)[0]);
        }
        assert_eq!(leaders.len(), 8, "rotation should reach every start index");
        let mut seen: Vec<usize> = InterestDraw::visit_order(5);
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 2, 3, 4], "order must be a permutation");
    }

    /// Standing still leads, but it must saturate — otherwise the biggest club
    /// in the country is the only destination that ever wins.
    #[test]
    fn destination_standing_saturates() {
        let appeal = |rep: u16| {
            DestinationAppeal {
                borrower_rep: rep,
                borrower_league_rep: 5_000,
                parent_league_rep: 5_000,
                minutes_headroom: 0.6,
                training_rating: 12,
                existing_placements: 0,
                is_development: true,
            }
            .score()
        };
        let low_gap = appeal(3_000) - appeal(1_000);
        let high_gap = appeal(9_000) - appeal(7_000);
        assert!(appeal(9_000) > appeal(1_000), "standing must still lead");
        assert!(
            high_gap < low_gap,
            "reputation gaps must matter less at the top: {high_gap} vs {low_gap}"
        );
    }

    /// A borrower already holding this parent's loanees is a less attractive
    /// next destination.
    #[test]
    fn crowding_damps_a_repeat_destination() {
        let appeal = |placements: u8| {
            DestinationAppeal {
                borrower_rep: 4_000,
                borrower_league_rep: 4_000,
                parent_league_rep: 6_000,
                minutes_headroom: 0.8,
                training_rating: 14,
                existing_placements: placements,
                is_development: true,
            }
            .score()
        };
        assert!(appeal(0) > appeal(1));
        assert!(appeal(1) > appeal(3));
    }

    /// A development loan is placed on minutes and coaching; a cover loan is
    /// placed mostly on standing. The two must not rank destinations alike.
    #[test]
    fn development_and_cover_loans_rank_destinations_differently() {
        let big_but_crowded = |dev: bool| {
            DestinationAppeal {
                borrower_rep: 7_000,
                borrower_league_rep: 6_000,
                parent_league_rep: 6_000,
                minutes_headroom: 0.15,
                training_rating: 8,
                existing_placements: 0,
                is_development: dev,
            }
            .score()
        };
        let small_but_playing = |dev: bool| {
            DestinationAppeal {
                borrower_rep: 2_500,
                borrower_league_rep: 3_000,
                parent_league_rep: 6_000,
                minutes_headroom: 1.0,
                training_rating: 16,
                existing_placements: 0,
                is_development: dev,
            }
            .score()
        };
        assert!(
            small_but_playing(true) > big_but_crowded(true),
            "a prospect goes where he plays"
        );
        assert!(
            big_but_crowded(false) > small_but_playing(false),
            "a cover loan follows standing"
        );
    }

    /// Repeat approaches must lengthen the standoff, and it must stay bounded
    /// so a target is never blacklisted for good.
    #[test]
    fn rebuff_standoff_escalates_and_caps() {
        let first = LoanApproachMemory::rebuff_days(1);
        assert!(
            (7..=40).contains(&first),
            "unexpected first standoff {first}"
        );
        assert!(LoanApproachMemory::rebuff_days(3) > first);
        assert!(LoanApproachMemory::rebuff_days(u8::MAX) <= 150);
    }
}
