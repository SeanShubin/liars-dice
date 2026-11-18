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
- **R6 — Is there a cap on dice gained?** *Answered:* five, the starting count. A correct
  spot on claim at the limit gains a shield token instead of a die.
- **R7 — Who starts the next round?** *Answered:* the player affected by the resolution,
  win or lose. Recorded under Exchanges in [game-rules.md](/specification/game-rules.md).
- **R8 — What is the turn order?** *Answered:* chosen at random when the match starts,
  wraps, and players are not reordered during a match.
- **R9 — What does "previous player" mean when that player was just eliminated?**
  *Dissolved:* the previous player is defined by turn order, not by who last claimed, so
  it always has a referent. Turn placement no longer depends on elimination at all.
- **R10 — Can two players be eliminated in one round?** *Answered:* no. A resolution
  affects exactly one player, and a forfeit ends the round without a resolution. The
  ranking would tolerate a tie anyway, since lasting longer is defined by who is
  eliminated first.
- **R11 — Can a player call Spot On against a claim that is not the immediately
  preceding one?** *Answered:* no. A claim is constrained by the previous claim of the
  same round, if one exists.
- **R12 — Is there a special rule when a player is down to one die?** *Answered:* no.
- **R13 — Is a match always to one survivor, or is there a round or time limit?**
  *Answered:* one survivor. Rounds continue until only one player remains. No round cap,
  no match clock and no turn clock. A match that cannot continue is saved and resumed.
- **R14 — Are there any constraints on the random starting player?** *Dissolved:* turn
  order is chosen at random at match start, so the starting player follows from it. The
  question of which randomness source is used is T10.

## A — Applications

All four application specifications are empty. The index names them; nothing says what
they are.

- **A1 — What is the minimal game minimal for?** *Answered:* minimal system
  requirements. The minimal presentation is the one that asks least of the machine it
  runs on, which is also the argument against shipping a game engine to draw it.
- **A2 — What does the pretty presentation add over the minimal one?** *Deferred with
  W12:* the pretty presentation waits until the minimal one works.
- **A3 — Is the die rendering prototype a throwaway spike or a component?**
  *Answered:* a throwaway spike. Nothing else depends on it, so the pretty presentation
  cannot be planned around reusing its geometry or texture code.
- **A4 — Data Browser:** *Answered:* the full event log, for an administrator only,
  identified by a configured Discord account. Reachable at
  seanshubin.com/liars-dice/admin. Recorded in
  [data-browser.md](/specification/data-browser.md).
- **A5 — Build order between the applications.** *Answered:* the minimal web game with
  its relay, then the pretty web game, then the data browser. Recorded in
  [implementation/README.md](/implementation/README.md).
- **A6 — Which applications are in scope for a first release**, and which are
  exploratory? *Answered:* the minimal web game and the minimal desktop game.
## P — Players and Product

- **P1 — Is this multiplayer only, or is there single-player?** *Answered:* both. A
  desktop game is single player against AI players with no network; a web game is
  multiplayer. Recorded in [applications.md](/specification/applications.md).
- **P2 — Are there AI players?** *Answered:* yes, specified in
  [ai-players.md](/specification/ai-players.md). They treat unknown dice as uniform and
  model nobody's behaviour. Where they run is answered by a desktop game having no
  network and a web game having no AI players.
- **P3 — Is local hot-seat play supported?** *Answered:* no. It contradicts
  "players don't know their opponent's dice" on a shared screen.
- **P4 — Player identity.** *Answered:* a player authenticates with Discord.
- **P5 — How does a match start?** *Answered:* a player hosts it, players join, the host
  starts it. Recorded in [multiplayer.md](/specification/multiplayer.md).
- **P6 — Does the time limit cover anything besides taking a turn?** *Dissolved:* there is
  no time limit. Nothing is timed.
- **P7 — Disconnect and reconnect.** *Answered:* a match has the same players from its
  first round to its last and cannot continue while any of them is absent. The host saves
  it and loads it when they return. Nobody is substituted and nobody is penalised.
  Recorded under Match in [game-rules.md](/specification/game-rules.md) and Saves in
  [multiplayer.md](/specification/multiplayer.md).
