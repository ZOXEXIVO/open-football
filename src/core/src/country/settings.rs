#[derive(Debug, Clone)]
pub struct CountrySettings {
    pub pricing: CountryPricing,
}

impl Default for CountrySettings {
    fn default() -> Self {
        CountrySettings {
            pricing: CountryPricing::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CountryPricing {
    pub price_level: f32,
}

impl Default for CountryPricing {
    fn default() -> Self {
        CountryPricing {
            price_level: 1.0,
        }
    }
}
