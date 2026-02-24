use crate::club::player::player::Player;
use crate::utils::DateUtils;

impl Player {
    pub fn as_llm(&self) -> String {
        let now = chrono::Local::now().date_naive();
        let age = DateUtils::age(self.birth_date, now);
        let positions = self.positions.display_positions().join(", ");
        let condition_pct = self.player_attributes.condition_percentage();

        let t = &self.skills.technical;
        let m = &self.skills.mental;
        let p = &self.skills.physical;

        let mut s = format!(
            "ID:{}, {}, age:{}, positions:[{}], foot:{}, condition:{}%, ability:{}/{}, height:{}, weight:{}",
            self.id,
            self.full_name,
            age,
            positions,
            self.preferred_foot_str(),
            condition_pct,
            self.player_attributes.current_ability,
            self.player_attributes.potential_ability,
            self.player_attributes.height,
            self.player_attributes.weight,
        );

        if self.player_attributes.is_injured {
            s.push_str(&format!(", INJURED({}d remaining)", self.player_attributes.injury_days_remaining));
        }
        if self.player_attributes.is_banned {
            s.push_str(", BANNED");
        }
        if self.player_attributes.is_in_recovery() {
            s.push_str(&format!(", RECOVERING({}d remaining)", self.player_attributes.recovery_days_remaining));
        }

        s.push_str(&format!(
            "\n  technical(1-20): corners:{}, crossing:{}, dribbling:{}, finishing:{}, first_touch:{}, free_kicks:{}, heading:{}, long_shots:{}, long_throws:{}, marking:{}, passing:{}, penalty_taking:{}, tackling:{}, technique:{}",
            t.corners as u8, t.crossing as u8, t.dribbling as u8, t.finishing as u8,
            t.first_touch as u8, t.free_kicks as u8, t.heading as u8, t.long_shots as u8,
            t.long_throws as u8, t.marking as u8, t.passing as u8, t.penalty_taking as u8,
            t.tackling as u8, t.technique as u8,
        ));

        s.push_str(&format!(
            "\n  mental(1-20): aggression:{}, anticipation:{}, bravery:{}, composure:{}, concentration:{}, decisions:{}, determination:{}, flair:{}, leadership:{}, off_the_ball:{}, positioning:{}, teamwork:{}, vision:{}, work_rate:{}",
            m.aggression as u8, m.anticipation as u8, m.bravery as u8, m.composure as u8,
            m.concentration as u8, m.decisions as u8, m.determination as u8, m.flair as u8,
            m.leadership as u8, m.off_the_ball as u8, m.positioning as u8, m.teamwork as u8,
            m.vision as u8, m.work_rate as u8,
        ));

        s.push_str(&format!(
            "\n  physical(1-20): acceleration:{}, agility:{}, balance:{}, jumping:{}, natural_fitness:{}, pace:{}, stamina:{}, strength:{}, match_readiness:{}",
            p.acceleration as u8, p.agility as u8, p.balance as u8, p.jumping as u8,
            p.natural_fitness as u8, p.pace as u8, p.stamina as u8, p.strength as u8,
            p.match_readiness as u8,
        ));

        let stats = &self.statistics;
        if stats.played > 0 || stats.played_subs > 0 {
            s.push_str(&format!(
                "\n  stats: played:{}, subs:{}, goals:{}, assists:{}, yellow:{}, red:{}, avg_rating:{:.1}",
                stats.played, stats.played_subs, stats.goals, stats.assists,
                stats.yellow_cards, stats.red_cards, stats.average_rating,
            ));
        }

        s
    }
}
