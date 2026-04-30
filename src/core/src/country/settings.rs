#[derive(Debug, Clone)]
pub struct CountrySettings {
    pub pricing: CountryPricing,
    pub skin_colors: SkinColorDistribution,
}

impl Default for CountrySettings {
    fn default() -> Self {
        CountrySettings {
            pricing: CountryPricing::default(),
            skin_colors: SkinColorDistribution::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CountryPricing {
    pub price_level: f32,
}

impl Default for CountryPricing {
    fn default() -> Self {
        CountryPricing { price_level: 1.0 }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SkinColorDistribution {
    pub white: u8,
    pub black: u8,
    pub metis: u8,
}

impl Default for SkinColorDistribution {
    fn default() -> Self {
        SkinColorDistribution {
            white: 50,
            black: 20,
            metis: 30,
        }
    }
}
