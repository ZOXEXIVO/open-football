pub struct FormattingUtils;

impl FormattingUtils {
    /// Round to a "nice" negotiation-friendly number.
    ///   < 1K        → nearest 100
    ///   1K - 100K   → nearest 1K
    ///   100K - 1M   → nearest 10K
    ///   1M+         → nearest 100K
    #[inline]
    pub fn round_fee(amount: f64) -> f64 {
        let val = amount.abs();
        let step = if val >= 1_000_000.0 {
            100_000.0
        } else if val >= 100_000.0 {
            10_000.0
        } else if val >= 1_000.0 {
            1_000.0
        } else {
            100.0
        };
        (amount / step).round() * step
    }

    #[inline]
    pub fn format_money(amount: f64) -> String {
        let val = amount.abs();

        if val >= 1_000_000.0 {
            format!("{:.1}M", amount / 1_000_000.0)
        } else if val >= 1_000.0 {
            format!("{:.1}K", amount / 1_000.0)
        } else if val > -1_000.0 {
            format!("{:.2}", amount)
        } else {
            format!("{:.0}K", amount / 1_000.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_fee() {
        assert_eq!(FormattingUtils::round_fee(29_241.28), 29_000.0);
        assert_eq!(FormattingUtils::round_fee(345_678.0), 350_000.0);
        assert_eq!(FormattingUtils::round_fee(1_234_567.0), 1_200_000.0);
        assert_eq!(FormattingUtils::round_fee(550.0), 600.0);
        assert_eq!(FormattingUtils::round_fee(2_500_000.0), 2_500_000.0);
    }

    #[test]
    fn test_format_money_millions() {
        assert_eq!(FormattingUtils::format_money(1_000_000.0), "1.0M");
        assert_eq!(FormattingUtils::format_money(2_500_000.0), "2.5M");
    }

    #[test]
    fn test_format_money_thousands() {
        assert_eq!(FormattingUtils::format_money(1_000.0), "1.0K");
        assert_eq!(FormattingUtils::format_money(29_000.0), "29.0K");
        assert_eq!(FormattingUtils::format_money(350_000.0), "350.0K");
    }
}
