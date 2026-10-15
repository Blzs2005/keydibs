# keydibs

Find keyboard shortcuts that clash between your desktop, terminal, shell and editor.

When you press a key, every layer of your setup gets a chance to claim it, in order:

```
compositor (KDE, Hyprland, Sway…) → terminal (Alacritty, Konsole…) → multiplexer (tmux) → shell (fish, zsh) → app (nvim, micro…)
```

The first layer that has a binding for the chord calls dibs. Every binding further
down for the same chord is blocked, and nothing tells you. keydibs reads all those
configs, plus each tool's built-in shortcuts, and tells you.

For example, on a stock KDE setup with Konsole, fish and micro, nothing has been
customized and there are already 20 clashes:

```console
$ keydibs check
shadowed   Ctrl+Alt+H
  fish "backward-kill-word" (built-in default)
  is blocked by konsole "SSH Manager: Quick Access" (built-in default)

shadowed   Shift+Right
  micro "SelectRight" (built-in default)
  is blocked by konsole "Next Tab (with 2+ tabs)" (built-in default)
…
20 conflicts in 534 bindings
```

`why` shows everything that wants one key combination, in the order a key press
reaches them:

```console
$ keydibs why ctrl+alt+h
Ctrl+Alt+H
  terminal    konsole    takes it              SSH Manager: Quick Access  (built-in default)
  shell       fish       SHADOWED by konsole   backward-kill-word  (built-in default)
  app         micro      SHADOWED by konsole   DeleteWordLeft  (built-in default)
```

It knows the rules each layer actually follows, so it doesn't cry wolf:

- A terminal binding with Alacritty's `mode = "~Alt"` only takes the key outside
  full-screen apps, so it shadows your shell's binding but not your editor's.
- Bindings for a terminal's own modes (vi mode, search) never shadow anything.
- Alacritty's `Ctrl+L` clears its notice *and* passes `^L` on, so your shell still gets it.
- The shell and full-screen apps take turns rather than stacking, so a fish binding
  never shadows a micro one.
- Tools in the same layer (Alacritty and Konsole) are alternatives, not rivals.
- Konsole's tab and split shortcuts are switched off until you have several tabs or
  a split view, so conflicts with them say so: `Next Tab (with 2+ tabs)`.

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
| compositor | Hyprland (live keymap, any config format; submaps) | ✅ |
| compositor | Sway, niri, GNOME | planned |
| terminal | Alacritty (`alacritty.toml` + built-in defaults) | ✅ |
| terminal | Konsole (built-in shortcuts + changes made in its settings) | ✅ |
| terminal | kitty, wezterm, foot, ghostty | planned |
| multiplexer | tmux, zellij | planned |
| shell | fish (every mode, including vi bindings) | ✅ |
| shell | zsh, bash (readline) | planned |
| app | micro (`bindings.json` + built-in defaults, every pane) | ✅ |
| app | nvim, helix | planned |

Hyprland is configured in Lua since 0.55, where binds can come from loops and
functions, so keydibs reads the live keymap with `hyprctl -j binds` instead of
parsing the config. Hyprland has to be running; if it isn't, it's skipped quietly.

fish builds most of its keymap in code when a session starts, so keydibs asks fish
itself: it runs `fish -i` with your config (minus the greeting) and reads `bind`.
That's the same thing that happens when you open a new terminal.

Built-in shortcuts are taken from each tool's source code for a specific version:
Alacritty 0.17, Konsole 26.08 and micro 2.0.15. Other versions may differ slightly.

## Roadmap

- **Built-in defaults** for every tool, as Alacritty, Konsole and micro already have.
  Most conflicts involve shortcuts nobody wrote down, like a terminal's default
  `Ctrl+Shift+V`.
- **Alacritty:** follow `general.import`, and support scancode bindings (`key = 123`).
- **Konsole:** read the Quick Commands and SSH Manager quick-access shortcuts when
  they're changed in those plugins' own settings, and KDE-wide changes to standard
  shortcuts (`kdeglobals`).
- **micro:** raw escape-sequence bindings (`"\u001b[1;5D"`) are skipped.
- **Hyprland:** binds on raw keycodes (`code:28`) are skipped, since reading them
  needs the keyboard layout. Combos of plain keys (`Super_L + Alt_L`) show only their
  last key, because that's all `hyprctl` reports. Mouse binds are out of scope.
- **fish:** map raw escape-sequence bindings (`\e\[1\;9A`) to the keys they stand for;
  they're skipped for now. Add a timeout in case a config hangs.
- **Unreachable chords.** Many terminals can't send `Ctrl+Shift+letter` or `Ctrl+;`
  as distinct keys unless they support the kitty keyboard protocol, so apps bound to
  them never see them.
- **Keyboard layouts.** `Meta+~` and `Meta+Shift+`\` are the same keys on a US layout.

## Adding a tool

Each tool is one file in `src/adapters/`. See [CONTRIBUTING.md](CONTRIBUTING.md) for a
step-by-step guide, and the [`adapter` issues](https://github.com/Blzs2005/keydibs/issues?q=is%3Aissue+is%3Aopen+label%3Aadapter)
for tools people want; the ones labeled `good first issue` are the easiest to start with.

## License

MIT
