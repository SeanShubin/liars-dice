# Implementation

How the [specification](/specification/README.md) will be realized. Written before
code, reviewed by the user, then built.

## Rules

- One document per coherent unit of work: a module, a service, a milestone.
- Every document cites the specification sections it realizes. A detail that cannot be
  traced back to the specification is either a specification gap (raise it in
  [OPEN-QUESTIONS.md](/OPEN-QUESTIONS.md)) or an implementation decision, which must be
  recorded in the document with its alternatives and the reason for the choice.
- Documents describe the intended end state, not a diary of what happened.
- The user may edit any document here to steer the implementation.
- No code is written for a unit of work until its document exists and the user has had
  the chance to review it.

## Document Template

```markdown
# <Name>

## Realizes
Links to the specification sections this implements.

## Approach
How it works, in prose, at a level someone could re-derive the code from.

## Boundaries
What this owns, what it depends on, what depends on it, and the shape of each seam.

## Decisions
Choices not dictated by the specification: the options, the choice, the reason.

## Determinism
Which non-determinism this touches, and how it routes through the single pluggable
abstraction so the whole thing is testable in a simulated environment.

## Testing
What is verified and how, including the simulated-environment scenarios.

## Status
Not started | In progress | Built
```

## Documents

- [Die Rendering Prototype](/implementation/die-rendering-prototype.md) — mesh, materials, glyphs, settings panel
- [Minimal Desktop Game](/implementation/game-minimal.md) — probability model, AI decision rule, colour assignment, interaction
- [Rules Engine](/implementation/rules-engine.md) — the dice limit, shield tokens, the affected player
- [Game State](/implementation/game-state.md) — the whole game, one player's view, the operations
- [Coordinator](/implementation/coordinator.md) — the client, and whatever drives it
- [Relay and Lobby](/implementation/relay.md) — hosting, joining, dealing, fan-out, reconnection
- [Relay Service](/implementation/relay-service.md) — the deployed relay: DynamoDB, the WebSocket, the Lambda
- [Dice Integrity](/implementation/integrity.md) — commitment, seed derivation, reveal
- [Composition](/implementation/composition.md) — where every program observes the world, and the pipeline for when wiring needs a fetched value
- [Protocol](/implementation/protocol.md) — seats, the version line, shield events
- [Deployment](/implementation/deployment.md) — the GitHub pipeline, AWS wiring, Discord OAuth
- [Hotseat](/implementation/hotseat.md) — one screen passed between seats, and the one presentation both deliveries draw
- [Identity](/implementation/identity.md) — guests, Discord tickets, AI players, and what makes a match ranked
- [Multiplayer Client](/implementation/multiplayer-client.md) — the browser half of a networked match: a socket, one seat, the same presentation
- [Ending a Match](/implementation/ending.md) — completion, cancelling, abandonment, and the relay's first clock reading
- [Lobby](/implementation/lobby.md) — every match in progress, which of them you can join, and the one you are already in
- [Unattended](/implementation/unattended.md) — what a client does when nobody is touching it: show what arrives, stay able to act

## Order

Confirmed by the user, answering A5 and A6:

1. The minimal web game, with everything it needs — the dice limit and shield tokens in
   the rules engine, the event log, the relay, Discord authentication and saves. Because
   there is no solo play on the web, none of these can be deferred behind the others.
2. The minimal desktop game, which is the same presentation over the existing local
   composition root. In the first release alongside the above.
3. The pretty web game, once the minimal one works and W12 is answered.
4. The data browser.

The pretty desktop game falls out of the pretty presentation at no extra cost. The die
rendering prototype is a throwaway spike that nothing else depends on, so it is not in
this sequence.
