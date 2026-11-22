# Unattended

What a client does when nobody is touching it: keep showing what arrives, and stay able to
act.

## Realizes
/specification/multiplayer.md — Unattended.
/specification/architecture.md — Composition, `An observation that arrives unasked wakes
the program`.
/specification/glossary.md — Unattended.

## Approach

### What is wrong

egui runs a frame when there is input, or when something asked it to. `apps/web` asks in
exactly one place:

```rust
if self.seated.is_some() {
    ui.ctx().request_repaint_after(TICK);
}
```

So an unseated page — the sign-in screen and the dashboard — runs no frames at all unless a
person moves the mouse over it. Everything asynchronous lands in a cell and waits for a
frame to drain it: `Arrivals` for relay frames, `Mishap` for a socket that failed, and
`FromExchange` for the exchange's answer. Nothing drains, so nothing shows.

This is observable on the published page today. It sits on *Asking whether Discord is
available…* forever; one click anywhere and the sign-in button appears, because the click
bought a frame and the frame found an answer that had been waiting.

`TICK`'s own doc comment says *"Arrivals wake the frame loop themselves, so this exists only
to notice silence"*. That is the design. It was written down and never built, and the
comment has been asserting it ever since — which is why the gap survived review: the code
reads as though the mechanism is somewhere else.

### Two different needs, which is why one fix is not enough

**Something arrived.** A fetch answered, a frame came over the socket, a socket failed.
The page should show it at once, and knows nothing about when it will happen.

**Nothing arrived, and that itself means something.** The keepalive is due every four
minutes whether or not anybody has said a word; `quiet` is true after twenty seconds of
hearing nothing, and silence is not an event that can wake anyone.

Waking on arrival does not cover the second, and polling does not cover the first without
a poll fast enough to be a busy loop. Both go in.

### Arrivals wake the page

One type knows how to wake, and every mailbox calls it:

```rust
/// What to poke when something arrives while nobody is looking.
#[derive(Clone)]
struct Waker(Rc<dyn Fn()>);

impl Waker {
    fn over(context: egui::Context) -> Self {
        Self(Rc::new(move || context.request_repaint()))
    }

    fn wake(&self) {
        (self.0)();
    }
}
```

A function rather than the `egui::Context` itself. A context paints and cannot count, and
Determinism below asks for a waker a test can compose over, so holding the context
directly would make this document contradict itself.

`Arrivals::push`, `Mishap::say` and `FromExchange::put` each hold one and call it. They are
the three places an observation enters this program unasked, and they are already the three
places that exist for exactly that reason — so the wake belongs with the delivery rather
than beside each of the five callbacks that deliver.

The context comes from the composition root. Both roots currently throw away the
`CreationContext` they are handed:

```rust
Box::new(|_| Ok(Box::new(Page::new())))
```

and become `Box::new(|cc| Ok(Box::new(Page::new(cc.egui_ctx.clone()))))`. One argument, no
logic, in the one place allowed to reach outside the program — which is what `Only a
composition root observes anything outside its program` asks for. `Page` keeps the waker
because it rebuilds the mailboxes whenever it opens a socket.

### A frame owed by the clock

The repaint request stops being about the seat:

```rust
if self.wire.is_some() {
    ui.ctx().request_repaint_after(TICK);
}
```

A seat implies a socket, so this is strictly wider, and it is the right boundary: the two
things that need a frame with nothing arriving — the keepalive and `quiet` — both exist
exactly as long as the socket does. A page with no socket owes nothing on a clock and goes
back to sleep, which is why this is not a busy loop on the sign-in screen.

The sign-in screen is also the one place waiting on something with no socket at all: the
exchange's answer. That is covered by the wake and not by the clock, which is the whole
point of having both.

### What this repairs that was already built

The keepalive shipped with `dashboard.md` and has never run on the dashboard screen,
because the dashboard screen runs no frames. A page listing matches holds an open socket, says nothing for
ten minutes, and is dropped — and the drop is invisible until the next click fails. The
mechanism was correct and simply never reached, which is the failure mode a test on the
mechanism alone cannot see.

## Boundaries

