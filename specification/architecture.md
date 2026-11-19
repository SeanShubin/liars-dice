# Architecture

## Technology Stack
- Rust programming language
- egui for the minimal presentation
- Bevy rendering engine for the pretty presentation
- Amazon Web Services

## Dependencies
- Dependencies are kept at their latest released versions
- A broken upgrade is discovered automatically
- An upgrade that breaks the build is fixed before other work

## Module Structure
- Thin module at top acts composition root, no behavior mixed in
- If composition roots need behavior, used composition pipeline that alternates composition and behavior, similar to how a reader monad operates
- Consolidate all non-determinism into a single pluggable abstraction, so that all application logic can be tested within a simulated environment
- Rendering and behavior should not know about each other, a higher level coordinator should manage them
- Desktop code, web code, and common code are separate modules
- Common code does not depend on desktop code or web code
- A value that determines the content of an event is computed exactly, with integer or rational arithmetic only
- A value that is only displayed to a person may use floating point and transcendental functions freely
- No value crosses from the floating point side to the exact side
- No value that determines the content of an event depends on the iteration order of a collection
- Arithmetic that determines the content of an event never wraps

## State
- The event log is the only state
- All match state is derivable from the event log
- The event log is append-only
- No event is ever modified or removed
- A view derived from the event log can change
- The relay holds no game state between messages except what the log holds, or what derives from the log plus a long-lived secret
- The relay holds the connections of the players in a match
- The connections a relay holds survive the replacement of the relay
- A relay may be stopped and replaced between any two messages without affecting a match in progress
- A round in progress survives a relay restart
- The relay knows no game rules

## Architecture
- Every client derives the same match state from the same events
- A client derives no dice but its own before the round resolves
- Relational logical model
- Cheap and Efficient physical model
- Thin relay coordinates clients
- Fully auditable event log
- Clients must not be able to choose their roll beforehand or alter it after the fact

## Deployment
- The deploy pipeline runs in GitHub
- seanshubin.com links to the liars dice game
- The pretty web game is reachable at seanshubin.com/liars-dice
- The minimal web game is reachable at seanshubin.com/liars-dice/minimal
- seanshubin.com/liars-dice/pretty redirects to seanshubin.com/liars-dice
- The data browser is reachable at seanshubin.com/liars-dice/admin
- The hotseat match is reachable at seanshubin.com/liars-dice/hotseat
- Every page is reachable from seanshubin.com

## Authentication
- A human player authenticates with Discord
- Any Discord account can authenticate
- A human player is identified by their Discord account id
- An event refers to a player by their seat
- A join event records the Discord account id of the human player in a seat
- A human player's name is their Discord display name
- A player's name is recorded once, when they join a match
- No value derives from a player's name
