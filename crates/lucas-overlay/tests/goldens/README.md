# Renderer goldens

These 21 transparent PNGs are synthetic outputs of `render::frame`, authored in
this repository, under the repository's MIT OR Apache-2.0 licence. They contain
no webcam frames, game artwork, or captured user content.

`tests/renderer.rs::cases` specifies every input: a fractional, negative-origin
200-pixel map, ripple phases at 180/540/960 ms for each of four saliences, and
breathing/corner/comet frames at 360 ms in amber, cyan and white. The geometry
rounds out to 201 × 201 content pixels plus a 53-pixel margin on each side.

Initial candidates were generated on Linux with tiny-skia 0.11.4 and visually
reviewed together on a dark neutral ground: clear map interior, outward-growing
rings, two at medium/strong versus one at soft/minimal, and visibly declining
width/intensity. Medium/Strong fixtures were refreshed for repeating reminders:
thicker size-scaled cores, higher opacity, and a slower ripple fade, retaining
the original surface bounds. Soft/Minimal fixtures are unchanged. The external
4K review generator in `tests/renderer.rs` captures 400px map surroundings at the
same three phases; independent tests enforce clear map/outer-edge pixels on
400/800px targets and a minimum bright core width.
The normal test compares decoded pixels exactly. Encoding and
decoding both sides accounts for PNG's straight-alpha representation.

To update intentionally, generate **candidates** into a scratch directory:

```sh
LUCAS_GOLDEN_DIR=/tmp/lucas-golden-candidates \
  cargo test -p lucas-overlay --test renderer generate_golden_candidates -- --ignored --exact
```

Review the images and input/renderer diff, then explicitly copy the accepted
PNGs here. Normal tests never overwrite the reference images. Full minimap
transparency and outer-edge tests are independent of the goldens and must still
pass. Do not refresh fixtures just to make an unexplained mismatch disappear.
