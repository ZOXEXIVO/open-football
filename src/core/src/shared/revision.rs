use std::sync::atomic::{AtomicU64, Ordering};

/// One state of a roster's membership. Every structural change draws a
/// fresh value from a process-wide counter, so a roster replaced wholesale
/// never repeats a revision an index has already seen. Compared for
/// equality only: the values depend on thread interleaving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RosterRevision(u64);

impl RosterRevision {
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        RosterRevision(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, Staff, StaffClubContract,
        StaffCollection, StaffPosition, StaffStatus, StaffStub,
    };
    use chrono::NaiveDate;

    fn player(id: u32) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".to_string(), format!("Player{id}")))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 15,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn staff(id: u32) -> Staff {
        let mut staff = StaffStub::build();
        staff.id = id;
        staff
    }

    fn manager(id: u32) -> Staff {
        let mut manager = staff(id);
        manager.contract = Some(StaffClubContract::new(
            1_000,
            NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
            StaffPosition::Manager,
            StaffStatus::Active,
        ));
        manager
    }

    #[test]
    fn every_structural_change_moves_a_player_roster_revision() {
        let mut roster = PlayerCollection::new(vec![player(1), player(2)]);
        let mut seen = vec![roster.revision()];

        roster.add(player(3));
        assert!(!seen.contains(&roster.revision()));
        seen.push(roster.revision());

        roster.add_range(vec![player(4)]);
        assert!(!seen.contains(&roster.revision()));
        seen.push(roster.revision());

        roster.take_player(&1);
        assert!(!seen.contains(&roster.revision()));

        let unchanged = roster.revision();
        assert!(roster.take_player(&99).is_none());
        assert_eq!(roster.revision(), unchanged);
    }

    #[test]
    fn every_structural_change_moves_a_staff_roster_revision() {
        let mut roster = StaffCollection::new(vec![staff(1), manager(2), staff(3)]);
        let mut seen = vec![roster.revision()];

        roster.push(staff(4));
        assert!(!seen.contains(&roster.revision()));
        seen.push(roster.revision());

        roster.take_by_id(1);
        assert!(!seen.contains(&roster.revision()));
        seen.push(roster.revision());

        roster.take_by_position(StaffPosition::Manager);
        assert!(!seen.contains(&roster.revision()));
        seen.push(roster.revision());

        roster.retain(|member| member.id != 3);
        assert!(!seen.contains(&roster.revision()));

        let unchanged = roster.revision();
        roster.retain(|_| true);
        assert!(roster.take_by_id(99).is_none());
        assert_eq!(roster.revision(), unchanged);
    }

    #[test]
    fn edits_in_place_keep_the_revision() {
        let mut players = PlayerCollection::new(vec![player(1), player(2)]);
        let revision = players.revision();
        players.find_mut(1).unwrap().player_attributes.condition += 1;
        for member in players.iter_mut() {
            member.player_attributes.condition += 1;
        }
        assert_eq!(players.revision(), revision);

        let mut staff = StaffCollection::new(vec![staff(1)]);
        let revision = staff.revision();
        staff.find_mut(1).unwrap().birth_date = NaiveDate::from_ymd_opt(1980, 1, 1).unwrap();
        for member in staff.iter_mut() {
            member.birth_date = NaiveDate::from_ymd_opt(1981, 1, 1).unwrap();
        }
        assert_eq!(staff.revision(), revision);
    }

    #[test]
    fn a_clone_keeps_the_revision_and_new_rosters_differ() {
        let players = PlayerCollection::new(vec![player(1)]);
        assert_eq!(players.clone().revision(), players.revision());
        assert_ne!(
            PlayerCollection::new(Vec::new()).revision(),
            players.revision()
        );

        let staff = StaffCollection::new(Vec::new());
        assert_eq!(staff.clone().revision(), staff.revision());
        assert_ne!(
            StaffCollection::new(Vec::new()).revision(),
            staff.revision()
        );
    }
}
