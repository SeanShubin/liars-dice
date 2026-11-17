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
- [Minimal Game](/implementation/game-minimal.md) — probability model, AI decision rule, colour assignment, interaction

## Suggested Order

The specification implies a build order, but has not stated one. A candidate sequence,
for the user to confirm or replace:

1. The logical model and rules engine, pure and deterministic, no rendering, no network.
2. The non-determinism abstraction and the simulated environment that drives the above.
3. The minimal application, driving the model locally.
4. The event log and its storage format.
5. The relay and the client/server protocol.
6. The die rendering prototype.
7. The pretty application.
8. The data browser.

This ordering is an AI suggestion, not specification. See
[OPEN-QUESTIONS.md](/OPEN-QUESTIONS.md).