- **P8 — Spectators.** *See W13.* Allowed? If so, what do they see — all dice, or only what is
  public?
- **P9 — Target platforms.** *Answered:* the minimal presentation is single-touch and
  asks least of the machine it runs on, so a touch screen and a tablet keyboard are
  targets. The pretty presentation's targets follow from W12, which is deferred.
- **P10 — Is there any persistence the player sees** — match history, statistics,
  ratings? *Deferred:* nothing player-facing for now. The administrator sees everything
  through the data browser.
## T — Technical

- **T1 — Trust model.** *Answered:* the relay deals and is trusted to deal fairly; the
  adversary is other players, not the operator. Recorded under Trust in
  [dice-integrity.md](/specification/dice-integrity.md).
- **T2 — Deterministic lockstep versus hidden information.** *Answered:* lockstep
  applies to public state, which every client derives identically from the same events.
  Dice are a delayed input — the relay sends each player only their own, so there is no
  shared random stream mid-round, and the reveal puts every die in the log. Recorded
  under Architecture in [architecture.md](/specification/architecture.md).
- **T3 — What exactly is "thin" about the relay?** *Answered:* the relay knows no game
  rules. It deals dice from a seed, orders messages, and publishes the seed at round end.
  Claim validity is therefore decided by clients, which raises T13.
- **T4 — Event log: format, location, retention.** *Answered:* three artifacts, only the
  first canonical — an append-only DynamoDB event log, a projection computed on demand
  rather than stored, and one S3 text file per finished or saved match. Recorded in
  [deployment.md](/implementation/deployment.md).
- **T5 — Event log versioning.** *Answered:* one log per version, which the join and
  load checks already guarantee — no match can span two protocol versions, so no log can
  contain two. A version line at the top of the file records which.
- **T6 — Auditable by whom, and how?** *Answered:* a player can read the event log of a
  complete match they played in, which is when secrecy is already over; the data browser,
  which reads everything, stays administrator-only. This is what keeps the Trust
  statements in [dice-integrity.md](/specification/dice-integrity.md) honest —
  administrator-only auditing would have asked players to trust an administrator who is
  also a player.
- **T7 — Which AWS services?** *Answered:* S3, CloudFront, ACM, Route 53, API Gateway
  (HTTP and WebSocket), Lambda, DynamoDB, SSM Parameter Store, Secrets Manager,
  CloudWatch, SNS — with the cost constraint from T16. Recorded in
  [deployment.md](/implementation/deployment.md).
- **T8 — Transport and protocol.** *See W7.* WebSocket, QUIC, plain TCP? Message schema and
  encoding? How is protocol version mismatch handled?
- **T9 — "Relational logical model" and "cheap and efficient physical model".**
  *Answered:* as `../vote` does it. The logical model is a relational projection behind
  one query interface, so tests and admin tools see tables and joins; the physical model
  is whatever is cheapest underneath, which there is DynamoDB single-table with the
  append-only event log kept in its own table for its own access pattern. Swapping the
  physical model changes no code outside the repository layer.
- **T10 — What is the single non-determinism abstraction's surface?** *Answered:*
  randomness, the clock, and persistence, which `apps/minimal` already has as `Dice`,
  `Moment` and `Audit`. The web delivery adds the network.
- **T11 — Determinism requirements for Rust specifically.** *Answered:* three
  statements now cover it. A value determining the content of an event is computed
  exactly and nothing crosses from the floating point side; no such value depends on the
  iteration order of a collection; and arithmetic producing one never wraps. Recorded
  under Module Structure in [architecture.md](/specification/architecture.md).
- **T12 — Collusion.** *Answered:* out of scope. Not prevented, not detected.
- **T13 — Server authority over claims.** *See W4.* Can a malicious client send an illegal claim,
  a claim out of turn, or a claim on behalf of another player? Where is that rejected?
- **T14 — Observability.** *Answered:* copy `../vote` — a CloudWatch log group with
  short retention, a metric filter on errors, an SNS topic that emails, and two alarms.
  Recorded in [deployment.md](/implementation/deployment.md).
