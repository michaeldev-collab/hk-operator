# Test Plan

Two layers: an automated logic smoke test (runs in Node, no browser) and a
manual UI smoke test (5 minutes in a browser).

## 1. Automated logic smoke test
```bash
cd hk-operator/mcc
node test/smoke.mjs
```
**Covers:** constants, action validation (good + each failure mode), the URL
`http(s)://` rule, `normalizeAction` (trim, id, timestamps, tag parsing,
identity-preserving edits), seed integrity (every seed action validates), and
`filterActions` (category / type / text / favorites filtering + favorites-first
sort).

**Expected (verified on this build):**
```
31 passed, 0 failed   (exit code 0)
```
Coverage includes combined category+type+query filtering, empty-array filtering,
`validateAction(null/undefined)`, whitespace-only values, uppercase/`ftp`/
whitespace URL cases, and no-tags normalization (added after the review pass).

## 2. Manual UI smoke test
Start the app:
```bash
cd hk-operator/mcc/src
python3 -m http.server 8000
# open http://localhost:8000
```

Run through these. Each should pass:

| # | Action | Expected |
|---|--------|----------|
| 1 | Load the page | Dashboard renders seeded cards; no console errors |
| 2 | Type "docker" in search | Only Docker actions remain; count updates |
| 3 | Pick a category in the filter | Only that category shows |
| 4 | Pick a type (e.g. `prompt`) | Only prompt actions show |
| 5 | Click ★ Favorites | Only favorites show; toggle off restores all |
| 6 | Click **Copy** on a command card | Toast "Copied…"; paste elsewhere = exact value; card shows "used …" |
| 7 | Click **Open ↗** on a URL card | URL opens in a new tab |
| 7b | Click the secondary **Copy** on a URL card | URL text copied to clipboard |
| 7c | Focus search, type, press **Enter** | Top result is copied (or opened, if it's a URL) |
| 8 | Click ☆ on a card | Turns gold; card jumps to the top (favorites-first) |
| 9 | **+ New action**, fill it, Save | New card appears; persists |
| 10 | Submit the form with an empty name | Inline error; not saved |
| 11 | Add a `url` action with value `example.com` (no http) | Inline error: must start with http(s):// |
| 12 | **Edit** a card, change it, Save | Card updates; same position/id |
| 13 | **Delete** a card, confirm | Card removed |
| 14 | **Reload the page** | All your changes are still there (localStorage) |
| 15 | **Export** | `macro-actions.json` downloads |
| 16 | **Import** that file | Confirm prompt warns it REPLACES all current actions; on confirm, toast "Imported N actions" and cards match; on cancel, nothing changes |

## 3. Reset to seed (if needed)
In the browser console:
```js
localStorage.removeItem("hk.operator.actions.v1"); location.reload();
```

## Known limitations (not failures)
- Commands are copy-only; the app never executes anything (by design).
- Single browser/profile storage; use Export/Import to move data.
- **Import is destructive** — it replaces all current actions (after a confirm
  prompt). Export first to back up.
