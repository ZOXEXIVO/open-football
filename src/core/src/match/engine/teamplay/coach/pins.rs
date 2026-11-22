//! **The harness's pins**: a stated coach for either side
//! (`OF_COACH=home:<spec>,away:<spec>`) and a stated plan for both
//! (`OF_PLAN=neutral|<dials>`). Read once per process. Without a pin a side
//! is managed by its own coach, or by one drawn for the match.

use crate::r#match::engine::teamplay::coach::card::CoachCard;
use crate::r#match::engine::teamplay::coach::plan::GamePlan;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TeamPins {
    pub home_coach: Option<CoachCard>,
    pub away_coach: Option<CoachCard>,
    pub plan: Option<GamePlan>,
}

impl TeamPins {
    pub const USAGE: &'static str = "OF_COACH=home:<coach>,away:<coach> where <coach> is drawn, a preset \
         (pressing, lowblock, cautious, risktaker, stubborn, adaptable) or keys risk, caution, \
         stubborn, adapt, volatile, read, manage, motivate and plan dials joined by `/` \
         (risk=0.9/press=0.8); OF_PLAN=neutral or plan dials block, press, compact, width, \
         tempo, direct, commit joined by `/` (block=0.8/press=0.9)";

    pub fn parse(coach: Option<&str>, plan: Option<&str>) -> Result<Self, String> {
        let mut pins = TeamPins::default();
        if let Some(coach) = coach.filter(|c| !c.trim().is_empty()) {
            for part in coach.split(',') {
                let (venue, spec) = part
                    .split_once(':')
                    .ok_or_else(|| format!("coach pin `{part}` names no side (home: or away:)"))?;
                let card = if spec.trim().eq_ignore_ascii_case("drawn") {
                    None
                } else {
                    Some(CoachCard::from_pin(spec)?)
                };
                match venue.trim().to_ascii_lowercase().as_str() {
                    "home" => pins.home_coach = card,
                    "away" => pins.away_coach = card,
                    other => return Err(format!("unknown side `{other}`; use home or away")),
                }
            }
        }
        if let Some(plan) = plan.filter(|p| !p.trim().is_empty()) {
            pins.plan = Some(GamePlan::from_pin(plan)?);
        }
        Ok(pins)
    }

    /// The pins this process runs under, or why they cannot be read.
    pub fn from_env() -> &'static Result<TeamPins, String> {
        static PINS: OnceLock<Result<TeamPins, String>> = OnceLock::new();
        PINS.get_or_init(|| {
            Self::parse(
                std::env::var("OF_COACH").ok().as_deref(),
                std::env::var("OF_PLAN").ok().as_deref(),
            )
        })
    }

    /// The pins in force. An unreadable pin stops the process before a
    /// match is played under a coach nobody asked for.
    pub fn current() -> &'static TeamPins {
        match Self::from_env() {
            Ok(pins) => pins,
            Err(why) => panic!("{why}\n{}", Self::USAGE),
        }
    }

    pub fn coach(&self, home: bool) -> Option<CoachCard> {
        if home {
            self.home_coach
        } else {
            self.away_coach
        }
    }

    pub fn describe(&self) -> String {
        let coach = |card: &Option<CoachCard>| match card {
            Some(card) => card.describe(),
            None => "drawn".to_string(),
        };
        let plan = match &self.plan {
            Some(plan) => plan.describe(),
            None => "the coach's".to_string(),
        };
        format!(
            "coach home: {}; coach away: {}; plan: {}",
            coach(&self.home_coach),
            coach(&self.away_coach),
            plan
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_read_both_sides_and_the_plan() {
        let pins = TeamPins::parse(
            Some("home:pressing,away:risk=0.9/caution=0.1"),
            Some("neutral"),
        )
        .unwrap();
        assert_eq!(pins.home_coach.unwrap().preference.press, 0.9);
        let away = pins.away_coach.unwrap();
        assert_eq!(away.temperament.risk_tolerance, 0.9);
        assert_eq!(away.temperament.caution, 0.1);
        assert_eq!(pins.plan, Some(GamePlan::neutral()));
        assert!(pins.describe().contains("plan: block 0.50"));
    }

    #[test]
    fn no_pin_draws_the_coach() {
        let pins = TeamPins::parse(None, None).unwrap();
        assert_eq!(pins, TeamPins::default());
        assert!(pins.describe().contains("coach home: drawn"));
        let drawn = TeamPins::parse(Some("home:drawn"), None).unwrap();
        assert_eq!(drawn.home_coach, None);
    }

    #[test]
    fn an_unknown_pin_is_refused() {
        assert!(TeamPins::parse(Some("home:genius"), None).is_err());
        assert!(TeamPins::parse(Some("middle:pressing"), None).is_err());
        assert!(TeamPins::parse(Some("pressing"), None).is_err());
        assert!(TeamPins::parse(Some("home:risk=2"), None).is_err());
        assert!(TeamPins::parse(None, Some("deep")).is_err());
    }
}
