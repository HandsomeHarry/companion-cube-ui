# Companion Cube

**v0.2.2**

Companion Cube helps you work *with* your brain, not against it. Instead of blocking distractions, it gently saves them for later—because that rabbit hole about mechanical keyboards might actually be worth exploring, just not right now.

<img width="1143" height="739" alt="image" src="https://github.com/user-attachments/assets/544c8f5f-4fbd-418a-bd19-c533c0b8fc66" />

## Getting Started

### Installation
1. Install [Ollama](https://ollama.ai) and follow the setup, then pull the default model:
   ```bash
   ollama pull gemma4:e4b
   ```

2. Download Companion Cube from [Releases](https://github.com/HandsomeHarry/companion-cube-ui/releases), keep it in a separate folder.

3. Run the downloaded binary. A tray icon appears — pick **Open Dashboard** (or browse to `http://127.0.0.1:7431`).

To use a different model or LLM endpoint, copy `.env.example` to `.env` next to the binary and adjust `CCUBE_LLM_MODEL` / `CCUBE_LLM_URL`.



## Features

### 📜 History
Auto-organized timeline of your activities. No manual tracking.

- Activities grouped by focus sessions
- Drag items between groups to correct misclassifications
- Your corrections teach the AI to understand *your* patterns
<img width="1459" height="1005" alt="image" src="https://github.com/user-attachments/assets/71881d73-5b29-4d83-9cec-31abcf734a99" />

### 🏦 Vault
Where distractions go to become future inspiration.

- One click to save "for later"
- Search and favorite saved ideas
- Gentle reminders for stale items ("Still interested in GPU benchmarks?")
<img width="1026" height="904" alt="image" src="https://github.com/user-attachments/assets/3e9a57b6-e662-45aa-8cdf-1328db789de1" />


### 🎵 Rhythm
Spotify Wrapped, but for your focus patterns.

- **Best Focus Window**: Discover when you naturally focus best
- **Focus Fingerprint**: Your unique deep-work app combinations
- **Drift Patterns**: Where your rabbit holes usually start (no judgment)
- **Heatmaps**: Visualize your weekly patterns at a glance

<img width="920" height="915" alt="image" src="https://github.com/user-attachments/assets/24a404c9-4bf7-4fd5-8e39-c251e13507d9" />


### 💡 Aura
*Planned — not in the current release.*

Your room reflects your focus state.

- Connects to existing smart lights (HomeKit, Home Assistant)
- Warm light when focused, cooler when drifting
- 30-second gradual transitions—no flashing, no alarms
- Peripheral awareness without demanding attention
<img width="619" height="641" alt="image" src="https://github.com/user-attachments/assets/92fddbbd-bd9d-47ac-be11-fc74dcfc7181" />


### 🔔 Nudges
Gentle, not naggy.

- "You're watching keyboard reviews—save for later?"
- Snooze with friction (hold 3 seconds to add intentionality)
- Snooze too often? Suggests a break instead of guilt
<img width="729" height="708" alt="image" src="https://github.com/user-attachments/assets/4a57f644-aa1b-41e3-8f33-921063d588f7" />

---
## How It Works

```
┌──────────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  Native Capture      │────▶│   Local LLM     │────▶│   Nudge User    │
│  (app focus, title,  │     │  (via Ollama)   │     │   or Stay Silent│
│   URL, OCR, idle)    │     └────────┬────────┘     └─────────────────┘
└──────────────────────┘               │
                                       ▼
                              ┌─────────────────┐
                              │  Learning Loop  │
                              │ (prompt memory) │
                              └─────────────────┘
                                       ▲
                                       │
                              ┌─────────────────┐
                              │ User Corrections│
                              │ (drag/edit)     │
                              └─────────────────┘
```
---

## Development

### Commands
```bash
# Development
npm install
npm run dev                 # Frontend dev server (http://localhost:1420)
cargo run -p ccube-daemon   # Daemon: capture, scheduler, HTTP API + UI on 127.0.0.1:7431
cargo run -p ccube-cli      # `ccube` CLI (detect, briefing, daemon control, ...)

# Building
npm run build           # Build frontend first — the daemon embeds build/ at compile time
cargo build --release   # Production build of daemon + CLI

# macOS app bundle (Companion Cube.app in dist/ — branded notifications,
# menu-bar identity; ad-hoc signed)
./scripts/make-bundle.sh
```

Note: the daemon's `include_dir!` embeds the SvelteKit `build/` output, so run `npm run build` before any `cargo build` of `ccube-daemon`.

### System Requirements
- **Memory**: 50-100MB typical usage
- **Storage**: <10MB for the application, variable for activity logs. The LLM itself can take several GB depending on the model; Gemma 4 E4B (the default) is recommended.

## Data Privacy
It doesn't need internet to function. Everything is kept on your computer.

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=HandsomeHarry/companion-cube-ui&type=date&legend=top-left)](https://www.star-history.com/#HandsomeHarry/companion-cube-ui&type=date&legend=top-left)
Star if you find this useful!

## License

This project is licensed under the MIT License.
