use super::table::TableStakes;

/// What a side knows of a tie in a bracket.
#[derive(Debug, Clone, Copy)]
pub struct KnockoutTieReading {
    /// 1-based round; the final is `total_rounds`.
    pub round: u8,
    pub total_rounds: u8,
    /// The opponent's strength over the side's own. 1.0 is an even tie.
    pub opponent_ratio: f32,
    /// For a second leg, the side's lead on aggregate before it; negative
    /// when behind.
    pub aggregate_lead: Option<i16>,
}

/// How much one result can still change for a side in a competition with a
/// bracket.
pub struct KnockoutStakes;

impl KnockoutStakes {
    /// A first-round tie against opposition that cannot hurt the side.
    const OPENING_ROUND: f32 = 0.20;
    /// Later rounds are worth more, and the last ones much more.
    const ROUND_CURVE: f32 = 1.5;
    /// How sharply a stronger opponent turns a tie into an occasion.
    const OPPONENT_CURVE: f32 = 3.0;
    /// An equal opponent is this many times short of making a tie an
    /// occasion: at 2 he counts for a third, so an early round against an
    /// equal is worth less than a routine league match.
    const EQUAL_OPPONENT: f32 = 2.0;
    /// Goals on aggregate at which a second leg is about two thirds live.
    const AGGREGATE_SCALE: f32 = 1.6;

    pub fn of(tie: KnockoutTieReading) -> f32 {
        if tie.total_rounds <= 1 || tie.round >= tie.total_rounds {
            return 1.0;
        }
        if let Some(lead) = tie.aggregate_lead {
            return Self::second_leg(lead);
        }
        let progress = f32::from(tie.round.saturating_sub(1)) / f32::from(tie.total_rounds - 1);
        let round_value =
            Self::OPENING_ROUND + (1.0 - Self::OPENING_ROUND) * progress.powf(Self::ROUND_CURVE);
        round_value + (1.0 - round_value) * Self::threat(tie.opponent_ratio)
    }

    /// 0 against a side that cannot hurt, a third against an equal, towards
    /// 1 against a giant.
    fn threat(opponent_ratio: f32) -> f32 {
        let ratio = opponent_ratio.clamp(0.05, 20.0).powf(Self::OPPONENT_CURVE);
        ratio / (Self::EQUAL_OPPONENT + ratio)
    }

    /// A level tie is a must-win; a tie settled by a wide margin is a
    /// fixture with nothing on it.
    fn second_leg(aggregate_lead: i16) -> f32 {
        let margin = f32::from(aggregate_lead.abs()) / Self::AGGREGATE_SCALE;
        let live = (-margin * margin).exp();
        TableStakes::PROFESSIONAL_FLOOR + (1.0 - TableStakes::PROFESSIONAL_FLOOR) * live
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tie(round: u8, total_rounds: u8, opponent_ratio: f32) -> f32 {
        KnockoutStakes::of(KnockoutTieReading {
            round,
            total_rounds,
            opponent_ratio,
            aggregate_lead: None,
        })
    }

    fn second_leg(aggregate_lead: i16) -> f32 {
        KnockoutStakes::of(KnockoutTieReading {
            round: 2,
            total_rounds: 5,
            opponent_ratio: 1.0,
            aggregate_lead: Some(aggregate_lead),
        })
    }

    #[test]
    fn stakes_rise_with_every_round() {
        let rounds: Vec<f32> = (1..=6).map(|round| tie(round, 6, 1.0)).collect();
        assert!(
            rounds.windows(2).all(|pair| pair[1] > pair[0]),
            "{rounds:?}"
        );
    }

    #[test]
    fn stakes_rise_with_a_stronger_opponent_and_never_jump() {
        let mut last = tie(1, 6, 0.2);
        for step in 1..=140 {
            let ratio = 0.2 + step as f32 * 0.02;
            let now = tie(1, 6, ratio);
            assert!(now >= last, "ratio {ratio}");
            assert!(now - last < 0.03, "ratio {ratio} jumped by {}", now - last);
            last = now;
        }
        assert!(tie(1, 6, 0.4) < 0.35, "a giant in an early round has room");
        assert!(
            tie(1, 6, 2.5) > 0.9,
            "a minnow drawn against a giant has none"
        );
    }

    #[test]
    fn a_final_is_a_must_win_whoever_the_opponent_is() {
        assert_eq!(tie(6, 6, 0.3), 1.0);
        assert_eq!(tie(1, 1, 3.0), 1.0);
    }

    #[test]
    fn a_second_leg_reads_the_aggregate() {
        assert_eq!(second_leg(0), 1.0, "level on aggregate");
        assert!(second_leg(1) > 0.75);
        assert!(
            second_leg(4) < 0.5,
            "four ahead is settled: {}",
            second_leg(4)
        );
        assert_eq!(
            second_leg(-3),
            second_leg(3),
            "settled is settled for both sides"
        );
        assert!(second_leg(1) > second_leg(2) && second_leg(2) > second_leg(3));
    }
}
