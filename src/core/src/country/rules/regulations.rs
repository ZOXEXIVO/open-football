use crate::club::Player;

/// Per-country competition rules. Each field is `None` to mean
/// "rule disabled / not enforced" — the simulator must opt in by
/// populating these via the country builder.
#[derive(Debug, Clone)]
pub struct CountryRegulations {
    /// Maximum non-domestic players allowed in a squad. `None` means
    /// no limit (typical for top-five European leagues post-Bosman).
    pub foreign_player_limit: Option<u8>,
    /// Country-wide salary cap on total annual wage bill (in the
    /// country's pricing unit). `None` = no cap.
    pub salary_cap: Option<f64>,
    /// Minimum number of homegrown-eligible players required in the
    /// registered squad. "Homegrown" is approximated as
    /// `player.country_id == club.country_id`. `None` = no requirement.
    pub homegrown_requirements: Option<u8>,
    pub ffp_enabled: bool, // Financial Fair Play
}

impl CountryRegulations {
    pub fn new() -> Self {
        CountryRegulations {
            foreign_player_limit: None,
            salary_cap: None,
            homegrown_requirements: None,
            ffp_enabled: false,
        }
    }

    /// Squad rules for one country, from that country's public
    /// registration regulations.
    ///
    /// Only rules this model can actually express are populated, and the
    /// distinction matters: the fields here are NATIONALITY-based (domestic
    /// vs foreign, by `country_id`), so a country whose real rule is
    /// EU/non-EU (Spain, Italy) or club-trained (the UEFA homegrown rule as
    /// most of western Europe applies it) is left `None` rather than
    /// approximated by a nationality count that would be badly wrong — La
    /// Liga squads are full of EU "foreigners" its three-slot non-EU rule
    /// never touched.
    ///
    /// What IS populated is the set of leagues whose rule genuinely counts
    /// passports: the Turkish, Russian, Ukrainian, Asian, Gulf, Brazilian
    /// and North-American limits, plus England's homegrown minimum, where
    /// the nationality proxy the field documents is close enough to the
    /// real quota to be worth enforcing.
    ///
    /// Both two- and three-letter codes are matched because the shipped
    /// database uses both, exactly as the transfer calendar's own lookup
    /// does.
    pub fn for_country_code(code: &str) -> Self {
        let normalized = code.trim().to_ascii_lowercase();
        let mut regulations = Self::new();
        regulations.foreign_player_limit = match normalized.as_str() {
            // Registered-foreigner quotas. Figures are the squad
            // REGISTRATION limits (not the narrower on-pitch ones), since
            // that is what `omitted_for_foreign_limit` enforces.
            "tr" | "tur" => Some(14),
            "ru" | "rus" => Some(13),
            "ua" | "ukr" => Some(11),
            "mx" | "mex" => Some(10),
            "br" | "bra" => Some(9),
            "sa" | "sau" => Some(8),
            "ae" | "are" => Some(8),
            "us" | "usa" | "ca" | "can" => Some(8),
            "cn" | "chn" => Some(6),
            "jp" | "jpn" => Some(5),
            "kr" | "kor" => Some(5),
            "qa" | "qat" => Some(5),
            "ir" | "irn" => Some(4),
            _ => None,
        };
        regulations.homegrown_requirements = match normalized.as_str() {
            // Eight of a twenty-five-man list. The real rule counts
            // club-trained players rather than passports; the nationality
            // proxy this field documents lands close because the two
            // overlap heavily in practice.
            "gb" | "eng" | "gbr" => Some(8),
            _ => None,
        };
        regulations
    }

