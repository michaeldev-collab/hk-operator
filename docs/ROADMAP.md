# Roadmap

- [x] Choose LICENSE — **MIT** (see `LICENSE`)
- [x] Phase 0 baseline — [`docs/portfolio-engineering-plan.md`](./portfolio-engineering-plan.md)
- [x] Phase 1 harness stubs — [`docs/verification/harness.md`](./verification/harness.md)
- [x] Phase 2 protocol / dispatch / composer regressions — `npm test` under `mcc/`
- [x] Phase 3 sanitized threat model — [`docs/security-threat-model.md`](./security-threat-model.md)
- [x] Phase 4 — P3-01…P3-09 remediated (fire token through URL/MacroEvent edge gates)
- [x] Phase 5 — GitHub Actions CI: Rust check/test, JS tests, firmware source gate ([run](https://github.com/michaeldev-collab/hk-operator/actions/runs/30478533368) green)
- [x] Phase 6 — Sanitized Cyberpad→MCC portfolio evidence ([`docs/portfolio-evidence/`](./portfolio-evidence/))
- [x] Phase 7 start — Release packaging plan ([`docs/release-packaging.md`](./release-packaging.md), [`docs/COMPATIBILITY.md`](./COMPATIBILITY.md))
- [x] Phase 8 start — Cyberpad V2 hardware draft ([`hardware/cyberpad-v2/`](../hardware/cyberpad-v2/), [`docs/hardware-v2-pcb.md`](./hardware-v2-pcb.md))
- [x] Rename runtime config dir to `~/.config/hk-operator/` in MCC binary
- [x] Git-aware profile sync UI (MCC panel)
- [ ] Enter-key composer reset (global hotkey)
- [ ] Phase 7 execute — cut `v0.2.0` tag to produce Release `.deb` (workflow: [`release.yml`](../.github/workflows/release.yml))
- [ ] Phase 8 execute — KiCad footprints/Gerbers, NeoPixel power decision, case dims lock to PCB
