//! L4 — what the buyer can actually hold.
//!
//! The offered wage used to be the buyer's LEVEL, never the buyer's MEANS:
//! `ContractValuation` anchored on the buyer's club and league reputation,
//! and no budget capped it in either direction. So a club sitting on
//! $293M offered a Premier League star 47 % of his current wage, he
//! refused, and nothing about the deal ever mentioned the money in the
//! bank. Meanwhile a break-even side could offer its level wage to anyone
//! it fancied, for ever, because nothing said it could not.
//!
//! [`WagePower`] is the ceiling and [`WagePower::star_offer`] is the bid.
//! The ceiling is the greater of three honest numbers — a stretch on the
//! club's own level, the room left under the board's wage mandate, and the
//! slice of the owner's yearly subsidy one shirt of this brief tier may
//! carry. The bid is then the player's own reservation wage, clamped into
//! `[level_wage, power]`: the buyer pays what it takes, up to what it has,
//! and never more than the man is asking for.
//!
//! No league list and no money-club flag: everything above the club's own
//! revenue comes from [`ClubBenefactor`], which is one ratio off the
//! balance sheet (memory `feedback_balance_system_not_cases`).

use crate::club::player::calculators::{ContractValuation, ValuationContext};
use crate::transfers::negotiation::NegotiationStatus;
use crate::transfers::offer::PromisedSquadStatus;
use crate::transfers::pipeline::planning::BriefTier;
use crate::{Club, Country, Player, PlayerSquadStatus};

/// The owner's yearly cheque, split into one envelope per brief tier and
/// drawn down as shirts are actually signed.
///
/// Without a ledger the subsidy was re-read live at every negotiation and
/// nothing remembered the shirts already carrying it: four July
/// negotiations at one benefactor club each saw the whole cheque, and four
/// $30M stars landed on a $55M subsidy. The envelope is the ledger, and
/// it is the mechanism behind "one shirt per brief tier" (Part VIII, "the
/// drain").
///
/// Granted once at season start from the same `benefactor × appetite ×
/// idle × 0.35` figure the wage mandate spends; consumed by the part of an
/// agreed wage the club's own level cannot explain.
#[derive(Debug, Clone, Copy, Default)]
pub struct OwnerEnvelopes {
    granted: [f64; 3],
    consumed: [f64; 3],
}

impl OwnerEnvelopes {
    #[inline]
    fn index(tier: BriefTier) -> usize {
        match tier {
            BriefTier::A => 0,
            BriefTier::B => 1,
            BriefTier::C => 2,
        }
    }

    /// Split one year's subsidy across the three tiers.
    pub fn split(subsidy_per_year: f64) -> Self {
        let subsidy = subsidy_per_year.max(0.0);
        OwnerEnvelopes {
            granted: [
                subsidy * WagePower::tier_share(BriefTier::A),
                subsidy * WagePower::tier_share(BriefTier::B),
                subsidy * WagePower::tier_share(BriefTier::C),
            ],
            consumed: [0.0; 3],
        }
    }

    /// What this tier was given for the season.
    #[inline]
    pub fn granted(&self, tier: BriefTier) -> f64 {
        self.granted[Self::index(tier)]
    }

    /// What this tier has already spent.
    #[inline]
    pub fn consumed(&self, tier: BriefTier) -> f64 {
        self.consumed[Self::index(tier)]
    }

    /// What is left for this tier once the shirts already signed and the
    /// negotiations already open are taken off.
    ///
    /// `reserved` is folded live from the club's own pending negotiations
    /// — concurrent deals on the same day must not each see the whole
    /// envelope.
    #[inline]
    pub fn remaining(&self, tier: BriefTier, reserved: f64) -> f64 {
        (self.granted(tier) - self.consumed(tier) - reserved.max(0.0)).max(0.0)
    }

