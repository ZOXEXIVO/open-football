//! What real football looks like for each headline row, and where each
//! figure comes from.

use std::fmt::Write as _;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Computed from recorded league appearances: the history rows of the
    /// shipped database, complete club-seasons of 2023 to 2025
    /// (`measurements/section-1/recorded_reference.txt`).
    Recorded,
    /// A working estimate from general knowledge of top-flight football.
    Estimate,
    /// Follows from the laws of the game or from how every club behaves.
    Rule,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Recorded => "recorded",
            Source::Estimate => "estimate",
            Source::Rule => "rule",
        }
    }
}

pub struct Band {
    pub low: f64,
    pub high: f64,
    pub source: Source,
    pub note: &'static str,
}

impl Band {
    const fn new(low: f64, high: f64, source: Source, note: &'static str) -> Self {
        Band {
            low,
            high,
            source,
            note,
        }
    }

    pub fn range(&self) -> String {
        format!("[{:.2}..{:.2}]", self.low, self.high)
    }

    pub fn verdict(&self, value: f64) -> &'static str {
        if value >= self.low && value <= self.high {
            "inside"
        } else {
            "outside"
        }
    }
}

/// A band and the tiers it describes.
struct Reference {
    tiers: &'static [u8],
    key: &'static str,
    band: Band,
}

pub struct RealFootball;

impl RealFootball {
    const TOP: &'static [u8] = &[1];
    const SECOND: &'static [u8] = &[2];
    const BOTH: &'static [u8] = &[1, 2];

