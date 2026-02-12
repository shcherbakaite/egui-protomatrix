# KiCad 9 Footprint Library Design

## Overview

Design for a Rust library to read KiCad 9 footprint files (`.kicad_mod`). Samples are in `/usr/share/kicad/footprints/`. The existing codebase uses `kicad_parse_gen` which targets older KiCad formats; KiCad 9 introduces several breaking changes.

---

## KiCad 9 Format Summary

### Root Structure

```
(footprint "NAME"
  (version 20241229)
  (generator "pcbnew")
  (generator_version "9.0")
  (layer "F.Cu")
  (descr "...")
  (tags "...")
  (property "Reference" "REF**" ...)
  (property "Value" "NAME" ...)
  (attr smd|through_hole)
  ...graphic elements...
  ...pads...
  (embedded_fonts no)
  (model "${KICAD9_3DMODEL_DIR}/Path.step" ...)
)
```

**Key KiCad 9 vs legacy (kicad_parse_gen) differences:**
- Root: `(footprint ...)` not `(module ...)`
- fp_line, fp_circle, fp_arc, fp_poly: use `(stroke (width W)(type solid))` instead of bare `(width W)`
- fp_arc: uses `(start)(mid)(end)` not `(start)(end)(angle)`
- fp_rect: KiCad 7+ addition (kicad_parse_gen does not support)
- embedded_fonts: KiCad 9 addition
- Pad roundrect: `(roundrect_rratio R)` + shape `roundrect` (kicad_parse_gen has no RoundRect)

---

## Graphic Elements (fp_*)

| Element   | KiCad 9 Format                         | kicad_parse_gen Support |
|----------|-----------------------------------------|--------------------------|
| fp_line  | (start)(end)(stroke)(layer)             | ✓ (needs stroke→width)   |
| fp_rect  | (start)(end)(stroke)(fill)(layer)       | ✗                        |
| fp_circle| (center)(end)(stroke)(fill)(layer)      | ✓ (needs stroke→width)   |
| fp_arc   | (start)(mid)(end)(stroke)(layer)        | ✗ (expects angle, not mid) |
| fp_poly  | (pts)(stroke)(fill)(layer)              | ✓ (needs stroke→width)   |
| fp_curve | (pts 4 points)(stroke)(layer)           | ✗ (Bezier)               |
| fp_text  | reference/value/user + at, layer, effects | ✓                      |

---

## Pad Format

```
(pad "1" thru_hole circle
  (at 0 0 [180])
  (size 2 2)
  (drill 1)                    ; or (drill oval 1 3)
  (layers "*.Cu" "*.Mask")
  (roundrect_rratio 0.25)      ; for roundrect shape
)
```

**Pad types:** thru_hole, smd, np_thru_hole, connect  
**Pad shapes:** circle, rect, oval, trapezoid, roundrect, custom

kicad_parse_gen supports: circle, rect, oval, trapezoid. No roundrect, no custom.

---

## Library Architecture Options

### Option A: Pre-process + kicad_parse_gen (Current Approach)

**Pros:** Reuses existing parser, minimal new code  
**Cons:** Incomplete support; fp_arc (mid), fp_rect, roundrect pads, fp_curve fail

**Required normalizations:**
1. `(footprint` → `(module`
2. Strip `fp_rect`, `embedded_fonts`, `fp_curve`
3. Extract `(stroke (width W)...)` → `(width W)` for fp_line, fp_circle, fp_poly
4. **fp_arc:** Convert `(start)(mid)(end)` → `(start)(end)(angle)`:
   - Compute arc center and radius from 3 points
   - Compute sweep angle from start→mid→end
5. **roundrect pads:** Approximate as `rect` or `oval` (lose corner radius)

### Option B: Dedicated KiCad 9 Parser

**Pros:** Full fidelity, no hacks  
**Cons:** More work, duplicate logic

**Structure:**
```
kicad9_footprint/
├── mod.rs
├── parse.rs      # S-expression parser
├── data.rs       # Footprint, Pad, FpLine, FpArc, etc.
├── normalize.rs  # Optional: convert to kicad_parse_gen Module
└── error.rs
```

