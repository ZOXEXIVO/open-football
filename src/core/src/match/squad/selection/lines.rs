use crate::club::PlayerPositionType;
use crate::{Player, Tactics};

/// The five lines of an outfield, as a bench covers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PitchLine {
    CentralDefence,
    WideDefence,
    CentralMidfield,
    WideAttack,
    Forward,
}

impl PitchLine {
    /// A substitute is cover for a line he plays at this familiarity or
    /// better, on the 1..20 scale: a man the coach would send on there.
    const COVER_LEVEL: u8 = 14;

    pub fn of(position: PlayerPositionType) -> Option<PitchLine> {
        use PlayerPositionType::*;
        match position {
            Goalkeeper => None,
            Sweeper | DefenderCenterLeft | DefenderCenter | DefenderCenterRight => {
                Some(PitchLine::CentralDefence)
            }
            DefenderLeft | DefenderRight | WingbackLeft | WingbackRight => {
                Some(PitchLine::WideDefence)
            }
            DefensiveMidfielder
            | MidfielderCenterLeft
            | MidfielderCenter
            | MidfielderCenterRight => Some(PitchLine::CentralMidfield),
            MidfielderLeft
            | MidfielderRight
            | AttackingMidfielderLeft
            | AttackingMidfielderCenter
            | AttackingMidfielderRight => Some(PitchLine::WideAttack),
            Striker | ForwardLeft | ForwardCenter | ForwardRight => Some(PitchLine::Forward),
        }
    }

    /// The lines the side's shape puts on the pitch, back to front.
    pub fn on_the_pitch(tactics: &Tactics) -> Vec<PitchLine> {
        let mut lines: Vec<PitchLine> = Vec::with_capacity(5);
        for line in tactics
            .positions()
            .iter()
            .filter_map(|slot| Self::of(*slot))
        {
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
        lines
    }

    pub fn is_covered_by(self, player: &Player) -> bool {
        player
            .positions
            .positions
            .iter()
            .any(|p| p.level >= Self::COVER_LEVEL && Self::of(p.position) == Some(self))
    }
}
