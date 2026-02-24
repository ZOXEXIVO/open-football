#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjurySeverity {
    Minor,
    Moderate,
    Severe,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyPart {
    Hamstring = 1,
    Knee = 2,
    Ankle = 3,
    Calf = 4,
    Groin = 5,
    Shoulder = 6,
    Foot = 7,
    Back = 8,
    Hip = 9,
    Head = 10,
    Quad = 11,
    Shin = 12,
}

impl BodyPart {
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(val: u8) -> Option<BodyPart> {
        match val {
            1 => Some(BodyPart::Hamstring),
            2 => Some(BodyPart::Knee),
            3 => Some(BodyPart::Ankle),
            4 => Some(BodyPart::Calf),
            5 => Some(BodyPart::Groin),
            6 => Some(BodyPart::Shoulder),
            7 => Some(BodyPart::Foot),
            8 => Some(BodyPart::Back),
            9 => Some(BodyPart::Hip),
            10 => Some(BodyPart::Head),
            11 => Some(BodyPart::Quad),
            12 => Some(BodyPart::Shin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjuryType {
    // Minor (3-10 days)
    Bruise,
    MinorKnock,
    Cramp,
    DeadLeg,
    MinorConcussion,
    // Moderate (14-42 days)
    HamstringStrain,
    CalfStrain,
    AnkleSprain,
    GroinStrain,
    HipFlexorStrain,
    QuadStrain,
    BackSpasm,
    // Severe (60-180 days)
    TornMeniscus,
    ShoulderDislocation,
    StressFracture,
    MCLSprain,
    LateralLigament,
    HerniatedDisc,
    // Critical (180-300 days)
    ACLTear,
    BrokenLeg,
    AchillesRupture,
    PCLTear,
}

impl InjuryType {
    /// Returns (min_days, max_days) for this injury type
    pub fn duration_range(&self) -> (u16, u16) {
        match self {
            // Minor: 3-10 days
            InjuryType::Bruise => (3, 7),
            InjuryType::MinorKnock => (3, 8),
            InjuryType::Cramp => (3, 6),
            InjuryType::DeadLeg => (4, 10),
            InjuryType::MinorConcussion => (5, 10),
            // Moderate: 14-42 days
            InjuryType::HamstringStrain => (14, 35),
            InjuryType::CalfStrain => (14, 28),
            InjuryType::AnkleSprain => (14, 42),
            InjuryType::GroinStrain => (14, 35),
            InjuryType::HipFlexorStrain => (14, 28),
            InjuryType::QuadStrain => (14, 35),
            InjuryType::BackSpasm => (14, 28),
            // Severe: 60-180 days
            InjuryType::TornMeniscus => (60, 120),
            InjuryType::ShoulderDislocation => (60, 90),
            InjuryType::StressFracture => (60, 150),
            InjuryType::MCLSprain => (60, 120),
            InjuryType::LateralLigament => (60, 150),
            InjuryType::HerniatedDisc => (90, 180),
            // Critical: 180-300 days
            InjuryType::ACLTear => (180, 300),
            InjuryType::BrokenLeg => (180, 270),
            InjuryType::AchillesRupture => (180, 300),
            InjuryType::PCLTear => (200, 300),
        }
    }

    pub fn severity(&self) -> InjurySeverity {
        match self {
            InjuryType::Bruise
            | InjuryType::MinorKnock
            | InjuryType::Cramp
            | InjuryType::DeadLeg
            | InjuryType::MinorConcussion => InjurySeverity::Minor,

            InjuryType::HamstringStrain
            | InjuryType::CalfStrain
            | InjuryType::AnkleSprain
            | InjuryType::GroinStrain
            | InjuryType::HipFlexorStrain
            | InjuryType::QuadStrain
            | InjuryType::BackSpasm => InjurySeverity::Moderate,

            InjuryType::TornMeniscus
            | InjuryType::ShoulderDislocation
            | InjuryType::StressFracture
            | InjuryType::MCLSprain
            | InjuryType::LateralLigament
            | InjuryType::HerniatedDisc => InjurySeverity::Severe,

            InjuryType::ACLTear
            | InjuryType::BrokenLeg
            | InjuryType::AchillesRupture
            | InjuryType::PCLTear => InjurySeverity::Critical,
        }
    }

    pub fn body_part(&self) -> BodyPart {
        match self {
            InjuryType::Bruise => BodyPart::Shin,
            InjuryType::MinorKnock => BodyPart::Knee,
            InjuryType::Cramp => BodyPart::Calf,
            InjuryType::DeadLeg => BodyPart::Quad,
            InjuryType::MinorConcussion => BodyPart::Head,
            InjuryType::HamstringStrain => BodyPart::Hamstring,
            InjuryType::CalfStrain => BodyPart::Calf,
            InjuryType::AnkleSprain => BodyPart::Ankle,
            InjuryType::GroinStrain => BodyPart::Groin,
            InjuryType::HipFlexorStrain => BodyPart::Hip,
            InjuryType::QuadStrain => BodyPart::Quad,
            InjuryType::BackSpasm => BodyPart::Back,
            InjuryType::TornMeniscus => BodyPart::Knee,
            InjuryType::ShoulderDislocation => BodyPart::Shoulder,
            InjuryType::StressFracture => BodyPart::Foot,
            InjuryType::MCLSprain => BodyPart::Knee,
            InjuryType::LateralLigament => BodyPart::Ankle,
            InjuryType::HerniatedDisc => BodyPart::Back,
            InjuryType::ACLTear => BodyPart::Knee,
            InjuryType::BrokenLeg => BodyPart::Shin,
            InjuryType::AchillesRupture => BodyPart::Ankle,
            InjuryType::PCLTear => BodyPart::Knee,
        }
    }

    /// Recovery days after injury heals (low match fitness phase)
    pub fn recovery_days(&self) -> u16 {
        let (min, max) = match self.severity() {
            InjurySeverity::Minor => (3, 5),
            InjurySeverity::Moderate => (7, 14),
            InjurySeverity::Severe => (14, 30),
            InjurySeverity::Critical => (21, 30),
        };
        let range = max - min + 1;
        min + (rand::random::<u16>() % range)
    }

    /// Generate a random duration within this injury's range
    pub fn random_duration(&self) -> u16 {
        let (min, max) = self.duration_range();
        let range = max - min + 1;
        min + (rand::random::<u16>() % range)
    }

    /// Pick a random match injury — weighted toward contact/muscle injuries
    pub fn random_match_injury(
        minutes_played: f32,
        age: u8,
        condition_pct: u32,
        natural_fitness: f32,
        injury_proneness: u8,
    ) -> InjuryType {
        let roll: f32 = rand::random::<f32>();

        // Older players, low fitness, low condition, high proneness → more severe
        let severity_modifier = (age as f32 - 25.0).max(0.0) * 0.008
            + (100.0 - condition_pct as f32) * 0.001
            + (20.0 - natural_fitness).max(0.0) * 0.005
            + (injury_proneness as f32 - 10.0).max(0.0) * 0.005
            + (minutes_played / 90.0 - 0.5).max(0.0) * 0.02;

        let adjusted_roll = roll + severity_modifier;

        if adjusted_roll < 0.45 {
            // 45% minor
            match rand::random::<u8>() % 5 {
                0 => InjuryType::Cramp,
                1 => InjuryType::MinorKnock,
                2 => InjuryType::Bruise,
                3 => InjuryType::DeadLeg,
                _ => InjuryType::MinorConcussion,
            }
        } else if adjusted_roll < 0.85 {
            // 40% moderate — heavily weighted toward muscle injuries in matches
            match rand::random::<u8>() % 7 {
                0 => InjuryType::HamstringStrain,
                1 => InjuryType::CalfStrain,
                2 => InjuryType::AnkleSprain,
                3 => InjuryType::GroinStrain,
                4 => InjuryType::HipFlexorStrain,
                5 => InjuryType::QuadStrain,
                _ => InjuryType::BackSpasm,
            }
        } else if adjusted_roll < 0.96 {
            // 11% severe
            match rand::random::<u8>() % 6 {
                0 => InjuryType::TornMeniscus,
                1 => InjuryType::ShoulderDislocation,
                2 => InjuryType::StressFracture,
                3 => InjuryType::MCLSprain,
                4 => InjuryType::LateralLigament,
                _ => InjuryType::HerniatedDisc,
            }
        } else {
            // 4% critical
            match rand::random::<u8>() % 4 {
                0 => InjuryType::ACLTear,
                1 => InjuryType::BrokenLeg,
                2 => InjuryType::AchillesRupture,
                _ => InjuryType::PCLTear,
            }
        }
    }

    /// Pick a random training injury (weighted towards minor/moderate muscle injuries)
    pub fn random_training_injury(age: u8, condition_pct: u32, natural_fitness: f32, injury_proneness: u8) -> InjuryType {
        let roll: f32 = rand::random::<f32>();

        let severity_modifier = (age as f32 - 25.0).max(0.0) * 0.01
            + (100.0 - condition_pct as f32) * 0.001
            + (20.0 - natural_fitness).max(0.0) * 0.005
            + (injury_proneness as f32 - 10.0).max(0.0) * 0.005;

        let adjusted_roll = roll + severity_modifier;

        if adjusted_roll < 0.50 {
            // 50% minor
            match rand::random::<u8>() % 5 {
                0 => InjuryType::Cramp,
                1 => InjuryType::MinorKnock,
                2 => InjuryType::Bruise,
                3 => InjuryType::DeadLeg,
                _ => InjuryType::MinorConcussion,
            }
        } else if adjusted_roll < 0.92 {
            // 42% moderate (training injuries are typically muscle strains)
            match rand::random::<u8>() % 7 {
                0 => InjuryType::HamstringStrain,
                1 => InjuryType::CalfStrain,
                2 => InjuryType::GroinStrain,
                3 => InjuryType::AnkleSprain,
                4 => InjuryType::HipFlexorStrain,
                5 => InjuryType::QuadStrain,
                _ => InjuryType::BackSpasm,
            }
        } else if adjusted_roll < 0.98 {
            // 6% severe
            match rand::random::<u8>() % 6 {
                0 => InjuryType::TornMeniscus,
                1 => InjuryType::ShoulderDislocation,
                2 => InjuryType::StressFracture,
                3 => InjuryType::MCLSprain,
                4 => InjuryType::LateralLigament,
                _ => InjuryType::HerniatedDisc,
            }
        } else {
            // 2% critical
            match rand::random::<u8>() % 4 {
                0 => InjuryType::ACLTear,
                1 => InjuryType::BrokenLeg,
                2 => InjuryType::AchillesRupture,
                _ => InjuryType::PCLTear,
            }
        }
    }

    /// Pick a random daily spontaneous injury
    pub fn random_spontaneous_injury(injury_proneness: u8) -> InjuryType {
        let roll: f32 = rand::random::<f32>();

        let severity_modifier = (injury_proneness as f32 - 10.0).max(0.0) * 0.005;
        let adjusted_roll = roll + severity_modifier;

        if adjusted_roll < 0.65 {
            match rand::random::<u8>() % 5 {
                0 => InjuryType::MinorKnock,
                1 => InjuryType::Cramp,
                2 => InjuryType::Bruise,
                3 => InjuryType::DeadLeg,
                _ => InjuryType::MinorConcussion,
            }
        } else if adjusted_roll < 0.90 {
            match rand::random::<u8>() % 7 {
                0 => InjuryType::HamstringStrain,
                1 => InjuryType::CalfStrain,
                2 => InjuryType::AnkleSprain,
                3 => InjuryType::GroinStrain,
                4 => InjuryType::HipFlexorStrain,
                5 => InjuryType::QuadStrain,
                _ => InjuryType::BackSpasm,
            }
        } else if adjusted_roll < 0.97 {
            match rand::random::<u8>() % 6 {
                0 => InjuryType::TornMeniscus,
                1 => InjuryType::ShoulderDislocation,
                2 => InjuryType::StressFracture,
                3 => InjuryType::MCLSprain,
                4 => InjuryType::LateralLigament,
                _ => InjuryType::HerniatedDisc,
            }
        } else {
            match rand::random::<u8>() % 4 {
                0 => InjuryType::ACLTear,
                1 => InjuryType::BrokenLeg,
                2 => InjuryType::AchillesRupture,
                _ => InjuryType::PCLTear,
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
            InjuryType::DeadLeg => write!(f, "Dead Leg"),
            InjuryType::MinorConcussion => write!(f, "Minor Concussion"),
            InjuryType::HamstringStrain => write!(f, "Hamstring Strain"),
            InjuryType::CalfStrain => write!(f, "Calf Strain"),
            InjuryType::AnkleSprain => write!(f, "Ankle Sprain"),
            InjuryType::GroinStrain => write!(f, "Groin Strain"),
            InjuryType::HipFlexorStrain => write!(f, "Hip Flexor Strain"),
            InjuryType::QuadStrain => write!(f, "Quad Strain"),
            InjuryType::BackSpasm => write!(f, "Back Spasm"),
            InjuryType::TornMeniscus => write!(f, "Torn Meniscus"),
            InjuryType::ShoulderDislocation => write!(f, "Shoulder Dislocation"),
            InjuryType::StressFracture => write!(f, "Stress Fracture"),
            InjuryType::MCLSprain => write!(f, "MCL Sprain"),
            InjuryType::LateralLigament => write!(f, "Lateral Ligament"),
            InjuryType::HerniatedDisc => write!(f, "Herniated Disc"),
            InjuryType::ACLTear => write!(f, "ACL Tear"),
            InjuryType::BrokenLeg => write!(f, "Broken Leg"),
            InjuryType::AchillesRupture => write!(f, "Achilles Rupture"),
            InjuryType::PCLTear => write!(f, "PCL Tear"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_injury_duration_ranges() {
        assert_eq!(InjuryType::Bruise.duration_range(), (3, 7));
        assert_eq!(InjuryType::MinorKnock.duration_range(), (3, 8));
        assert_eq!(InjuryType::Cramp.duration_range(), (3, 6));
        assert_eq!(InjuryType::DeadLeg.duration_range(), (4, 10));
        assert_eq!(InjuryType::MinorConcussion.duration_range(), (5, 10));
        assert_eq!(InjuryType::HamstringStrain.duration_range(), (14, 35));
        assert_eq!(InjuryType::ACLTear.duration_range(), (180, 300));
        assert_eq!(InjuryType::BrokenLeg.duration_range(), (180, 270));
        assert_eq!(InjuryType::AchillesRupture.duration_range(), (180, 300));
        assert_eq!(InjuryType::PCLTear.duration_range(), (200, 300));
    }

    #[test]
    fn test_injury_severity() {
        assert_eq!(InjuryType::Cramp.severity(), InjurySeverity::Minor);
        assert_eq!(InjuryType::Bruise.severity(), InjurySeverity::Minor);
        assert_eq!(InjuryType::DeadLeg.severity(), InjurySeverity::Minor);
        assert_eq!(InjuryType::MinorConcussion.severity(), InjurySeverity::Minor);
        assert_eq!(InjuryType::HamstringStrain.severity(), InjurySeverity::Moderate);
        assert_eq!(InjuryType::CalfStrain.severity(), InjurySeverity::Moderate);
        assert_eq!(InjuryType::HipFlexorStrain.severity(), InjurySeverity::Moderate);
        assert_eq!(InjuryType::QuadStrain.severity(), InjurySeverity::Moderate);
        assert_eq!(InjuryType::BackSpasm.severity(), InjurySeverity::Moderate);
        assert_eq!(InjuryType::TornMeniscus.severity(), InjurySeverity::Severe);
        assert_eq!(InjuryType::MCLSprain.severity(), InjurySeverity::Severe);
        assert_eq!(InjuryType::StressFracture.severity(), InjurySeverity::Severe);
        assert_eq!(InjuryType::HerniatedDisc.severity(), InjurySeverity::Severe);
        assert_eq!(InjuryType::ACLTear.severity(), InjurySeverity::Critical);
        assert_eq!(InjuryType::BrokenLeg.severity(), InjurySeverity::Critical);
        assert_eq!(InjuryType::AchillesRupture.severity(), InjurySeverity::Critical);
        assert_eq!(InjuryType::PCLTear.severity(), InjurySeverity::Critical);
    }

    #[test]
    fn test_injury_body_parts() {
        assert_eq!(InjuryType::HamstringStrain.body_part(), BodyPart::Hamstring);
        assert_eq!(InjuryType::ACLTear.body_part(), BodyPart::Knee);
        assert_eq!(InjuryType::AnkleSprain.body_part(), BodyPart::Ankle);
        assert_eq!(InjuryType::CalfStrain.body_part(), BodyPart::Calf);
        assert_eq!(InjuryType::GroinStrain.body_part(), BodyPart::Groin);
        assert_eq!(InjuryType::BackSpasm.body_part(), BodyPart::Back);
        assert_eq!(InjuryType::MinorConcussion.body_part(), BodyPart::Head);
    }

    #[test]
    fn test_random_duration_in_range() {
        for _ in 0..100 {
            let duration = InjuryType::Bruise.random_duration();
            assert!(duration >= 3 && duration <= 7);
        }
        for _ in 0..100 {
            let duration = InjuryType::ACLTear.random_duration();
            assert!(duration >= 180 && duration <= 300);
        }
    }

    #[test]
    fn test_recovery_days_in_range() {
        for _ in 0..100 {
            let days = InjuryType::Cramp.recovery_days();
            assert!(days >= 3 && days <= 5, "Minor recovery {} not in 3-5", days);
        }
        for _ in 0..100 {
            let days = InjuryType::HamstringStrain.recovery_days();
            assert!(days >= 7 && days <= 14, "Moderate recovery {} not in 7-14", days);
        }
        for _ in 0..100 {
            let days = InjuryType::ACLTear.recovery_days();
            assert!(days >= 21 && days <= 30, "Critical recovery {} not in 21-30", days);
        }
    }

    #[test]
    fn test_body_part_encoding() {
        for val in 1..=12u8 {
            let bp = BodyPart::from_u8(val).unwrap();
            assert_eq!(bp.to_u8(), val);
        }
        assert!(BodyPart::from_u8(0).is_none());
        assert!(BodyPart::from_u8(13).is_none());
    }

    #[test]
    fn test_match_injury_produces_valid_type() {
        for _ in 0..100 {
            let injury = InjuryType::random_match_injury(90.0, 25, 80, 12.0, 10);
            let (min, max) = injury.duration_range();
            assert!(min >= 3);
            assert!(max <= 300);
        }
    }

    #[test]
    fn test_injury_recovery_countdown() {
        let injury = InjuryType::MinorKnock;
        let mut days = injury.random_duration();
        assert!(days >= 3 && days <= 8);

        while days > 0 {
            days -= 1;
        }
        assert_eq!(days, 0);
    }
}
