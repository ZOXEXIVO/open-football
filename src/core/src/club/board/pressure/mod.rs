//! Supporter / media / dressing-room / financial / regulatory pressure.
//!
//! Pressure is a set of slow-moving 0-100 gauges that the board reads as
//! *inputs* to confidence and meetings — it colours decisions without
//! dictating them. A reckless rich owner shrugs off a fan revolt; a
//! member-owned club lurches at the first derby defeat. The owner's
//! `supporter_sensitivity` scales how much supporter/media heat actually
//! lands on board confidence.

use super::ownership::OwnershipType;

/// Discrete events that spike supporter / media pressure for a tick.
/// Surfaced by the season's narrative so the board reacts to *stories*,
/// not just the league table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupporterEvent {
    DerbyWin,
    DerbyLoss,
    InRelegationZone,
    InPromotionRace,
    SoldFanFavourite,
    SignedStar,
    LongWinlessRun,
    HumiliatingCupExit,
    YouthProspectBreakthrough,
}

impl SupporterEvent {
    /// Signed supporter-pressure delta. Positive = unhappy fans (more
    /// pressure); negative = delighted fans (pressure relief).
    pub fn supporter_delta(self) -> i16 {
        match self {
            SupporterEvent::DerbyWin => -12,
            SupporterEvent::DerbyLoss => 18,
            SupporterEvent::InRelegationZone => 14,
            SupporterEvent::InPromotionRace => -10,
            SupporterEvent::SoldFanFavourite => 16,
            SupporterEvent::SignedStar => -14,
            SupporterEvent::LongWinlessRun => 12,
            SupporterEvent::HumiliatingCupExit => 20,
            SupporterEvent::YouthProspectBreakthrough => -8,
        }
    }

    /// Media reaction tends to run hotter than the terraces on negatives.
    pub fn media_delta(self) -> i16 {
        let base = self.supporter_delta();
        if base > 0 {
            (base as f32 * 1.2) as i16
        } else {
            (base as f32 * 0.7) as i16
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BoardPressure {
    pub supporter_pressure: u8,
    pub media_pressure: u8,
    pub dressing_room_pressure: u8,
    pub financial_pressure: u8,
    pub regulatory_pressure: u8,
}

impl BoardPressure {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decay all gauges towards calm each tick before fresh events/inputs
    /// are applied — pressure fades if nothing keeps stoking it.
    pub fn decay(&mut self) {
        for g in [
            &mut self.supporter_pressure,
            &mut self.media_pressure,
            &mut self.dressing_room_pressure,
        ] {
            *g = g.saturating_sub(4);
        }
        // Financial / regulatory pressure is recomputed from hard numbers
        // each tick, so it decays faster (it'll be re-set if still true).
        self.financial_pressure = self.financial_pressure.saturating_sub(8);
        self.regulatory_pressure = self.regulatory_pressure.saturating_sub(8);
    }

    fn apply_signed(gauge: &mut u8, delta: i16) {
        let v = (*gauge as i16 + delta).clamp(0, 100);
        *gauge = v as u8;
    }

    /// Fold a discrete narrative event into supporter + media gauges.
    pub fn apply_event(&mut self, event: SupporterEvent) {
        Self::apply_signed(&mut self.supporter_pressure, event.supporter_delta());
        Self::apply_signed(&mut self.media_pressure, event.media_delta());
    }

    /// Set the hard-number gauges from finance / regulatory inputs.
    /// `wage_usage` is wage spend / budget; `ffp_breach`/`ffp_watch`
    /// flag regulatory standing; `debt_ratio` is debt / annual revenue.
    pub fn set_financial(&mut self, wage_usage: f32, debt_ratio: f32, profit_loss_12m: i64) {
        let mut fin = 0i16;
        if wage_usage > 1.1 {
            fin += 40;
        } else if wage_usage > 1.0 {
            fin += 25;
        } else if wage_usage > 0.9 {
            fin += 10;
        }
        if debt_ratio > 1.0 {
            fin += 35;
        } else if debt_ratio > 0.5 {
            fin += 18;
        }
        if profit_loss_12m < 0 {
            fin += 15;
        }
        self.financial_pressure = self.financial_pressure.max(fin.clamp(0, 100) as u8);
    }

    pub fn set_regulatory(&mut self, ffp_breach: bool, ffp_watch: bool) {
        let reg = if ffp_breach {
            70
        } else if ffp_watch {
            35
        } else {
            0
        };
        self.regulatory_pressure = self.regulatory_pressure.max(reg);
    }

    pub fn set_dressing_room(&mut self, key_player_unrest: u8) {
        let dr = (key_player_unrest as u16 * 18).min(100) as u8;
        self.dressing_room_pressure = self.dressing_room_pressure.max(dr);
    }

    /// Aggregate downward drag on board confidence this tick, after the
    /// owner's sensitivity to fan/media noise is applied. Hard-number
    /// pressure (financial/regulatory/dressing-room) always lands; only
    /// the supporter/media component is dampened by thick-skinned owners.
    pub fn confidence_drag(&self, owner: OwnershipType) -> i32 {
        let sens = owner.supporter_sensitivity();
        let soft = (self.supporter_pressure as f32 + self.media_pressure as f32) * 0.5 * sens;
        let hard = self.financial_pressure as f32 * 0.5
            + self.regulatory_pressure as f32 * 0.4
            + self.dressing_room_pressure as f32 * 0.35;
        // Scale the 0..~180 raw into a modest confidence drag.
        ((soft + hard) / 12.0).round() as i32
    }

    /// Whether pressure alone is high enough to force a board meeting even
    /// when raw results are acceptable.
    pub fn demands_meeting(&self, owner: OwnershipType) -> bool {
        let sens = owner.supporter_sensitivity();
        let fan_heat = (self.supporter_pressure as f32 + self.media_pressure as f32) * 0.5 * sens;
        fan_heat >= 60.0 || self.regulatory_pressure >= 70 || self.financial_pressure >= 70
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derby_loss_raises_pressure_then_decays() {
        let mut p = BoardPressure::new();
        p.apply_event(SupporterEvent::DerbyLoss);
        let after = p.supporter_pressure;
        assert!(after > 0);
        p.decay();
        assert!(p.supporter_pressure < after);
    }

    #[test]
    fn member_owned_feels_fans_more_than_state_backed() {
        let mut p = BoardPressure::new();
        p.supporter_pressure = 80;
        p.media_pressure = 80;
        let member = p.confidence_drag(OwnershipType::MemberOwned);
        let state = p.confidence_drag(OwnershipType::StateBacked);
        assert!(
            member > state,
            "member-owned should feel fan pressure harder: {member} vs {state}"
        );
    }

    #[test]
    fn ffp_breach_maxes_regulatory_pressure() {
        let mut p = BoardPressure::new();
        p.set_regulatory(true, false);
        assert_eq!(p.regulatory_pressure, 70);
    }

    #[test]
    fn signing_a_star_relieves_supporters() {
        let mut p = BoardPressure::new();
        p.supporter_pressure = 50;
        p.apply_event(SupporterEvent::SignedStar);
        assert!(p.supporter_pressure < 50);
    }
}
