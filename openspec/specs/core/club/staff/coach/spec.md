# core/club/staff/coach Specification

## Purpose
The manager's standing plan for his squad: a role for every shirt of the formation he plays, the share of the
season's football each role carries, a ledger of how far each player is from his share, and the commitments that set
floors on it. Selection, renewals, the shop window and succession read the plan; nothing else decides them.

## Requirements

### Requirement: The plan gives every shirt of the formation a first choice and the men behind him
A manager's plan SHALL be built on the formation his team plays. For each shirt it SHALL name a first choice, and
behind him the players who would be used there, ordered by the level his staff perceive and by how well each plays
that position. A player who can play several shirts SHALL be planned in the one where the squad needs him most.

A player's planned role SHALL follow from where he stands for a shirt: built around, a starter, a rotation player, a
deputy with a competition of his own, an heir being groomed for a named incumbent, a prospect on a development path,
cover, in the shop window, or not in the plans. The plan SHALL read only what coaches can observe.

The plan SHALL be revised monthly, on a change of formation, and on the day a player joins or leaves. A new manager
SHALL start a plan of his own.

#### Scenario: A back four
- **WHEN** a manager plays four defenders and has six fit ones
- **THEN** the plan names four of them as first choices for the four shirts, and the other two behind

#### Scenario: A change of shape
- **WHEN** the manager changes from four defenders to three
- **THEN** the plan is revised and names three first-choice defenders

#### Scenario: A full-back who also plays on the wing
- **WHEN** a player is the squad's best left-back and its third-best left winger
- **THEN** he is planned as the left-back

#### Scenario: A trusted man keeps his role
- **WHEN** the manager has started a defender in every match for two months and a marginally stronger defender arrives
- **THEN** the plan keeps the incumbent as first choice for that shirt until the newcomer has shown more

### Requirement: Planned shares add up to the football there is
Each planned role SHALL carry a share of the team's football over the season. Summed over the squad, the shares SHALL
equal the football the formation plays: eleven shirts for every minute of every competitive fixture the club expects.

How the football of one shirt is divided between its first choice and the men behind him SHALL be continuous in:
- how many fixtures the club expects this season across its competitions, a busier club spreading football wider;
- the manager's own tendency to rotate;
- the gap in level between the first choice and the next man;
- the demands of the position, a goalkeeper sharing least.

#### Scenario: A club with one competition
- **WHEN** a club expects league fixtures a week apart and little else
- **THEN** its first choices are planned for most of the football and its cover for little

#### Scenario: A club in Europe
- **WHEN** a club expects sixty fixtures in the season
- **THEN** each outfield first choice is planned for a smaller share and each player behind him for a larger one

#### Scenario: The shares add up
- **WHEN** the shares of every player in a plan are summed
- **THEN** the total is the football of eleven shirts

#### Scenario: A clear gap
- **WHEN** a shirt's first choice is far better than the next man
- **THEN** the first choice is planned for nearly all of that shirt's football

### Requirement: The plan keeps a ledger of football owed
For every player the plan SHALL keep how much football he has had since his role was set and how much his share
would have given him, counted in the fixtures he was eligible for. The difference, in matches, is the football he is
owed or is ahead by. Fixtures he missed through injury, suspension or international duty SHALL NOT count as owed.

The ledger SHALL be the one reading of "this player needs football" that selection, the bench and the release of
players to the club's other sides use, for every role alike.

#### Scenario: A rotation player left out for a month
- **WHEN** a rotation player planned for a third of the football has not played in five eligible fixtures
- **THEN** he is owed more than a match, and that is what selection reads

#### Scenario: A starter who has played everything
- **WHEN** a first choice has started every fixture of a crowded month
- **THEN** he is ahead of his share, and that is what selection reads

#### Scenario: Injured for six weeks
- **WHEN** a player misses six weeks injured
- **THEN** he is owed nothing for those weeks when he returns

### Requirement: Commitments set a floor under a player's share
A player's planned share SHALL NOT be below what the club has committed to him:
- the appearances a loan agreement names, for a borrowed player;
- the role the board bought him for, for a signing still inside the period its mandate covers;
- a promise the manager made him, until it is kept or broken.

When commitments ask for more football than a shirt has, the plan SHALL keep them in that order and SHALL record that
the later ones cannot be kept.

#### Scenario: A loan with agreed appearances
- **WHEN** a club borrows a player on an agreement that names twenty appearances
- **THEN** his planned share is at least what twenty appearances need, whatever his rank

#### Scenario: A marquee signing
- **WHEN** the board buys a player as a starter and the manager's depth chart places him second for his shirt
- **THEN** he is planned at a starter's share while the mandate holds

### Requirement: A player the club is selling is put in the shop window
A player SHALL hold the shop-window role when the club has decided to sell him and has put him on the market, and
only then. The role SHALL carry a share of football in fixtures that leave room, friendlies and pre-season included,
sized by how much being seen can still raise his price: larger for a player whose value has fallen for want of
football, and nothing for a player whose price would not move. The role SHALL end when the club takes him off the
market, when a move for him is agreed, or when the board's review decides otherwise.

A manager who has frozen a player out SHALL give the shop-window share only as far as his relationship with the
board requires.

#### Scenario: A listed player in pre-season
- **WHEN** a club lists a 27-year-old regular in June
- **THEN** he is planned for a real share of the pre-season friendlies and of the early fixtures that leave room

#### Scenario: A stalled player nobody is selling
- **WHEN** a 26-year-old has not played for two seasons and the club has not put him on the market
- **THEN** he does not hold the shop-window role

#### Scenario: The move is agreed
- **WHEN** the club accepts a bid for a player in the shop window
- **THEN** the role ends and he is not selected

### Requirement: A departure moves the man behind up
On the day a player leaves the club, the plan SHALL be revised for the shirts he held. The next man for each shirt
SHALL take the role above his own, and an heir groomed for the departed player SHALL become the first choice.

#### Scenario: The starter is sold
- **WHEN** a club sells its first-choice left-back and has an heir planned behind him
- **THEN** on the day of the sale the heir is the planned first choice at left-back

#### Scenario: The right shirt
- **WHEN** the same club also has a strong young centre-back
- **THEN** the centre-back's role does not change because a left-back was sold

### Requirement: A prospect's share follows the staff's belief in him
A player on a development role SHALL be planned a share that rises continuously with how far the staff believe he
will go beyond his present level and with how close he is to the level of the man ahead of him, and that is larger at
a club whose philosophy is to develop players. The share SHALL NOT step at a birthday.

#### Scenario: Two prospects of different ages
- **WHEN** a 22-year-old and a 20-year-old stand equally far behind the same first choice and the staff believe in them equally
- **THEN** they are planned equal shares

#### Scenario: A developing club
- **WHEN** the same prospect is held by a club whose philosophy is to develop and sell
- **THEN** his planned share is larger than at a club that buys to compete