    const TABLE: &'static [Reference] = &[
        Reference {
            tiers: Self::TOP,
            key: "rotation.league_to_league",
            band: Band::new(
                1.5,
                3.0,
                Source::Estimate,
                "two or three changes between league matches a week apart; the low end keeps the settled XI",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "rotation.into_cup",
            band: Band::new(
                4.0,
                6.5,
                Source::Estimate,
                "four to six across all rounds and clubs; seven to ten only for strong clubs in early rounds",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "rotation.into_continental",
            band: Band::new(3.0, 5.0, Source::Estimate, "three to five in a European week"),
        },
        Reference {
            tiers: Self::TOP,
            key: "rotation.rest_le4",
            band: Band::new(
                3.0,
                5.0,
                Source::Estimate,
                "three to five after four days' rest or fewer, whatever the competition",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "spread.top11_share_of_league_starts",
            band: Band::new(
                68.0,
                77.0,
                Source::Estimate,
                "70 to 75% of league starts go to the eleven most-used players",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "spread.players_starting_80pct",
            band: Band::new(3.0, 5.0, Source::Estimate, "three to five ever-presents a club"),
        },
        Reference {
            tiers: Self::TOP,
            key: "spread.players_given_league_start",
            band: Band::new(22.0, 26.0, Source::Estimate, "22 to 26 players start a league match"),
        },
        Reference {
            tiers: Self::TOP,
            key: "spread.top11_share_of_league_appearances",
            band: Band::new(
                59.5,
                66.0,
                Source::Recorded,
                "season means 61.9 to 63.3% over 827 top-division club-seasons",
            ),
        },
        Reference {
            tiers: Self::SECOND,
            key: "spread.top11_share_of_league_appearances",
            band: Band::new(
                60.0,
                67.5,
                Source::Recorded,
                "season means 62.3 to 65.2% over 317 second-division club-seasons",
            ),
        },
        Reference {
            tiers: Self::BOTH,
            key: "spread.players_appearing_80pct",
            band: Band::new(
                5.8,
                8.2,
                Source::Recorded,
                "season means 6.8 to 7.2 players appear in four league matches in five",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "spread.players_given_league_appearance",
            band: Band::new(
                26.0,
                31.5,
                Source::Recorded,
                "season means 28.2 to 29.3 players given a league appearance",
            ),
        },
        Reference {
            tiers: Self::SECOND,
            key: "spread.players_given_league_appearance",
            band: Band::new(
                24.5,
                30.0,
                Source::Recorded,
                "season means 26.5 to 28.1 players given a league appearance",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "appearances.league.season_age_le21",
            band: Band::new(
                13.0,
                19.5,
                Source::Recorded,
                "season means 15.7 to 16.8% of league appearances, age as season year minus birth year",
            ),
        },
        Reference {
            tiers: Self::SECOND,
            key: "appearances.league.season_age_le21",
            band: Band::new(
                12.0,
                19.7,
                Source::Recorded,
                "season means 14.7 to 17.2% of league appearances, age as season year minus birth year",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "appearances.league.season_age_ge30",
            band: Band::new(
                20.0,
                26.5,
                Source::Recorded,
                "22.4 and 23.9% in the two latest seasons; earlier seasons have lost retired players",
            ),
        },
        Reference {
            tiers: Self::SECOND,
            key: "appearances.league.season_age_ge30",
            band: Band::new(
                17.0,
                26.5,
                Source::Recorded,
                "19.3 and 24.1% in the two latest seasons; earlier seasons have lost retired players",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "starts.league.age_le21",
            band: Band::new(
                6.0,
                12.0,
                Source::Estimate,
                "6 to 12% of league starts; the recorded 16% of appearances includes substitutes",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "starts.league.age_ge30",
            band: Band::new(
                20.0,
                25.0,
                Source::Estimate,
                "20 to 25% of league starts; outside this change's scope",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "starts.cup.key_regular",
            band: Band::new(
                40.0,
                60.0,
                Source::Estimate,
                "about half of domestic cup starts go to Key and Regular players",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "starts.league.other_squad",
            band: Band::new(0.0, 1.0, Source::Estimate, "a small share of league starts"),
        },
        Reference {
            tiers: Self::TOP,
            key: "starts.cup.other_squad",
            band: Band::new(1.0, 6.0, Source::Estimate, "a few percent of cup starts"),
        },
        Reference {
            tiers: Self::BOTH,
            key: "starts.against_parent_club",
            band: Band::new(0.0, 0.0, Source::Rule, "a loan agreement bars it"),
        },
        Reference {
            tiers: Self::BOTH,
            key: "starts.cup_tied",
            band: Band::new(0.0, 0.0, Source::Rule, "a cup-tied player sits the competition out"),
        },
        Reference {
            tiers: Self::TOP,
            key: "keepers.most_used_share",
            band: Band::new(
                77.5,
                83.0,
                Source::Recorded,
                "season means 80.2 to 80.5% of a club's keeper appearances",
            ),
        },
        Reference {
            tiers: Self::SECOND,
            key: "keepers.most_used_share",
            band: Band::new(
                77.5,
                86.0,
                Source::Recorded,
                "season means 80.2 to 83.6% of a club's keeper appearances",
            ),
        },
        Reference {
            tiers: Self::TOP,
            key: "keepers.clubs_most_used_ge85",
            band: Band::new(
                43.0,
                60.0,
                Source::Recorded,
                "50 to 53% of clubs a season; the planning estimate was 75 to 85%",
            ),
        },
        Reference {
            tiers: Self::SECOND,
            key: "keepers.clubs_most_used_ge85",
            band: Band::new(43.0, 71.0, Source::Recorded, "50 to 64% of clubs a season"),
        },
        Reference {
            tiers: Self::TOP,
            key: "keepers.given_league_start",
            band: Band::new(1.9, 2.5, Source::Recorded, "season means 2.20 to 2.23 keepers a club"),
        },
        Reference {
            tiers: Self::SECOND,
            key: "keepers.given_league_start",
            band: Band::new(1.75, 2.45, Source::Recorded, "season means 2.04 to 2.15 keepers a club"),
        },
        Reference {
            tiers: Self::TOP,
            key: "keepers.most_used_share_of_cup_starts",
            band: Band::new(
                35.0,
                65.0,
                Source::Estimate,
                "the league number one keeps about half of the domestic cup ties",
            ),
        },
        Reference {
            tiers: Self::BOTH,
            key: "keepers.outfielder_in_goal.free_fit_keeper",
            band: Band::new(
                0.0,
                0.0,
                Source::Rule,
                "no side starts an outfielder in goal while the club has a fit keeper free",
            ),
        },
        Reference {
            tiers: Self::BOTH,
            key: "bench.keepers.two_plus.le9",
            band: Band::new(
                0.0,
                1.0,
                Source::Rule,
                "a bench of nine or fewer names one keeper",
            ),
        },
        Reference {
            tiers: Self::BOTH,
            key: "bench.subs_used",
            band: Band::new(
                3.3,
                4.2,
                Source::Recorded,
                "season means 3.4 to 3.9 substitutes used a match",
            ),
        },
    ];

    pub fn band(tier: u8, key: &str) -> Option<&'static Band> {
        Self::TABLE
            .iter()
            .find(|r| r.key == key && r.tiers.contains(&tier))
            .map(|r| &r.band)
    }

    pub fn render() -> String {
        let mut out = String::new();
        for reference in Self::TABLE {
            let tiers: Vec<String> = reference.tiers.iter().map(|t| format!("T{t}")).collect();
            let _ = writeln!(
                out,
                "{:<5} {:<46} {:<18} {:<9} {}",
                tiers.join("+"),
                reference.key,
                reference.band.range(),
                reference.band.source.label(),
                reference.band.note
            );
        }
        out
    }
}
