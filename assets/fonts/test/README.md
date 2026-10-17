# Test font

Inter Variable, unmodified, from the official [Inter repository](https://github.com/rsms/inter).

- Commit: `353b61b9f4430d5f420d56605a6e7993e0941470`
- File: `docs/font-files/InterVariable.ttf`
- SHA-256: `4989b125924991b90d05b2d16e0e388c48f7d5bb8b30539bbf9c755278d0ccaf`
- License: SIL Open Font License 1.1, complete text in [OFL.txt](OFL.txt).

Render fixtures load only this font, never the host font database. Keep font bytes fixed when comparing golden files; an update requires deliberate regeneration of affected goldens.

The offline renderer also embeds these licensed font bytes as a fallback when no system fonts are available. `ensure_default_font` preserves a nonempty caller database; golden fixtures continue to inject only Inter.