- **`apps/web`** owns all of it. `Waker`, the three mailboxes that call it, the repaint
  condition, and the one argument each composition root now passes.
- **Nothing else changes.** No crate, no message, no deployment, no specification outside
  the three statements above. The relay cannot tell the difference.
- **Depends on** `dashboard.md` only in the sense that it repairs the keepalive that unit
  added. Nothing depends on this.

## Decisions

**The mailboxes wake, not the callbacks.** The alternative is for each JS closure in the
composition root to push and then poke, which is wiring and would be defensible. It is
rejected because there are five such closures and one rule: a value arriving unasked wakes
the program. Five copies of a rule is how one of them gets forgotten, and the forgotten one
is a screen that silently stops updating.

**One condition, not a computed deadline.** A page could ask for a frame exactly when the
next keepalive is due, rather than every `TICK`. Rejected: `quiet` needs the same frames
anyway, so the computed deadline would collapse back to `TICK` in every case where a socket
is open, and arrive carrying arithmetic that no screen can tell apart from the constant.

**`Waker` has no default.** Both composition roots have a context, so there is no case to
carry, and the mailboxes lose their derived `Default` so that every construction site has
to be handed a real one. A `Waker` that might not wake is a `Waker` nobody can rely on,
and a `Default` is exactly how one would get built by accident.

**Not fixing the dashboard's refresh.** A listing is still only true of the moment it was read,
and `look_again` is still a button. Making the dashboard poll is a design change and not this
bug; a page that redraws correctly and shows a listing from a minute ago is behaving as
specified.

## Determinism

Nothing new. A repaint reads no state and decides nothing — the frame it causes makes the
same decisions from the same values it would have made had a mouse caused it. The clock
reading inside the frame is unchanged and still one per frame, as `One clock reading serves
one frame` requires.

The `egui::Context` is an observation in the architecture's sense: it comes from outside
the program and enters at the composition root as a value. A program composed over a
simulated outside world supplies a waker that counts instead of one that paints, which is
what the tests below do.

## Testing

- **A frame is owed whenever a socket is open**, seat or no seat. This is the test that
  fails today: with a socket and no seat, nothing is currently asked for.
- A page with no socket asks for no timed frame, so the sign-in screen does not spin.
- **Pushing to `Arrivals` wakes.** A counting waker in place of the real one, asserting the
  count rose. Likewise `Mishap::say` and `FromExchange::put`. Each fails today, there being
  nothing to count.
- The keepalive fires at four minutes and not before, driven over a fake `Wire` that
  records what it was told. This passes today — the mechanism was never the broken part —
  and is written because nothing currently holds it still.
- Every existing test in `apps/web` keeps working with the mailboxes taking a waker, which
  is the compiler's problem and not a test's.

What cannot be tested here is the thing that was actually broken: whether a browser runs a
frame. The tests above assert that the page asks; only the published page can show that
asking works. So this unit is not done until the published page shows the sign-in button
with nobody touching it.

## Open Items

**No test covers a page left alone.** Everything here tests the request, not the frame.
Catching the real defect needs a browser driven by something that deliberately does
nothing, and there is no such harness in this project. Worth knowing that the gap this unit
closes is the same shape as the gap that let it in.

## Status
Built. 312 tests pass across the workspace, four of them new; fmt and clippy clean on the
host and on `wasm32-unknown-unknown`; the release bundle builds under trunk.

Three things the document did not say.

**`Waker` holds a function, not the context.** Written up above, because the sketch and the
Determinism section disagreed and only one of them could be built.

**The repaint condition became a named method.** `Page::frame_owed` rather than a condition
inline in `ui`, because a condition in a frame loop can only be tested by running a frame
loop. As a method it is asserted directly: a page holding a socket and no seat is owed a
frame, which is the assertion that fails if anybody puts the seat back.

**A fourth test, that draining a mailbox wakes nobody.** Not in the plan, and worth having:
waking on `take` as well as on `push` would be invisible — every frame would ask for the
next one, the page would never sleep, and nothing would look wrong except the battery.

The `allow(dead_code)` the first draft of `Waker::over` carried turned out to be
unnecessary — a test binary compiles `main`, so the composition roots are callers like any
other — and removing it is how that was established rather than assumed.
