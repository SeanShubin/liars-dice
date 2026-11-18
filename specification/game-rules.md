# Game Rules

## Design Principles
- A player does not lose a die unless they make a false claim
- A true claim is always available to the current player
- Every variant preserves the design principles

## Default Rules
- The default rules are the rules with every variant off
- Named variants from other rulesets are variants, never the default

## Game Mechanics
- A match requires two or more players to start
- A match has at most ten players
- Each player starts with five dice
- A player holds at most five dice
- Each player takes a turn until the round ends
- New rounds continue until the match ends

## Match
- Rounds continue until only one player remains
- A match in which only one player remains is complete
- The last remaining player wins a complete match
- A match has the same players from its first round to its last
- A match cannot continue while any of its players is absent
- A player that loses their connection is absent
- A player who reconnects resumes their seat
- The host can see which players are absent
- The event log of a match is retained whether or not the match is completed

## Elimination
- A player with no dice is eliminated
- An eliminated player takes no further part in the match
- A player lasts longer than another player if the other is eliminated first

## Ranking
- A complete match produces a ranking of its players
- A player beats another player by lasting longer in the match

## Round
- All players roll their dice
- Players know their own dice
- Players don't know their opponent's dice
- Players take turns until the round ends
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

## Exchanges
- A round ends with an exchange between the player who ends it and the previous player
- Exactly one player in an exchange is affected by its resolution
- The affected player makes the first claim of the next round
- If the affected player is eliminated, the other player in the exchange makes the first claim

## Raise
- Declare that at least a certain amount of a certain face value is present among all players
- If the quantity is the same the face value must go up
- If the quantity is greater the face value is unrestricted
- Quantity may not go down
- Turn passes to next player

## Spot On
- Ends the round
- Declare that the previous claim is exact rather than at least
- A correct claim below the dice limit gains one die
- A correct claim at the dice limit gains a shield token
- An incorrect claim loses one die

## Liar
- Ends the round
- Declare that the previous claim is false
- If player is correct, previous player loses 1 die
- If player is incorrect, loses 1 die

## Shield Tokens
- A shield token prevents its holder's next die loss
- A shield token is spent when it prevents a die loss
- A player may hold any number of shield tokens
- Every player can see how many shield tokens each player holds

## Variants
- Each variant is a single configurable boolean
- Variants are fixed for the whole match

### Ones Wild
- Default: ones are not wild
- Variant: ones count toward every face value when a claim is resolved
- Variant: a claim may not name the one face
- Both variant effects are controlled by one setting
