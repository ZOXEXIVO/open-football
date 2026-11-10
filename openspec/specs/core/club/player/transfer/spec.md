# core/club/player/transfer Specification

## Purpose
Owns a player's own transfer-market posture — big-stage pull, free-agent state, market-availability processing and the availability/block-reason diagnosis for a signed, market-exposed player.

## Requirements

### Requirement: Big-stage pull escalates through inclination, mood and formal request
A player's desire to move to a bigger competition SHALL be modeled as one continuous score with three tiers of consequence — a silent inclination the market can act on, a visible recurring mood event with a morale drag, and a formal transfer request — and escalation to a request SHALL require persistence of the itch or a denied move, not a period of unhappiness.

#### Scenario: A good player in a sub-elite league has gone unbought
- **WHEN** a player's big-stage pull has persisted for a season without an incoming bid, or a concrete move for him was blocked
- **THEN** the player's own transfer request is triggered even though he was not previously unhappy

### Requirement: A durable block-reason diagnosis explains a stalled availability
When the market fails to produce interest in a signed, market-available player, the player's transfer state SHALL retain a ranked reason explaining why, refreshed on every scan.

#### Scenario: Listed player draws no interest after weeks on the market
- **WHEN** a transfer-listed player has been scanned by the market repeatedly with no plausible buyer found
- **THEN** the block reason is recorded (ranked from a shallow "no plausible buyer" up to richer, closer-to-a-deal reasons) and refreshed on every scan, rather than leaving the absence of interest unexplained

### Requirement: A player answers a settlement offer from his own market prospects and his wish to play
Offered a mutual termination, a player SHALL name the least settlement he will accept. That figure is the wages left
on his contract, less:

- what he expects to earn elsewhere over the same span, from the wage his ability commands at the level his
  sights have fallen to, net of the time he expects to spend finding a club
- a premium for getting back to football, which grows with the time he has gone without first-team football, his
  ambition, and a standing transfer request or unhappiness

The more he belongs where he is, the less of those two deductions he SHALL allow. A man who has arrived and is still
playing there SHALL ask for all his remaining wages, because no settlement pays for what he would be giving up. Being
kept out of the side SHALL wear this down, so a frozen-out man prices his exit as before.

The figure SHALL NOT be negative, and it SHALL NOT depend on why he was listed.

#### Scenario: A starved player with a market asks for little
- **WHEN** a 25-year-old who has been frozen out for a season, holds a transfer request, and could earn nearly his current wage elsewhere is offered a settlement
- **THEN** the least he will accept is a small fraction of his remaining wages

#### Scenario: A veteran without a market holds out for his money
- **WHEN** a 33-year-old on a large wage, whose ability commands a fraction of it elsewhere and who shows little ambition, is offered a settlement
- **THEN** the least he will accept is close to all of his remaining wages

#### Scenario: Wanting to play lowers the price of leaving
- **WHEN** two players are otherwise identical, but one holds a transfer request and has gone a season without first-team football while the other is content
- **THEN** the player who wants out names the lower settlement

#### Scenario: A man living his dream is not for sale at any settlement
- **WHEN** a 25-year-old who has arrived at the level he dreamed of, starting most matches, could earn the same wage elsewhere and is offered a settlement
- **THEN** the least he will accept is all of his remaining wages

### Requirement: A player's level floor is the lowest division he will play in
A player SHALL carry a level floor on the league-reputation scale (0–10000), made of a standing and a tolerance.

The standing SHALL be the strongest of three claims:

- the division his observable ability places him at, on the same curve clubs' starter baselines are measured on
- his market reputation, carried further by senior international caps (full reach at about thirty caps)
- the division he plays in now, scaled by how much of a regular he is there, counted only once enough of his matches
  have been seen

The standing SHALL NOT exceed the higher of the division he plays in now and his held summit, when either is known. A
player with neither known has no cap on his standing.

The tolerance SHALL be one division's step-down for a player who has not recently been a regular at his standing. It
SHALL narrow to half of that for a recent regular there in his prime, and return to a full step-down as his summit's
hold fades or as he passes thirty. It SHALL narrow the same way, in proportion, as far as his observable ability alone
starts a side in the division he plays in, growing in as the youth band closes and fading as he passes thirty. It SHALL
widen continuously with youth (most at sixteen, nothing extra from twenty-three). It SHALL also widen with months
unsold or benched and with his own career plan to step down a level, each only as far as his ability falls short of
the division he plays in.