    /// Draw down this tier's envelope by a signed shirt's excess.
    pub fn consume(&mut self, tier: BriefTier, amount: f64) {
        if amount <= 0.0 {
            return;
        }
        let i = Self::index(tier);
        self.consumed[i] = (self.consumed[i] + amount).min(self.granted[i]);
    }

    /// The whole cheque, for the census and the budget line.
    #[inline]
    pub fn total_granted(&self) -> f64 {
        self.granted.iter().sum()
    }
}

/// What one club can pay one player.
#[derive(Debug, Clone, Copy)]
pub struct WagePower {
    /// The wage the buyer's own level implies for the promised shirt.
    pub level_wage: f64,
    /// The most it can hold for this shirt, all sources considered.
    pub ceiling: f64,
    /// What is left of this shirt's tier envelope — the owner's money, not
    /// the club's.
    pub owner_subsidy: f64,
    /// Room left under the board's wage mandate, with the owner's cheque
    /// taken back OUT of it: the mandate already contains the subsidy, so
    /// leaving it in would let a benefactor club count the same money
    /// twice.
    pub wage_headroom: f64,
}

impl WagePower {
    /// How far above its own level wage a club will stretch on revenue
    /// alone. Every club overpays a little for a signing it wants; this is
    /// how much, and it is what keeps ordinary domestic business moving
    /// when the reservation lands just above the market figure.
    pub const LEVEL_STRETCH: f64 = 1.30;

    /// Share of the owner's yearly subsidy one shirt of this tier may
    /// carry. Mirrors [`BriefTier`]'s fee envelope: a club that briefs one
    /// of each has committed its owner's cheque exactly once, which is what
    /// stops a benefactor club stripping the elite band in one window
    /// (Part VIII, "the drain").
    pub fn tier_share(tier: BriefTier) -> f64 {
        match tier {
            BriefTier::A => BriefTier::A_SHARE,
            BriefTier::B => BriefTier::B_SHARE,
            BriefTier::C => BriefTier::C_SHARE,
        }
    }

    /// Read the buyer's power for one shirt.
    ///
    /// `level_wage` is what the buyer's reputation says the promised role
    /// is worth — the figure the old model offered and then stopped at.
    /// `reserved` is what this club's OTHER open negotiations have already
    /// claimed from the same tier envelope, folded live by the caller.
    ///
    /// The two money arms are disjoint on purpose. The board's wage
    /// mandate already has the owner's cheque added into it
    /// (`SeasonTargets::owner_subsidy`), so a ceiling that took the room
    /// under the mandate AND a share of the subsidy counted the same money
    /// twice — the doc's $33M shirt was $55M in the tree.
    pub fn for_player(club: &Club, level_wage: f64, tier: BriefTier, reserved: f64) -> Self {
        let committed: f64 = club
            .teams
            .iter()
            .map(|t| t.get_annual_salary() as f64)
            .sum();
        let targets = club.board.season_targets.as_ref();
        let mandate = targets.map(|t| t.wage_budget.max(0) as f64).unwrap_or(0.0);
        let subsidy_in_mandate = targets.map(|t| t.owner_subsidy.max(0) as f64).unwrap_or(0.0);
        let wage_headroom = (mandate - subsidy_in_mandate - committed).max(0.0);

        let owner_subsidy = targets
            .map(|t| t.owner_envelopes.remaining(tier, reserved))
            .unwrap_or(0.0);

        let ceiling = (level_wage * Self::LEVEL_STRETCH)
            .max(wage_headroom)
            .max(owner_subsidy);

        WagePower {
            level_wage,
            ceiling,
            owner_subsidy,
            wage_headroom,
        }
    }

    /// The part of an agreed wage the club's own level cannot explain —
    /// what a signed shirt draws from its tier envelope.
    #[inline]
    pub fn envelope_draw(level_wage: f64, agreed_wage: f64) -> f64 {
        (agreed_wage - level_wage * Self::LEVEL_STRETCH).max(0.0)
    }

