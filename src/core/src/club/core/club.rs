use crate::TeamCollection;
use crate::club::academy::{AcademyDevelopmentIdentity, ClubAcademy};
use crate::club::board::ClubBoard;
use crate::club::board::vision::VisionYouthFocus;
use crate::club::facilities::ClubFacilities;
use crate::club::news::{ClubAffair, ClubAffairLog};
use crate::club::status::ClubStatus;
use crate::club::{ClubFinances, Player};
use crate::shared::Location;
use crate::transfers::market::knowledge::ClubMarketLedger;
use crate::transfers::pipeline::ClubTransferPlan;
use chrono::NaiveDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClubPhilosophy {
    /// Develop youth and sell for profit (Ajax, Benfica, Dortmund)
    DevelopAndSell,
    /// Sign established players, compete now (PSG, Chelsea, Man City)
    SignToCompete,
    /// Loan-heavy strategy, minimal spending (smaller clubs)
    LoanFocused,
    /// Balanced approach (most clubs)
    Balanced,
}

impl ClubPhilosophy {
    /// What the club is FOR, derived from the brief its board actually
    /// wrote and the academy it actually runs.
    ///
    /// Reputation alone mapped three of the four onto a ladder and never
    /// produced the fourth at all, so no club in the world traded players
    /// as a policy. Youth focus, academy standard and the academy's own
    /// identity are the signals that separate a Benfica from a Chelsea at
    /// the same reputation.
    pub fn derive(
        youth_focus: VisionYouthFocus,
        academy_tier: u8,
        identity: AcademyDevelopmentIdentity,
    ) -> Self {
        /// Academy standard at which a club can actually supply its own
        /// first team, and therefore has something to sell.
        const TRADING_ACADEMY_TIER: u8 = 6;

        // What the board wants, against whether the academy can supply
        // it. Standing says nothing on its own: a small club with a real
        // academy sells players, and a big one without still has to buy
        // or borrow them.
        let supplies_itself = academy_tier >= TRADING_ACADEMY_TIER;
        if matches!(identity, AcademyDevelopmentIdentity::PlayerTrading)
            || (matches!(youth_focus, VisionYouthFocus::DevelopYouth) && supplies_itself)
        {
            return ClubPhilosophy::DevelopAndSell;
        }
        match youth_focus {
            VisionYouthFocus::SignExperienced => ClubPhilosophy::SignToCompete,
            // It wants young players and cannot produce them, so it
            // borrows them.
            VisionYouthFocus::DevelopYouth => ClubPhilosophy::LoanFocused,
            VisionYouthFocus::Balanced => ClubPhilosophy::Balanced,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClubColors {
    pub background: String,
    pub foreground: String,
}

impl Default for ClubColors {
    fn default() -> Self {
        ClubColors {
            background: "#1e272d".to_string(),
            foreground: "#ffffff".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Club {
    pub id: u32,
    pub name: String,

    pub location: Location,

    pub board: ClubBoard,

    pub finance: ClubFinances,

    pub status: ClubStatus,

    pub academy: ClubAcademy,

    pub colors: ClubColors,

    pub teams: TeamCollection,

    pub transfer_plan: ClubTransferPlan,

    pub philosophy: ClubPhilosophy,
    /// One monthly review has said the club is no longer a trading one.
    /// A second has to agree before it stops being one: an academy that
    /// hovers on the line would otherwise flip the club's whole identity
    /// every month.
    pub philosophy_under_review: bool,

    pub facilities: ClubFacilities,

    pub rivals: Vec<u32>,

    /// Which foreign markets this club has actually done business in, and
    /// when. Half of what the club KNOWS of a market (the other half being
    /// its scouts) and the half that survives a scout leaving — see
    /// [`ClubMarketKnowledge`]. Bootstrapped at world load from the squad's
    /// own foreign nationalities, so the shipped world is its own evidence.
    pub market_ledger: ClubMarketLedger,

    /// The club's own diary: dated boardroom and dugout happenings the
    /// press cannot recompute from state. Written where each thing
    /// actually occurs — see [`ClubAffairLog`].
    pub affairs: ClubAffairLog,
}

impl Club {
    pub fn new(
        id: u32,
        name: String,
        location: Location,
        finance: ClubFinances,
        academy: ClubAcademy,
        status: ClubStatus,
        colors: ClubColors,
        teams: TeamCollection,
        facilities: ClubFacilities,
    ) -> Self {
        // The board writes its brief on the first tick, so the youth
        // focus is its default until then — but the academy is real from
        // world load, and it is what separates a trading club from a
        // buying one at the same standing.
        let philosophy = ClubPhilosophy::derive(
            VisionYouthFocus::Balanced,
            academy.tier().value(),
            academy.development_identity,
        );

        Club {
            id,
            name,
            location,
            finance,
            status,
            academy,
            colors,
            board: ClubBoard::new(),
            teams,
            transfer_plan: ClubTransferPlan::new(),
            philosophy,
            philosophy_under_review: false,
            facilities,
            rivals: Vec::new(),
            market_ledger: ClubMarketLedger::default(),
            affairs: ClubAffairLog::new(),
        }
    }

    pub fn is_rival(&self, other_club_id: u32) -> bool {
        self.rivals.contains(&other_club_id)
    }

    /// File a dated happening in the club's diary. The single entry
    /// point, so every writer records the date the same way and the
    /// press never has to guess when something occurred.
    pub fn record_affair(&mut self, affair: ClubAffair, date: NaiveDate) {
        self.affairs.record(affair, date);
    }

    /// Every force-selected player across the club, regardless of the
    /// team they're rostered on. Callers pass these straight to the
    /// squad selector as the first reserves so the +1000 selection
    /// bonus pins them into the match-day XI before the usual scoring
    /// logic decides anything else.
    pub fn get_force_selected_players(&self) -> Vec<&Player> {
        self.teams
            .teams
            .iter()
            .flat_map(|t| t.players.iter())
            .filter(|p| p.is_force_match_selection)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_philosophy_is_reachable_from_a_real_brief() {
        assert_eq!(
            ClubPhilosophy::derive(
                VisionYouthFocus::DevelopYouth,
                8,
                AcademyDevelopmentIdentity::Balanced,
            ),
            ClubPhilosophy::DevelopAndSell
        );
        assert_eq!(
            ClubPhilosophy::derive(
                VisionYouthFocus::Balanced,
                3,
                AcademyDevelopmentIdentity::PlayerTrading,
            ),
            ClubPhilosophy::DevelopAndSell,
            "a trading academy is a trading club at any standing"
        );
        assert_eq!(
            ClubPhilosophy::derive(
                VisionYouthFocus::SignExperienced,
                4,
                AcademyDevelopmentIdentity::Balanced,
            ),
            ClubPhilosophy::SignToCompete
        );
        assert_eq!(
            ClubPhilosophy::derive(
                VisionYouthFocus::Balanced,
                4,
                AcademyDevelopmentIdentity::Balanced,
            ),
            ClubPhilosophy::Balanced
        );
        assert_eq!(
            ClubPhilosophy::derive(
                VisionYouthFocus::DevelopYouth,
                2,
                AcademyDevelopmentIdentity::Balanced,
            ),
            ClubPhilosophy::LoanFocused,
            "it wants young players and its academy cannot produce them"
        );
    }

    #[test]
    fn a_club_with_no_stated_preference_is_not_thereby_a_loan_club() {
        for tier in [1u8, 4, 8] {
            assert_eq!(
                ClubPhilosophy::derive(
                    VisionYouthFocus::Balanced,
                    tier,
                    AcademyDevelopmentIdentity::Balanced,
                ),
                ClubPhilosophy::Balanced,
                "a club that has decided nothing is Balanced at academy tier {tier},                  whatever division it plays in — a catch-all here hands most of                  the world a financial-relief loan trigger it never asked for"
            );
        }
    }
}
