# Contributing

Thanks for looking. [AGENTS.md](AGENTS.md) is the guide to Jig: the concept,
how the code is laid out, and how to work on it. Read its "Working on Jig"
section first.

## Before a pull request

```sh
cargo fmt
cargo clippy --all-targets   # zero warnings
cargo test
```

CI runs the same on macOS, Linux and Windows, plus `cargo deny check`
([deny.toml](deny.toml)) for known vulnerabilities, licences and sources.

- Keep a change to one thing, with tests for what it fixes or adds; UI tests
  go in `crates/jig-app/src/workspace/tests/`, one file per feature.
- Match the surrounding style: doc comments say why, in plain sentences;
  user-facing text is short and specific.
- When a change alters the concept, the layout or the Jig-mode contract,
  update AGENTS.md in the same pull request.
- A new dependency needs a licence `deny.toml` accepts, and a reason.
