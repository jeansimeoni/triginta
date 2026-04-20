## Summary

Describe the change and why it is needed.

## Testing

- [ ] `mise exec -- cargo fmt --check`
- [ ] `mise exec -- cargo clippy --all-targets -- -D warnings`
- [ ] `mise exec -- cargo test --locked`
- [ ] `cargo deny check`

## Documentation And Configuration

- [ ] I updated user-facing documentation where behavior or configuration changed.
- [ ] I updated `docs/configuration.md` if configuration behavior changed.
- [ ] Not applicable.

## Dependency And License Impact

- [ ] No dependency or license changes.
- [ ] Dependency changes preserve exact pins, `Cargo.lock`, and `deny.toml`.
