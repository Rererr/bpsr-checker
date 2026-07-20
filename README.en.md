# bpsr-checker

**[日本語](./README.md) | [English](./README.en.md)**

**A lightweight DPS checker for Blue Protocol: Star Resonance (Windows only)**

[![Latest release](https://img.shields.io/github/v/release/Rererr/bpsr-checker?display_name=tag&sort=semver)](https://github.com/Rererr/bpsr-checker/releases)
[![License](https://img.shields.io/github/license/Rererr/bpsr-checker)](./LICENSE)
[![Downloads](https://img.shields.io/github/downloads/Rererr/bpsr-checker/total)](https://github.com/Rererr/bpsr-checker/releases)
![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-blue)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/exU3gPBx3)

Built with **Slint (a native Rust GUI)**. It focuses on just the features you actually need during combat and measurement, so it keeps CPU and memory usage low and stays smooth even over long sessions, while still letting you display a semi-transparent overlay on top of the game. **It never sends any data to external servers.**

<p align="center">
  <img src="docs/images/main.png" alt="Main window — DPS list (semi-transparent overlay)" width="820">
</p>

## Features

- **DPS / Healing / Damage Taken / History tabs** — Aggregates damage dealt, healing, and damage taken with tab switching (column headers and totals follow the tab). Encounters are saved automatically when combat ends, and **past logs survive app restarts** (stored on disk).
- **Per-skill breakdown** — Click a player to see damage, hit count, and crit rate for each of that player's skills.
- **Battle Imagine name** — Shows each player's equipped Battle Imagines (Tina / Airona / Tatta / Basilisk, etc.) with their tier level appended to the name column of the DPS list (e.g. `Sora-Tina(3)/Airona(1)`). Your own is detected on every map transition; other players are detected when a dungeon loads or a boss room transition occurs, so even Imagines that rarely activate are reliably shown. Simplified Battle Imagines slotted into the Role Skill category (up to 4 at once) are shown separately from your two equipped Battle Imagines, appended in `(R:name)` form (e.g. `Sora-Tina(3)/Airona(1) (R:Tempest Ogre/Fafala)`). The `{imagine}` token in the name-column template lets you reposition it, or remove it to hide. It is always shown in the header of the per-skill breakdown you open by clicking a player. Imagine names can be maintained manually in a dedicated editor window opened from Settings — override display names or hide specific Imagines (overrides affect display only, never detection or records).
- **Measurement mode** — A dedicated mode for training dummies and boss practice that aggregates for a fixed duration from the start of combat (default 180s, adjustable). The results screen shows per-character and per-skill DPS trend graphs (with time and DPS axes) and a pie chart of the skill breakdown (TOP 10 + others). It also **auto-records your personal best per measurement duration** and celebrates with a "New record!" badge; the TOP 3 are highlighted in gold/silver/bronze, and a timestamped result image can be copied with one click to share on Discord and elsewhere.
- **Imagine (Battle Imagine) debuff timer** — Displays the remaining immunity-debuff time for Tina / Aluna / Tata / Basilisk in a separate overlay. By default it syncs with the DPS list, automatically following which players are shown and their order (order-following can be turned off individually); use the pin icon to hide/show each player in the timer. Turn sync off to fall back to the classic mode that only shows targets you pinned manually, with a clear-all action provided. The Imagine types shown can be selected individually to match your party composition. A compact dense layout is also supported. Column headers show the katakana name and a per-character color ring (Tina red / Aluna green / Tata purple / Basilisk brown) for visibility.
- **Self buff/debuff display** — Shows the buffs/debuffs currently on your own character in a separate window. They are shown as icons with a remaining-time bar so you can grasp them at a glance. Stacking buffs follow the stack count (×N) and timer updates.
- **Self status display** — Shows your character's stats such as Attack, Crit, and HP in a dedicated real-time overlay. Which items to show can be freely customized per in-game panel group (bulk ON/OFF supported).
- **Food / syrup display** — Shows each player's food and syrup buff usage as a vertical remaining-time timer in the name column of the DPS list. Hover an icon to see the effect and remaining time. Remaining time is preserved across combat end, manual reset, and app restart (expired ones disappear automatically). Can be toggled ON/OFF in settings.
- **Debuff-timer-only mode (lightweight)** — A resource-saving mode that stops DPS/healing aggregation and runs only the debuff timer.
- **2-column compact layout** — A layout mode that splits the DPS table into two columns to save horizontal space.
- **DPS trend graph** — Visualizes each player row's DPS trend as a small sparkline.
- **Always-on-top / click-through / taskbar–tray switching** — Essential for overlay use. Buttons in the title bar let you minimize and toggle always-on-top pinning with one tap, and when the window is narrow the header automatically switches to icon-only. Minimizing collapses to the **system tray** by default (minimizing the main window also tucks away the overlays; restore via left-click on the tray icon or right-click → "Show main"). Enabling "Keep in taskbar" in settings makes the main window and each overlay their **own taskbar buttons**, so each window's minimize button minimizes it to the taskbar. Each overlay can be closed with the × button at the top right (linked to the show toggle in settings). Click-through (passing clicks through to the game behind) is toggled and released from the tray. Each window can be resized by dragging its edges.
- **Overlay appearance customization** — The background opacity, font, text size, bold, and text color of each overlay can be configured independently of the main window. Text color and the app's accent color can be freely adjusted with an HSV picker.
- **Copy templates** — Copy aggregated results to the clipboard in any format (e.g. for pasting into Discord).
- **Multi-language** — Japanese / English.
- **Character selection** — When auto-detection is wrong, you can fix your own character by entering its UID directly in the settings panel or selecting from the current player candidates (the name is resolved and shown from the name cache).
- **Footer contact links** — A semi-transparent footer at the bottom of the main window (can be hidden in settings) lets you open a contact form or file a GitHub issue in your default browser.

### Screenshots

| | |
|---|---|
| <img src="docs/images/result-3min.png" alt="3-minute measurement results" width="400"><br>**3-minute measurement results** — Per-character/skill DPS trends and a breakdown pie chart | <img src="docs/images/debuff-timer.png" alt="Imagine debuff timer" width="400"><br>**Imagine debuff timer** — Immunity-debuff remaining time shown with per-character color rings |
| <img src="docs/images/self-status.png" alt="Self buff/debuff display" width="400"><br>**Self buff/debuff display** — A glanceable list of icons with remaining-time bars | <img src="docs/images/settings.png" alt="Settings panel" width="400"><br>**Settings panel** — Opacity, column visibility, copy templates, and more |

## Installation

Download the latest `bpsr-checker-setup-x.x.x.exe` (installer) from [Releases](https://github.com/Rererr/bpsr-checker/releases) and run it. There is also a no-install portable version, `bpsr-checker-portable-x.x.x.zip` (unzip and run `bpsr-checker.exe`).

- You can install updates without closing the running app.
- Settings and history are preserved across reinstalls (`%APPDATA%\bpsr-checker`).

### Requirements

- Windows 10 / 11 (x64)
- Administrator privileges (required to load the WinDivert kernel driver)

## Safety & Privacy

Answers to common concerns about this tool.

### Will I get banned for using this?

**It does not modify any game files, memory, or network traffic.** It merely passively observes incoming packets and reconstructs the damage-display values — it performs no injection, patching, or automation against the game client.

That said, this is an **unofficial, individually-developed tool**, and the possibility that it stops being tolerated due to a future change in the operator's terms cannot be ruled out. **The final decision to use it is at your own risk.** (See the disclaimer at the end of the [License](#license) section.)

### Is it a virus? My antivirus flagged it

**It is a false positive.** It bundles the [WinDivert](https://github.com/basil00/WinDivert) driver, which captures packets at the kernel level, so some antivirus software may warn about it as a "network monitoring tool."

For example, on VirusTotal, Kaspersky may report `Not-a-virus:HEUR:RiskTool.Multi.WinDivert.gen`. This classifies the bundled WinDivert driver as "riskware" (a network tool) — it is **not malware** (note the `Not-a-virus` prefix).

What to do:
- Add the WinDivert driver (`WinDivert.dll`, `WinDivert64.sys`) and the install folder to your antivirus exclusions.
- If you are worried, you can review the [source code](https://github.com/Rererr/bpsr-checker) and [build it yourself](#building-from-source) (GPL-3.0).
- Every release is scanned on VirusTotal: [installer](https://www.virustotal.com/gui/file/458989e1a1038839d29f6a257007190c554c97fbf482e376fe92afac5b9a5b1f/detection) · [portable](https://www.virustotal.com/gui/file/f8ffa9cd25eaf3aac6ddd338f472d88aebe258547c727399e1d59976700494a6/detection).

### Chrome blocks the download with "Virus detected"

This Chrome warning is a Google Safe Browsing verdict. Like the above, it is a **false positive caused by insufficient reputation for an unsigned exe bundling WinDivert**. If the SHA256 of the downloaded file matches the hash in the VirusTotal links above, it is a genuine, untampered release.

How to download:
1. Open Chrome's downloads list (`Ctrl+J`), then on the blocked item choose "⋮" → "Keep".
2. If "Keep" is not offered, download it directly with PowerShell (bypasses the browser scan):
   ```powershell
   irm https://github.com/Rererr/bpsr-checker/releases/download/<version>/bpsr-checker-setup-<version>.exe -OutFile bpsr-checker-setup.exe
   ```
3. Or use the portable zip from the [releases page](https://github.com/Rererr/bpsr-checker/releases/latest) (an exe inside a zip can avoid the download-time scan).

### Windows SmartScreen shows "Windows protected your PC"

Code signing (via [SignPath](https://signpath.org/)) is in progress, but newly signed apps can trigger a SmartScreen warning during the transition period until reputation (a track record of executions) accumulates.

How to bypass:
1. Click "More info" in the dialog.
2. Click the "Run anyway" button that appears.

### Does it send data anywhere?

**No.** The app contains no HTTP client library, and it performs no automatic sending of telemetry, analytics, or crash reports. Everything is processed locally (except checking GitHub Releases for updates).

### How it works (simplified)

1. Start WinDivert in **SNIFF mode** (passive observation only).
2. Observe TCP packets going to/from the game server.
3. Decode the payload as [protobuf](https://protobuf.dev/) and extract damage/healing events from messages such as `SyncNearDeltaInfo`.
4. Aggregate per UID and display in the UI.

For details, see [`core/src/capture/windivert.rs`](./core/src/capture/windivert.rs).

## Usage

1. Launch the app (UAC will request administrator privileges, just like the game).
2. Start the game and begin combat — damage is detected automatically.
3. Click a player row to see the per-skill breakdown.
4. When combat ends (no damage for 10 seconds by default), it is saved to history automatically.

### System tray

**Left-click** the tray icon to **restore the main window**; **right-click** to open the menu.

- **Click-through** — Toggle on/off. While on, all windows pass the mouse through (so you can operate the game behind them), which is why you **must always disable it from the tray menu**.
- **Show/hide main**
- **Quit**

### Settings panel

Open it with the **S** button in the header. Main items:

- Fixing your character UID / selecting from candidates
- Opacity, font size, column visibility (including ON/OFF for food/syrup display)
- Copy templates (placeholders such as `{name} {dmg} {dps}`)
- Time setting for the 3-minute measurement mode
- Imagine debuff timer display toggle / sync with the main DPS list (order-following ON/OFF, clear-all watch) / individual selection of which Imagine types to show / dense layout / debuff-timer-only mode (stops DPS aggregation for lighter operation) / 2-column compact layout ON/OFF
- Self buff/debuff display ON/OFF
- Adding to the watchlist is done via the pin icon next to the player row in the DPS list
- Startup tab (DPS / Healing / History)

## Known limitations

- **About nearby characters shown right after launch / reset**
  Because this tool passively observes the packets the game client receives, it may fail to obtain the name, class, and gear-score info — which the server sends only once — for characters already in view at the moment of launch or reset.
  Such characters are shown faintly as "Player #XXXX," and their class is auto-estimated from their skills. UIDs observed in the past are restored automatically from a 30-day name cache. When they re-enter your view via a zone change or re-login, the correct info is obtained.

## Troubleshooting

| Symptom | What to do |
| --- | --- |
| No damage is detected | Check that you launched as administrator. If you have a VPN or ping reducer (ExitLag / NoPing, etc.) enabled, disable it and try again. |
| Antivirus flags it | See the [section above](#is-it-a-virus-my-antivirus-flagged-it). |
| Won't start / quits immediately | Check that `WinDivert.dll` and `WinDivert64.sys` are in the same folder as `bpsr-checker.exe` (the installer bundles them automatically). |
| License differs from older releases | v0.7.8 and later are GPL-3.0; earlier versions were MIT. ([details](#license)) |

Report bugs and requests via [Issues](https://github.com/Rererr/bpsr-checker/issues) or [Discord](https://discord.gg/exU3gPBx3).

## Building from source

```bash
# Prerequisites: Rust stable, Protoc, Visual Studio Build Tools (Windows)

git clone https://github.com/Rererr/bpsr-checker.git
cd bpsr-checker

# Obtain WinDivert (Windows only)
# Download the v2.2.2 "A" build from https://github.com/basil00/WinDivert/releases
# and place WinDivert.dll / WinDivert64.sys into windivert/

# Run for development (administrator privileges required)
cargo run -p bpsr-app

# Build distributables (release exe + bundled WinDivert -> zip; an installer too if makensis is present)
pwsh scripts/package-slint.ps1
```

The artifacts are generated under `dist-slint/` (the portable zip, and an installer if NSIS is present).

## Related projects

There are other DPS meters being developed for the same game. This project takes inspiration from their strengths.

- [winjwinj/bpsr-logs](https://github.com/winjwinj/bpsr-logs) — Rust + Tauri + Svelte, active Discord community
- [anying1073/StarResonanceDps](https://github.com/anying1073/StarResonanceDps) — .NET + WPF, feature-rich
- [dmlgzs/StarResonanceDamageCounter](https://github.com/dmlgzs/StarResonanceDamageCounter) — the origin of many derived implementations

## A note on usage (please read)

This tool is intended for **a player's personal review**. Please do not use it for:

- Publicly exposing other players' scores to insult or provoke them
- Demanding gear from / refusing to play with people in pickup groups

DPS varies greatly with gear, skill rotation, situation, and role. Treat the numbers as a reference only.

## Support

If you would like to support continued development, you can do so via [GitHub Sponsors](https://github.com/sponsors/Rererr).

## License

This software is distributed under the [**GNU General Public License v3.0 only (GPL-3.0-only)**](./LICENSE).

- If you distribute a modified version, you must publish the source code under the same GPL-3.0 license.
- Keep the copyright notice, the full license text, and an indication of your changes.

> **Note**: v0.7.7 and earlier were distributed under the MIT license; from v0.7.8 the license changed to GPL-3.0.

### Disclaimer

This software is provided as-is, **without any warranty of any kind, express or implied**. The author is not liable for any damages arising from the use of or inability to use this software. Use it at your own risk.

Copyright (C) 2025 Rererr
