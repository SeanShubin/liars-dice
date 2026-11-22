# Multiplayer

## Scope
- Every match in a web game is hosted

## Hosting
- A player can host a match
- A player cannot host a match while holding a seat in another match
- The host can start the match
- The host can add an AI player to a hosted match
- An AI player does not authenticate
- An AI player's turn is taken by the client that added it
- Two matches never share an identity
- A repeated request to host creates no second match

## Joining
- Every match that can be joined is listed
- A started match is listed
- A match that is over is not listed
- A listed match shows its host
- A listed match shows how many players have joined
- A listed match shows whether it can be joined
- A listed match shows whether the player asking holds a seat in it
- A player can join a hosted match
- A player cannot join a match that has started
- A player cannot join a match while holding a seat in another match
- A client returns its player to the match they hold a seat in, without asking
- The host can bar a player from joining the match
- The host can lift a bar
- A match is not listed to a player barred from it
- A seat is given a colour when it is taken
- A player can change their seat's colour until the match starts
- A player cannot change their seat's colour once the match has started
- A colour another player has chosen cannot be chosen
- Two players never share a seat
- A player holds one seat in a match

## Unattended
- An unattended client shows what it has received
- An unattended client stays able to act

## Ending
- A match is complete when every seat present in it says so
- A log says whether its match is complete
- Only the host can cancel a match
- The host can cancel a match that has not started
- The host can cancel a started match in which no event has been recorded for a minute
- The host cannot cancel a started match in which an event has been recorded within the minute
- A cancel the host cannot yet make is refused with the time remaining
- A client shows the host the time remaining before they can cancel
- A match in which no event has been recorded for an hour is cancelled
- A host who leaves a match that has not started cancels it
- A cancelled match is over
- A cancelled match cannot be joined
- No event is recorded in a match that is over
- A log says whether its match was cancelled
- A log says whether a cancelled match was cancelled by its host

## Ranked
- A match is ranked when every seat in it is held by an authenticated player
- Whether a match is ranked is settled when it starts
- Whether a match is ranked is derived from the accounts in its log

## Claims
- A client does not send an illegal claim
- A client ignores an illegal claim it receives
- The relay records every message it receives
- A message either happens entirely or not at all

## Versions
- A match records the protocol version of the client that hosted it
- A client cannot join a match whose protocol version differs from its own
- A client cannot load a save whose protocol version differs from its own

## Saves
- The host can save a match
- The host can name a save
- A save records the time it was made
- A save is visible only to its host
- The host can find a save by name
- The host can find a save by the time it was made
- The host can load a saved match
- A loaded match continues with the players that started it
- The host can delete a save
- A deleted save is not listed
