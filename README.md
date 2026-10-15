# keydibs

Find keyboard shortcuts that clash between your desktop, terminal, shell and editor.

## Why?

You press a shortcut in your editor and nothing happens. Or something else happens.

That's usually a **clash**. Your desktop, your terminal, your shell and your editor
each have their own keyboard shortcuts, and when two of them use the same keys, the
one that sees the key press first wins. The other shortcut silently stops working.

```
your key press → desktop → terminal → shell → editor
                 (first)                      (last)
```

keydibs reads the shortcuts of every tool you use, including the built-in ones
nobody ever wrote down, and tells you which ones clash.

## Try it

```sh
cargo install keydibs
keydibs
```

That's it. There's nothing to set up: keydibs finds the tools you have, and it only
**reads** your settings, never changes them.

> You'll need Rust to install it. If `cargo` isn't found, get it from
> [rustup.rs](https://rustup.rs). keydibs works on Linux.

## Reading the results

Here's what keydibs finds on a fresh CachyOS install (KDE, Alacritty, Konsole, fish
and micro) before changing a single setting:

```console
$ keydibs
scanned kde (compositor): 112 bindings
scanned alacritty (terminal): 90 bindings
scanned konsole (terminal): 77 bindings
scanned fish (shell): 99 bindings
scanned micro (app): 156 bindings

shadowed   Ctrl+Alt+H
  fish "backward-kill-word" (built-in default)
  is blocked by konsole "SSH Manager: Quick Access" (built-in default)

shadowed   Shift+Right
  micro "SelectRight" (built-in default)
  is blocked by konsole "Next Tab (with 2+ tabs)" (built-in default)
…
20 conflicts in 534 bindings
```

The first lines show which tools keydibs found and how many shortcuts each has.
After that, each clash reads like a sentence:

- **`Ctrl+Alt+H`** is the key combination.
- **fish** wants it to delete the previous word, but it never gets there, because
  **Konsole** uses it first to open its SSH Manager.
- **(built-in default)** means nobody configured this; it's how the tool ships.
  When a shortcut comes from your own config, keydibs shows the file and line instead.

Some clashes only happen sometimes, and keydibs says when. `Next Tab (with 2+ tabs)`
means Konsole only takes `Shift+Right` while you have more than one tab open. With a
single tab, micro gets it as usual.

### Looking up one shortcut

Wondering why a particular shortcut doesn't work? Ask about it:

```console
$ keydibs why ctrl+alt+h
Ctrl+Alt+H
  terminal    konsole    takes it              SSH Manager: Quick Access  (built-in default)
  shell       fish       SHADOWED by konsole   backward-kill-word  (built-in default)
  app         micro      SHADOWED by konsole   DeleteWordLeft  (built-in default)
```

This lists every tool that uses the key combination, in the order your key press
reaches them, and which one actually gets it.

## Fixing a clash

Pick the shortcut you care about less, and change it in that tool:

| Tool | Where to change shortcuts |
|---|---|
| KDE Plasma | System Settings → Keyboard → Shortcuts |
| Konsole | Settings → Configure Keyboard Shortcuts (`Ctrl+Alt+,`) |
| Alacritty | the `[keyboard]` section of `~/.config/alacritty/alacritty.toml` |
| fish | a `bind` line in `~/.config/fish/config.fish` |
| micro | `~/.config/micro/bindings.json` |
| Hyprland | your Hyprland config |

Then run `keydibs` again to check it's gone.

## Commands

| Command | What it does |
|---|---|
| `keydibs` | Same as `keydibs check` |
| `keydibs check` | Lists every clash. Exits with code 1 if there are any, so you can use it in scripts |
| `keydibs why <keys>` | Shows every tool that uses one key combination, and which one wins |
| `keydibs list` | Lists every shortcut keydibs found |

Key combinations aren't case-sensitive: `ctrl+shift+c`, `Ctrl+Shift+C` and
`shift+ctrl+c` are all the same. `Meta` and `Super` both mean the Windows/logo key.

## Supported tools

| | Works now | Coming |
|---|---|---|
| **Desktop** | KDE Plasma, Hyprland | Sway, niri, GNOME |
| **Terminal** | Alacritty, Konsole | kitty, WezTerm, foot, Ghostty |
| **Terminal multiplexer** | | tmux, Zellij |
| **Shell** | fish | zsh, bash |
| **Editor** | micro | Neovim, Helix |

Tools you don't have are skipped automatically.
Missing one you use? [Each planned tool has an issue](https://github.com/Blzs2005/keydibs/issues?q=is%3Aissue+is%3Aopen+label%3Aadapter),
and comments or help are welcome.

<details>
<summary><b>How keydibs avoids false alarms</b></summary>

A shortcut only counts as blocked when the key really can't get through:

- **Different screens.** Some terminal shortcuts only work at the shell prompt and
  step aside inside full-screen programs like editors (Alacritty's `Shift+PageUp`,
  for example). Those can block your shell, but not your editor.
- **Special modes.** Shortcuts that only exist in a tool's own mode, like Alacritty's
  vi mode or Konsole's search bar, never block anything.
- **Shortcuts that pass the key on.** Alacritty's `Ctrl+L` clears its notice *and*
  still sends `Ctrl+L` to your shell, so nothing is blocked.
- **Taking turns.** Your shell and a full-screen editor never read keys at the same
  time, so a fish shortcut can't block a micro one.
- **Alternatives.** Alacritty and Konsole are two different terminals; you use one or
  the other, so their shortcuts can't clash with each other.

</details>

<details>
<summary><b>Good to know</b></summary>

- **Built-in shortcuts** come from each tool's source code for a specific version:
  Alacritty 0.17, Konsole 26.08 and micro 2.0.15. Other versions may differ slightly.
- **fish** sets up most of its shortcuts when it starts, so keydibs starts fish (with
  your config, minus the greeting) and asks it, just like opening a new terminal does.
- **Hyprland** is read from the running desktop, so it has to be running. If it isn't,
  it's skipped.
- **Not covered yet:**
  - Alacritty files pulled in with `general.import`
  - Konsole plugin shortcuts changed in the plugins' own settings, and KDE-wide changes
    to standard shortcuts
  - Raw key codes and escape sequences in Alacritty, Hyprland, fish and micro
  - Mouse bindings
  - Keyboard layouts other than US for symbol keys: `Meta+~` and `Meta+Shift+`\`
    are the same keys on a US layout, but keydibs treats them as different
  - Key combinations your terminal can't send at all (like `Ctrl+;` in many
    terminals), which no program can ever receive

</details>

## Contributing

Adding support for a tool means writing one file. [CONTRIBUTING.md](CONTRIBUTING.md)
walks you through it, and issues labeled
[`good first issue`](https://github.com/Blzs2005/keydibs/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)
are the easiest place to start.

To install the latest unreleased version from `main`:

```sh
cargo install --git https://github.com/Blzs2005/keydibs
```

## License

MIT
