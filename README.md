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
and micro, with a Hungarian keyboard) before changing a single setting:

```console
$ keydibs
scanned kde (compositor): 112 bindings
scanned alacritty (terminal): 90 bindings
scanned konsole (terminal): 77 bindings
scanned fish (shell): 99 bindings
scanned micro (app): 156 bindings
keyboard layout: hu

shadowed   Ctrl+Alt+H
  fish "backward-kill-word" (built-in default)
  is blocked by konsole "SSH Manager: Quick Access" (built-in default)

shadowed   Shift+Right
  micro "SelectRight" (built-in default)
  is blocked by konsole "Next Tab (with 2+ tabs)" (built-in default)
…
21 conflicts in 534 bindings
```

The first lines show which tools keydibs found, how many shortcuts each has, and
your keyboard layout. After that, each clash reads like a sentence:

- **`Ctrl+Alt+H`** is the key combination.
- **fish** wants it to delete the previous word, but it never gets there, because
  **Konsole** uses it first to open its SSH Manager.
- **(built-in default)** means nobody configured this; it's how the tool ships.
  When a shortcut comes from your own config, keydibs shows the file and line instead.

Some clashes only happen sometimes, and keydibs says when. `Next Tab (with 2+ tabs)`
means Konsole only takes `Shift+Right` while you have more than one tab open. With a
single tab, micro gets it as usual.

### Shortcuts that can't arrive

Some key combinations can't get through a terminal at all. Most terminals send
`Ctrl+Shift+S` exactly like `Ctrl+S`, `Shift+Enter` like `Enter`, and don't pass
Super on, so a shell or editor bound to one of those never sees it:

```console
can't arrive Ctrl+Shift+S
  micro "SaveAll" (~/.config/micro/bindings.json:2)
  alacritty and konsole send it as Ctrl+S, because micro doesn't use the kitty keyboard protocol
```

The *kitty keyboard protocol* fixes this, but only when the terminal supports it
**and** the program turns it on. keydibs knows which of your tools do (Alacritty,
Konsole and fish do; micro doesn't), and only reports what really can't arrive.

### Your keyboard layout

On a Hungarian keyboard, `(` is Shift+8; on a US one it's Shift+9. Tools write such
shortcuts either way (`Ctrl+(` or `Ctrl+Shift+8`), so keydibs checks your layout and
compares the keys you actually press. That's why it shows `Ctrl+(` as
`Ctrl+Shift+8` on a Hungarian layout, and why it finds clashes like this one, which
only exist on layouts where `/` needs Shift:

```console
shadowed   Ctrl+Shift+6
  fish "undo" (built-in default)
  is blocked by konsole "Copy Input to None" (built-in default)
```

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

### Clashes you don't mind

Some clashes never bother you: maybe you never split Konsole's view, so it doesn't
matter that it takes `Ctrl+Shift+Up`. List them in `~/.config/keydibs/ignore`,
one per line, with the keys as keydibs prints them:

```sh
# I never split the view in Konsole
Ctrl+Shift+Up
Ctrl+Shift+Down

# Only clashes involving Konsole; fish still gets checked
Shift+Right konsole
```

Naming one or more tools after the keys limits the line to clashes they're part
of. A line also covers key sequences that start with its keys, and `#` starts a
comment.

keydibs then leaves those out, says how many it left out, and exits with 0 when
nothing else is left, so `keydibs` can run in scripts. It also tells you about lines
that match nothing, which catches typos and clashes you've fixed since.
`keydibs check --no-ignore` shows everything again.

## Commands

| Command | What it does |
|---|---|
| `keydibs` | Same as `keydibs check` |
| `keydibs check` | Lists every clash and every shortcut that can't arrive, except the ones [you've chosen to ignore](#clashes-you-dont-mind). Exits with code 1 if there are any, so you can use it in scripts. Add `--no-ignore` to see them all |
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
  Alacritty 0.17, Konsole 26.08 and micro 2.0.15. If you have a different version,
  keydibs says so, since a few shortcuts may differ.
- **fish** sets up most of its shortcuts when it starts, so keydibs starts fish (with
  your config, minus the greeting) and asks it, just like opening a new terminal does.
  If a tool takes more than 5 seconds to answer, keydibs skips it rather than hang.
- **Hyprland** is read from the running desktop, so it has to be running. If it isn't,
  it's skipped.
- **Your keyboard layout** is read with libxkbcommon, which every Linux desktop has.
  If it's missing, symbols are compared as written. Symbols typed with AltGr aren't
  matched yet.
- **Not covered yet:**
  - Raw key codes (Alacritty `key = 123`, Hyprland `code:28`)
  - Mouse bindings

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
