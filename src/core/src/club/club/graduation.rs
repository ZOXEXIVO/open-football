use super::Club;
use crate::Player;
use crate::club::player::language::{Language, PlayerLanguage};
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::{CompletedTransfer, TransferType};
use crate::{PlayerStatusType, TeamType};
use chrono::NaiveDate;
use log::debug;

impl Club {
    /// Pre-season reset: restore player conditions and clear lingering statuses.
    /// Called once at season start so teams begin with full healthy squads.
    pub(super) fn process_pre_season_reset(&mut self) {
        for team in &mut self.teams.teams {
            for player in &mut team.players.players {
                // Restore condition to pre-season fitness level (85%)
                if player.player_attributes.condition < 8500 && !player.player_attributes.is_injured
                {
                    player.player_attributes.condition = 8500;
                }

                // Clear stale Int / IntU21 status (should have been released by
                // national team, but safety net in case tournament release was missed)
                player.statuses.remove(PlayerStatusType::Int);
                player.statuses.remove(PlayerStatusType::IntU21);

                // Reset ban flags for new season
                player.player_attributes.is_banned = false;

                // NOTE: Do NOT reset player.statistics here!
                // The season-end snapshot (snapshot_player_season_statistics) takes
                // stats via std::mem::take in on_season_end. If we reset here first,
                // the snapshot captures zeroed stats and the season's history is lost.

                // Reset days since last match (pre-season training counts)
                player.player_attributes.days_since_last_match = 7;
            }
        }
    }

    /// Graduate best academy players to U18 team (8-12 per year).
    /// Move overage youth players to main team.
    /// Aged-out academy players are released onto the global free-agent
    /// pool. Returns completed transfer records and the released
    /// player roster so the country processing layer can route them.
    pub(super) fn process_academy_graduations(
        &mut self,
        date: NaiveDate,
        country_code: &str,
    ) -> (Vec<CompletedTransfer>, Vec<Player>) {
        let mut transfers = Vec::new();
        let mut released_players: Vec<Player> = Vec::new();

        // Clean the youth squads FIRST: promote overage youth up the
        // progression (and into the main team) so room frees up before we
        // graduate. Without this, a nominally-full youth team would stall
        // academy throughput even when plenty of academy players are ready.
        self.rebalance_squads(date);

        // Find the lowest youth team to graduate into (U18 → U19 → U20 → U21 → U23)
        let youth_idx = TeamType::YOUTH_PROGRESSION
            .iter()
            .find_map(|tt| self.teams.index_of_type(*tt));

        // Graduate academy players BEFORE releasing aged-out ones, so 16+
        // year olds get a chance to graduate instead of being deleted.
        //
        // Throughput target (not just "top the squad up"): a healthy
        // academy ships 5-8 graduates a season, up to 12, plus 0-2 elite
        // overshoot, always bounded by the youth soft-max of 30. The
        // academy's `recommended_graduates` / `elite_overshoot_count`
        // helpers own the actual count so there's one place to tune it.
        if let Some(idx) = youth_idx {
            let youth_count = self.teams.teams[idx].players.len();
            let eligible_count = self.academy.graduation_candidates(date).len();
            let normal = self
                .academy
                .recommended_graduates(youth_count, eligible_count);
            let elite_overshoot = self.academy.elite_overshoot_count(date);
            let to_graduate = self
                .academy
                .graduation_ceiling(youth_count, normal, elite_overshoot);

            // Main team name for contract registration
            let main_team_name = self
                .teams
                .main()
                .map(|t| t.name.clone())
                .unwrap_or_else(|| self.name.clone());

            let youth_team_type = self.teams.teams[idx].team_type;
            let graduated = self.academy.graduate_to_youth(date, to_graduate);
            if !graduated.is_empty() {
                debug!(
                    "academy {}: {} players graduated (contract: {}, assigned: {:?}, was {})",
                    self.name,
                    graduated.len(),
                    main_team_name,
                    youth_team_type,
                    youth_count
                );
                for mut player in graduated {
                    // Assign native languages based on player's nationality
                    if player.languages.is_empty() {
                        player.languages = Language::from_country_code(country_code)
                            .into_iter()
                            .map(|lang| PlayerLanguage::native(lang))
                            .collect();
                    }

                    transfers.push(
                        CompletedTransfer::new(
                            player.id,
                            player.full_name.to_string(),
                            0,
                            0,
                            "Academy".to_string(),
                            self.id,
                            main_team_name.clone(),
                            date,
                            CurrencyValue::new(0.0, Currency::Usd),
                            TransferType::Free,
                        )
                        .with_reason("Academy graduation — youth contract signed".to_string()),
                    );
                    self.teams.teams[idx].players.add(player);
                }
            }
        }

        // Release aged-out academy players (18+) that were NOT graduated.
        // Each release records a free transfer event AND stamps the
        // player as `Frt` with a cleared contract so the global
        // free-agent pipeline picks them up — previously they exited
        // the academy but never reached the senior free-agent pool.
        let released = self.academy.release_aged_out_players(date);
        if !released.is_empty() {
            debug!(
                "academy {}: {} aged-out players released to free agents",
                self.name,
                released.len()
            );
            for player in released {
                // `release_aged_out_players` already cleared the
                // contract and stamped Frt; we only need to record
                // the transfer history line and surface the player.
                transfers.push(
                    CompletedTransfer::new(
                        player.id,
                        player.full_name.to_string(),
                        0,
                        0,
                        "Academy".to_string(),
                        0,
                        "Free Agents".to_string(),
                        date,
                        CurrencyValue::new(0.0, Currency::Usd),
                        TransferType::Free,
                    )
                    .with_reason("Academy aged-out release".to_string()),
                );
                released_players.push(player);
            }
        }

        // Rebalance: overage moves, talent promotions, backfill
        self.rebalance_squads(date);

        (transfers, released_players)
    }
}

/// Graduation salary: ability sets the tier, club reputation scales it.
/// A youth graduate at Man City earns 50x what the same ability player earns in Chad.
pub(super) fn graduation_salary(current_ability: u8, club_reputation: u16) -> u32 {
    let base = match current_ability {
        0..=60 => 2_000,
        61..=80 => 5_000,
        81..=100 => 12_000,
        101..=120 => 30_000,
        121..=150 => 80_000,
        _ => 200_000,
    };

    // Club reputation multiplier: cubic curve
    let norm = (club_reputation as f64 / 10000.0).clamp(0.0, 1.0);
    let multiplier = 0.10 + 2.90 * norm * norm * norm;

    (base as f64 * multiplier).max(500.0) as u32
}
