# Open Questions

Questions an AI assistant needs the user to answer. Each has an id so answers can be
given as `R3: yes`. When a question is answered, the answer is folded into
[specification/](/specification/README.md) and the question is deleted from this file.

Where a question notes a **common default**, that is the usual answer in the wider
world, offered to make the question cheap to answer. It is not a recommendation and
nothing has been assumed.

---

## R — Game Rules

The rules as written cannot be executed without inventing answers to these.

- **R1 — Ones wild.** *Answered:* a configurable boolean, off in vanilla. Two details it
  leaves open, both only relevant when the variant is on:
  - **R1a — Can a player raise on ones?** *Answered:* no. When ones are wild, a claim may
    not name the one face.
  - **R1b — Does the face-value ordering change?** *Dissolved by R1a.* The one face cannot
    be named in a claim, so where it ranks never arises.
- **R2 — What are the legal face values?** *Answered:* six faces, numbered one through
  six. Recorded in [dice.md](/specification/dice.md).
- **R3 — What is the first claim of a round allowed to be?** *Answered by derivation:*
  a raise. Declaring false and declaring spot on both reference the previous claim, so
  neither is available when there is none. Deliberately not stated.
- **R4 — Can a raise exceed the number of dice in play?** *Answered:* no. A claim must be
  possible according to the dice in play, though it may be known false by the player
  making it.
- **R5a — Does losing all dice cause a loss?** *Answered:* a player with no dice is
  eliminated, and an eliminated player takes no further part in the match.
- **R5b — Retire one of "lose" and "eliminated."** *Answered:* "eliminated" names being
  out of the match. "Lose" is free for losing a die and for losing a match.
- **R6 — Is there a cap on dice gained?** *Answered:* no cap. A player's dice may exceed
  their starting count without limit.
- **R7 — Who starts the next round?** *Answered:* the winner of the exchange. Recorded
  under Exchanges in [game-rules.md](/specification/game-rules.md).
- **R8 — What is the turn order?** *Answered:* chosen at random when the match starts,
  wraps, and players are not reordered during a match.
- **R9 — What does "previous player" mean when that player was just eliminated?**
  *Dissolved:* the previous player is defined by turn order, not by who last claimed, so
  it always has a referent. Turn placement no longer depends on elimination at all.
- **R10 — Can two players be eliminated in one round?** *Answered:* no. One resolution
  costs one player one die, and a timeout ends the round. The ranking would tolerate a
  tie anyway, since lasting longer is defined by who is eliminated first.
- **R11 — Can a player call Spot On against a claim that is not the immediately
  preceding one?** *Answered:* no. A claim is constrained by the previous claim of the
  same round, if one exists.
- **R12 — Is there a special rule when a player is down to one die?** Many variants
  (palifico) change the rules at that point. Silence here probably means no, but it
  should be stated rather than inferred.
- **R13 — Is a match always to one survivor, or is there a round or time limit?**
  *Answered:* one survivor. Rounds continue until only one player remains. No round cap
  and no match clock; the only clock is the per-turn time limit.
- **R14 — Are there any constraints on the random starting player?** *Dissolved:* turn
  order is chosen at random at match start, so the starting player follows from it. The
  question of which randomness source is used is T10.

## A — Applications

All four application specifications are empty. The index names them; nothing says what
they are.

- **A1 — Minimal:** what is it minimal *for*? Rules-complete with no presentation? A
  test harness? The reference implementation the others must match?
- **A2 — Pretty:** what does it add over Minimal — presentation only, or features too?
  Do they share one model, or are they separate programs?
- **A3 — Die Prototype:** is this a throwaway spike or a component the Pretty game
  consumes? Does it ship?
- **A4 — Data Browser:** what data does it browse — the event log, live matches,
  historical matches, all of it? Who is the audience: the developer, players,
  spectators?
- **A5 — Build order and dependencies** between the four. See the suggested order in
  [implementation/README.md](/implementation/README.md).
- **A6 — Which applications are in scope for a first release**, and which are
  exploratory?

## P — Players and Product

- **P1 — Is this multiplayer only, or is there single-player?** The architecture has a
  relay coordinating clients, which implies networked play, but nothing says whether a
  solo player can play at all.
- **P2 — Are there AI opponents?** If yes, that is a large unspecified subsystem: where
  it runs (client, relay, separate service), how strong it is, whether it sees hidden
  dice.
- **P3 — Is local hot-seat play supported?** It conflicts directly with
  "players don't know their opponent's dice" on a shared screen.
- **P4 — Player identity.** Accounts, anonymous sessions, persistent names, or none?
- **P5 — How does a match start?** Lobby, invite link, matchmaking, fixed roster?
- **P6 — Does the time limit cover anything besides taking a turn?** *Answered:* no. The
  time limit applies to turns only. A player acts only on their own turn, so there is
  nothing else to be late for.
