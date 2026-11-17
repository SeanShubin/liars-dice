# Architecture

## Technology Stack
- Rust programming language
- Bevy rendering engine
- Amazon Web Services

## Module Structure
- Thin module at top acts composition root, no behavior mixed in
- If composition roots need behavior, used composition pipeline that alternates composition and behavior, similar to how a reader monad operates
- Consolidate all non-determinism into a single pluggable abstraction, so that all application logic can be tested within a simulated environment
- Rendering and behavior should not know about each other, a higher level coordinator should manage them
- Floating point rounding and transcendental functions should be isolated to rendering only, and only when necessary, the model should be protected from this

## State
- The event log is the only state
- All match state is derivable from the event log
- The relay holds nothing between messages except what the log holds, or what derives from the log plus a long-lived secret
- A relay may be stopped and replaced between any two messages without affecting a match in progress
- A round in progress survives a relay restart
- The relay knows no game rules

## Architecture
- Deterministic lockstep
- Relational logical model
- Cheap and Efficient physical model
- Thin relay coordinates clients
- Fully auditable event log
- Clients must not be able to choose their roll beforehand or alter it after the fact
