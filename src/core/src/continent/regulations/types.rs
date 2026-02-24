use crate::continent::EconomicZone;

#[derive(Debug, Clone)]
pub struct ContinentalRegulations {
    pub ffp_rules: FinancialFairPlayRules,
    pub foreign_player_limits: ForeignPlayerLimits,
    pub youth_requirements: YouthRequirements,
}

impl ContinentalRegulations {
    pub fn new() -> Self {
        ContinentalRegulations {
            ffp_rules: FinancialFairPlayRules::new(),
            foreign_player_limits: ForeignPlayerLimits::new(),
            youth_requirements: YouthRequirements::new(),
        }
    }

    pub fn update_ffp_thresholds(&mut self, economic_zone: &EconomicZone) {
        // Adjust FFP based on economic conditions
        self.ffp_rules
            .update_thresholds(economic_zone.get_overall_health());
    }

    pub fn review_foreign_player_rules(&mut self, _rankings: &crate::continent::ContinentalRankings) {
        // Potentially adjust foreign player rules
    }

    pub fn update_youth_requirements(&mut self) {
        // Update youth development requirements
    }
}

#[derive(Debug, Clone)]
pub struct FinancialFairPlayRules {
    pub max_deficit: f64,
    pub monitoring_period_years: u8,
    pub squad_cost_ratio_limit: f32,
}

impl FinancialFairPlayRules {
    pub fn new() -> Self {
        FinancialFairPlayRules {
            max_deficit: 30_000_000.0,
            monitoring_period_years: 3,
            squad_cost_ratio_limit: 0.7,
        }
    }

    pub fn update_thresholds(&mut self, economic_health: f32) {
        // Adjust based on economic conditions
        if economic_health < 0.5 {
            self.max_deficit *= 0.8;
        } else if economic_health > 0.8 {
            self.max_deficit *= 1.1;
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForeignPlayerLimits {
    pub max_non_eu_players: Option<u8>,
    pub homegrown_minimum: u8,
}

impl ForeignPlayerLimits {
    pub fn new() -> Self {
        ForeignPlayerLimits {
            max_non_eu_players: Some(3),
            homegrown_minimum: 8,
        }
    }
}

#[derive(Debug, Clone)]
pub struct YouthRequirements {
    pub minimum_academy_investment: f64,
    pub minimum_youth_squad_size: u8,
}

impl YouthRequirements {
    pub fn new() -> Self {
        YouthRequirements {
            minimum_academy_investment: 1_000_000.0,
            minimum_youth_squad_size: 20,
        }
    }
}
