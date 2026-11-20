//! Weeks without work: what a body that is not used gives back, and how
//! quickly work returns it.

use crate::club::player::maturation::MaturationGroup;

/// An injured player, a free agent and a player whose club holds no session
/// are the same case: weeks in which no session and no match arrived. After
/// four of them his physical and explosive families give back a little of
/// their height each week, to at most `MAX_GIVEN`; once work resumes it is
/// repaid at several times the rate it was first built.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Layoff {
    idle_weeks: u8,
    worked: bool,
    given_back: [f32; 2],
}

impl Layoff {
    /// Weeks without work before a body starts to give back.
    const IDLE_WEEKS: u8 = 4;
    /// Share of a family's height given back in a week once laid off.
    pub(super) const STEP: f32 = 0.006;
    /// The most a lay-off takes: a share of the height.
    pub(super) const MAX_GIVEN: f32 = 0.06;
    /// Share repaid in a week of work.
    pub(super) const REPAY: f32 = 0.02;

    pub(super) fn on_work(&mut self) {
        self.worked = true;
    }

    /// Close a week, saying whether any work arrived in it.
    pub(super) fn close_week(&mut self) -> bool {
        let worked = self.worked;
        self.worked = false;
        self.idle_weeks = if worked {
            0
        } else {
            self.idle_weeks.saturating_add(1)
        };
        worked
    }

    pub fn laid_off(&self) -> bool {
        self.idle_weeks > Self::IDLE_WEEKS
    }

    pub fn idle_weeks(&self) -> u8 {
        self.idle_weeks
    }

    /// The share of `family`'s height given back and not yet repaid.
    pub fn given_back(&self, family: MaturationGroup) -> f32 {
        Self::slot(family).map_or(0.0, |s| self.given_back[s])
    }

    pub(super) fn record(&mut self, family: MaturationGroup, change: f32) {
        if let Some(s) = Self::slot(family) {
            self.given_back[s] = (self.given_back[s] + change).clamp(0.0, Self::MAX_GIVEN);
        }
    }

    pub(super) fn slot(family: MaturationGroup) -> Option<usize> {
        match family {
            MaturationGroup::Physical => Some(0),
            MaturationGroup::Explosive => Some(1),
            _ => None,
        }
    }
}
