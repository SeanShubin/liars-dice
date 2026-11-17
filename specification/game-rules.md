# Game Rules

## Design Principles
- A player does not lose a die unless they make a false claim or fail to claim in time
- A true claim is always available to the current player
- Every variant preserves the design principles

## Default Rules
- The default rules are the rules with every variant off
- Named variants from other rulesets are variants, never the default

## Game Mechanics
- A match requires two or more players to start
- A match has at most ten players
- each player starts with 5 dice
- each player takes a turn until the round ends
- new rounds continue until the match ends

## Match
- Rounds continue until only one player remains
- The last remaining player wins the match

## Elimination
- A player with no dice is eliminated
- An eliminated player takes no further part in the match
- A player lasts longer than another player if the other is eliminated first

## Ranking
- A match produces a ranking of its players
- A player beats another player by lasting longer in the match

## Round
- All players roll their dice
- Players know their own dice
- Players don't know their opponent's dice
- Players take turns until a turn triggers round end
- Players reveal their dice
- The round is resolved
- Players start a new round

## Turn
- Current player makes a claim
- Claim must be one of the following
  - Raise
  - Liar
  - Spot On
- A claim must be possible according to the number of dice in play
- A player may make a claim they know to be false
- Claim is restricted by previous claim, if there is one this round
- A turn produces at most one claim
- A player acts only on their own turn

## Turn Order
- Players are arranged into a turn order that wraps
- Turn order is chosen at random when the match starts
- Players are not reordered during a match
- The previous player is the player before the current player in turn order

## Time Limit
- A match has a configurable time limit for taking a turn
- The time limit applies to turns only
- A player that fails to claim within the time limit loses all of their dice

## Exchanges
- A round ends with an exchange between the player who ends it and the previous player
- The player who ends a round wins the exchange if and only if their claim is true
- A player who fails to claim in time loses the exchange
- The winner of an exchange makes the first claim of the next round

## Raise
- Declare that at least a certain amount of a certain face value is present among all players
- If the quantity is the same the face value must go up
- If the quantity is greater the face value is unrestricted
- Quantity may not go down
- Turn passes to next player

## Spot On
- Ends the round
- Declare that the previous claim is exact rather than at least
- If player is correct, gains one die, otherwise loses 1 die

## Liar
- Ends the round
- Declare that the previous claim is false
- If player is correct, previous player loses 1 die
- If player is incorrect, loses 1 die

## Variants
- Each variant is a single configurable boolean
- Variants are fixed for the whole match

### Ones Wild
- Default: ones are not wild
- Variant: ones count toward every face value when a claim is resolved
- Variant: a claim may not name the one face
- Both variant effects are controlled by one setting
