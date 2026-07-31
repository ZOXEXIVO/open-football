use crate::transfers::pipeline::ScoutingRecommendation;

/// Why a transfer happened, kept in pieces the UI can localise.
///
/// The motive itself is an i18n key — never prose. A reason assembled as
/// an English sentence in the simulator reaches the transfers page as
/// English no matter which locale the reader picked, which is exactly how
/// "Squad depth — need backup for position group" ended up untranslated
/// under a Russian heading. The scout verdict keeps its raw assessment
/// rather than a rendered phrase for the same reason: the view bands and
/// phrases it at render time, in the reader's language.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransferReason {
    /// i18n key for the motive line ("signing_reason_depth_cover",
    /// "dec_reason_contract_expired", ...). Empty = no reason recorded.
    pub key: String,
    /// Scout verdict that backed the signing, when a report existed.
    pub scout: Option<ScoutVerdict>,
    /// The buyer raided a rival's squad — the view marks the move.
    pub rival: bool,
}

impl TransferReason {
    /// A bare motive key with no scout report behind it.
    pub fn key(key: impl Into<String>) -> Self {
        TransferReason {
            key: key.into(),
            scout: None,
            rival: false,
        }
    }

    pub fn with_scout(mut self, scout: Option<ScoutVerdict>) -> Self {
        self.scout = scout;
        self
    }

    pub fn as_rival(mut self) -> Self {
        self.rival = true;
        self
    }

    /// Nothing to show: no motive key and no scout verdict behind it.
    pub fn is_empty(&self) -> bool {
        self.key.is_empty() && self.scout.is_none()
    }
}

/// A scout's verdict at the moment the signing was agreed, stored as the
/// raw assessment the report carried. The scouting page renders the same
/// numbers directly; the transfers page bands them (see [`AbilityBand`])
/// because a completed move is read as a story, not a table.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoutVerdict {
    pub recommendation: ScoutingRecommendation,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    /// 0.0..1.0 — how sure the scout was when the report was filed.
    pub confidence: f32,
}

impl ScoutVerdict {
    pub fn ability_band(&self) -> AbilityBand {
        AbilityBand::from_assessment(self.assessed_ability)
    }

    pub fn potential_band(&self) -> AbilityBand {
        AbilityBand::from_assessment(self.assessed_potential)
    }

    /// Confidence as a whole percentage, for display.
    pub fn confidence_pct(&self) -> u8 {
        (self.confidence * 100.0).round().clamp(0.0, 100.0) as u8
    }
}

/// Qualitative band a scout's numeric assessment falls into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbilityBand {
    VeryPoor,
    Poor,
    BelowAverage,
    Average,
    Decent,
    Good,
    VeryGood,
    Excellent,
    WorldClass,
    Unknown,
}

impl AbilityBand {
    pub fn from_assessment(value: u8) -> Self {
        match value {
            0..=30 => AbilityBand::VeryPoor,
            31..=60 => AbilityBand::Poor,
            61..=80 => AbilityBand::BelowAverage,
            81..=100 => AbilityBand::Average,
            101..=120 => AbilityBand::Decent,
            121..=140 => AbilityBand::Good,
            141..=160 => AbilityBand::VeryGood,
            161..=180 => AbilityBand::Excellent,
            181..=200 => AbilityBand::WorldClass,
            _ => AbilityBand::Unknown,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            AbilityBand::VeryPoor => "ability_band_very_poor",
            AbilityBand::Poor => "ability_band_poor",
            AbilityBand::BelowAverage => "ability_band_below_average",
            AbilityBand::Average => "ability_band_average",
            AbilityBand::Decent => "ability_band_decent",
            AbilityBand::Good => "ability_band_good",
            AbilityBand::VeryGood => "ability_band_very_good",
            AbilityBand::Excellent => "ability_band_excellent",
            AbilityBand::WorldClass => "ability_band_world_class",
            AbilityBand::Unknown => "ability_band_unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_cover_the_whole_assessment_range() {
        assert_eq!(AbilityBand::from_assessment(0), AbilityBand::VeryPoor);
        assert_eq!(AbilityBand::from_assessment(95), AbilityBand::Average);
        assert_eq!(AbilityBand::from_assessment(110), AbilityBand::Decent);
        assert_eq!(AbilityBand::from_assessment(200), AbilityBand::WorldClass);
        assert_eq!(AbilityBand::from_assessment(255), AbilityBand::Unknown);
    }

    #[test]
    fn a_reason_with_only_a_scout_verdict_still_renders() {
        let reason = TransferReason::default().with_scout(Some(ScoutVerdict {
            recommendation: ScoutingRecommendation::Buy,
            assessed_ability: 95,
            assessed_potential: 110,
            confidence: 0.4,
        }));

        assert!(!reason.is_empty());
        assert!(TransferReason::default().is_empty());
    }

    #[test]
    fn confidence_renders_as_whole_percent() {
        let verdict = ScoutVerdict {
            recommendation: ScoutingRecommendation::Consider,
            assessed_ability: 95,
            assessed_potential: 110,
            confidence: 0.404,
        };
        assert_eq!(verdict.confidence_pct(), 40);
    }
}