- **P7 — Disconnect and reconnect.** Can a player rejoin mid-match? What happens to
  their dice while they are gone? Is a substitute or auto-play used?
- **P8 — Spectators.** Allowed? If so, what do they see — all dice, or only what is
  public?
- **P9 — Target platforms.** Native desktop, web via WASM, mobile? Bevy supports
  several; the Data Browser hints at a web surface but nothing states it.
- **P10 — Is there any persistence the player sees** — match history, statistics,
  ratings?

## T — Technical

- **T1 — Trust model.** *Answered:* the relay deals and is trusted to deal fairly; the
  adversary is other players, not the operator. Recorded under Trust in
  [dice-integrity.md](/specification/dice-integrity.md).
- **T2 — Deterministic lockstep versus hidden information.** Lockstep normally means
  every client runs the same simulation from the same inputs. That cannot be true while
  opponents' dice are secret. Which is it: clients run a partial simulation over public
  events only, with the relay authoritative over hidden state? That reconciliation
  needs to be written down, because every module boundary depends on it.
- **T3 — What exactly is "thin" about the relay?** *Answered:* the relay knows no game
  rules. It deals dice from a seed, orders messages, and publishes the seed at round end.
  Claim validity is therefore decided by clients, which raises T13.
- **T4 — Event log: format, location, retention.** What is an event, what identifies it,
  is it append-only, where does it live, how long is it kept, who can read it?
- **T5 — Event log versioning.** When the rules change, old logs still exist. Are they
  migrated, versioned, or discarded?
- **T6 — Auditable by whom, and how?** A player checking their own match, a developer
  debugging, or a third party verifying nobody cheated? "Fully auditable" means very
  different things to each.
- **T7 — Which AWS services?** "Amazon Web Services" names a vendor, not an
  architecture. Compute, storage, and transport each need a choice.
- **T8 — Transport and protocol.** WebSocket, QUIC, plain TCP? Message schema and
  encoding? How is protocol version mismatch handled?
- **T9 — "Relational logical model" and "cheap and efficient physical model" — what is
  the boundary?** Is the relational model an actual relational database, an in-memory
  relational structure, or a modelling discipline? What does the physical model trade
  away, and what must it preserve?
- **T10 — What is the single non-determinism abstraction's surface?** Randomness and
  time are obvious members. Also network arrival order, file and storage access,
  process identity, environment? The list belongs in the specification, because
  "all non-determinism" is only testable if the set is closed.
- **T11 — Determinism requirements for Rust specifically.** Iteration order of hash
  maps, integer overflow behavior, and any float in the model are all ways determinism
  silently breaks. Is there a stated prohibition?
- **T12 — Collusion.** *Answered:* out of scope. Not prevented, not detected.
- **T13 — Server authority over claims.** Can a malicious client send an illegal claim,
  a claim out of turn, or a claim on behalf of another player? Where is that rejected?
- **T14 — Observability.** Logging, metrics, tracing, alerting: anything required, or
  deferred?
- **T15 — Does a match survive a relay restart?** *Answered:* yes. The event log is the
  only state, and the round seed derives from a long-lived secret plus the log, so a
  restarted relay re-derives it and nothing is lost. Recorded under State in
  [architecture.md](/specification/architecture.md).
- **T16 — What is the cost or scale target?** "Cheap and efficient" has no number
  attached. Concurrent matches, latency budget, monthly cost ceiling?

## D — Development Process

- **D1 — This directory is not a git repository.** Only `.idea/` exists, with its own
  `.gitignore`. Was that intentional? There is no top-level `.gitignore` for Rust
  (`target/`) either.
- **D2 — Cargo layout.** One workspace with several crates, or separate repositories per
  application? This decision shapes every implementation document.
- **D3 — Where does code live relative to these directories?** `implementation/` holds
  documents; nothing says whether code goes in `src/`, `crates/`, or elsewhere.
- **D4 — Rust toolchain and Bevy versions,** and the policy for upgrading them.
- **D5 — Testing strategy beyond the simulated environment.** Property tests against the
  rules? Replay tests against recorded event logs? Any coverage expectation?
- **D6 — CI.** Required? Where does it run, what does it gate?
- **D7 — License.** Nothing states one. Absent a license, the work is all-rights-reserved
  by default.
- **D8 — Is this a solo project or will others contribute?** Determines whether
  contribution guidance is needed at all.
- **D9 — Does the specification need status markers** (draft / accepted / implemented)
  per section, or is everything present treated as accepted?
- **D10 — Should architecture decisions get their own records** in `specification/`, or
  do they live inside implementation documents as the Decisions section currently
  assumes?
