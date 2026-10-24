# Making a Rosorah Studio theme

A theme is one small text file (TOML) that changes Rosorah's colours and corners, and can bring
a background picture. Anyone can make one in a text editor and share it as a single file.

## Quick start

1. Settings → Look & feel → **New theme from this look**. Rosorah writes `my-theme.toml` with
   every colour filled in, chooses it, and opens it in your text editor.
2. Change a colour and save. Rosorah updates within a second; no restart.
3. Rename the file and change `name = "…"` when you're happy with it.

Themes live in the themes folder (**Open themes folder** in the same place):

- Linux: `~/.config/rosorah-studio/themes/`
- Windows: `%APPDATA%\rosorah-studio\themes\`

A theme is either `name.toml` in that folder, or a folder `name/theme.toml` when it comes with a
picture. To install someone's theme, use **Import theme…** or copy it (or its folder) into the
themes folder.

## The file

Everything is optional. Anything left out comes from the `base` theme, so this is a complete
theme:

```toml
name = "Sunset"
[colors]
accent = "#ff8a3d"
```

All the settings:

```toml
name = "Forest"            # shown in Settings
author = "you"             # shown when hovering the theme
base = "dark"              # "dark" or "light": fills in everything not listed

[colors]                   # "#rgb", "#rrggbb", or "#rrggbbaa" (last pair = opacity)
background = "#0e1a12"     # behind the panels
panel = "#16261b"          # panels, windows and Settings cards
card = "#1d3324"           # chat messages and other blocks inside panels
button = "#24402d"
button_hover = "#2f5439"
text = "#e8f5e9"
muted_text = "#a5c8ab"     # hints and secondary text
accent = "#8bd17c"         # selected buttons and highlights; overrides Settings' accent colour
on_accent = "#0b2410"      # text on accent-coloured buttons
good = "#6ed28c"           # connected, ready
warning = "#e6be5a"        # waiting, experimental
error = "#ff8080"          # failures and missing files
live = "#ff5c5c"           # LIVE, REC and the program outline

[shape]
panel_corners = 4          # 0 (square) to 40
button_corners = 6         # 0 (square) to 40

[background]               # applied when someone chooses the theme; they can change it after
picture = "background.jpg" # a file next to theme.toml (PNG, JPEG, WebP)
color = "#282048"          # used when there's no picture
fit = "fill"               # fill, fit or stretch
panels = "frosted"         # solid, transparent, frosted, matte or liquid
transparency = 40          # 0-95 (%)
clarity = 50               # 0-100 (%): how sharp the picture looks through glass
```

## Mistakes

A misspelt setting or a colour Rosorah can't read is shown under the theme in Settings, with the
line it's on. Rosorah keeps showing the theme's last working version until the file is fixed.
For safety, a theme's picture must sit next to its file: paths with folders (`../`, `C:\…`) are
refused.

## Sharing

Send the `.toml` file, or zip the theme's folder when it has a picture. Themes can only change
how Rosorah looks; they can't run anything.
