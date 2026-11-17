# Liars Dice

A networked implementation of the dice game Liar's Dice, written in Rust with Bevy
for rendering and AWS for hosting. The game model is deterministic and
fully auditable; clients are thin and cannot see or influence hidden state.

**Status:** pre-implementation. This repository currently contains documentation only.
No code has been written yet, and no code should be written until the specification
and the implementation documents for the piece being built are settled.

## The Specification
- The specification is a set of statements
- A statement is a proposition that is either true or false
- A statement is an atomic concept
- A statement is read in the context of its path
- A statement does not refer to another statement
- No two statements contradict each other
- A statement that no observation could falsify is not in the specification
- A statement says what is required, not how it is achieved
- A definition is a statement
- Nothing enters the specification without approval

## Repository Layout

| Path                | Owned by | Contains                                                              |
|---------------------|----------|-----------------------------------------------------------------------|
| `specification/`    | User     | What the system is and must do. The authority for all ideas.           |
| `implementation/`   | Shared   | How the specification will be realized. Written before code.           |
| `ai-managed-notes/` | AI       | Working memory for AI assistants. Not authoritative.                   |
| `OPEN-QUESTIONS.md` | Shared   | Questions the AI needs the user to answer. AI asks, user answers.      |

Each directory is indexed by its own `README.md`, which explains the rules for that
directory and links to every document inside it. Directory indexes are kept current:
a new document in a directory is not finished until its index links to it.

## Documentation Directories

### specification
- All ideas are user managed.
- AI assistants may split and merge files, split and merge sections, fix typos
  and phrasing, and repair broken links.
- AI assistants may not add, remove or modify the underlying ideas except at the
  explicit direction of the user.
- When an AI assistant believes an idea is missing, ambiguous, or contradictory,
  it records the issue in `OPEN-QUESTIONS.md` rather than resolving it in place.

### implementation
- AI assistants document implementation details here before realizing them.
- The user may edit these files to direct the AI towards other implementation details.
- Every implementation document cites the specification sections it realizes.
  If a detail cannot be traced back to the specification, it is either a gap in the
  specification (raise it in `OPEN-QUESTIONS.md`) or an implementation decision that
  must be recorded and justified in the implementation document itself.

### ai-managed-notes
- Generally not read or edited by the user.
- Intention is to help AI assistants maintain coherence as sessions come and go.
- Nothing here is authoritative. If a note conflicts with the specification, the
  specification wins and the note is wrong.

## Document Lifecycle

```
idea  ->  specification/  ->  implementation/  ->  code
              ^                     |
              |                     v
         OPEN-QUESTIONS.md  <--  ambiguity found
```

1. The user states an idea; it lands in `specification/`.
2. An AI assistant writes an implementation document describing how it will be built,
   cross-referenced to the specification.
3. The user reviews and edits the implementation document.
4. Only then is code written.
5. Anything ambiguous discovered at any stage becomes an entry in `OPEN-QUESTIONS.md`.
   When the user answers, the answer is folded into `specification/` and the question
   is removed.

## Conventions

- **File names** are lower-kebab-case with a `.md` extension, named for the topic,
  not the author or the date: `game-rules.md`, not `notes-2.md`.
- **One `# H1` per file**, matching the topic. Sections use `##` and below.
- **Links** are absolute from the repository root: `/specification/game-rules.md`.
- **Terminology** is defined once, in the specification's glossary, and used
  consistently everywhere. If two words mean the same thing, one of them is wrong.
- **Facts live in exactly one place.** Other documents link to that place rather than
  restating it. A restated fact becomes a stale fact.
- **Prose is declarative.** Specification documents say what is true of the system,
  not what someone should do.

## Working Agreements for AI Assistants

- Do not invent specification. Ask.
- Do not write code until the relevant implementation document exists and the user
  has had the chance to review it.
- Record questions in `OPEN-QUESTIONS.md` as they are found, with enough context that
  the user can answer without re-reading the whole repository.
- Keep directory `README.md` indexes in sync with directory contents.
- Leave `ai-managed-notes/` in a state the next session can pick up cold.
