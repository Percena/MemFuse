# Contributing to MemFuse

Thank you for your interest in contributing to MemFuse!

## Getting Started

```bash
# Clone the repo
git clone https://github.com/Percena/MemFuse.git
cd MemFuse

# Build (requires Rust 1.85+)
cargo build --workspace

# Run tests
cargo test --workspace

# Run lints
cargo clippy --workspace
cargo fmt --all -- --check
```

## SDK Development

```bash
cd sdk
npm install
npm run build
npm run lint
npm test
```

## Pull Requests

1. Fork the repository and create a feature branch from `main`
2. Ensure `cargo test --workspace` passes
3. Ensure `cargo clippy --workspace` has no warnings
4. Ensure `cargo fmt --all -- --check` passes
5. If SDK changes: ensure `cd sdk && npm run lint && npm test` passes
6. Write a clear PR description explaining what changed and why

## Reporting Issues

Open an issue on GitHub with:
- Steps to reproduce
- Expected vs actual behavior
- Environment details (OS, Rust version, Node version)

## Code Style

- Follow existing patterns in the codebase
- Rust: workspace clippy and fmt rules apply
- TypeScript: match existing SDK conventions
- Prefer minimal, focused changes over large refactors

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
