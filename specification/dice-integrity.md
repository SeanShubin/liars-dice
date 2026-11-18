# Dice Integrity

## Trust
- Players are not trusted
- Players are not asked to trust each other
- The source code and the build pipeline are public
- The relay is trusted to deal fairly

## Verifiability
- Every die in a round is fixed before any claim in that round
- The event log is sufficient to recompute every die of every round
- A player can detect dice that disagree with the event log
- Verification requires nothing beyond the event log
- A player can read the event log of a complete match they played in

## Rolls
- No player can choose or influence their own roll
- No player can alter their roll after the fact
- Every round deals the dice limit to every seat
- A player uses as many of their seat's dice as they hold

## Secrecy
- No player can learn another player's dice before the reveal
- Dice are secret only until the round resolves
- Resolving a round reveals every player's dice

## Out of Scope
- Collusion between players is not prevented
- No scheme can prevent it, and detecting it is not attempted