    /// Can this club reach his demand at all?
    #[inline]
    pub fn can_reach(&self, reservation: f64) -> bool {
        reservation <= self.ceiling
    }

    /// The spend-power proxy the fee model reads: what the club earns plus
    /// what its owner puts in. Without the second term a benefactor club's
    /// transfer ceiling was its (small) league revenue and the fee could
    /// never clear.
    #[inline]
    pub fn spend_power(annual_income: f64, owner_subsidy: f64) -> f64 {
        annual_income.max(0.0) + owner_subsidy.max(0.0)
    }
}

/// What a club's own open negotiations have already claimed from a tier
/// envelope.
///
/// Folded live rather than stored, the way the loan depth cap folds
/// pending incoming loans: four negotiations opened at one benefactor club
/// on the same day must not each read the whole cheque, and a deal that
/// dies has to give its share straight back.
pub struct OwnerEnvelopeReservations;

impl OwnerEnvelopeReservations {
    /// The sum of what this club's OTHER live negotiations in this tier
    /// would draw if they all completed.
    pub fn open(
        country: &Country,
        buyer_club_id: u32,
        tier: BriefTier,
        exclude_negotiation_id: u32,
    ) -> f64 {
        country
            .transfer_market
            .negotiations
            .values()
            .filter(|n| {
                n.buying_club_id == buyer_club_id
                    && n.id != exclude_negotiation_id
                    && !n.is_loan
                    && matches!(
                        n.status,
                        NegotiationStatus::Pending | NegotiationStatus::Countered
                    )
                    && n.brief_tier.unwrap_or(BriefTier::C) == tier
            })
            .map(|n| {
                let level = n.opening_salary.or(n.offered_salary).unwrap_or(0) as f64;
                let asked = n.offered_salary.unwrap_or(0) as f64;
                WagePower::envelope_draw(level, asked)
            })
            .sum()
    }
}

/// The wage the buyer's own level implies — the anchor both sides of the
/// negotiation share.
pub struct BuyerLevelWage;

impl BuyerLevelWage {
    /// [`crate::club::player::calculators::ContractValuation`] at the
    /// buyer's club and league reputation, for the role the buyer is
    /// PROMISING — not the one the player currently holds.
    ///
    /// The old `buyer_wage_context` used his current status, so a club
    /// offering a bench seat still priced him as the key player he was
    /// leaving behind, and a club offering the shirt priced him as the
    /// backup he had become. Both are the wrong number for the deal on
    /// the table.
    ///
    /// This is the ONE wage curve for the buying side: the offer staged on
    /// the negotiation and the annual wage in the personal-terms package
    /// are the same figure, so the man is installed on exactly the wage he
    /// said yes to.
    pub fn evaluate(
        player: &Player,
        age: u8,
        buyer_reputation_score: f32,
        buyer_league_reputation: u16,
        promised: Option<PromisedSquadStatus>,
    ) -> u32 {
        ContractValuation::evaluate(
            player,
            &Self::context(
                player,
                age,
                buyer_reputation_score,
                buyer_league_reputation,
                promised,
            ),
        )
        .expected_wage
    }

