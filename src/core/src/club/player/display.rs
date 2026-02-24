use crate::club::player::player::Player;
use crate::utils::DateUtils;

impl Player {
    /// Compact single-line LLM representation using abbreviated skill keys.
    /// Call `Player::as_llm_legend()` once per prompt for the key descriptions.
    pub fn as_llm(&self) -> String {
        let now = chrono::Local::now().date_naive();
        let age = DateUtils::age(self.birth_date, now);
        let pos = self.positions.display_positions().join(",");
        let cond = self.player_attributes.condition_percentage();

        let t = &self.skills.technical;
        let m = &self.skills.mental;
        let p = &self.skills.physical;

        let mut s = format!(
            "P|{}|{}|{}|{}|{}|{}%|{}/{}",
            self.id, self.full_name, age, pos,
            self.preferred_foot_str(),
            cond,
            self.player_attributes.current_ability,
            self.player_attributes.potential_ability,
        );

        if self.player_attributes.is_injured {
            s.push_str(&format!("|INJ:{}d", self.player_attributes.injury_days_remaining));
        }
        if self.player_attributes.is_banned {
            s.push_str("|BAN");
        }
        if self.player_attributes.is_in_recovery() {
            s.push_str(&format!("|REC:{}d", self.player_attributes.recovery_days_remaining));
        }

        // Technical: cor,cro,dri,fin,ft,fk,hea,ls,lt,mar,pas,pen,tck,tec
        s.push_str(&format!(
            "|T:{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            t.corners as u8, t.crossing as u8, t.dribbling as u8, t.finishing as u8,
            t.first_touch as u8, t.free_kicks as u8, t.heading as u8, t.long_shots as u8,
            t.long_throws as u8, t.marking as u8, t.passing as u8, t.penalty_taking as u8,
            t.tackling as u8, t.technique as u8,
        ));

        // Mental: agg,ant,bra,cmp,con,dec,det,fla,ldr,otb,pos,tea,vis,wr
        s.push_str(&format!(
            "|M:{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            m.aggression as u8, m.anticipation as u8, m.bravery as u8, m.composure as u8,
            m.concentration as u8, m.decisions as u8, m.determination as u8, m.flair as u8,
            m.leadership as u8, m.off_the_ball as u8, m.positioning as u8, m.teamwork as u8,
            m.vision as u8, m.work_rate as u8,
        ));

        // Physical: acc,agi,bal,jum,nf,pac,sta,str,mr
        s.push_str(&format!(
            "|PH:{},{},{},{},{},{},{},{},{}",
            p.acceleration as u8, p.agility as u8, p.balance as u8, p.jumping as u8,
            p.natural_fitness as u8, p.pace as u8, p.stamina as u8, p.strength as u8,
            p.match_readiness as u8,
        ));

        let st = &self.statistics;
        if st.played > 0 || st.played_subs > 0 {
            s.push_str(&format!(
                "|S:{},{},{},{},{},{},{:.1}",
                st.played, st.played_subs, st.goals, st.assists,
                st.yellow_cards, st.red_cards, st.average_rating,
            ));
        }

        s
    }

    /// Legend explaining the compact `as_llm()` format. Include once at the top of a prompt.
    pub fn as_llm_legend() -> &'static str {
        "Player format: P|id|name|age|positions|foot|condition|ability/potential[|INJ:days|BAN|REC:days]|T:cor,cro,dri,fin,ft,fk,hea,ls,lt,mar,pas,pen,tck,tec|M:agg,ant,bra,cmp,con,dec,det,fla,ldr,otb,pos,tea,vis,wr|PH:acc,agi,bal,jum,nf,pac,sta,str,mr[|S:played,subs,goals,assists,yel,red,avg_rating]\nAll skills 1-20."
    }
}
