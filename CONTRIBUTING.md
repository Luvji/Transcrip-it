# Contributing to Transcrip It

## Development baseline

- Use Node.js 22 as declared in `.nvmrc`.
- Install dependencies with `npm ci` when a lockfile is present.
- Run `npm run check` before committing.
- Keep work associated with the active ticket in `dev.tkt`.
- Do not commit recordings, meeting content, generated notification records, secrets, model files, or local databases.

## Code standards

- Prefer small modules with explicit inputs and outputs.
- Use ECMAScript modules and built-in platform APIs where practical.
- Keep TypeScript strict when the desktop application is introduced.
- Format Rust with `rustfmt` and require clean `clippy` output when the Rust toolchain is added.
- Use four-space indentation and type annotations for future Python processing services; add Ruff and pytest with that service.
- Treat original recordings and transcript segments as immutable evidence. Store corrections and generated interpretations separately.
- Never write meeting content, transcripts, secrets, or raw model prompts to diagnostic logs.

## Tests and documentation

- Add focused automated tests for success, error, interruption, and recovery paths.
- Update user-facing instructions and architecture decisions with behavior changes.
- Preserve raw evidence when adding derived audio, transcripts, translations, or summaries.
- Record meaningful architecture and privacy choices under `docs/decisions/`.

## Local Git checks

The repository uses a versioned pre-commit hook. Enable it once after cloning:

```bash
git config core.hooksPath .githooks
```

The hook runs the same dependency-free validation and tests as continuous integration.
