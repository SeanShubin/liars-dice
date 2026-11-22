# Handles

How one browser holds more than one player, so two tabs can be two people.

## Realizes
/specification/architecture.md — Authentication, the handle statements.
/specification/glossary.md — Handle.

## Approach

### What is wrong

`guest_id()` reads `localStorage`, which is per-origin and not per-tab. Every tab on the
site is the same player, and since the lobby unit landed they do not merely share an
identity, they fight over it: the second tab lists, sees a match marked `mine`, resumes
into the first tab's seat, and the relay evicts the first tab's socket because a seat holds
one socket. Three rules behaving exactly as specified, combining into a result nobody
wanted.

### A handle names a player, and is not one

`#as=bob` means *use the player this browser files under "bob"*. It does not mean *be
"bob"*. The distinction is not a preference; `An unauthenticated player's id is not derived
from anything they typed` has been in the specification since the identity unit and
forbids the other reading outright. A handle selects a key; the value under that key is
still sixteen bytes from `crypto.getRandomValues`.

So `The same handle in two browsers names two players` falls out rather than being
arranged: a browser that has never seen "bob" makes a new player for the name, and two
people who both call themselves "bob" are two people, as they should be.

### The fragment, because it never leaves the browser

A handle answers a question no server asks, so it goes in the one part of a URI that is
never sent: not in a request, not in CloudFront's logs, not in a `Referer` to anybody.
`A handle is never part of a request` is this stated as a requirement rather than as a
mechanism.

The query is spoken for — Discord returns `?code=`, and the page already reads and scrubs
it. A path segment is worse than it looks: the site is a static tree assembled by the
pipeline, which also checks that every page is reachable from the root, so
`/multiplayer/bob/` is a 404 until something routes it.

### Read from the URL, kept in the tab

A fragment is how a handle is *set*. `sessionStorage` is where it *lives*, which is what
`A tab keeps its player across a reload` asks for, and it is per-tab in exactly the way
`localStorage` is not.

```
Page::new
  handle:  the fragment, if it carries one — then stored and taken out of the address bar
           otherwise whatever this tab already had
  guest:   the id filed under that handle, or a new one
```

Keeping it in the tab also settles the Discord round trip. A fragment survives a redirect
only unreliably, and nothing here depends on it: the handle is already in `sessionStorage`
before anybody leaves for Discord, and is still there when they come back.

### The default is the player you already are

`A browser holds one player until another is chosen`, so a tab with no handle reads the
key it reads today, unchanged:

| handle | guest id | ticket |
|---|---|---|
| none | `liars-dice-guest` | `liars-dice-ticket` |
| `bob` | `liars-dice-guest:bob` | `liars-dice-ticket:bob` |

Anything else would give every existing player a new identity on their next reload, and
with it a lost seat in any match they were in.

The ticket is keyed the same way because of `A handle names a player whether or not they
have authenticated`. One keyring per handle, holding both what a guest is and what a signed
in player has — otherwise two tabs both signed in with Discord are still one person, which
fails for precisely the players who matter most.

### Taking it out of the address bar

The handle is removed from the fragment once read, by `replaceState` rather than a
navigation, which is what `forget_code` already does for Discord's code and for the same
reason: a URL people paste should not carry who they are.

Only the `as` key is removed, and any other fragment is left alone. There is no other
fragment key today, so this costs a few lines and buys the shape being right when there is
— a link naming a *match* is meant to be shared, and a link naming a *player* is not. That
asymmetry is the reason the scrub is per-key rather than a blanket.

Worth recording that `forget_code` rebuilds the URL from `pathname` alone, so it already
drops the whole fragment. Today that is harmless, because by the time a code comes back
the handle is in `sessionStorage`. It will stop being harmless the moment a second fragment
key exists.

## Boundaries

- **`apps/web`** owns all of it: the handle, the keying, the scrub. Four pure functions and
  a field on `Page`.
- **Nothing else changes.** No protocol, no relay, no deployment, no new message. The relay
  cannot tell a handled player from any other guest, which is the same property that lets a
  bot be a player.
- **Depends on** `identity.md` for the guest id and the ticket this keys. Nothing depends
  on this.

## Decisions

**Select, not carry.** `#as=bob` naming the account directly would be simpler and is
forbidden by a statement that predates this unit. It would also turn an unguessable id into
a typeable one — the identity unit accepts that a leaked guest id costs an unranked seat,
which is not the same as accepting that a common first name is one.

**The fragment, not the query or the path.** Only the fragment stays out of requests and
logs. The path would need routing on a static site, and the query is where Discord's code
arrives.

**`sessionStorage` for the handle, `localStorage` for the identity.** The handle is a
property of this tab; the player is a property of this browser. Putting both in
`sessionStorage` would lose the player when the tab closes; putting both in `localStorage`
would make tabs share the handle and solve nothing.

**The default key is unchanged.** Every player who already has an id keeps it.

**No way to see or change a handle in the page.** A fragment is the whole interface for
now. A chooser on screen is a reasonable thing to want and a different unit; building it
here would mean deciding what happens to a match in progress when somebody switches, which
this unit has no reason to answer.

## Determinism

Nothing new. A handle is an observation — read once, at the composition root, and passed
inward as a value, like the guest id it selects. No value derives from it: it picks a
storage key and reaches nothing else, and in particular nothing in the log changes shape.
`No value derives from a player's name` is untouched, since a handle is not a name and
never reaches a join.

## Testing

The browser-facing calls are `wasm32` only, so the logic is pulled out as free functions
and tested on the host:

- A fragment carrying `as` yields the handle; one without yields none; an empty value
  yields none.
- A handle with nothing in it but spaces is not a handle.
- A handle too long, or holding characters a storage key should not, is refused rather
  than cleaned up — a silently repaired handle is a player silently becoming a different
  player.
- `storage_key` with no handle is exactly the key used today, asserted against the literal
  string, because that is the test that fails if anybody renames it and takes every
  existing player's seat with them.
- `storage_key` with a handle differs per handle, and two different handles never collide.
- Removing `as` from a fragment leaves the other keys in place and removes nothing else.
- Removing `as` from a fragment that has nothing else leaves an empty fragment.

What cannot be tested here is the part that matters most — that two real tabs are two real
players — for the same reason the unattended unit could not test that a browser runs a
frame. It needs two tabs and a person.

## Open Items

**Nothing shows which player a tab is.** The page says a name, which is typed per match and
is not the handle. Somebody with three tabs open has no way to tell them apart except by
what is on the screen. A quiet indicator is probably the right answer and is not in the
specification.

**`#match=` is not part of this.** A link that joins a match is a good idea and a separate
one; it is noted here only because it is the reason the scrub is per-key.

## Status
Built. 317 tests pass across the workspace, five of them new; fmt and clippy clean on the
host and on `wasm32-unknown-unknown`; the release bundle builds under trunk.

One thing the document did not say. **The pure functions carry a `dead_code` allow on the
host target**, because the address bar is something only a browser has — the same
`cfg_attr` the mailboxes already carry for the same reason. The tests reach them, but a
plain host build of the binary does not, and the gate builds that too.

And one test that had to be corrected rather than kept. `#as=%20%20` was asserted to be
refused *because it is made of spaces*; it is refused because `%` is not a character a
handle may hold, since nothing is percent-decoded and a handle never needs to be. The
assertion passed either way, which is what made it worth fixing — a test that passes for a
reason other than the one it states is a test that will go on passing when the reason
stops being true.
