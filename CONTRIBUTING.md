# Contributing to keydibs

Thanks for helping! Most contributions are **adapters**: one file that teaches keydibs
to read one tool's keybindings. These are good ones to learn from:

| Adapter | How it reads bindings | Worth copying for |
|---|---|---|
| `src/adapters/kde.rs` | Parses a config file | A small, plain parser with line numbers |
| `src/adapters/alacritty.rs` | Built-in defaults + config file on top | Tools with default bindings, modes |
| `src/adapters/fish.rs` | Runs the tool and parses its output | Keymaps built by code at startup |
| `src/adapters/hyprland.rs` | Asks the running program over IPC (JSON) | Tools with a "list bindings" command |
| `src/adapters/nvim.rs` | Runs the tool with a script that prints JSON | Tools with a scripting API; built-ins read from the tool's own docs |

## Writing an adapter

1. **Create `src/adapters/<tool>.rs`** with a struct that implements `Adapter`
   (`src/adapters/mod.rs`):
   - `tool()`: a short lowercase name, like `"kitty"`.
   - `layer()`: where the tool sits in the chain a key press travels down:
     `Compositor`, `Terminal`, `Multiplexer`, `Shell` or `App`.
   - `where_to_change()`: where the user changes the tool's shortcuts, as in the
     README's table. The app shows it next to each clash.
   - `load()`: return `Ok(None)` when the tool isn't installed or set up, so users
     who don't have it see nothing. Return `Err` only for real problems, like a
     config file that exists but can't be read.
   - `kitty_keyboard()` (terminals, shells and apps): whether a terminal supports
     the kitty keyboard protocol, or a shell or app turns it on. It decides which
     shortcuts can't arrive, so check the tool's docs; the default is `false`.
   - `notes()` (optional): things to tell the user, like an installed version that
     differs from the one your built-in defaults come from (see `version_note`).

2. **Keep the parsing a pure function** (`pub fn parse(text: &str, …) -> Vec<Binding>`)
   so it can be tested with a fixture string, without the tool installed. `load()`
   just finds the input and calls it.

3. **Fill in each `Binding`** (`src/binding.rs`):
   - `chord`: build it with `Chord::from_str` (`"ctrl+shift+c".parse()`) or from
     `Mods` + `Key::parse`. Translate the tool's key names; if a name means the same
     key as another spelling (`Return`/`enter`), add the alias to `Key::parse` in
     `src/chord.rs` so it matches across tools. Watch out for `meta`: `Mods::parse_one`
     treats it as Super, but terminals and editors often mean Alt.
   - `action`: what the binding does, readable by a human. Use `shorten()` for long
     commands.
   - `grabs`: whether it takes the key from the layers below. `Always` for most.
     `Never` if the tool acts *and* passes the key on, or the binding only exists in
     the tool's own mode. Terminals can also have `NormalScreen` / `AltScreen` (see
     `alacritty.rs`).
   - `mode`: the tool's own mode the binding lives in (`"vi"`, `"insert"`, a tmux key
     table…), or `None`. Bindings in different modes never count as duplicates.
   - `then`: the rest of a key *sequence* (`Ctrl+X, Ctrl+E`, a tmux prefix, a Vim
     leader sequence). The first key goes in `chord`.
   - `source`: `Source::Config { path, line }` when you can point to a file (and line),
     `Source::Default` for the tool's built-ins, `Source::Runtime` otherwise. If the
     tool can't say where a binding was made, `ConfigFiles` can look for the line
     in the files it read (see `bash.rs` and `zsh.rs`).

4. **Skip what you can't represent** instead of guessing: mouse bindings, raw
   scancodes, and modifiers keydibs doesn't model (Caps Lock, Hyper). Note what's
   skipped in the module's doc comment.

5. **Register it** in `all()` in `src/adapters/mod.rs`, and add a row to the
   *Supported tools* table in the README.

## Tests

Every adapter needs unit tests on a fixture: ideally a trimmed copy of a real
config or real command output. Cover the tricky parts of the tool's format, like
modes, overrides and unusual key names.

If you can, also run `cargo run -- check` and `cargo run -- why <chord>` on a
machine that has the tool, and mention in the PR what you checked.

The app (`src/gui/`) is tested in `src/gui/tests.rs`, which clicks through it with
[egui_kittest](https://docs.rs/egui_kittest) without opening a window. Shared logic,
like what counts as a clash, lives in the library (`src/report.rs`, `src/scan.rs`)
so the command line and the app agree.

## Before opening a PR

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same three. Keep to the existing style: small functions, comments that
say *why*, and no new dependencies unless the format really needs one (a TOML,
JSON or KDL parser is fine).
