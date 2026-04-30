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
            "ng" | "gh" | "ci" | "cm" | "sn" | "ml" | "gm" | "gw" | "sl" | "bf" | "tg" | "bj"
            | "ne" | "lr" | "cv" | "mr" | "gq" | "ga" | "cg" | "cf" | "td" | "st" => {
                ScoutingRegion::WestAfrica
            }
            // East & South Africa
            _ => ScoutingRegion::EastSouthAfrica,
        }
    }

    fn classify_european_country(code: &str) -> ScoutingRegion {
        match code {
            // Western Europe (the big 5 + neighbors)
            "gb" | "fr" | "de" | "es" | "it" | "nl" | "be" | "pt" | "at" | "ch" | "lu" | "li"
            | "mc" | "ie" | "sc" | "gi" | "sm" | "ad" | "mt" => ScoutingRegion::WesternEurope,
            // Scandinavia
            "se" | "no" | "dk" | "fi" | "is" | "fo" => ScoutingRegion::Scandinavia,
            // Turkey/Caucasus/Middle-East-adjacent Europe
            "tr" | "il" | "cy" | "ge" | "am" | "az" | "kz" => ScoutingRegion::MiddleEastEurope,
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
            "sa" | "ae" | "qa" | "ir" | "iq" | "kw" | "bh" | "om" | "jo" | "lb" | "ye" | "ps" => {
                ScoutingRegion::MiddleEast
            }
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
            // S.American clubs scout neighbors — their players export to Europe,
            // but the clubs themselves don't scout Europe or Asia.
            ScoutingRegion::SouthAmerica => &[
                (ScoutingRegion::CentralAmericaCaribbean, 20),
                (ScoutingRegion::NorthAmerica, 8),
                (ScoutingRegion::WestAfrica, 4),
            ],
            // W.African clubs scout within Africa — the European pipeline is
            // one-way player export, not two-way scouting.
            ScoutingRegion::WestAfrica => &[
                (ScoutingRegion::NorthAfrica, 18),
                (ScoutingRegion::EastSouthAfrica, 12),
                (ScoutingRegion::CentralAmericaCaribbean, 3),
            ],
            ScoutingRegion::NorthAfrica => &[
                (ScoutingRegion::WestAfrica, 18),
                (ScoutingRegion::MiddleEast, 12),
                (ScoutingRegion::EastSouthAfrica, 8),
                (ScoutingRegion::MiddleEastEurope, 5),
            ],
            ScoutingRegion::EastSouthAfrica => &[
                (ScoutingRegion::WestAfrica, 15),
                (ScoutingRegion::NorthAfrica, 10),
                (ScoutingRegion::CentralAmericaCaribbean, 3),
            ],
            // North American clubs
            ScoutingRegion::NorthAmerica => &[
                (ScoutingRegion::SouthAmerica, 30),
                (ScoutingRegion::WesternEurope, 20),
                (ScoutingRegion::CentralAmericaCaribbean, 15),
                (ScoutingRegion::WestAfrica, 10),
                (ScoutingRegion::EasternEurope, 8),
            ],
            // CONCACAF clubs scout their own neighborhood; they don't scout Europe.
            ScoutingRegion::CentralAmericaCaribbean => &[
                (ScoutingRegion::SouthAmerica, 20),
                (ScoutingRegion::NorthAmerica, 15),
                (ScoutingRegion::WestAfrica, 3),
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
            // Oceanian clubs scout Asia-Pacific + S.America; players export to Europe.
            ScoutingRegion::Oceania => &[
                (ScoutingRegion::EastAsia, 15),
                (ScoutingRegion::SoutheastAsia, 12),
                (ScoutingRegion::SouthAmerica, 5),
            ],
        }
    }

    /// Relative league prestige for each region.
    /// Used by players when evaluating cross-region transfers —
    /// players resist moves to less prestigious regions.
    pub fn league_prestige(&self) -> f32 {
        match self {
            ScoutingRegion::WesternEurope => 1.0,
            ScoutingRegion::EasternEurope => 0.50,
            ScoutingRegion::Scandinavia => 0.45,
            ScoutingRegion::SouthAmerica => 0.45,
            ScoutingRegion::MiddleEastEurope => 0.40,
            ScoutingRegion::MiddleEast => 0.40,
            ScoutingRegion::NorthAmerica => 0.35,
            ScoutingRegion::EastAsia => 0.30,
            ScoutingRegion::NorthAfrica => 0.25,
            ScoutingRegion::CentralAmericaCaribbean => 0.20,
            ScoutingRegion::WestAfrica => 0.20,
            ScoutingRegion::Oceania => 0.20,
            ScoutingRegion::EastSouthAfrica => 0.15,
            ScoutingRegion::SoutheastAsia => 0.15,
            ScoutingRegion::SouthAsia => 0.10,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            ScoutingRegion::WesternEurope => "region_western_europe",
            ScoutingRegion::EasternEurope => "region_eastern_europe",
            ScoutingRegion::Scandinavia => "region_scandinavia",
            ScoutingRegion::MiddleEastEurope => "region_middle_east_europe",
            ScoutingRegion::SouthAmerica => "region_south_america",
            ScoutingRegion::WestAfrica => "region_west_africa",
            ScoutingRegion::NorthAfrica => "region_north_africa",
            ScoutingRegion::EastSouthAfrica => "region_east_south_africa",
            ScoutingRegion::NorthAmerica => "region_north_america",
            ScoutingRegion::CentralAmericaCaribbean => "region_central_america_caribbean",
            ScoutingRegion::EastAsia => "region_east_asia",
            ScoutingRegion::SoutheastAsia => "region_southeast_asia",
            ScoutingRegion::MiddleEast => "region_middle_east",
            ScoutingRegion::SouthAsia => "region_south_asia",
            ScoutingRegion::Oceania => "region_oceania",
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
