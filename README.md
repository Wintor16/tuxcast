# 🎣 TuxCast

A GPO (Grand Piece Online) fishing companion for Roblox that doesn't stop at Windows — native Tauri/Rust app for **Windows** and **Linux** (KDE/Wayland) alike, so your penguin gets to fish too.

Started as a fork of the original Windows-only GPO Autofish; grew a full native Linux backend (KWin-driven window management, portal-based screen capture and input, tesseract OCR) plus a round of Wayland-specific bug fixes along the way.

## What is this?

A transparent, fully open-source fishing macro — no black-box binary, no "trust me":

- ✅ **Fully open source** — read every line, build it yourself
- ✅ **No viruses** — nothing hidden, nothing to take on faith
- ✅ **Actually cross-platform** — real native backends on both sides, not a Windows app limping along under Wine

**Features:**

- **🎣 Fishing System** — Automatic fish detection and tracking with a physics-based controller
- **🍎 Devil Fruit Detection** — OCR-powered detection of devil fruit drops with keyword matching
- **🌟 Fruit Spawn Alerts** — Detects and webhooks when devil fruits spawn with exact fruit name recognition
- **📦 Auto Fruit Storage** — Automatically stores devil fruits in your fruit slots when detected
- **🔔 Discord Webhook Alerts** — Notifications for devil fruit catches, world spawns, purchases and recoveries
- **🛒 Auto-Purchase** — Configurable bait purchasing every X fish
- **🪱 Auto Bait** — Re-selects your bait before every cast
- **🎯 Auto Setup** — Zoom control and cast positioning
- **🛟 Watchdog** — Restarts a stuck loop on its own
- **💾 Presets** — Save and load full settings snapshots
- **⌨️ Global hotkey support** (F1/F2/F3/F4, all rebindable)

## 🚀 Key Features

### 🍎 Devil Fruit Detection

- **OCR Detection**: Detects devil fruit drops via text recognition (Windows OCR on Windows, tesseract on Linux)
- **Spawn Detection**: Detects when devil fruits spawn in the world (all 33 GPO fruits)
- **Fuzzy Matching**: Handles OCR errors with a similarity threshold
- **Auto Storage**: Automatically stores caught fruits into two hotbar slots and re-equips the rod
- **Webhook Alerts**: Discord notifications for catches and spawns

### 🎯 Auto Setup

- **Zoom Control**: Automatically zooms out/in for fishing
- **Cast Positioning**: Moves to the casting position (auto center or a custom point)
- **Menu Clearing**: Right-clicks to clear menus

### 🛒 Auto-Purchase

- **Configurable Intervals**: Buy bait every X fish caught
- **Point System**: Holds the shop key next to the bait barrel, then clicks Confirm, Quantity and an optional Cancel point
- **Auto-save**: Settings persist between sessions

### ⚡ Performance

- **Fast Detection**: Bar tracking runs in microseconds, not Python pixel loops
- **Tiny Footprint**: Small installer, no runtime to install
- **Logging**: Live activity feed in the Dashboard plus a log file in the data folder

## Installation

### Windows

1. **Download** the latest installer from [Releases](../../releases/latest)
2. **Run it** — no admin needed
3. **Launch TuxCast** — the panel opens; the HUD appears once Roblox is running

Requires Windows 10 1809 or newer. Windows OCR needs an English language pack, present on nearly every install; the Setup page tells you if it's missing.

### Linux (KDE Plasma / Wayland)

1. **Download** the `.AppImage` or `.deb` from [Releases](../../releases/latest)
2. Install/run it (`chmod +x *.AppImage && ./TuxCast*.AppImage`, or `sudo dpkg -i tuxcast*.deb`)
3. Install `tesseract` for devil-fruit OCR (e.g. `sudo pacman -S tesseract tesseract-data-eng` / `sudo apt install tesseract-ocr`)
4. On first run, approve the screen-share + input-control dialog KDE shows — this grants the companion the same screen-reading and click access the Windows build gets natively

Requires KDE Plasma on Wayland. The window-management pieces (always-on-top, placement) are implemented against KWin specifically.

### 🔧 Build it yourself