    /// The valuation context the buyer prices the promised shirt in.
    pub fn context(
        player: &Player,
        age: u8,
        buyer_reputation_score: f32,
        buyer_league_reputation: u16,
        promised: Option<PromisedSquadStatus>,
    ) -> ValuationContext {
        let squad_status = promised.map(|p| p.as_squad_status()).unwrap_or_else(|| {
            // No promise made: the club has not said, so it prices the
            // shirt he already wears.
            player
                .contract
                .as_ref()
                .map(|c| c.squad_status.clone())
                .unwrap_or(PlayerSquadStatus::FirstTeamRegular)
        });
        ValuationContext {
            age,
            club_reputation_score: buyer_reputation_score,
            league_reputation: buyer_league_reputation,
            squad_status,
            current_salary: player.contract.as_ref().map(|c| c.salary).unwrap_or(0),
            // Neutral: `months_remaining` only widens the acceptable
            // band, never `expected_wage`, and the leverage that
            // actually matters now lives in the appraisal's money
            // weight.
            months_remaining: 24,
            has_market_interest: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tier_shares_commit_the_owners_cheque_exactly_once() {
        let total = WagePower::tier_share(BriefTier::A)
            + WagePower::tier_share(BriefTier::B)
            + WagePower::tier_share(BriefTier::C);
        assert!((total - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_buyer_reaches_its_ceiling_and_no_further() {
        let power = WagePower {
            level_wage: 4_000_000.0,
            ceiling: 33_000_000.0,
            owner_subsidy: 33_000_000.0,
            wage_headroom: 0.0,
        };
        assert!(power.can_reach(30_000_000.0));
        assert!(!power.can_reach(50_000_000.0));
    }

    #[test]
    fn one_cheque_pays_for_one_shirt_per_tier() {
        // A $55M subsidy: tier A gets $33M, and the first star signed on
        // it takes the envelope with him.
        let mut envelopes = OwnerEnvelopes::split(55_000_000.0);
        assert!((envelopes.granted(BriefTier::A) - 33_000_000.0).abs() < 1.0);
        assert_eq!(envelopes.remaining(BriefTier::A, 0.0), 33_000_000.0);

        // A shirt agreed at $30M on a level wage of $4M draws the part
        // the club's own level cannot explain.
        let draw = WagePower::envelope_draw(4_000_000.0, 30_000_000.0);
        assert!((draw - 24_800_000.0).abs() < 1.0, "{draw}");
        envelopes.consume(BriefTier::A, draw);
        assert!(envelopes.remaining(BriefTier::A, 0.0) < 8_300_000.0);

        // An open negotiation reserves its own share on top, so two
        // concurrent deals on the same day cannot both see the whole
        // envelope.
        assert_eq!(envelopes.remaining(BriefTier::A, 50_000_000.0), 0.0);
    }

    #[test]
    fn an_envelope_never_overdraws() {
        let mut envelopes = OwnerEnvelopes::split(10_000_000.0);
        envelopes.consume(BriefTier::C, 99_000_000.0);
        assert_eq!(envelopes.remaining(BriefTier::C, 0.0), 0.0);
        assert!((envelopes.consumed(BriefTier::C) - 1_000_000.0).abs() < 1.0);
    }

    #[test]
    fn four_shirts_on_the_same_day_share_one_envelope() {
        // A $150M subsidy: tier A holds $90M. Four negotiations opened at
        // the same benefactor club on the same day, each for a man asking
        // $30M against a $4M level wage — so each in flight reserves the
        // part of its wage the club's own level cannot explain.
        //
        // The first three fit. The fourth finds too little left, its
        // ceiling falls back to the club's own stretched level, and it is
        // refused on the wage — which is exactly the drain guard: one
        // cheque does not buy four stars.
        let envelopes = OwnerEnvelopes::split(150_000_000.0);
        let level = 4_000_000.0;
        let demand = 30_000_000.0;
        let per_shirt = WagePower::envelope_draw(level, demand);
        let reached = (0..4)
            .filter(|open_before| {
                let ceiling = envelopes
                    .remaining(BriefTier::A, per_shirt * *open_before as f64)
                    .max(level * WagePower::LEVEL_STRETCH);
                WagePower {
                    level_wage: level,
                    ceiling,
                    owner_subsidy: ceiling,
                    wage_headroom: 0.0,
                }
                .can_reach(demand)
            })
            .count();
        assert_eq!(reached, 3, "the fourth star is out of the envelope");
    }

    #[test]
    fn no_owner_means_no_envelope() {
        let envelopes = OwnerEnvelopes::split(0.0);
        assert_eq!(envelopes.total_granted(), 0.0);
        assert_eq!(envelopes.remaining(BriefTier::A, 0.0), 0.0);
    }
}
