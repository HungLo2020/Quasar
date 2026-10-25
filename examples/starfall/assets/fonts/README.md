# Starfall fonts

These fonts are shipped only so the reference game is self-contained and
deterministic across machines.

| File | License |
|---|---|
| `DejaVuSans.ttf`, `DejaVuSans-Bold.ttf` | Bitstream Vera license; DejaVu changes are public domain. See `LICENSE-DejaVu.txt`. |
| `NotoSansDevanagari-Regular.ttf` | SIL Open Font License 1.1. See `LICENSE-Noto.txt`. |

DejaVu Sans supplies the Latin/Greek/Cyrillic/Hebrew/Arabic faces and is the
default UI family. Noto Sans Devanagari is registered as an explicit fallback
face for Devanagari shaping. Starfall builds the fallback list from the files in
this directory only; it does not scan system fonts.