    /// Decide which players a squad must omit at registration time to
    /// satisfy `foreign_player_limit`. Players are sorted by ability
    /// ascending — the lowest-ability foreigners are dropped first so
    /// expected regulars (high-CA imports) keep their slot. Ties on
    /// ability are broken by descending player_id to keep the choice
    /// deterministic across runs.
    ///
    /// `club_country_id` is the home country of the club; players
    /// matching this are treated as domestic. Returns the player ids
    /// that are NOT registered.
    pub fn omitted_for_foreign_limit(&self, players: &[&Player], club_country_id: u32) -> Vec<u32> {
        let limit = match self.foreign_player_limit {
            Some(n) => n as usize,
            None => return Vec::new(),
        };
        let mut foreigners: Vec<(u32, u8)> = players
            .iter()
            .filter(|p| p.country_id != club_country_id)
            .map(|p| (p.id, p.player_attributes.current_ability))
            .collect();
        if foreigners.len() <= limit {
            return Vec::new();
        }
        // Stronger foreigners keep their slot — sort weakest first,
        // then drop the leftover head.
        foreigners.sort_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)));
        let drop_count = foreigners.len() - limit;
        foreigners
            .into_iter()
            .take(drop_count)
            .map(|(id, _)| id)
            .collect()
    }

    /// Count of homegrown-eligible players currently in `players`.
    /// Helper for the registration view; doesn't enforce — merely
    /// reports the gap so callers can decide what to do.
    pub fn homegrown_count(&self, players: &[&Player], club_country_id: u32) -> u8 {
        players
            .iter()
            .filter(|p| p.country_id == club_country_id)
            .count()
            .min(u8::MAX as usize) as u8
    }

    /// True when `total_annual_wages` exceeds `salary_cap`. False when
    /// no cap is set. Caller decides what to do with the verdict —
    /// reject a transfer, fine the club, or surface a warning.
    pub fn salary_cap_exceeded(&self, total_annual_wages: f64) -> bool {
        match self.salary_cap {
            Some(cap) => total_annual_wages > cap,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{PersonAttributes, PlayerAttributes, PlayerPositions, PlayerSkills};
    use chrono::NaiveDate;

    fn make_player(id: u32, country_id: u32, ability: u8) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ability;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(1995, 1, 1).unwrap())
            .country_id(country_id)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions { positions: vec![] })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    #[test]
    fn no_limit_means_no_omissions() {
        let regs = CountryRegulations::new();
        let players: Vec<Player> = (1..=15).map(|i| make_player(i, 99, 100)).collect();
        let refs: Vec<&Player> = players.iter().collect();
        // Club country is 1; everyone else is foreign — but no limit
        // configured means no one dropped.
        assert!(regs.omitted_for_foreign_limit(&refs, 1).is_empty());
    }

    #[test]
    fn limit_drops_weakest_foreigners_first() {
        let mut regs = CountryRegulations::new();
        regs.foreign_player_limit = Some(2);
        // 4 foreigners with abilities 50, 80, 120, 160. Limit 2 → drop
        // the bottom 2 (50 and 80).
        let players = vec![
            make_player(10, 99, 50),
            make_player(11, 99, 80),
            make_player(12, 99, 120),
            make_player(13, 99, 160),
            // Domestic player — never gets dropped.
            make_player(14, 1, 60),
        ];
        let refs: Vec<&Player> = players.iter().collect();
        let omitted = regs.omitted_for_foreign_limit(&refs, 1);
        assert_eq!(omitted.len(), 2);
        assert!(omitted.contains(&10));
        assert!(omitted.contains(&11));
        assert!(!omitted.contains(&14));
    }

    #[test]
    fn limit_does_not_drop_anyone_when_under_quota() {
        let mut regs = CountryRegulations::new();
        regs.foreign_player_limit = Some(5);
        let players = vec![make_player(1, 99, 100), make_player(2, 99, 110)];
        let refs: Vec<&Player> = players.iter().collect();
        assert!(regs.omitted_for_foreign_limit(&refs, 1).is_empty());
    }

    #[test]
    fn homegrown_count_matches_country_id() {
        let regs = CountryRegulations::new();
        let players = vec![
            make_player(1, 1, 100),  // domestic
            make_player(2, 99, 100), // foreign
            make_player(3, 1, 100),  // domestic
        ];
        let refs: Vec<&Player> = players.iter().collect();
        assert_eq!(regs.homegrown_count(&refs, 1), 2);
    }

    #[test]
    fn salary_cap_exceeded_returns_false_when_no_cap() {
        let regs = CountryRegulations::new();
        assert!(!regs.salary_cap_exceeded(1_000_000_000.0));
    }

    #[test]
    fn salary_cap_exceeded_returns_true_above_cap() {
        let mut regs = CountryRegulations::new();
        regs.salary_cap = Some(50_000_000.0);
        assert!(regs.salary_cap_exceeded(60_000_000.0));
        assert!(!regs.salary_cap_exceeded(40_000_000.0));
    }

    #[test]
    fn a_country_whose_quota_counts_passports_gets_one() {
        assert_eq!(
            CountryRegulations::for_country_code("tr").foreign_player_limit,
            Some(14)
        );
        assert_eq!(
            CountryRegulations::for_country_code("TUR").foreign_player_limit,
            Some(14),
            "both code lengths and either case, exactly as the transfer calendar reads them"
        );
    }

    #[test]
    fn a_country_whose_real_rule_is_eu_based_gets_nothing() {
        // Spain's three-slot rule counts NON-EU players; La Liga squads are
        // full of EU "foreigners" it never touched. A nationality proxy here
        // would bench half the league.
        for code in ["es", "esp", "it", "ita", "de", "deu", "fr", "fra"] {
            let regs = CountryRegulations::for_country_code(code);
            assert_eq!(
                regs.foreign_player_limit, None,
                "{code} has no passport-counting quota this model can express"
            );
        }
    }

    #[test]
    fn england_carries_a_homegrown_minimum_and_no_foreigner_cap() {
        let regs = CountryRegulations::for_country_code("eng");
        assert_eq!(regs.homegrown_requirements, Some(8));
        assert_eq!(regs.foreign_player_limit, None);
    }

    #[test]
    fn an_unknown_country_is_left_entirely_alone() {
        let regs = CountryRegulations::for_country_code("zz");
        assert_eq!(regs.foreign_player_limit, None);
        assert_eq!(regs.homegrown_requirements, None);
        assert_eq!(regs.salary_cap, None);
    }
}
