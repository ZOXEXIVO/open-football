use core::Player;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WatchKey {
    pub id: u32,
    pub fingerprint: u64,
}

#[derive(Debug)]
pub struct MalformedWatchKey;

impl WatchKey {
    const FINGERPRINT_DIGITS: usize = 16;

    pub fn of(player: &Player) -> Self {
        WatchKey {
            id: player.id,
            fingerprint: player.identity_fingerprint(),
        }
    }

    pub fn matches(&self, player: &Player) -> bool {
        *self == Self::of(player)
    }

    /// Integer parsing alone would let `+7` or `00ABC…` through.
    fn is_canonical(id: &str, fingerprint: &str) -> bool {
        !id.is_empty()
            && id.bytes().all(|byte| byte.is_ascii_digit())
            && fingerprint.len() == Self::FINGERPRINT_DIGITS
            && fingerprint
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    }
}

impl Display for WatchKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{:016x}", self.id, self.fingerprint)
    }
}

impl FromStr for WatchKey {
    type Err = MalformedWatchKey;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let (id, fingerprint) = raw.split_once('-').ok_or(MalformedWatchKey)?;
        if !Self::is_canonical(id, fingerprint) {
            return Err(MalformedWatchKey);
        }
        Ok(WatchKey {
            id: id.parse().map_err(|_| MalformedWatchKey)?,
            fingerprint: u64::from_str_radix(fingerprint, 16).map_err(|_| MalformedWatchKey)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use core::club::player::builder::PlayerBuilder;
    use core::shared::fullname::FullName;
    use core::{PersonAttributes, PlayerAttributes, PlayerPositions, PlayerSkills};

    fn player(id: u32, birth: NaiveDate) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("John".to_string(), "Smith".to_string()))
            .birth_date(birth)
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions { positions: vec![] })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn born(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(1990, 5, day).unwrap()
    }

    #[test]
    fn display_and_parse_round_trip() {
        let key = WatchKey {
            id: 42,
            fingerprint: 0x00ab_cdef_0123_4567,
        };
        assert_eq!(key.to_string(), "42-00abcdef01234567");
        assert_eq!(key.to_string().parse::<WatchKey>().unwrap(), key);
    }

    #[test]
    fn parse_rejects_malformed_keys() {
        for raw in [
            "4200abcdef01234567",
            "x2-00abcdef01234567",
            "+42-00abcdef01234567",
            "-00abcdef01234567",
            "4294967296-00abcdef01234567",
            "42-00ABCDEF01234567",
            "42-00abcdef0123456g",
            "42-00abcdef0123456",
            "42-00abcdef012345678",
        ] {
            assert!(raw.parse::<WatchKey>().is_err(), "{raw} parsed");
        }
    }

    #[test]
    fn matches_needs_the_same_identity() {
        let key = WatchKey::of(&player(7, born(17)));
        assert!(key.matches(&player(7, born(17))));
        assert!(!key.matches(&player(7, born(18))));
        assert!(!key.matches(&player(8, born(17))));
    }
}
