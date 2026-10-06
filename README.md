# Jig

<img width="1123" height="798" alt="Jig editing a file" src="https://github.com/user-attachments/assets/3359dfad-a506-4c8e-8a56-5474d16d8a00" />

<img width="1134" height="813" alt="Jig previewing a change" src="https://github.com/user-attachments/assets/ceabdaed-93f9-4953-a564-bb4d00b9d1e5" />

<img width="597" height="411" alt="image" src="https://github.com/user-attachments/assets/1b28e39a-5494-4138-8174-a2a59ec864ab" />

Every AI workflow I tried pulled me away from the code. There was a chat panel
to explain things to, an agent somewhere else to wait on, and a pile of diffs to
read afterwards. The code became something I reviewed, not something I wrote.

I still wanted the power of AI, just where it's actually useful: in the code,
at the cursor, on the lines I'm looking at. So I built Jig.

Select some code, open the jig commands, pick a jig or type what you want, and the
change shows up inline. **Tab** keeps it, **Esc** throws it away. Nothing
reaches the disk until you say so.


## Building

You need Rust; `rust-toolchain.toml` pins the version.

```sh
cargo run -p jig-app -- path/to/project
```

On macOS, `script/bundle-macos.sh` builds `target/Jig.app`. Connect a model
provider in Settings > Model. The Agent lane also needs the
[`opencode`](https://opencode.ai) CLI on your `PATH`.

[`AGENTS.md`](AGENTS.md) describes how Jig works and how the code is laid out.

## License

[Apache-2.0](LICENSE). Language icons are from
[Simple Icons](https://simpleicons.org) (CC0-1.0); see
[`assets/icons/README.md`](assets/icons/README.md).