How far a division sits under the floor SHALL read from zero at the floor to one at a full refusal span under it. It
SHALL read zero when either the division's reputation or the player's standing is unknown. A division at or above the
one he plays in now SHALL always read zero: he does not refuse the level he is at.

#### Scenario: An international does not play in the third tier
- **WHEN** a 27-year-old fifty-cap international whose ability starts a top-flight side, with no record as a regular at
  that level, has just been signed by a top-flight giant (8750) and not yet picked, and is measured against that
  country's second (5500) and third (3500) divisions
- **THEN** the second division sits at or above his floor and the third sits a full refusal span under it

#### Scenario: A prime regular does not drop a division
- **WHEN** a 28-year-old who has started most of last season in a 9500 division is measured against that country's
  second division (7000) and against another country's top flight (8750)
- **THEN** the second division sits a full refusal span under his floor and the other top flight sits at or above it

#### Scenario: A frozen-out regular keeps his floor for a year
- **WHEN** the same player was dropped from the side four months ago
- **THEN** his floor still reads as a recent regular's

#### Scenario: A man good enough for his division does not drop one to play
- **WHEN** a 25-year-old backup whose observable ability starts a side in the 9500 division he plays in, with no record
  as a regular there, is measured against that country's second division (7000) and another country's top flight
  (8750), with and without a plan to prove himself on loan, and after a season on the bench
- **THEN** the second division sits a full refusal span under his floor and the other top flight sits at or above it,
  in every case
- **AND** at 34, with the plan, the second division sits at or above his floor

#### Scenario: A backup can go down to play
- **WHEN** a 24-year-old who has never been a regular in a 9500 division plays there as a backup, and is measured
  against that country's second division (7000)
- **THEN** the second division sits at or above his floor

#### Scenario: A fall is not measured from the last fall
- **WHEN** a 27-year-old regular in a 9000 division moved a year ago to a 6000 division, and is measured against a 4000
  division
- **THEN** the 4000 division sits a full refusal span under his floor
- **AND** the 6000 division he plays in reads as not under his floor at all

#### Scenario: The strongest claim stands
- **WHEN** one player's ability reads lower than his name, and another's name reads lower than the top-flight
  division he starts nearly every match in
- **THEN** the first player's standing is his name, and the second's is that division

#### Scenario: The floor never sits above the division he already plays in
- **WHEN** a player whose name reads as top-flight plays in the second division and has never been a regular higher up
- **THEN** his standing is capped at the second division's reputation

#### Scenario: The tolerance widens with youth, months unsold and a plan to step down
- **WHEN** players aged 16, 19 and 23 with no market resignation and no plan are compared, and a 27-year-old who is
  fully resigned, or fully committed to stepping down, is compared with one who is neither
- **THEN** the tolerance falls from 16 to 19 to 23 and is flat from 23 onward, and is wider for the resigned and for the
  committed 27-year-old
- **AND** a season unsold lets the international above stop refusing the third division outright

#### Scenario: No view objects to nothing
- **WHEN** either the division's reputation or the player's standing is unknown
- **THEN** the division reads as not under his floor at all

### Requirement: A transfer request takes persistence and stands until it is answered

A player SHALL make a formal transfer request only after his reasons have held continuously for two weeks. Once made,
the request SHALL stand until he moves, or until no reason has held for thirty days, when he SHALL withdraw it with a
dated record. A reason returning inside those thirty days SHALL continue the same request.

Two events with his manager SHALL each make a player mean to leave at the next registration window: his manager refusing
him in a playing-time talk, and a falling-out with his manager. Each SHALL be a named reason that makes a request at
once from when clubs may first agree terms for him in that window until it shuts, and SHALL then be spent. A refusal
SHALL be a reason only while he starts less often than he expects to. Until it is spent, a player who holds either SHALL
raise no talk with his manager, and his friction with the manager SHALL escalate no further, as if he had already asked.
Every standing request SHALL carry at least one reason, whichever route made it. A player SHALL raise a playing-time or
loan talk no sooner than that talk's cooldown allows, whether his plan or his benching raises it.

A player who has arrived SHALL raise no reason that asks for a bigger stage: outgrowing his club, ambition mismatch, a
stronger league, a new challenge, or continental football.

