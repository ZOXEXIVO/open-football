use chrono::NaiveDate;
use crate::{Person, Player, PlayerPositionType};
use crate::shared::CurrencyValue;
use crate::transfers::offer::{TransferClause, TransferOffer};
use crate::transfers::window::PlayerValuationCalculator;

pub struct ClubTransferStrategy {
    pub club_id: u32,
    pub budget: Option<CurrencyValue>,
    pub selling_willingness: f32, // 0.0-1.0
    pub buying_aggressiveness: f32, // 0.0-1.0
    pub target_positions: Vec<PlayerPositionType>,
    pub reputation_level: u16,
}

impl ClubTransferStrategy {
    pub fn new(club_id: u32) -> Self {
        ClubTransferStrategy {
            club_id,
            budget: None,
            selling_willingness: 0.5,
            buying_aggressiveness: 0.5,
            target_positions: Vec::new(),
            reputation_level: 50,
        }
    }

    pub fn decide_player_interest(&self, player: &Player) -> bool {
        // Decide if the club should be interested in this player

        // Position need
        let position_needed = self.target_positions.contains(&player.position());
        if !position_needed && self.target_positions.len() > 0 {
            return false;
        }

        // Age policy
        let age = player.age(chrono::Local::now().naive_local().date());
        if self.reputation_level > 80 && age > 30 {
            // Top clubs rarely sign older players
            return false;
        }

        // Quality check
        let player_ability = player.player_attributes.current_ability as u16;

        // Is player good enough for the club?
        if player_ability < self.reputation_level / 2 {
            return false;
        }

        // Is player too good for the club?
        if player_ability > self.reputation_level * 2 {
            // Only aggressive buyers go for much better players
            return self.buying_aggressiveness > 0.8;
        }

        true
    }

    pub fn calculate_initial_offer(
        &self,
        player: &Player,
        asking_price: &CurrencyValue,
        current_date: NaiveDate
    ) -> TransferOffer {
        // Club budget check
        let max_budget = match &self.budget {
            Some(budget) => budget.amount,
            None => f64::MAX, // No budget constraint
        };

        // Calculate base valuation
        let player_value = PlayerValuationCalculator::calculate_value(player, current_date);

        // Adjust based on asking price
        let mut offer_amount = if asking_price.amount > 0.0 {
            // Start with 60-90% of asking price depending on aggressiveness
            asking_price.amount * (0.6 + (self.buying_aggressiveness as f64 * 0.3))
        } else {
            // No asking price - use our valuation but discount it
            player_value.amount * (0.7 + (self.buying_aggressiveness as f64 * 0.2f64))
        };

        // Cap by budget - never offer more than 80% of available budget
        let budget_cap = max_budget * 0.8;
        if offer_amount > budget_cap {
            offer_amount = budget_cap;
        }

        // Create the base offer
        let mut offer = TransferOffer::new(
            CurrencyValue {
                amount: offer_amount,
                currency: crate::shared::Currency::Usd,
            },
            self.club_id,
            current_date,
        );

        // Add clauses based on player profile and club strategy

        // 1. Add sell-on clause for young players with potential
        let age = player.age(current_date);
        let potential_gap = player.player_attributes.potential_ability as i16 -
            player.player_attributes.current_ability as i16;

        if age < 23 && potential_gap > 10 {
            // Add sell-on clause for promising youngsters
            let sell_on_percentage = 0.1 + (potential_gap as f32 / 100.0).min(0.1);
            offer = offer.with_clause(TransferClause::SellOnClause(sell_on_percentage));
        }

        // 2. Add appearance bonuses for older players to reduce risk
        if age > 28 {
            let appearance_amount = offer_amount * 0.1; // 10% of transfer fee
            offer = offer.with_clause(TransferClause::AppearanceFee(
                CurrencyValue {
                    amount: appearance_amount,
                    currency: crate::shared::Currency::Usd,
                },
                20 // After 20 appearances
            ));
        }

        // 3. Add goal bonus for attackers
        if player.position().is_forward() && player.statistics.goals > 5 {
            let goals_bonus = offer_amount * 0.15; // 15% of transfer fee
            offer = offer.with_clause(TransferClause::GoalBonus(
                CurrencyValue {
                    amount: goals_bonus,
                    currency: crate::shared::Currency::Usd,
                },
                15 // After 15 goals
            ));
        }

        // 4. Add promotion bonus for lower reputation clubs
        if self.reputation_level < 60 {
            let promotion_bonus = offer_amount * 0.2; // 20% of transfer fee
            offer = offer.with_clause(TransferClause::PromotionBonus(
                CurrencyValue {
                    amount: promotion_bonus,
                    currency: crate::shared::Currency::Usd,
                }
            ));
        }

        // Set contract length based on player age
        let contract_years = if age < 24 {
            5 // Long contract for young players
        } else if age < 28 {
            4 // Standard length for prime players
        } else if age < 32 {
            2 // Shorter for older players
        } else {
            1 // One year for veterans
        };

        offer.with_contract_length(contract_years)
    }
}
