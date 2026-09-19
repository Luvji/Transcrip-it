# Repository layout

- `apps/` — user-facing desktop applications.
- `services/` — local processing and application services.
- `packages/` — shared schemas, contracts, and reusable libraries.
- `spikes/` — time-bounded feasibility experiments that are not production architecture.
- `tests/` — cross-component and end-to-end fixtures.
- `docs/` — architecture, decisions, feasibility evidence, and contributor guidance.
- `scripts/` — repository automation and validation.
- `.tickets/` — local tracker support data and ticket evidence.

The current audio recorder remains under `spikes/audio-capture/` until its behavior is incorporated into the production capture service. Meeting recordings under `recordings/` are local data and are ignored by Git.

The desktop backend and its ordered SQLite migrations live under `apps/desktop/src-tauri/src/`. The persistence contract is documented in [`persistence.md`](persistence.md).

Replaceable processing contracts live under `apps/desktop/src-tauri/src/engines/` and are documented in [`engine-interfaces.md`](engine-interfaces.md).
