/// Geographic scouting regions that scouts can specialize in.
/// Instead of knowing individual country IDs, scouts know whole regions —
/// allowing a single scout covering "WestAfrica" to find players from
/// Nigeria, Ghana, Ivory Coast, Cameroon, Senegal, etc.
///
/// This replaces the old `known_regions: Vec<u32>` (country IDs) system.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScoutingRegion {
    /// England, France, Germany, Spain, Italy, Netherlands, Belgium, Portugal, Switzerland, Austria
    WesternEurope,
    /// Poland, Czech, Romania, Ukraine, Russia, Serbia, Croatia, Bosnia, Bulgaria, Hungary, Slovakia, Slovenia, Greece, Albania, etc.
    EasternEurope,
    /// Sweden, Norway, Denmark, Finland, Iceland, Faroe Islands
    Scandinavia,
    /// Turkey, Israel, Cyprus, Georgia, Armenia, Azerbaijan, Kazakhstan
    MiddleEastEurope,
    /// Brazil, Argentina, Colombia, Uruguay, Chile, Paraguay, Ecuador, Peru, Bolivia, Venezuela
    SouthAmerica,
    /// Nigeria, Ghana, Ivory Coast, Cameroon, Senegal, Mali, Gambia, Guinea, Sierra Leone, Burkina Faso, Togo, Benin, Niger, Liberia, Cape Verde
    WestAfrica,
    /// Morocco, Egypt, Tunisia, Algeria, Libya
    NorthAfrica,
    /// South Africa, Kenya, Tanzania, Uganda, Ethiopia, Zimbabwe, Zambia, Mozambique, etc.
    EastSouthAfrica,
    /// USA, Canada, Mexico
    NorthAmerica,
    /// Costa Rica, Honduras, Panama, Guatemala, Jamaica, Trinidad, etc.
    CentralAmericaCaribbean,
    /// Japan, South Korea, China, North Korea, Taiwan, Hong Kong, Macau
    EastAsia,
    /// Thailand, Vietnam, Indonesia, Malaysia, Philippines, Singapore, Myanmar, Cambodia
    SoutheastAsia,
    /// Saudi Arabia, UAE, Qatar, Iran, Iraq, Kuwait, Bahrain, Oman, Jordan, Lebanon
    MiddleEast,
    /// India, Pakistan, Bangladesh, Sri Lanka, Nepal
    SouthAsia,
    /// Australia, New Zealand, Fiji, Papua New Guinea
    Oceania,
}

impl ScoutingRegion {
    /// Map a country's continent_id and country code to its scouting region.
    /// continent_id: 0=Africa, 1=Europe, 2=NorthCentralAmerica, 3=SouthAmerica, 4=Asia, 5=Oceania
    pub fn from_country(continent_id: u32, country_code: &str) -> ScoutingRegion {
        match continent_id {
            0 => Self::classify_african_country(country_code),
            1 => Self::classify_european_country(country_code),
            2 => Self::classify_concacaf_country(country_code),
            3 => ScoutingRegion::SouthAmerica,
            4 => Self::classify_asian_country(country_code),
            5 => ScoutingRegion::Oceania,
            _ => ScoutingRegion::WesternEurope,
        }
    }

    fn classify_african_country(code: &str) -> ScoutingRegion {
        match code {
            // North Africa
            "ma" | "eg" | "tn" | "dz" | "ly" => ScoutingRegion::NorthAfrica,
            // West Africa
            "ng" | "gh" | "ci" | "cm" | "sn" | "ml" | "gm" | "gw" | "sl" | "bf" | "tg"
            | "bj" | "ne" | "lr" | "cv" | "mr" | "gq" | "ga" | "cg" | "cf" | "td" | "st" => {
                ScoutingRegion::WestAfrica
            }
            // East & South Africa
            _ => ScoutingRegion::EastSouthAfrica,
        }
    }

    fn classify_european_country(code: &str) -> ScoutingRegion {
        match code {
            // Western Europe (the big 5 + neighbors)
            "gb" | "fr" | "de" | "es" | "it" | "nl" | "be" | "pt" | "at" | "ch" | "lu"
            | "li" | "mc" | "ie" | "sc" | "gi" | "sm" | "ad" | "mt" => {
                ScoutingRegion::WesternEurope
            }
            // Scandinavia
            "se" | "no" | "dk" | "fi" | "is" | "fo" => ScoutingRegion::Scandinavia,
            // Turkey/Caucasus/Middle-East-adjacent Europe
            "tr" | "il" | "cy" | "ge" | "am" | "az" | "kz" => {
                ScoutingRegion::MiddleEastEurope
            }
            // Eastern Europe (everything else)
            _ => ScoutingRegion::EasternEurope,
        }
    }

