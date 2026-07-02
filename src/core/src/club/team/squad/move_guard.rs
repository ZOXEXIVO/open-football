use crate::club::{PlayerFieldPositionGroup, PlayerPositionType, PlayerSquadStatus};
use crate::utils::DateUtils;
use crate::{Player, PlayerStatusType, Team};
use chrono::NaiveDate;
use log::debug;

/// Minimum match condition (%) for a teammate to count as realistic cover —
/// "available or close to available". A walking-wounded teammate below this
/// can't credibly replace the player we're proposing to demote.
const COVER_MIN_CONDITION: u32 = 30;
/// A cover candidate may be at most this many current-ability points below the
/// player — beyond it he is "clearly far below the demoted player's level".
const COVER_CA_MARGIN: i32 = 25;
/// Youngest age that counts as senior cover for a first-team role.
const COVER_MIN_SENIOR_AGE: u8 = 18;
/// Same-group (but not exact-position) cover must be at least this strong in
/// its own primary role to count as a credible tactical replacement, rather
/// than a desperate out-of-position fill.
const SAME_GROUP_COVER_MIN_LEVEL: u8 = 14;

/// Centralised guard for Main → reserve/youth demotions. Encodes the
/// **want-away invariant in code** (not just in LLM prompt text): a contracted
/// `Lst` / `Req` / `Unh` player who is still useful remains match-selectable
/// unless there is a real football/administrative reason — he is surplus, has a
/// serious discipline/absence issue, or has genuine positional cover — to move
/// him out of the senior group.
///
/// Used by the internal squad-move path that can demote a Main player: the
/// daily administrative sweep (`SquadManager::manage_critical_moves`).
/// Centralising it means no squad move can bypass the invariant.
pub(crate) struct MainSquadMoveGuard<'a> {
    main_team: &'a Team,
    date: NaiveDate,
}

impl<'a> MainSquadMoveGuard<'a> {
    pub(crate) fn new(main_team: &'a Team, date: NaiveDate) -> Self {
        MainSquadMoveGuard { main_team, date }
    }

    /// Whether `player` may be demoted from the Main squad. `reason` is the
    /// caller's human-readable justification, used only for diagnostics.
    pub(crate) fn allow_demote_from_main(&self, player: &Player, reason: &str) -> bool {
        // Manager-pinned players are never moved administratively.
        if player.is_force_match_selection {
            return false;
        }
        // A serious discipline / unauthorised-absence case can always be moved
        // out; the squad-size guard is the remaining backstop.
        if Self::has_serious_discipline_issue(player) {
            return true;
        }

        let want_away = Self::is_want_away(player);
        let surplus = self.is_surplus(player);

        if want_away && !surplus {
            // Want-away INVARIANT: a useful listed/requested/unhappy player stays
            // unless there is genuine, importance-scaled cover for his role.
            let ok = self.has_required_cover(player);
            if !ok {
                debug!(
                    "MainSquadMoveGuard kept useful want-away player {} on Main \
                     (reason='{}', insufficient cover)",
                    player.id, reason
                );
            }
            return ok;
        }

        // Surplus or non-want-away players: only veto a move that would strip the
        // position of its last credible senior cover (especially the last keeper).
        let ok = self.has_minimum_cover(player);
        if !ok {
            debug!(
                "MainSquadMoveGuard blocked unit-stripping demote of {} (reason='{}')",
                player.id, reason
            );
        }
        ok
    }

    /// Carries a market status the invariant protects (`Lst`/`Req`/`Unh`).
    /// Loan-listing (`Loa`) is excluded — loaning a player out for development
    /// is itself a reason to move him. Exposed for the daily sweep's trigger.
    pub(crate) fn is_want_away(player: &Player) -> bool {
        let s = player.statuses.get();
        s.contains(&PlayerStatusType::Lst)
            || s.contains(&PlayerStatusType::Req)
            || s.contains(&PlayerStatusType::Unh)
    }

    /// Clearly surplus: the club's own squad-status view says NotNeeded/Invalid,
    /// or the player sits in the bottom fifth of the Main squad by current
    /// ability (not first-team quality relative to the squad).
    pub(crate) fn is_surplus(&self, player: &Player) -> bool {
        let status_surplus = player
            .contract
            .as_ref()
            .map(|c| {
                matches!(
                    c.squad_status,
                    PlayerSquadStatus::NotNeeded | PlayerSquadStatus::Invalid
                )
            })
            .unwrap_or(false);
        status_surplus || self.ability_percentile(player) > 0.80
    }

    /// Essential: KeyPlayer/FirstTeamRegular, or top-quartile by current ability.
    /// Essential players need stronger cover before any demotion is permitted.
    fn is_essential(&self, player: &Player) -> bool {
        let status_essential = player
            .contract
            .as_ref()
            .map(|c| {
                matches!(
                    c.squad_status,
                    PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
                )
            })
            .unwrap_or(false);
        status_essential || self.ability_percentile(player) < 0.25
    }

