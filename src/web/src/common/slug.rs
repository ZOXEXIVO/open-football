use crate::{ApiError, ApiResult};
use axum::response::{IntoResponse, Redirect, Response};
use core::shared::fullname::slug_from_display;
use core::{Player, SimulatorData, Team};

pub fn parse_slug_id(slug: &str) -> Option<u32> {
    let digits: String = slug.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Build a `{id}-{name}` URL segment for a historical transfer / loan record
/// where only the stored `player_name` string is available. Tries to resolve
/// the current `Player` first so the slug matches the canonical one that the
/// player page would produce; falls back to slugifying the stored name when
/// the player has been evicted from indexes (retired + old enough, etc.).
/// Final fallback is the bare numeric id — still valid; it 301-redirects to
/// the canonical URL on click.
pub fn player_history_slug(data: &SimulatorData, id: u32, stored_name: &str) -> String {
    if let Some((player, _)) = data.player_with_team(id) {
        return player.slug();
    }
    if let Some(player) = data.retired_player(id) {
        return player.slug();
    }
    let name_slug = slug_from_display(stored_name);
    if name_slug.is_empty() {
        id.to_string()
    } else {
        format!("{}-{}", id, name_slug)
    }
}

pub enum PlayerPage<'a> {
    Found {
        player: &'a Player,
        team: Option<&'a Team>,
        canonical_slug: String,
    },
    Redirect(Response),
}

/// Resolve the `{player_slug}` path segment for any player-scoped page.
///
/// Returns `PlayerPage::Found` when the incoming slug matches the canonical
/// `{id}-{name}` form — handlers then render normally. Returns
/// `PlayerPage::Redirect` with a 301 when the player exists but the slug is
/// stale / missing / wrong, so legacy `/players/{id}` URLs redirect to the
/// full canonical link without handlers needing per-page redirect logic.
///
/// `subpath` is the suffix after the slug, e.g. `""` for the overview page
/// or `"/contract"` for the contract tab — it's appended to the canonical
/// URL so the user lands on the same tab.
pub fn resolve_player_page<'a>(
    data: &'a SimulatorData,
    slug: &str,
    lang: &str,
    subpath: &str,
) -> ApiResult<PlayerPage<'a>> {
    let player_id = parse_slug_id(slug)
        .ok_or_else(|| ApiError::NotFound(format!("Player slug {} is malformed", slug)))?;

    let (player, team) = if let Some((p, t)) = data.player_with_team(player_id) {
        (p, Some(t))
    } else if let Some(p) = data.retired_player(player_id) {
        (p, None)
    } else {
        return Err(ApiError::NotFound(format!(
            "Player with ID {} not found",
            player_id
        )));
    };

    let canonical_slug = player.slug();
    if slug != canonical_slug {
        let url = format!("/{}/players/{}{}", lang, canonical_slug, subpath);
        return Ok(PlayerPage::Redirect(
            Redirect::permanent(&url).into_response(),
        ));
    }

    Ok(PlayerPage::Found {
        player,
        team,
        canonical_slug,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_slug_id_extracts_leading_digits() {
        assert_eq!(parse_slug_id("67200923"), Some(67200923));
        assert_eq!(parse_slug_id("67200923-unai-simon"), Some(67200923));
        assert_eq!(parse_slug_id("67200923-"), Some(67200923));
        assert_eq!(parse_slug_id("abc"), None);
        assert_eq!(parse_slug_id(""), None);
    }
}
