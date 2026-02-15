#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjurySeverity {
    Minor,
    Moderate,
    Severe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjuryType {
    // Minor (1-7 days)
    Bruise,
    MinorKnock,
    Cramp,
    // Moderate (7-28 days)
    HamstringStrain,
    CalfStrain,
    AnkleSprain,
    GroinStrain,
    // Severe (30-270 days)
    ACLTear,
    BrokenLeg,
    TornMeniscus,
    ShoulderDislocation,
}

impl InjuryType {
    /// Returns (min_days, max_days) for this injury type
    pub fn duration_range(&self) -> (u16, u16) {
        match self {
            InjuryType::Bruise => (3, 7),
            InjuryType::MinorKnock => (1, 3),
            InjuryType::Cramp => (1, 2),
            InjuryType::HamstringStrain => (14, 28),
            InjuryType::CalfStrain => (10, 21),
            InjuryType::AnkleSprain => (7, 21),
            InjuryType::GroinStrain => (14, 28),
            InjuryType::ACLTear => (180, 270),
            InjuryType::BrokenLeg => (90, 180),
            InjuryType::TornMeniscus => (60, 120),
            InjuryType::ShoulderDislocation => (30, 60),
        }
    }

    pub fn severity(&self) -> InjurySeverity {
        match self {
            InjuryType::Bruise | InjuryType::MinorKnock | InjuryType::Cramp => {
                InjurySeverity::Minor
            }
            InjuryType::HamstringStrain
            | InjuryType::CalfStrain
            | InjuryType::AnkleSprain
            | InjuryType::GroinStrain => InjurySeverity::Moderate,
            InjuryType::ACLTear
            | InjuryType::BrokenLeg
            | InjuryType::TornMeniscus
            | InjuryType::ShoulderDislocation => InjurySeverity::Severe,
        }
    }

    /// Generate a random duration within this injury's range
    pub fn random_duration(&self) -> u16 {
        let (min, max) = self.duration_range();
        let range = max - min + 1;
        min + (rand::random::<u16>() % range)
    }

    /// Pick a random training injury (weighted towards minor/moderate muscle injuries)
    pub fn random_training_injury(age: u8, condition_pct: u32, natural_fitness: f32) -> InjuryType {
        let roll: f32 = rand::random::<f32>();

        // Older players and those with lower fitness get more severe injuries
        let severity_modifier = (age as f32 - 25.0).max(0.0) * 0.01
            + (100.0 - condition_pct as f32) * 0.001
            + (20.0 - natural_fitness).max(0.0) * 0.005;

        let adjusted_roll = roll + severity_modifier;

        if adjusted_roll < 0.50 {
            // 50% minor
            match rand::random::<u8>() % 3 {
                0 => InjuryType::Cramp,
                1 => InjuryType::MinorKnock,
                _ => InjuryType::Bruise,
            }
        } else if adjusted_roll < 0.92 {
            // 42% moderate (training injuries are typically muscle strains)
            match rand::random::<u8>() % 4 {
                0 => InjuryType::HamstringStrain,
                1 => InjuryType::CalfStrain,
                2 => InjuryType::GroinStrain,
                _ => InjuryType::AnkleSprain,
            }
        } else {
            // 8% severe
            match rand::random::<u8>() % 4 {
                0 => InjuryType::ACLTear,
                1 => InjuryType::TornMeniscus,
                2 => InjuryType::ShoulderDislocation,
                _ => InjuryType::BrokenLeg,
            }
        }
    }

    /// Pick a random daily spontaneous injury
    pub fn random_spontaneous_injury() -> InjuryType {
        let roll: f32 = rand::random::<f32>();

        if roll < 0.60 {
            match rand::random::<u8>() % 3 {
                0 => InjuryType::MinorKnock,
                1 => InjuryType::Cramp,
                _ => InjuryType::Bruise,
            }
        } else if roll < 0.90 {
            match rand::random::<u8>() % 4 {
                0 => InjuryType::HamstringStrain,
                1 => InjuryType::CalfStrain,
                2 => InjuryType::AnkleSprain,
                _ => InjuryType::GroinStrain,
            }
        } else {
            match rand::random::<u8>() % 3 {
                0 => InjuryType::TornMeniscus,
                1 => InjuryType::ShoulderDislocation,
                _ => InjuryType::ACLTear,
            }
        }
    }
}

impl std::fmt::Display for InjuryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InjuryType::Bruise => write!(f, "Bruise"),
            InjuryType::MinorKnock => write!(f, "Minor Knock"),
            InjuryType::Cramp => write!(f, "Cramp"),
            InjuryType::HamstringStrain => write!(f, "Hamstring Strain"),
            InjuryType::CalfStrain => write!(f, "Calf Strain"),
            InjuryType::AnkleSprain => write!(f, "Ankle Sprain"),
            InjuryType::GroinStrain => write!(f, "Groin Strain"),
            InjuryType::ACLTear => write!(f, "ACL Tear"),
            InjuryType::BrokenLeg => write!(f, "Broken Leg"),
            InjuryType::TornMeniscus => write!(f, "Torn Meniscus"),
            InjuryType::ShoulderDislocation => write!(f, "Shoulder Dislocation"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_injury_duration_ranges() {
        assert_eq!(InjuryType::Bruise.duration_range(), (3, 7));
        assert_eq!(InjuryType::MinorKnock.duration_range(), (1, 3));
        assert_eq!(InjuryType::Cramp.duration_range(), (1, 2));
        assert_eq!(InjuryType::HamstringStrain.duration_range(), (14, 28));
        assert_eq!(InjuryType::ACLTear.duration_range(), (180, 270));
        assert_eq!(InjuryType::BrokenLeg.duration_range(), (90, 180));
    }

    #[test]
    fn test_injury_severity() {
        assert_eq!(InjuryType::Cramp.severity(), InjurySeverity::Minor);
        assert_eq!(InjuryType::Bruise.severity(), InjurySeverity::Minor);
        assert_eq!(
            InjuryType::HamstringStrain.severity(),
            InjurySeverity::Moderate
        );
        assert_eq!(InjuryType::CalfStrain.severity(), InjurySeverity::Moderate);
        assert_eq!(InjuryType::ACLTear.severity(), InjurySeverity::Severe);
        assert_eq!(InjuryType::BrokenLeg.severity(), InjurySeverity::Severe);
    }

    #[test]
    fn test_random_duration_in_range() {
        for _ in 0..100 {
            let duration = InjuryType::Bruise.random_duration();
            assert!(duration >= 3 && duration <= 7);
        }
        for _ in 0..100 {
            let duration = InjuryType::ACLTear.random_duration();
            assert!(duration >= 180 && duration <= 270);
        }
    }

    #[test]
    fn test_injury_recovery_countdown() {
        let injury = InjuryType::MinorKnock;
        let mut days = injury.random_duration();
        assert!(days >= 1 && days <= 3);

        while days > 0 {
            days -= 1;
        }
        assert_eq!(days, 0);
    }
}
