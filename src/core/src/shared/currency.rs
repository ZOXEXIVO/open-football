#[derive(Debug, Clone)]
pub struct CurrencyValue {
    pub amount: f64,
    pub currency: Currency,
}

impl CurrencyValue {
    pub fn new(amount: f64, currency: Currency) -> Self {
        CurrencyValue { amount, currency }
    }
}

#[derive(Debug, Clone)]
pub enum Currency {
    Usd,
}
