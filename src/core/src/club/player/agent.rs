//! Per-player agent archetype, derived on demand from personality. No extra
//! state on `Player`: the player's own ambition / loyalty / controversy
//! values already encode how their representative behaves in negotiations.
//! Agents bias personal-terms acceptance — greedy agents push wage demands
//! up, loyal agents push their client to stay.

use crate::club::player::player::Player;

/// Lens over a player's personality values shaped like an agent profile.
/// Both components in 0.0–1.0; neutral = 0.5.
#[derive(Debug, Clone, Copy)]
pub struct PlayerAgent {
    /// How aggressively the agent chases top dollar. 0 = hometown discount,
    /// 1 = holds out for every last cent.
    pub greed: f32,
    /// How hard the agent pushes the player to stay put. 0 = revolving door,
    /// 1 = "my client is happy where he is".
    pub loyalty: f32,
}

impl PlayerAgent {
    /// Derive agent profile from the player's personality. Ambition and
    /// controversy both drive greed (visible entitlement + willingness to
    /// rock the boat for a bigger deal); raw loyalty drives agent loyalty.
    pub fn for_player(player: &Player) -> Self {
        let greed = ((player.attributes.ambition + player.attributes.controversy) / 40.0)
            .clamp(0.0, 1.0);
        let loyalty = (player.attributes.loyalty / 20.0).clamp(0.0, 1.0);
        Self { greed, loyalty }
    }

    /// Personal-terms delta applied to the negotiation acceptance chance.
    /// Greedy agents depress chance; loyal agents depress it further when
    /// the move isn't a clear upward step.
    pub fn personal_terms_delta(&self, rep_diff: f32) -> f32 {
        let greed_penalty = (self.greed - 0.5) * 20.0;
        let loyalty_penalty = if rep_diff < 0.1 {
            self.loyalty * 8.0
        } else {
            0.0
        };
        -(greed_penalty + loyalty_penalty)
    }

    /// Renewal acceptance score delta. Positive = more likely to accept a
    /// same-or-small-raise offer; negative = agent pushes to hold out.
    /// Raise ratio is (offered / current). Greed fades once the raise is
    /// genuinely big (>30%), loyalty always leans toward accepting.
    pub fn renewal_delta(&self, raise_ratio: f32) -> f32 {
        self.renewal_delta_with(raise_ratio, 0.0, false)
    }

    /// Extended version that considers off-salary sweeteners. `sweetener_ratio`
    /// is (signing + loyalty bonuses) ÷ current salary; a release clause is a
    /// separate boolean because its value is in fee terms, not wage terms —
    /// it shifts the calculus without adding cash to the agent's pocket.
    /// Both fade greed non-linearly: the agent relaxes when the deal has
    /// depth beyond base wage.
    pub fn renewal_delta_with(
        &self,
        raise_ratio: f32,
        sweetener_ratio: f32,
        has_release_clause: bool,
    ) -> f32 {
        let raise_fade = (raise_ratio - 1.0).max(0.0) * 0.8;
        let sweetener_fade = sweetener_ratio.min(0.5) * 0.6;
        let clause_fade = if has_release_clause { 0.15 } else { 0.0 };
        let effective_greed = (self.greed - raise_fade - sweetener_fade - clause_fade).max(0.0);
        let greed_penalty = effective_greed * 12.0;
        let loyalty_bonus = self.loyalty * 6.0;
        loyalty_bonus - greed_penalty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PersonAttributes;

    fn attrs(ambition: f32, controversy: f32, loyalty: f32) -> PersonAttributes {
        PersonAttributes {
            adaptability: 10.0,
            ambition,
            controversy,
            loyalty,
            pressure: 10.0,
            professionalism: 10.0,
            sportsmanship: 10.0,
            temperament: 10.0,
            consistency: 10.0,
            important_matches: 10.0,
            dirtiness: 10.0,
        }
    }

    fn agent(a: f32, c: f32, l: f32) -> PlayerAgent {
        // Build directly from numeric personality to sidestep Player
        // construction in a unit test.
        let ambition = a;
        let controversy = c;
        let loyalty = l;
        let _ = attrs(ambition, controversy, loyalty); // sanity
        PlayerAgent {
            greed: ((ambition + controversy) / 40.0).clamp(0.0, 1.0),
            loyalty: (loyalty / 20.0).clamp(0.0, 1.0),
        }
    }

    #[test]
    fn neutral_agent_has_no_delta() {
        let a = agent(10.0, 10.0, 10.0);
        assert!(a.greed > 0.45 && a.greed < 0.55);
        assert!((a.personal_terms_delta(0.0)).abs() < 5.0);
    }

    #[test]
    fn greedy_agent_depresses_chance() {
        let greedy = agent(18.0, 15.0, 10.0);
        let neutral = agent(10.0, 10.0, 10.0);
        assert!(greedy.personal_terms_delta(0.0) < neutral.personal_terms_delta(0.0));
    }

    #[test]
    fn loyal_agent_penalises_lateral_moves_most() {
        let loyal = agent(10.0, 10.0, 20.0);
        let upward = loyal.personal_terms_delta(0.3);
        let lateral = loyal.personal_terms_delta(0.0);
        assert!(lateral < upward, "lateral {lateral} should hurt more than upward {upward}");
    }

    #[test]
    fn loyal_agent_does_not_affect_clear_upgrades() {
        // rep_diff > 0.1 means the step is clearly upward → loyalty shouldn't bite.
        let loyal = agent(10.0, 10.0, 20.0);
        let neutral = agent(10.0, 10.0, 10.0);
        let delta_loyal = loyal.personal_terms_delta(0.5);
        let delta_neutral = neutral.personal_terms_delta(0.5);
        assert!((delta_loyal - delta_neutral).abs() < 1.0);
    }
}
