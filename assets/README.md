# oah — logo asset pack

The mark is an open ring (the "o" of oah) with an agent node sitting in the gap.

## Colors

| Name   | Hex       | Use                                   |
|--------|-----------|---------------------------------------|
| Yellow | `#FFD43B` | Brand color, node on dark, icon tile  |
| Ink    | `#0E0E0F` | Dark background, mark on light/yellow |
| Light  | `#E6EDF3` | Ring and wordmark on dark             |
| Gold   | `#B38600` | Node on light backgrounds             |
| Paper  | `#F6F7F9` | Light background                      |

Wordmark: JetBrains Mono ExtraBold (outlined in the SVGs, no font needed).

## Files

- `svg/` — master vectors: marks, app icons, lockups
- `png/mark`, `png/icon` — 64–1024 px; `png/lockup` — 600/1200/2400 px wide
- `favicon/` — `favicon.ico` (16/32/48), `favicon.svg`, `apple-touch-icon.png`, `icon-192.png`, `icon-512.png`
- `social/` — GitHub social preview (1280×640) and README banners (dark/light)

## Rules

- Use `mark-on-dark` on dark, `mark-on-light` on light, `mark-black` / `mark-white` for one-color.
- Keep clear space around the mark of at least the node's diameter.
- Don't rotate the mark, close the ring, or move the node out of the gap.

## README header

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/social/oah-readme-banner-dark.png">
  <img alt="oah — open agent harness" src="assets/social/oah-readme-banner-light.png">
</picture>
```

## Web

```html
<link rel="icon" href="/favicon.ico" sizes="any">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="apple-touch-icon" href="/apple-touch-icon.png">
```
