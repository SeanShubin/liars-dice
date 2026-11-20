# Multiplayer

## Scope
- Every match in a web game is hosted

## Hosting
- A player can host a match
- The host can start the match
- The host can add an AI player to a hosted match
- An AI player does not authenticate
- An AI player's turn is taken by the client that added it
- Two matches never share an identity
- A repeated request to host creates no second match

## Joining
- Every match that can be joined is listed
- A listed match shows its host
- A listed match shows how many players have joined
- A player can join a hosted match
- A player cannot join a match that has started
- The host can bar a player from joining the match
- The host can lift a bar
- A match is not listed to a player barred from it
- A player chooses the colour of their dice when they join
- A colour another player has chosen cannot be chosen
- Two players never share a seat
- A player holds one seat in a match
- A match whose host has left cannot be joined

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
