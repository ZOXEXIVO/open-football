//! One official-match ledger for promises and the manager's minutes plan.
use super::player::{ManagerPromise, ManagerPromiseKind, Player};

#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerUsage {
    pub eligible: u16,
    pub starts: u16,
    pub appearances: u16,
    pub minutes: u32,
    /// True for the club-spell ledger, false for cold legacy statistics.
    pub tracked: bool,
}

impl PlayerUsage {
    pub fn of(player: &Player) -> Self {
        let h = &player.happiness;
        if h.eligible_official_matches_since_join > 0 {
            return Self {
                eligible: h.eligible_official_matches_since_join,
                starts: h.starts_since_join,
                appearances: h.starts_since_join.saturating_add(h.sub_apps_since_join),
                minutes: h.official_minutes_since_join,
                tracked: true,
            };
        }
        // Imported season totals are only a cold-start fallback. The first
        // tracked fixture begins a new observation period; `since` handles
        // that transition without charging historical appearances twice.
        if player.last_transfer_date.is_some() {
            return Self::default();
        }
        let starts = player
            .statistics
            .played
            .saturating_add(player.cup_statistics.played);
        let apps = starts
            .saturating_add(player.statistics.played_subs)
            .saturating_add(player.cup_statistics.played_subs);
        Self {
            eligible: apps,
            starts,
            appearances: apps,
            minutes: u32::from(starts) * 90 + u32::from(apps - starts) * 20,
            tracked: false,
        }
    }

    pub fn since(self, baseline: Self) -> Self {
        if self.tracked && !baseline.tracked {
            return self;
        }
        Self {
            eligible: self.eligible.saturating_sub(baseline.eligible),
            starts: self.starts.saturating_sub(baseline.starts),
            appearances: self.appearances.saturating_sub(baseline.appearances),
            minutes: self.minutes.saturating_sub(baseline.minutes),
            tracked: self.tracked,
        }
    }
}

impl ManagerPromise {
    pub fn usage_since_promise(&self, usage: PlayerUsage) -> PlayerUsage {
        usage.since(PlayerUsage {
            eligible: self.baseline_eligible,
            starts: self.baseline_starts,
            appearances: self.baseline_apps,
            minutes: 0,
            tracked: self.baseline_tracked,
        })
    }

    /// Delivered / required involvement, shared by selection and verification.
    /// Starts are a share of eligible team fixtures, including omissions.
    pub fn involvement_progress(
        &self,
        usage: PlayerUsage,
        loan_target: Option<u16>,
    ) -> Option<(u16, u16)> {
        let days = (self.deadline - self.made_on).num_days().max(1) as u16;
        let progress = self.usage_since_promise(usage);
        let apps = progress.appearances;
        match self.kind {
            ManagerPromiseKind::StartingRole => {
                let eligible = progress.eligible;
                let share = if self.target_value == 0 {
                    60
                } else {
                    self.target_value.min(100)
                };
                let required = (u32::from(eligible) * u32::from(share)).div_ceil(100) as u16;
                Some((progress.starts, required.max(1)))
            }
            ManagerPromiseKind::PlayingTime => Some((
                apps,
                if self.target_value == 0 {
                    (days / 10).max(1)
                } else {
                    self.target_value
                },
            )),
            ManagerPromiseKind::LoanDevelopment => {
                // A spell-wide contract target is measured from arrival, not
                // charged again each time the manager makes a new assurance.
                let remaining_loan = loan_target.unwrap_or(0).saturating_sub(self.baseline_apps);
                let target = self.target_value.max(remaining_loan);
                Some((
                    apps,
                    if target == 0 {
                        (days / 14).max(1)
                    } else {
                        target
                    },
                ))
            }
            _ => None,
        }
    }
}