    /// Fraction of Main-squad team-mates stronger than `player` by current
    /// ability (0.0 = best in squad, →1.0 = weakest).
    fn ability_percentile(&self, player: &Player) -> f32 {
        let ca = player.player_attributes.current_ability;
        let total = self
            .main_team
            .players
            .players
            .iter()
            .filter(|o| o.id != player.id)
            .count();
        if total == 0 {
            return 0.0;
        }
        let stronger = self
            .main_team
            .players
            .players
            .iter()
            .filter(|o| o.id != player.id)
            .filter(|o| o.player_attributes.current_ability > ca)
            .count();
        stronger as f32 / total as f32
    }

    fn has_serious_discipline_issue(player: &Player) -> bool {
        // Unauthorised absence (`Abs`) is the unambiguous disciplinary signal.
        player.statuses.get().contains(&PlayerStatusType::Abs)
    }

    /// Importance-scaled cover requirement for a useful want-away player.
    fn has_required_cover(&self, player: &Player) -> bool {
        if player.positions.is_goalkeeper() {
            // Don't leave fewer than two usable senior keepers behind a
            // non-surplus keeper.
            return self.usable_keeper_count(player) >= 2;
        }
        let need = if self.is_essential(player) { 2 } else { 1 };
        self.credible_cover_count(player) >= need
    }

    /// Absolute floor: never strip a position of its last credible senior cover.
    fn has_minimum_cover(&self, player: &Player) -> bool {
        if player.positions.is_goalkeeper() {
            return self.usable_keeper_count(player) >= 1;
        }
        self.credible_cover_count(player) >= 1
    }

    /// Other usable, senior keepers in the Main squad (no quality band — a body
    /// in goal is a body in goal).
    fn usable_keeper_count(&self, player: &Player) -> usize {
        self.main_team
            .players
            .players
            .iter()
            .filter(|o| o.id != player.id)
            .filter(|o| o.positions.is_goalkeeper())
            .filter(|o| self.is_usable_now(o))
            .filter(|o| !Self::is_leaving(o))
            .filter(|o| self.is_senior(o))
            .count()
    }

    /// Credible replacements for `player` in his primary role: usable now, not
    /// themselves leaving, senior, tactically compatible with his primary
    /// position (not merely the broad group), and within a sensible quality band.
    fn credible_cover_count(&self, player: &Player) -> usize {
        let role = player.position();
        let group = role.position_group();
        let player_ca = player.player_attributes.current_ability as i32;
        self.main_team
            .players
            .players
            .iter()
            .filter(|o| o.id != player.id)
            .filter(|o| self.is_usable_now(o))
            .filter(|o| !Self::is_leaving(o))
            .filter(|o| self.is_senior(o))
            .filter(|o| Self::role_compatible(o, role, group))
            .filter(|o| (o.player_attributes.current_ability as i32) + COVER_CA_MARGIN >= player_ca)
            .count()
    }

    /// Physically/legally able to play, or close to it — the cover candidate
    /// could actually step in. Mirrors the hard match-availability gate plus a
    /// "close to available" condition floor and a no-recovering rule.
    fn is_usable_now(&self, player: &Player) -> bool {
        !player.player_attributes.is_injured
            && !player.player_attributes.is_banned
            && !player.player_attributes.is_in_recovery()
            && !player.statuses.is_on_international_duty()
            && player.player_attributes.condition_percentage() >= COVER_MIN_CONDITION
    }

    /// The candidate is himself on his way out (`Trn`/`Bid`) and so can't be
    /// counted on as lasting cover.
    fn is_leaving(player: &Player) -> bool {
        let s = player.statuses.get();
        s.contains(&PlayerStatusType::Trn) || s.contains(&PlayerStatusType::Bid)
    }

    fn is_senior(&self, player: &Player) -> bool {
        DateUtils::age(player.birth_date, self.date) >= COVER_MIN_SENIOR_AGE
    }

    fn role_compatible(
        other: &Player,
        role: PlayerPositionType,
        group: PlayerFieldPositionGroup,
    ) -> bool {
        // Exact-position cover is the strongest signal.
        if other.positions.get_level(role) > 0 {
            return true;
        }
        // A genuine same-unit player (strong in his own primary role) is credible
        // cover; a weak out-of-position option is not.
        if other.position().position_group() == group {
            let primary = other
                .positions
                .positions
                .iter()
                .map(|p| p.level)
                .max()
                .unwrap_or(0);
            return primary >= SAME_GROUP_COVER_MIN_LEVEL;
        }
        false
    }
}
