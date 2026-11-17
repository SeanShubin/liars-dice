# Die Rendering Prototype

## Realizes
/specification/die-rendering-prototype.md, in full.
/specification/dice.md — Geometry and Appearance.
/specification/ground.md — Appearance.
/specification/architecture.md — thin composition root, rendering isolated from behavior,
floating point confined to rendering.

## Approach

A Bevy application showing one die on a ground plane, with a settings panel. Mesh
generation and material construction live in a library crate; the prototype itself is a
composition root plus a settings panel.

### Crate placement

```
crates/dice-render/     mesh generation, materials, glyphs, ground material
apps/prototype/         main, settings panel, camera control
```

The prototype does not own the dice rendering code. If it did, whatever is learned here
would have to be rewritten for the Pretty renderer, and the prototype would validate
nothing but itself. `crates/dice-render` is the crate the Pretty renderer will later
consume unchanged, and `apps/prototype` is a thin main that drives it — which is also
what the architecture statements require of a composition root.

### Geometry

The die is a cube intersected with one sphere, both centred on the origin. The sphere
removes the eight corners.

Because there is exactly one sphere, every curved point on the surface lies on that
sphere, and every flat point lies on one of six planes. The mesh is therefore constructed
analytically rather than approximated:

- **Six flat faces.** Tessellate each cube face plane, discarding anything outside the
  sphere. Each face is a square with its corners cut by circular arcs.
- **The curved region.** Tessellate the sphere, discarding anything outside the cube.

Analytic construction rather than meshing a signed distance field, because flat faces must
be exactly flat. Surface nets or marching cubes over an SDF approximates a plane as a grid
of triangles with slight stair-stepping, and the glyph sits on precisely those faces — a
prototype built to judge glyph crispness cannot afford an approximated face.

**The sphere radius is a setting**, expressed as a ratio to the cube half-extent and bounded
to the range where flat faces and clipped corners both exist. Crossing the square root of
two changes the shape's character: above it the twelve edges stay sharp and only the eight
corners are curved; below it the edges round as well. Both are reachable from the same
setting, so the choice is made by looking rather than by deciding in advance.

**Surface classification is free.** A triangle came either from a face plane or from the
sphere, and the generator knows which. No normal-angle threshold is needed, so there is
nothing to tune and no possibility of misclassification.

The mesh is split into two sub-meshes on that classification, one per material. Face
triangles take planar UVs from the plane they were generated on, which gives each of the
six faces a clean unit square to place a glyph in. Sphere triangles take UVs from their
position on the sphere, which is a natural parameterisation rather than a projection.

### Materials

A "texture" in the specification is not one image. It is a set of physically based
channels — base color, roughness, metallic, and a normal perturbation. The settings expose
those channels rather than asking for finished images.

Everything is generated procedurally from parameters. Speckles are a noise function
thresholded into the base color and the normal; roughness and glossiness are a single
scalar; the rough three dimensional appearance is normal perturbation amplitude. This is
what makes the settings continuous — a slider rather than a file picker — which is the
point of a prototype meant for exploring appearance.

Image loading is a later addition, not part of this.

### Glyphs

Glyph construction is a strategy, because the prototype exists partly to decide what a
glyph should be:

- pips, generated procedurally as circles in the face's unit square
- numerals, rasterized from a font
- images, loaded from files

The first pass implements pips only. The other two are why glyph construction is a
strategy rather than a function, but neither is built yet.

Pips alone cannot satisfy the acceptance statement, because a glyph that differs in kind
from the other glyphs is by definition not a pip. The first pass is accepted as not
acceptance-complete rather than narrowing the acceptance statement to match it.

### Pip layout

Pips sit on a three by three grid within the face's unit square, at cell centres offset a
quarter of the face width from the middle:

| Face | Occupied cells |
|------|----------------|
| 1 | centre |
| 2 | two opposite corners |
| 3 | two opposite corners and the centre |
| 4 | four corners |
| 5 | four corners and the centre |
| 6 | two full columns of three |

Pip radius is a setting, defaulting to roughly a tenth of the face width.

Face arrangement follows the Western convention: opposite faces sum to seven, and the die
is right-handed, meaning that with the one face up and the two face toward the viewer the
three face is on the right.

Pip orientation is not standardised across manufactured dice, so it is taken from a
physical die rather than chosen. Only three faces carry orientation; the one, four and
five faces are rotationally symmetric.