Requirements: [Node.js 20+](https://nodejs.org) and [Rust](https://rustup.rs). On Linux, also GStreamer + PipeWire development packages (see the Linux job in `.github/workflows/release.yml` for the exact package list) and `tesseract`.

```bash
npm install
npm run tauri build
```

## 🎮 Quick Start Guide

### Before you start

- **Rod in slot 1**: Put your fishing rod in the first hotbar slot (key `1`)
- **Empty inventory**: Clear everything else out of your hotbar so fruits and bait land where the bot expects them

### First Time Setup

1. **Launch**: Open Roblox, join GPO, then open TuxCast. The Setup page shows the window as detected
2. **Bar area**: Cast once by hand. When the blue bar shows, open Setup › Fishing bar area and press **Auto-detect**. The thumbnail turns green when matched
3. **Drop message area**: Draw it over the popup at the top middle of the screen where "New Item <Fruit>" and "A Fruit has spawned at Place" appear. Press **Read now** to confirm the OCR reads it
4. **Rod key**: Slot `1`
5. **Enable Features**: Turn on auto bait, auto buy, fruit storage and webhooks in the Features page. Each one lists its steps, keys and points. Each **Pick** opens a crosshair over Roblox
6. **Fish**: Press **F1** or the HUD play button

### Devil Fruit Storage Setup

1. **Enable Store fruits** in the Features page
2. **Set Fruit Keys**: Choose the two hotbar slots the fruit is moved through
3. **Set Fruit Point**: Pick the Store button that appears after switching to a fruit slot
4. **Set Rod Key**: Slot `1` holds your fishing rod (Setup › Rod key)
5. **Set Bait Point**: Pick the top bait in the rod menu (Features › Auto bait)

### Auto-Purchase Setup

1. **Stand next to the bait barrel** on the dock before starting
2. **Enable Auto buy bait** in the Features page and follow the numbered steps
3. **Shop key**: The bot holds it until the barrel's shop dialog opens
4. **Confirm / Quantity / Cancel**: Pick each button in the shop. Cancel is optional
5. **Amount and interval**: How much bait to type and how many fish between purchases

### Discord Webhook Setup

1. **Create Webhook**: In your Discord server → Channel Settings → Integrations → Webhooks
2. **Copy URL**: Paste the webhook URL in Features › Discord and press **Test**
3. **Configure Alerts**:
   - 🍎 Devil Fruit Catch Alerts - Notifications when you catch a fruit while fishing
   - 🌟 Devil Fruit Spawn Alerts - Notifications when fruits spawn in the world (with exact fruit name). Turning this on makes the bot read the drop message area between casts
   - 🐟 Fish Progress Updates - Regular progress reports
   - 🛒 Auto Purchase Alerts - Bait purchase confirmations
   - 🛟 Recovery Alerts - When the watchdog restarts or gives up
4. **Set Interval**: Choose how often to send fish progress updates

### Hotkeys

- **F1**: Start/Pause fishing loop
- **F2**: Edit areas on screen
- **F3**: Emergency stop and exit
- **F4**: Hide/show the HUD
- **Note**: Rebindable in Settings

### Performance Tips

- **Long Sessions**: Close the panel; the HUD and tray icon keep running
- **Webhook Monitoring**: Use Discord alerts for fruit spawns and catches instead of watching the screen
- **OCR Optimization**: Make the drop message area cover the whole popup for better fruit detection
- **Spawn Detection**: The bot detects all 33 GPO devil fruits automatically using fuzzy matching

---

## 🔧 Troubleshooting

### Runtime Issues

- **HUD not showing**: It only appears while Roblox is running, visible, and focused. Press F4 if you hid it
- **Hotkeys not working**: Another app may own the key. Rebind in Settings › Hotkeys
- **Fish detection failing**: Open Setup › Fishing bar area. If the match score is low, raise Settings › Color tolerance or redraw the area tighter around the bar
- **Devil fruit not detected**: Setup › Drop message area › Read now shows exactly what the OCR sees
- **Fruit spawns not detected**: Ensure the drop message area covers the spawn popup
- **Auto-purchase failing**: Verify the Confirm and Quantity points are set and you are standing next to the bait barrel
- **Logs**: Settings › Data folder › `logs/`
- **(Linux) Screen-share dialog reappears every launch**: expected — KDE doesn't allow persisting the input-control grant, only the screen-capture one

### Devil Fruit Issues

- **Fruits not being stored**: Check if OCR detected the fruit in the Dashboard activity feed
- **Storage sequence running without fruit**: Ensure the drop message area only covers the popup
- **Wrong inventory slot**: Verify the fruit keys match your hotbar
- **Rod not switching back**: Check the rod key (slot `1`) and bait point configuration

---

## 📁 Project Structure

```
src-tauri/src/
├── core/platform/       # OS traits (window, capture, input, OCR) + Windows/Linux implementations
├── core/vision.rs       # Bar / fish / marker detection
├── core/fruit.rs        # Drop and spawn text matching
├── core/controller.rs   # Reel controller
├── bot/                 # State machine, actions, watchdog, session stats
├── config.rs            # Settings, presets, v3 import
├── webhook.rs           # Discord embeds
└── commands.rs          # Tauri command surface
src/
├── windows/             # Hud, Panel, Overlay
├── pages/               # Dashboard, Setup, Features, Settings
└── components/          # Shared UI pieces
```

Everything OS-specific sits behind traits in `src-tauri/src/core/platform/mod.rs`, so other capture or OCR backends can be added without touching the bot logic. Fruit names and drop phrases live in settings under `lexicon`, so a game update does not need a rebuild.

## License

MIT. See [LICENSE](LICENSE).