- **T15 — Does a match survive a relay restart?** *Answered:* yes. The event log is the
  only state, and the round seed derives from a long-lived secret plus the log, so a
  restarted relay re-derives it and nothing is lost. Recorded under State in
  [architecture.md](/specification/architecture.md).
- **T16 — What is the cost or scale target?** *Answered:* under five dollars a month.
  `../vote`'s template is the model: CloudFront PriceClass_100, DynamoDB
  PAY_PER_REQUEST, Lambda on demand, one day log retention, and no always-on compute or
  NAT gateway. Only one thing here scales differently — CloudFront egress for the WASM
  bundle, which is W5's decision. Recorded in
  [deployment.md](/implementation/deployment.md).
- **T17 — Where do AI players run?** *Dissolved:* a hosted match has no AI players, so no
  AI runs server side. The minimal game's AI runs wherever that application runs, which
  for the browser build is the client.
- **T18 — Save storage.** *Answered:* a save is visible only to its host, and is stored
  in S3 keyed by the host's Discord account id with the name and timestamp in the key.
  Recorded under Saves in [multiplayer.md](/specification/multiplayer.md) and in
  [deployment.md](/implementation/deployment.md).
- **T19 — What if a player never returns?** *Answered:* the host can delete a save,
  which appends an event and drops it from the view. The log is append-only, so the
  match's events are retained either way. Recorded under Saves in
  [multiplayer.md](/specification/multiplayer.md).
- **T20 — Protocol version mismatch.** *Answered:* a match records the protocol version
  of the client that hosted it, and a client cannot join a match or load a save whose
  version differs from its own. Recorded under Versions in
  [multiplayer.md](/specification/multiplayer.md).
## W — Web Delivery

What stands between the specification as written and a working deployment at
seanshubin.com/liars-dice. Ordered roughly by what blocks what.

- **W1 — What ships first?** *Answered:* the minimal web game. Because there is no
  solo play on the web, that first deployment needs the relay, Discord authentication
  and save storage, not just a static page.
- **W2 — Is absence observable?** *Answered:* a lost connection makes a player absent,
  and the host can see who is absent. No clock and no automatic judgement; what to do
  about it is the host's call. Recorded under Match in
  [game-rules.md](/specification/game-rules.md).
- **W3 — How does a relay that knows no game rules deal the dice?** *Answered:* every
  round deals the dice limit to every seat, and a player uses as many of their seat's
  dice as they hold. The relay needs the seat roster and nothing else. Recorded under
  Rolls in [dice-integrity.md](/specification/dice-integrity.md).
- **W4 — Who rejects an illegal claim?** *Answered:* nobody applies one. A client does
  not send one, every client ignores one it receives, and the relay records it as
  received, so the attempt is in the audit trail. Recorded under Claims in
  [multiplayer.md](/specification/multiplayer.md).
- **W5 — How is Bevy delivered to a browser?** *Answered:* for the pretty presentation,
  Trunk with wasm-bindgen and `wasm-opt -Oz`, as `../boardgame` already does. For the
  minimal presentation, not at all — it is an egui application hosted inside Bevy, so the
  host becomes eframe and the same code runs natively and in the browser. Recorded in
  [deployment.md](/implementation/deployment.md).
- **W6 — One CloudFront distribution or two?** *Answered:* seanshubin.com has a Route 53
  zone and no A record, so there is nothing to share. This repository owns the hostname,
  its certificate and its distribution. Recorded in
  [deployment.md](/implementation/deployment.md), including why the decision has a shelf
  life.
- **W7 — What carries messages between clients and the relay?** *Answered:* API
  Gateway WebSocket plus Lambda, with a DynamoDB connection table. Recorded in
  [deployment.md](/implementation/deployment.md).
- **W8 — What does Discord authentication actually provide?** *Answered:* the account
  id identifies a player and appears in every event; the Discord display name is shown
  and is recorded once, when a player joins. No value derives from a name. Scope is
  `identify` only, with no guild gate. Recorded under Authentication in
  [architecture.md](/specification/architecture.md).
