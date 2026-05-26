//! Per-player [`SquadSocialView`] snapshots.
//!
//! Pre-compute compatriot / shared-language teammate counts from the
//! current roster so the desire / adaptation pipelines see the actual
//! squad rather than always-zero defaults.

use crate::Player;
use crate::club::player::core::player::SquadSocialView;
use crate::club::player::language::Language;

/// Minimum proficiency (0..100) at which a non-native language counts as
/// a "chat-ready" shared language between teammates. Below this, comms
/// are too broken to support a meaningful social bond.
const SHARED_LANGUAGE_PROFICIENCY: u8 = 40;

/// Builds [`SquadSocialView`] snapshots for every player on the team
/// from the current roster. Wraps the O(n²) walk so the team simulator
/// reads as one named call rather than an inline nested loop.
pub struct SquadSocialViewBuilder;

impl SquadSocialViewBuilder {
    /// Refresh each player's `squad_social_view` from the current roster.
    /// O(n²) over the squad, with n ≤ 30 — cheap. Same-language counts
    /// use the player's stored `languages` (≥`SHARED_LANGUAGE_PROFICIENCY`
    /// or native qualifies as a chat-ready buddy); same-nationality counts
    /// compare `country_id`. Self is excluded from both counts.
    pub fn refresh(players: &mut [Player]) {
        let snapshot: Vec<(u32, u32, Vec<Language>)> = players
            .iter()
            .map(|p| (p.id, p.country_id, Self::chat_ready_languages(p)))
            .collect();

        for player in players.iter_mut() {
            let player_langs = Self::chat_ready_languages(player);
            let mut same_nat: u32 = 0;
            let mut same_lang: u32 = 0;
            for (other_id, other_country, other_langs) in &snapshot {
                if *other_id == player.id {
                    continue;
                }
                if *other_country == player.country_id {
                    same_nat += 1;
                }
                if player_langs.iter().any(|l| other_langs.contains(l)) {
                    same_lang += 1;
                }
            }
            player.squad_social_view = Some(SquadSocialView {
                same_nationality_teammates: same_nat.min(u8::MAX as u32) as u8,
                same_language_teammates: same_lang.min(u8::MAX as u32) as u8,
            });
        }
    }

    fn chat_ready_languages(p: &Player) -> Vec<Language> {
        p.languages
            .iter()
            .filter(|l| l.is_native || l.proficiency >= SHARED_LANGUAGE_PROFICIENCY)
            .map(|l| l.language)
            .collect()
    }
}