- The six face's columns run parallel to the edges it shares with the five and two faces
- The three face's diagonal runs from the corner where three, five and six meet to the
  corner where three, one and two meet
- The two face's diagonal runs from the corner where two, four and six meet to the corner
  where two, one and three meet

Expressed by which faces meet at a corner rather than by a viewing direction, so the
statements hold however the die is oriented. None is exposed as a setting; they affect
nothing a player can act on.

Pips are rendered into the glyph texture and given depth by normal perturbation rather
than by geometry. Real dice have drilled pips, but modelling them would mean eight
hundred extra surfaces per die and would put the pips outside the glyph texture the
specification calls for.

The glyph is composited onto the face texture at build time, producing six finished face
images, one per face value. Recompositing on a settings change is cheap enough to do
synchronously.

This avoids a custom shader entirely: one standard material per sub-mesh, both built from
ordinary image handles.

### Ground

A plane with a procedural material from the same channel set. Felt is high roughness, no
metallic, low-amplitude high-frequency normal noise, and a flat base color.

### Camera

An orbit camera. Panning drags the camera around the die rather than rotating the die
itself — visually identical, and it keeps the die at the world origin where the mesh
generator puts it.

### Settings panel

`bevy_egui`. An immediate-mode panel is the right shape for a screen that is nothing but
live-editable parameters, and it costs one dependency. Its version is coupled to the Bevy
version, so the two are pinned together.

## Boundaries
- **`crates/dice-render`** — SDF, meshing, classification, UV assignment, procedural
  materials, glyph compositing. Knows nothing of Bevy app structure, settings, or input.
- **`apps/prototype`** — composition root. Builds the app, owns the settings state, owns
  the camera, calls into `dice-render` when a setting changes.
- **No dependency on** `rules`, `model`, `protocol`, or `integrity`. The prototype has no
  game, and the Out of Scope statements are enforced by the dependency graph rather than by
  discipline.

## Decisions

**Analytic mesh, rather than meshing a signed distance field.** One cube and one sphere is
simple enough to construct directly, and it keeps the six flat faces exactly flat. An SDF
would have been the right answer for eight separate corner spheres, where the surface has
no closed form worth writing by hand; it is the wrong answer here, because approximating a
plane is strictly worse than computing it.

**Classification by provenance, rather than by normal angle.** Each triangle is generated
either from a face plane or from the sphere, so the generator already knows its material.
An earlier draft classified by normal direction, which needed a tuned threshold angle and
could in principle disagree with the geometry. Provenance cannot.

**Glyph composited into the face texture, rather than sampled in a custom shader.** One
standard material per sub-mesh, no WGSL, and the whole thing stays inside Bevy's normal
pipeline. The cost is that a glyph cannot be lit or tinted independently of its face. If
that turns out to matter, a custom material replaces the compositing step without touching
anything else.

**Procedural materials, rather than authored images.** Continuous parameters are what the
settings statements imply, and nothing has to be drawn before the prototype is useful.

**Raymarching the shape directly** was considered and rejected. It gives exact geometry,
but it fights Bevy's lighting and shadowing, and the Pretty renderer will want ordinary
meshes.

## Determinism
- Mesh generation is a pure function of the settings. The same settings produce the same
  mesh, byte for byte.
- Noise is seeded explicitly, never from the clock or from an unseeded default.
- Floating point appears throughout, which is where the architecture statements say it
  belongs. Nothing here is shared with a model, so nothing has to agree across machines.

## Testing
- The mesh is watertight: every edge is shared by exactly two triangles.
- Every normal points away from the shape's interior.
- Classification is total: every triangle lands in exactly one of the two sub-meshes.
- Each of the six flat faces receives UVs covering the unit square without overlap.
- Mesh generation is deterministic across runs for fixed settings.
- Glyph compositing produces six distinct images.
- Acceptance is visual and cannot be automated. The acceptance statement — every
  appearance in the specification can be seen — is checked by a human driving the panel.

## Open Items

**What the bevel texture applies to depends on the radius.** Above a radius ratio of the
square root of two the twelve edges stay sharp and the bevel texture covers only the eight
corner patches. Below it the edges round and the bevel reads as a conventional chamfer.
This is answered by looking at the prototype, not before it.

Deferred: whether glyph strategies beyond pips are needed for the first pass.

Bevy is tracked at its latest release rather than pinned to a version. `bevy_egui` is
version-coupled to Bevy, so the pair moves together and an upgrade waits on whichever
lags.

## Status
Not started.