A player planning to step up SHALL name first-team football as his reason only when the plan came from his claim on a
place at his club, after a loan, going unanswered, and only while he is out of the side, read from his recent starts and
from last season's there. Any other player planning to step up asks for a bigger club.

#### Scenario: A one-day reason makes no request
- **WHEN** a key player's reason to leave fires for a single day and stops
- **THEN** he makes no request and nothing is listed

#### Scenario: A refused talk waits for the window
- **WHEN** a player's manager turns down his playing-time talk three months before the summer window opens
- **THEN** he makes no request and raises no talk until a fortnight before the window opens, when clubs may first agree
  terms for him
- **AND** then he asks, naming the refusal as his reason, if he is still starting less often than he expects to

#### Scenario: A refusal is spent with its window
- **WHEN** the window a refused player asked in shuts with him unsold
- **THEN** the refusal is no longer a reason, and his request lapses thirty days later unless another reason holds

#### Scenario: Back in the side by the window, he does not ask
- **WHEN** a refused player is starting as often as he expects to while the window is open
- **THEN** he makes no request, and the refusal is spent when the window shuts

#### Scenario: A falling-out asks in the window whatever his minutes
- **WHEN** a player who starts every week has fallen out with his manager and the window is open
- **THEN** he asks, naming the falling-out as his reason

#### Scenario: He does not ask the same thing every week
- **WHEN** a benched player's last playing-time talk is a week old and its cooldown has two weeks to run
- **THEN** he raises no talk that week, and no refusal can make him a request

#### Scenario: Every request says why
- **WHEN** any player holds a standing request
- **THEN** at least one reason can be read from it

#### Scenario: A flickering reason is one request
- **WHEN** a player's reason fires and lapses repeatedly for four months, never lapsing for thirty days
- **THEN** he has made one request in those four months

#### Scenario: A request lapses with a record
- **WHEN** a player's only reason to leave has been gone for thirty days and he has not moved
- **THEN** he withdraws his request, and his record shows the date

#### Scenario: Nobody at the top asks for a bigger stage
- **WHEN** a player has arrived at one of the world's biggest clubs
- **THEN** he raises no reason asking for a bigger stage, whatever his ambition

#### Scenario: A returnee passed over asks for football
- **WHEN** a player back from a loan, whose claim on a place went unanswered, plans to step up from a club that is still
  not starting him
- **THEN** he asks to leave, naming first-team football as his reason

#### Scenario: A backup who means to step up does not ask for football
- **WHEN** a backup who has not come back from a loan plans to step up from a club that is not starting him
- **THEN** he does not ask for first-team football

#### Scenario: Starting less than he hoped is not being out of the side
- **WHEN** a returnee planning to step up starts most matches, though less often than he expects to
- **THEN** he does not ask for first-team football

### Requirement: Being listed by his club is not asking to go

A move SHALL read as his own doing only when he had asked, formally or through what he wanted at the time. A club
listing him SHALL NOT read as his asking. A player sold while listed who had not asked SHALL remember being sold
against his will.

#### Scenario: Listed and sold while wanting to stay
- **WHEN** a player with no request, who wants to stay, is listed by his club and sold
- **THEN** he remembers being sold against his will by that club

#### Scenario: He asked
- **WHEN** a player with a standing request is listed and sold
- **THEN** he holds nothing against the club for it

### Requirement: A man in the side lets an unanswered request drop

A player with a standing transfer request who is starting most of his club's matches SHALL let it drop when two
transfer windows have closed on it with him still listed and unsold, with a dated record that nobody came for him. He
SHALL NOT make another request for a year, except through an event with his manager. The listing he asked for SHALL then
be withdrawn as for any request that ends. What he wants SHALL NOT change because the request dropped.

#### Scenario: Two windows and nobody came
- **WHEN** a player starting most of his club's matches has a standing request through two window closes with no move
- **THEN** he withdraws it, and his listing comes down the next day

#### Scenario: A man out of the side goes on asking
- **WHEN** the same request stands for a player who is not starting
- **THEN** it stands for as long as his reasons hold

#### Scenario: Not again inside the year
- **WHEN** his reasons to leave keep holding after he let the request drop
- **THEN** he makes no new request for a year, and may make one after it

#### Scenario: A falling-out does not wait
- **WHEN** he falls out with his manager inside that year
- **THEN** he asks to leave at once
