# ADR 0001: Repository layout

- Status: Accepted
- Date: 2026-09-19
- Ticket: TI-0021

## Context

The product combines a Tauri desktop interface, native audio capture, local Python processing, shared data contracts, and model integrations. Feasibility spikes must remain distinguishable from production components.

## Decision

Use one repository organized into `apps`, `services`, `packages`, `spikes`, `tests`, `docs`, and `scripts`. Keep platform and model integrations behind explicit interfaces. Do not move the audio spike into a production application until the desktop and capture boundaries are defined.

## Consequences

The repository can share contracts and checks while keeping ownership clear. Cross-language tooling will be added only with the component that needs it, avoiding unused dependencies during foundation work.
