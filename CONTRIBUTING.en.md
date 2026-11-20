# Contributing Guide

[简体中文](CONTRIBUTING.md) | English

Thanks for contributing to Oh My Gitty! Please take a few minutes to read this guide.

## Development Environment

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://www.rust-lang.org/) toolchain
- The system `git` CLI

```bash
npm ci
npm run dev    # development mode (auto-reloads on save)
npm run build  # native installer for the current OS: macOS .dmg / Windows NSIS setup.exe
```

## Filing Issues

- Search first to make sure the issue hasn't already been reported.
- For bugs, please include: OS environment, Tauri/app version, reproduction steps, expected vs. actual behavior, and relevant logs.
- For feature requests, describe the use case and the expected behavior.

## Submitting PRs

1. Fork the repository and create a branch: `git checkout -b feat/your-feature`
2. Before submitting, run `cargo test --manifest-path src-tauri/Cargo.toml` and `npm run build:debug`.
3. Follow [Conventional Commits](https://www.conventionalcommits.org/) (e.g., `feat:`, `fix:`, `refactor:`).
4. Make sure the PR description clearly covers what changed and why, and links any related issues.

## Code Style

- The frontend is native HTML/CSS/JS (ES Modules); do not introduce a bundler.
- The Rust backend follows the existing module layout (`git.rs` / `ai.rs` / `runner.rs` / `process.rs` / `watcher.rs` / `cli.rs` / `version.rs` / `config.rs` / `lib.rs`).
- Comments explain "why", not "what" or "how".

## Releasing (maintainers)

**Automatic release on version bump**: update the `version` field in all three files — `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` (they must match) — and push to `main`. The `Version Bump` workflow detects the change and calls the `Release` workflow to build installers for all three platforms and publish them. Pushing a `v*` tag manually still works too.

Builds are signed with the updater private key stored in repository Secrets:

- `TAURI_SIGNING_PRIVATE_KEY`: full contents of `.tauri/updater.key`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the key passphrase (this repo's key has none; set it to an empty string)

The private key lives only in local `.tauri/updater.key` (gitignored) and in repository Secrets (write-only). Back up `.tauri/updater.key` somewhere safe, such as a password manager; **if it is lost you must generate a new key and update the pubkey in `tauri.conf.json`, and already-released versions will no longer pass update signature verification**.

## License

By submitting code, you agree to license your contribution under the [MIT License](LICENSE).