**Data types:**
- `Footprint { name, version, layer, descr, tags, properties, attr, elements, model }`
- `FpArc { start, mid, end }` (native) or `{ start, end, angle }` (computed)
- `Pad { shape: PadShape }` with `RoundRect(rratio)`
- `FpRect`, `FpCurve` (Bezier)

### Option C: Fork/Patch kicad_parse_gen

Extend kicad_parse_gen to support KiCad 9 natively. Requires upstream changes.

---

## Recommended Design: Option A + Incremental Option B

**Phase 1 – Robust pre-processor (extends current footprint.rs):**

1. **S-expression reader**  
   Lightweight s-expr tokenizer that can:
   - Rewrite `(footprint` → `(module`
   - Strip unsupported top-level elements
   - Transform fp_arc (mid) → (angle)
   - Flatten stroke into width

2. **fp_arc mid→angle conversion**

   Given (start, mid, end), compute center and sweep:
   - Center: perpendicular bisectors of start-mid and mid-end
   - Radius: distance from center to start
   - Angle: signed sweep from start to end via mid

3. **Stroke extraction**  
   When we see `(stroke (width W)(type T))`, emit `(width W)` for fp_line, fp_circle, fp_arc, fp_poly.

**Phase 2 – Optional native types:**

- Add `FpArcMid { start, mid, end }` if we want to keep the original representation
- Add `PadShape::RoundRect(rratio)` and approximate for rendering (e.g. as rect with rounded corners)

---

## File Layout

```
src/
├── main.rs
├── footprint.rs          # Existing: load, draw, kicad_parse_gen integration
└── kicad9/
    ├── mod.rs            # Public API
    ├── sexp.rs           # S-expression parsing/rewriting
    ├── normalize.rs      # KiCad 9 → kicad_parse_gen compatible string
    └── arc_geom.rs       # mid→angle geometry
```

**Public API:**
```rust
pub fn read_footprint(path: &Path) -> Result<Module, Error>;
pub fn read_footprint_str(s: &str) -> Result<Module, Error>;
pub fn load_footprint_dir(dir: &Path) -> Result<HashMap<String, Module>, Error>;
```

---

## Implementation Notes

### fp_arc: mid → angle

Arc through (sx,sy), (mx,my), (ex,ey):
- Perpendicular bisector of s-m: mid_sm = ((sx+mx)/2, (sy+my)/2), dir = (my-sy, sx-mx)
- Perpendicular bisector of m-e: mid_me = ((mx+ex)/2, (my+ey)/2), dir = (ey-my, mx-ex)
- Intersection = center (cx, cy)
- Radius r = sqrt((sx-cx)² + (sy-cy)²)
- Start angle = atan2(sy-cy, sx-cx)
- End angle = atan2(ey-cy, ex-cx)
- Sweep = end - start (normalize to [-180, 180] for direction)

### Stroke extraction

Pattern: `(stroke (width W)(type T))`  
Action: Replace whole `(stroke ...)` with `(width W)` when inside fp_line, fp_circle, fp_arc, fp_poly.

### Roundrect pads

- `roundrect_rratio` ∈ [0,1]: corner radius = rratio × min(size.x, size.y) / 2
- Render: use rect with rounded corners (e.g. 4 quarter-circles + rectangles)
- Parse: treat as rect for kicad_parse_gen; store rratio in a side structure if needed for rendering

---

## Testing

Use footprints from `/usr/share/kicad/footprints/`:

- `Resistor_SMD.pretty/R_0603_1608Metric.kicad_mod` – SMD, roundrect pads
- `Connector_Pin.pretty/Pin_D1.0mm_L10.0mm.kicad_mod` – simple THT
- `Display.pretty/LM16255.kicad_mod` – complex, many pads
- `LED_THT.pretty/LED_D3.0mm_Clear.kicad_mod` – fp_arc (mid)
- `TerminalBlock_Philmore.pretty/...` – fp_poly, roundrect

---

## References

- [KiCad S-expression format](https://dev-docs.kicad.org/en/file-formats/sexpr-intro/)
- [Footprint library format](https://dev-docs.kicad.org/en/file-formats/sexpr-footprint/)
- kicad_parse_gen v7: `~/.cargo/registry/src/.../kicad_parse_gen-7.0.2/`
