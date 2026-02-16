#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IntentionKind {
    LookingToShoot,
    HoldUpPlay,
    SeekingThroughBall(u32),
    SwitchPlay,
    MakeRun,
    OneTwo(u32),
    BeatDefender,
    TrackRunner(u32),
    HoldPosition,
    DeliverCross,
}

#[derive(Debug, Clone, Copy)]
pub struct TimedIntention {
    pub kind: IntentionKind,
    pub created_tick: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MemoryEventType {
    ShotTaken,
    PassIntercepted,
    PassCompleted,
    TackleLost,
    TackleWon,
    LostPossession,
    ReceivedBall,
    MissedTeammateRun,
}

const MAX_INTENTIONS: usize = 3;
const MAX_EVENTS: usize = 8;
const SHOT_COOLDOWN_TICKS: u64 = 300;

#[derive(Debug, Clone)]
pub struct PlayerMemory {
    intentions: Vec<TimedIntention>,
    recent_events: Vec<MemoryEventType>,

    pub confidence: f32,

    pub last_shot_tick: u64,
    pub shots_taken: u32,
    pub shots_on_target: u32,

    pub last_pass_tick: u64,
    pub pass_streak: u32,

    pub last_xg: f32,
    pub last_xg_tick: u64,
}

impl PlayerMemory {
    pub fn new() -> Self {
        PlayerMemory {
            intentions: Vec::with_capacity(MAX_INTENTIONS),
            recent_events: Vec::with_capacity(MAX_EVENTS),
            confidence: 0.5,
            last_shot_tick: 0,
            shots_taken: 0,
            shots_on_target: 0,
            last_pass_tick: 0,
            pass_streak: 0,
            last_xg: 0.0,
            last_xg_tick: 0,
        }
    }

    pub fn push_intention(&mut self, kind: IntentionKind, tick: u64) {
        // Remove existing intention of same kind
        self.intentions.retain(|i| {
            std::mem::discriminant(&i.kind) != std::mem::discriminant(&kind)
        });

        if self.intentions.len() >= MAX_INTENTIONS {
            self.intentions.remove(0);
        }

        self.intentions.push(TimedIntention {
            kind,
            created_tick: tick,
        });
    }

    pub fn top_intention(&self) -> Option<&TimedIntention> {
        self.intentions.last()
    }

    pub fn has_intention(&self, kind: &IntentionKind) -> bool {
        self.intentions.iter().any(|i| {
            std::mem::discriminant(&i.kind) == std::mem::discriminant(kind)
        })
    }

    pub fn record_event(&mut self, event: MemoryEventType) {
        if self.recent_events.len() >= MAX_EVENTS {
            self.recent_events.remove(0);
        }
        self.recent_events.push(event);

        // Update confidence based on event
        match event {
            MemoryEventType::ShotTaken => {}
            MemoryEventType::PassCompleted => {
                self.confidence = (self.confidence + 0.03).min(1.0);
                self.pass_streak += 1;
            }
            MemoryEventType::PassIntercepted => {
                self.confidence = (self.confidence - 0.08).max(0.0);
                self.pass_streak = 0;
            }
            MemoryEventType::TackleWon => {
                self.confidence = (self.confidence + 0.05).min(1.0);
            }
            MemoryEventType::TackleLost => {
                self.confidence = (self.confidence - 0.06).max(0.0);
            }
            MemoryEventType::LostPossession => {
                self.confidence = (self.confidence - 0.04).max(0.0);
                self.pass_streak = 0;
            }
            MemoryEventType::ReceivedBall => {
                self.confidence = (self.confidence + 0.02).min(1.0);
            }
            MemoryEventType::MissedTeammateRun => {
                self.confidence = (self.confidence - 0.02).max(0.0);
            }
        }
    }

    pub fn can_shoot(&self, current_tick: u64) -> bool {
        if self.last_shot_tick == 0 {
            return true;
        }
        current_tick.saturating_sub(self.last_shot_tick) >= SHOT_COOLDOWN_TICKS
    }

    pub fn record_shot(&mut self, tick: u64, on_target: bool) {
        self.last_shot_tick = tick;
        self.shots_taken += 1;
        if on_target {
            self.shots_on_target += 1;
            self.confidence = (self.confidence + 0.05).min(1.0);
        } else {
            self.confidence = (self.confidence - 0.03).max(0.0);
        }
        self.record_event(MemoryEventType::ShotTaken);
    }

    pub fn decay(&mut self, _current_tick: u64) {
        // Regress confidence toward 0.5
        if self.confidence > 0.5 {
            self.confidence = (self.confidence - 0.02).max(0.5);
        } else if self.confidence < 0.5 {
            self.confidence = (self.confidence + 0.02).min(0.5);
        }

        // Remove old intentions (older than 500 ticks are stale)
        // We don't have the tick here easily, so just trim if full
        if self.intentions.len() > 1 {
            self.intentions.remove(0);
        }

        // Decay pass streak
        if self.pass_streak > 0 {
            self.pass_streak = self.pass_streak.saturating_sub(1);
        }
    }
}

impl Default for PlayerMemory {
    fn default() -> Self {
        Self::new()
    }
}
