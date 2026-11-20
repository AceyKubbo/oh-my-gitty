# Oh My Gitty UI Design System

[简体中文](ui-design-system.md) | English

## Design Direction

The visual baseline is the "terminal console" look of the reference design: black canvas, thin-line dividers, information density first, color reserved for state and data. The UI avoids large gradients, heavy shadows, and rounded cards.

## Colors

Semantic tokens come in dark and light sets; state colors stay the same across both. The table below lists the dark (default) theme values; see `[data-theme=light]` in `src/styles.css` for the light theme.

| Token | Dark value | Usage |
| --- | --- | --- |
| `--bg` | `#1b1b1b` | App canvas, code areas |
| `--panel` | `#222222` | Panels, cards |
| `--panel-2` | `#292929` | Menus, inputs, floating layers |
| `--border` | `#464646` | Default dividers |
| `--border-strong` | `#5b5b5b` | Focus, drag, and emphasized boundaries |
| `--text` | `#f0f0f0` | Body text and primary values |
| `--text-head` | `#dddddd` | Headings |
| `--text-dim` | `#b5b5b5` | Secondary info, paths, timestamps |
| `--accent` | `#7d5cdf` | Selection, AI, branch, and commit actions (purple by default) |
| `--orange` | `#ff9d00` | Running, ports, and activity data |
| `--green` | `#00d084` | Success, completed, running |
| `--red` | `#ff6b6b` | Conflicts, deletions, dangerous actions |

## Themes & Accents

- Light and dark themes follow the system preference by default and can be switched in Settings (`data-theme`).
- Accent colors are orthogonal to the theme: 7 palettes (purple, light/dark blue, yellow, light/dark green, red) each define separate `--accent` values for dark and light modes, under `[data-accent]` in `src/styles.css`. Swatches in the settings dialog are previews only.
- State colors (success, conflict, running, etc.) do not change with the theme or accent.

## Typography & Hierarchy

- Body text uses the system sans-serif font for reliable Chinese rendering.
- Numbers, paths, commands, status labels, and group headings use `ui-monospace`.
- Page titles 14–16px; group labels 11–12px using uppercase/letter-spacing for hierarchy; body 12–13px.
- Numbers use high-contrast white; explanatory text drops to `--text-dim` so nothing competes for attention.

## Layout & Components

- Pages use 1px dividers and an 8/12/16/24px spacing rhythm.
- Buttons, inputs, tags, badges, and cards are strictly square-cornered to avoid the generic SaaS card look.
- Interactive controls are transparent or use the surface color by default; hover raises one gray step; selected states use an accent-colored left border/outline instead of large accent fills.
- Data cards keep borders and use no shadows; chart grid lines use low-contrast gray.
- Destructive actions must be red and keep the existing confirmation flow.

## Page Application

- Overview: KPI in three columns, activity and type stats, project cards and run states; the accent color (purple by default) for commits/AI, orange for running/ports.
- Project page: project identity and branch at the top, a compact action bar; staged, modified, conflicts, and history as linear groups.
- Diff/logs: a darker code canvas with low-opacity green/red backgrounds for added/removed lines.
- Settings and confirmation dialogs: canvas background of the active theme, thin borders, square corners; danger notes appear only for irreversible actions.