- **W9 — Does the minimal game require signing in?** *Answered:* the minimal desktop
  game does not; the minimal web game does, because every match in a web game is hosted
  and a human player authenticates with Discord. Recorded in
  [applications.md](/specification/applications.md) and under Authentication in
  [architecture.md](/specification/architecture.md).
- **W10 — Who picks dice colours in a hosted match?** *Answered:* each player picks
  when they join, and a colour another player has chosen cannot be chosen. Recorded
  under Joining in [multiplayer.md](/specification/multiplayer.md).
- **W11 — Does the browser build write an audit log?** *Answered:* no. The desktop
  log is a tuning convenience and everything in it derives from the canonical event log,
  so the data browser grows reports that produce the same view on demand.
- **W12 — What is the pretty presentation?** *Deferred:* until the minimal game
  works. /specification/game-pretty.md stays three empty headings, and
  `A presentation determines no rule` is all that constrains it.
- **W13 — Are there spectators on the web?** *Deferred:* wanted later, not in the
  first networked release, and no statement either way for now. Each row of the
  connection table carries a role so that adding them later does not change it.
- **W14 — What does the pipeline gate?** *Answered:* a format check, clippy with
  warnings denied, and the tests. `../boardgame`'s pipeline is the shape: gate, deploy
  the artifact the gate built, then fuller verification afterwards as notify-only.
  Recorded in [deployment.md](/implementation/deployment.md).
- **W15 — What licence?** *Answered:* none, deliberately. Auditing needs the source to
  be readable, which a public repository already gives; a licence governs reuse, which is
  a different question and not one this project is trying to answer. The consequence
  accepted: absent a licence nobody may legally copy or modify the work.
- **W16 — Does a connection registry count as relay state?** *Answered:* the statement
  now says the relay holds no *game* state, and two statements say the relay holds the
  connections of the players in a match and that those survive its replacement. Recorded
  under State in [architecture.md](/specification/architecture.md).
- **W17 — Which application is the multiplayer game?** *Answered:* both presentations
  are delivered twice. A desktop game is single player, a web game is multiplayer, and
  two players using different presentations can share a match. Recorded in
  [applications.md](/specification/applications.md).

## D — Development Process

- **D1 — This directory is not a git repository.** *Dissolved:* it is one, with a
  top-level `.gitignore` covering `target/`, `.idea/`, `logs/` and
  `local-do-not-commit/`.
- **D2 — Cargo layout.** *Answered:* one workspace, already built. The desktop, web
  and common module split lands inside it.
- **D3 — Where does code live relative to the documentation directories?**
  *Answered:* `crates/` for libraries, `apps/` for binaries, as built and as the root
  README documents.
- **D4 — Rust toolchain and dependency versions,** and the policy for upgrading them.
  *Answered:* the gate builds from the committed lockfile so a deploy stays reproducible,
  and a notify-only job on every build runs `cargo update` into a throwaway lockfile and
  tests against it. That job is what makes "a broken upgrade is discovered automatically"
  true. Recorded in [deployment.md](/implementation/deployment.md).
- **D5 — Testing strategy beyond the simulated environment.** *Answered:* property
  tests over the rules engine invariants — exactly one player affected by every
  resolution, a true claim always available to the current player, and no player ever
  above the dice limit.
- **D6 — CI.** *Answered:* the deploy pipeline runs in GitHub. What it gates is still an
  implementation decision; the `vote` project is the reference, recorded in
  [deployment.md](/implementation/deployment.md).
- **D7 — License.** *See W15.* Answered there: none, deliberately.
- **D8 — Is this a solo project or will others contribute?** *Answered:* solo. No
  contribution guidance is needed.
- **D9 — Does the specification need status markers** (draft / accepted /
  implemented)? *Answered:* no. The specification says what is true of the game, not what
  is built; implementation documents carry the Status line.
- **D10 — Should architecture decisions get their own records** in `specification/`?
  *Answered:* no. The Decisions section of each implementation document already does
  this, and a decision with no implementation document has nowhere it belongs yet.
