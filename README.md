# keydibs

Find keybindings that never fire because another layer grabs them first.

When you press a key, every layer of your setup gets a chance to claim it, in order:

```
compositor (KDE, Hyprland, Sway…) → terminal (Alacritty, Konsole…) → multiplexer (tmux) → shell (fish, zsh) → app (nvim, micro…)
```

The first layer that has a binding for the chord calls dibs. Every binding further
down for the same chord is dead, and nothing tells you. keydibs reads all those configs
and tells you.

```console
$ keydibs why ctrl+shift+c
Ctrl+Shift+C
  compositor  kde        takes it          Take Screenshot [kwin]  (~/.config/kglobalshortcutsrc:2)
  terminal    alacritty  SHADOWED by kde   Copy  (~/.config/alacritty/alacritty.toml:82)
```

It knows the rules each layer actually follows, so it doesn't cry wolf:

- A terminal binding with Alacritty's `mode = "~Alt"` only takes the key outside
  full-screen apps, so it shadows your shell's binding but not your editor's.
- Bindings for a terminal's own modes (vi mode, search) never shadow anything.
- Alacritty's `Ctrl+L` clears its notice *and* passes `^L` on, so your shell still gets it.
- The shell and full-screen apps take turns rather than stacking, so a fish binding
  never shadows a micro one.
- Tools in the same layer (Alacritty and Konsole) are alternatives, not rivals.

## Install

You need Rust 1.88 or newer ([rustup.rs](https://rustup.rs)). keydibs reads Linux
desktop and terminal configs, so it's Linux-only for now.

```sh
cargo install keydibs
```

Or the latest from `main`:

```sh
cargo install --git https://github.com/Blzs2005/keydibs
```

## Usage

```sh
keydibs              # same as `keydibs check`
keydibs check        # report shadowed and duplicate bindings; exits 1 if any are found
keydibs why <chord>  # every layer that binds a chord, in the order a key press reaches them
keydibs list         # every binding keydibs found
```

Chords are case-insensitive: `ctrl+shift+c`, `Meta+E`, `super+escape`. `Meta` means
the Super/logo key, as it does in KDE.

## Supported tools

| Layer | Tool | Status |
|---|---|---|
| compositor | KDE Plasma (`kglobalshortcutsrc`) | ✅ |
| compositor | Hyprland, Sway, niri, GNOME | planned |
| terminal | Alacritty (`alacritty.toml` + built-in defaults) | ✅ |
| terminal | Konsole, kitty, wezterm, foot, ghostty | planned |
| multiplexer | tmux, zellij | planned |
| shell | fish (every mode, including vi bindings) | ✅ |
| shell | zsh, bash (readline) | planned |
| app | micro, nvim, helix | planned |

fish builds most of its keymap in code when a session starts, so keydibs asks fish
itself: it runs `fish -i` with your config (minus the greeting) and reads `bind`.
That's the same thing that happens when you open a new terminal.

## Roadmap

- **Built-in defaults** for every tool, as Alacritty already has. Most conflicts
  involve bindings nobody wrote down, like a terminal's default `Ctrl+Shift+V`.
- **Alacritty:** follow `general.import`, and support scancode bindings (`key = 123`).
- **fish:** map raw escape-sequence bindings (`\e\[1\;9A`) to the keys they stand for;
  they're skipped for now. Add a timeout in case a config hangs.
- **Unreachable chords.** Many terminals can't send `Ctrl+Shift+letter` or `Ctrl+;`
  as distinct keys unless they support the kitty keyboard protocol, so apps bound to
  them never see them.
- **Keyboard layouts.** `Meta+~` and `Meta+Shift+`\` are the same keys on a US layout.

## Adding a tool

Each tool is one file in `src/adapters/` that implements `Adapter`: find the config,
parse it into `Binding`s with a `Chord`, a human-readable action, and a source line.
Keep the parser a pure function over the file's text so it can be tested with a
fixture, like `src/adapters/kde.rs`.

## License

MIT
