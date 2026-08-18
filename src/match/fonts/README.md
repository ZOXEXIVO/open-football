# Viewer typefaces

`Outfit-subset.ttf` and `Manrope-subset.ttf` are the faces every label in the
match viewer is drawn with — the name plates over the players, the clock, the
chips, the loading notice.

## Why they are vendored

Bevy's `default_font` feature compiles in `FiraMono-subset.ttf`, and that subset
is close to ASCII-only. Football squads are not an ASCII data set: 117 distinct
non-ASCII characters occur in the shipped `database.db`, so a name plate read
`Concei□□o` rather than `Conceição`. Nothing falls back on the way out either —
`fontique` resolves system fonts, and a browser tab running WASM has none — so a
missing glyph is a box on screen, not a substituted letter.

## Why two of them

These are the same two faces the web UI already ships. `style.css` declares both
under `font-family: "Outfit"`: Outfit for the Latin ranges, Manrope for
`U+0400-045F`, because Outfit has no Cyrillic. A browser picks between them per
character; the viewer has no per-character machinery, so `Typeface` picks per
label instead — Outfit unless the label contains something Outfit cannot spell.

That choice cannot be made from the script. Outfit is missing 29 characters
scattered through Latin Extended-A, so `coverage.rs` — generated from the subset
itself — is what `Faces::face_for` reads.

## Provenance

| | Outfit | Manrope |
|---|---|---|
| Upstream | `google/fonts` `ofl/outfit/Outfit[wght].ttf` | `google/fonts` `ofl/manrope/Manrope[wght].ttf` |
| Licence | OFL 1.1 (`OFL-Outfit.txt`), no Reserved Font Name | OFL 1.1 (`OFL-Manrope.txt`), no Reserved Font Name |
| Instance | `wght` 400, which is what `body` renders at | same |
| Size | 111 KB upstream, 49 KB subset | 165 KB upstream, 66 KB subset |

## Known gaps

Two characters in `database.db` are in neither face: `Ə` (U+018F, Azerbaijani
capital schwa, 13 occurrences) and `ӧ` (U+04E7, 1 occurrence). Manrope has the
lowercase `ə`, but no face the web UI ships has the capital, so those names still
show a box. Closing that would mean a third face the rest of the app does not
use.

## Rebuilding

`subset.mjs` documents the ranges and the exact commands, and regenerates
`coverage.rs` alongside the fonts. Rerun it if the database starts shipping a
script the subsets do not cover; the check is that every non-ASCII character in
`database.db` resolves to a glyph in one of the two.
