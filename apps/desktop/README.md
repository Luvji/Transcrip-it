# Transcrip-it desktop

The desktop client is a Tauri 2 application with a React, TypeScript, Vite, and Tailwind CSS frontend.

## Linux prerequisites

Install Tauri's native Ubuntu/Debian dependencies once:

```bash
sudo apt update
sudo apt install build-essential curl wget file pkg-config libdbus-1-dev libglib2.0-dev libgtk-3-dev libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

If Ubuntu reports exact-version conflicts between Pango runtime and development packages, ensure the standard `jammy-updates` repository is enabled before retrying:

```bash
sudo add-apt-repository -y "deb http://in.archive.ubuntu.com/ubuntu/ jammy-updates main restricted universe multiverse"
sudo apt update
```

Rust is installed through `rustup`. If a newly opened terminal cannot find it, run `source "$HOME/.cargo/env"`.

## Run it

From the repository root:

```bash
npm install
npm run desktop:dev
```

For browser-only UI work, use `npm run desktop:web`. Run the complete JavaScript and frontend validation suite with `npm run check`.

## Local database

The Tauri backend creates `transcrip-it.sqlite3` in the platform application-data directory on startup. Schema changes are applied transactionally from the ordered migrations under `src-tauri/src/database/migrations/`. Run `npm run check:rust` from the repository root to format-check the backend and execute its migration tests.

Meeting and job state changes are implemented in `src-tauri/src/database/workflow.rs`. Callers must supply stable idempotency keys, and workers must use the lease token returned by their current job claim.