    fn classify_concacaf_country(code: &str) -> ScoutingRegion {
        match code {
            "us" | "ca" | "mx" => ScoutingRegion::NorthAmerica,
            _ => ScoutingRegion::CentralAmericaCaribbean,
        }
    }

    fn classify_asian_country(code: &str) -> ScoutingRegion {
        match code {
            // East Asia
            "jp" | "kr" | "cn" | "kp" | "tw" | "hk" | "mo" | "mn" => ScoutingRegion::EastAsia,
            // Middle East
            "sa" | "ae" | "qa" | "ir" | "iq" | "kw" | "bh" | "om" | "jo" | "lb" | "ye"
            | "ps" => ScoutingRegion::MiddleEast,
            // South Asia
            "in" | "pk" | "bd" | "lk" | "np" | "mv" | "bt" | "af" => ScoutingRegion::SouthAsia,
            // Southeast Asia
            _ => ScoutingRegion::SoutheastAsia,
        }
    }

    /// Check if a country (identified by continent_id + code) belongs to this region.
    pub fn contains_country(&self, continent_id: u32, country_code: &str) -> bool {
        Self::from_country(continent_id, country_code) == *self
    }

    /// Get transfer corridor weights from this region.
    /// Returns (target_region, weight) pairs — higher weight = more likely to scout there.
    /// Models real-world transfer corridors (Africa→Europe, SouthAmerica→Europe, etc.)
    pub fn transfer_corridors(&self) -> &'static [(ScoutingRegion, u8)] {
        match self {
            // European clubs: strong corridors to S.America, W.Africa, E.Europe
            ScoutingRegion::WesternEurope => &[
                (ScoutingRegion::SouthAmerica, 25),
                (ScoutingRegion::WestAfrica, 20),
                (ScoutingRegion::EasternEurope, 15),
                (ScoutingRegion::Scandinavia, 10),
                (ScoutingRegion::NorthAfrica, 8),
                (ScoutingRegion::EastSouthAfrica, 5),
                (ScoutingRegion::NorthAmerica, 5),
                (ScoutingRegion::MiddleEastEurope, 5),
                (ScoutingRegion::EastAsia, 4),
                (ScoutingRegion::CentralAmericaCaribbean, 3),
            ],
            ScoutingRegion::EasternEurope => &[
                (ScoutingRegion::WesternEurope, 25),
                (ScoutingRegion::SouthAmerica, 15),
                (ScoutingRegion::WestAfrica, 12),
                (ScoutingRegion::Scandinavia, 10),
                (ScoutingRegion::NorthAfrica, 8),
                (ScoutingRegion::MiddleEastEurope, 8),
                (ScoutingRegion::EastSouthAfrica, 5),
            ],
            ScoutingRegion::Scandinavia => &[
                (ScoutingRegion::WesternEurope, 25),
                (ScoutingRegion::EasternEurope, 15),
                (ScoutingRegion::WestAfrica, 12),
                (ScoutingRegion::SouthAmerica, 10),
                (ScoutingRegion::NorthAfrica, 8),
                (ScoutingRegion::NorthAmerica, 5),
            ],
            ScoutingRegion::MiddleEastEurope => &[
                (ScoutingRegion::WesternEurope, 20),
                (ScoutingRegion::EasternEurope, 18),
                (ScoutingRegion::SouthAmerica, 15),
                (ScoutingRegion::WestAfrica, 12),
                (ScoutingRegion::NorthAfrica, 10),
                (ScoutingRegion::MiddleEast, 10),
            ],
            // South American clubs: mostly sell to Europe
            ScoutingRegion::SouthAmerica => &[
                (ScoutingRegion::WesternEurope, 30),
                (ScoutingRegion::NorthAmerica, 15),
                (ScoutingRegion::EasternEurope, 10),
                (ScoutingRegion::MiddleEast, 10),
                (ScoutingRegion::CentralAmericaCaribbean, 8),
                (ScoutingRegion::EastAsia, 5),
            ],
            // West African clubs: pipeline to Europe, Middle East
            ScoutingRegion::WestAfrica => &[
                (ScoutingRegion::WesternEurope, 35),
                (ScoutingRegion::NorthAfrica, 15),
                (ScoutingRegion::EasternEurope, 12),
                (ScoutingRegion::MiddleEast, 10),
                (ScoutingRegion::Scandinavia, 8),
                (ScoutingRegion::EastSouthAfrica, 5),
            ],
            ScoutingRegion::NorthAfrica => &[
                (ScoutingRegion::WesternEurope, 35),
                (ScoutingRegion::MiddleEast, 15),
                (ScoutingRegion::EasternEurope, 10),
                (ScoutingRegion::WestAfrica, 10),
                (ScoutingRegion::MiddleEastEurope, 8),
            ],
            ScoutingRegion::EastSouthAfrica => &[
                (ScoutingRegion::WesternEurope, 30),
                (ScoutingRegion::NorthAfrica, 12),
                (ScoutingRegion::WestAfrica, 10),
                (ScoutingRegion::MiddleEast, 10),
                (ScoutingRegion::EasternEurope, 8),
                (ScoutingRegion::EastAsia, 5),
            ],
            // North American clubs
            ScoutingRegion::NorthAmerica => &[
                (ScoutingRegion::SouthAmerica, 30),
                (ScoutingRegion::WesternEurope, 20),
                (ScoutingRegion::CentralAmericaCaribbean, 15),
                (ScoutingRegion::WestAfrica, 10),
                (ScoutingRegion::EasternEurope, 8),
            ],
            ScoutingRegion::CentralAmericaCaribbean => &[
                (ScoutingRegion::SouthAmerica, 25),
                (ScoutingRegion::NorthAmerica, 20),
                (ScoutingRegion::WesternEurope, 15),
                (ScoutingRegion::WestAfrica, 8),
            ],
            // Asian leagues
            ScoutingRegion::EastAsia => &[
                (ScoutingRegion::SouthAmerica, 25),
                (ScoutingRegion::WesternEurope, 20),
                (ScoutingRegion::SoutheastAsia, 15),
                (ScoutingRegion::EasternEurope, 10),
                (ScoutingRegion::Oceania, 8),
                (ScoutingRegion::WestAfrica, 5),
            ],
            ScoutingRegion::SoutheastAsia => &[
                (ScoutingRegion::EastAsia, 25),
                (ScoutingRegion::SouthAmerica, 15),
                (ScoutingRegion::WesternEurope, 12),
                (ScoutingRegion::Oceania, 10),
                (ScoutingRegion::SouthAsia, 8),
            ],
            ScoutingRegion::MiddleEast => &[
                (ScoutingRegion::SouthAmerica, 25),
                (ScoutingRegion::WesternEurope, 20),
                (ScoutingRegion::WestAfrica, 15),
                (ScoutingRegion::NorthAfrica, 12),
                (ScoutingRegion::EasternEurope, 10),
                (ScoutingRegion::EastAsia, 5),
            ],
            ScoutingRegion::SouthAsia => &[
                (ScoutingRegion::WesternEurope, 20),
                (ScoutingRegion::MiddleEast, 15),
                (ScoutingRegion::EastAsia, 12),
                (ScoutingRegion::SoutheastAsia, 10),
            ],
            ScoutingRegion::Oceania => &[
                (ScoutingRegion::WesternEurope, 25),
                (ScoutingRegion::EastAsia, 20),
                (ScoutingRegion::SouthAmerica, 10),
                (ScoutingRegion::SoutheastAsia, 8),
            ],
        }
    }

    /// All regions as a static array (for iteration).
    pub fn all() -> &'static [ScoutingRegion] {
        &[
            ScoutingRegion::WesternEurope,
            ScoutingRegion::EasternEurope,
            ScoutingRegion::Scandinavia,
            ScoutingRegion::MiddleEastEurope,
            ScoutingRegion::SouthAmerica,
            ScoutingRegion::WestAfrica,
            ScoutingRegion::NorthAfrica,
            ScoutingRegion::EastSouthAfrica,
            ScoutingRegion::NorthAmerica,
            ScoutingRegion::CentralAmericaCaribbean,
            ScoutingRegion::EastAsia,
            ScoutingRegion::SoutheastAsia,
            ScoutingRegion::MiddleEast,
            ScoutingRegion::SouthAsia,
            ScoutingRegion::Oceania,
        ]
    }
}
