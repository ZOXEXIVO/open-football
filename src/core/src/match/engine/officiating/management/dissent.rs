//! Dissent: a decision goes against a man and he tells the referee what he
//! thinks of it.

use crate::r#match::MatchPlayer;
use crate::r#match::engine::officiating::referee::RefereeProfile;
use crate::r#match::player::strategies::players::ops::skill::traits_bias::personality_bias;

pub struct Dissent;

impl Dissent {
    /// The odds for a hot-headed man in front of an average referee: about
    /// one booking for dissent every three matches across ~25 decisions.
    const BASE: f32 = 0.05;

    /// Chance a decision against `player` turns into a booking for what he
    /// says about it — a short temper and an aggressive streak, a player
    /// known to argue, and a referee who reaches for his pocket.
    pub fn caution_chance(player: &MatchPlayer, referee: &RefereeProfile) -> f32 {
        let temper = 1.0 - (player.attributes.temperament / 20.0).clamp(0.0, 1.0);
        let aggression = (player.skills.mental.aggression / 20.0).clamp(0.0, 1.0);
        let argues = personality_bias(player).yellow_after_protest_chance;
        (Self::BASE * temper * aggression * (0.5 + referee.card_happiness) + argues).clamp(0.0, 0.2)
    }
}
