# core/match

Simulates a single football match tick-by-tick — kickoff through full time (and extra time / penalties where applicable) — covering player decision-making, ball physics, officiating, substitutions, fatigue and injury, and produces the matchday squad selection and the recorded match result that other modules consume.

This is a landing page, not an OpenSpec capability — each behavior is documented in one of the child specs below.

`match/engine`'s internal nesting is capped at its direct children — deeper internal state-machine folders are implementation detail rolled into the specs below rather than split further.

## Child specs

- [engine/ball](engine/ball/spec.md)
- [engine/flow](engine/flow/spec.md)
- [engine/officiating](engine/officiating/spec.md)
- [engine/player](engine/player/spec.md)
- [engine/psychology](engine/psychology/spec.md)
- [engine/rating](engine/rating/spec.md)
- [engine/state](engine/state/spec.md)
- [engine/substitution](engine/substitution/spec.md)
- [engine/tactics](engine/tactics/spec.md)
- [engine/teamplay](engine/teamplay/spec.md)
- [squad/selection](squad/selection/spec.md)
