# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Companion Cube** is a pure Tauri-based ADHD productivity assistant that monitors user activity through ActivityWatch and provides supportive interventions using AI. The application runs as a native desktop application with system tray functionality, tracking computer usage patterns and offering contextual assistance without judgment.

## Core Architecture

**IMPORTANT: Companion Cube is a PURE TAURI APPLICATION** - not a hybrid web app or CLI tool. It consists of:
- **Rust backend** with all business logic in `src-tauri/`
- **Minimal HTML interface** for the optional dashboard window  
- **System tray menu** as the primary user interface
- **No React, Vite, Node.js, or npm dependencies**

### Main Components

- **`src-tauri/src/main.rs`**: Tauri application entry point that calls `app_lib::run()`
- **`src-tauri/src/lib.rs`**: Core Tauri application with system tray, window management, and all business logic
- **`src-tauri/src/index.html`**: Minimal HTML dashboard interface (self-contained)
- **Root `cargo.toml`**: Workspace configuration that runs the Tauri application via `cargo run`

### System Tray Interface (Primary UI)

The application primarily operates through a **system tray menu**:
- **Right-click tray icon**: Access all features
- **Mode Selection**: Ghost, Coach, Study Buddy, Chill modes
- **Dashboard**: Optional window with status overview
- **Connection Check**: Test ActivityWatch and Ollama connectivity  
- **Quit**: Exit application

**Window Behavior:**
- Starts hidden on launch (`visible: false`)
- Closes to tray instead of exiting (`closable: false`)
- Double-click tray icon to show dashboard window

### Key Data Flow

1. **Data Collection**: ActivityWatch client fetches window/AFK events across multiple timeframes
2. **State Analysis**: LLM analyzes activity data to determine user state (flow/working/needs_nudge/afk)  
3. **Intervention Decision**: Based on state, mode, and cooldown timers
4. **Response Generation**: Creates contextual ADHD-friendly prompts and responses
5. **Logging & Summaries**: Tracks activity logs, generates summaries and reports

### Companion Modes

- **Ghost**: Monitoring only, no interventions
- **Coach**: Balanced interventions for focus and productivity
- **Study Buddy**: Frequent check-ins and support
- **Chill**: Minimal interventions, relaxed monitoring

## Prerequisites

**Required Dependencies:**
- **ActivityWatch**: Download and install from https://activitywatch.net/
  - Must be running on localhost:5600
  - Required for activity monitoring - application cannot start without it
- **Ollama** (optional but recommended): Install from https://ollama.ai
  - Used for AI-powered state analysis and interventions
  - Application will use fallback responses if not available
  - Run `ollama serve` to start the service
  - Pull desired model: `ollama pull mistral`

## Development Commands

### Building and Running
```bash
# Build the Tauri application
cargo build

# Run the Tauri GUI application (primary command)
cargo run

# Run from src-tauri directory (alternative)
cd src-tauri && cargo run
```

**IMPORTANT:** This is NOT a CLI application. `cargo run` launches the Tauri GUI app with system tray functionality.

### Development Tools
```bash
# Run tests
cargo test

# Check formatting
cargo fmt --check

# Run clippy lints  
cargo clippy

# Build for release
cargo build --release
```

## Key Configuration Points

### Tauri Configuration
- **Frontend**: Self-contained HTML in `src-tauri/src/index.html`
- **Window Settings**: Starts hidden, closes to tray
- **System Tray**: Full menu with mode switching and controls
- **Icons**: Located in `src-tauri/icons/`

### LLM Integration
- **Default Model**: `mistral` (hardcoded in Tauri commands)
- **Ollama URL**: `http://localhost:11434` (hardcoded in config)
- **Timeout Settings**: 10s for interventions, 30s for analysis
- **Fallback Responses**: Available when Ollama is unavailable

### ActivityWatch Integration
- **Default Port**: 5600
- **Required Buckets**: `aw-watcher-window_*`, `aw-watcher-afk_*`
- **Retry Logic**: 3 attempts with exponential backoff
- **Data Collection**: Multi-timeframe data (5min, 30min, 1hr, today)

### Intervention System
- **Cooldown Timers**: flow (45min), working (15min), needs_nudge (5min), afk (0min)
- **Focus Threshold**: 15+ minutes in same app considered focus session
- **Context Switching**: 5+ switches triggers "needs_nudge"

## File Structure and Data Management

### Application Structure
```
/mnt/e/cc/
├── cargo.toml              # Workspace config - runs Tauri app
├── src-tauri/              # Tauri application directory
│   ├── src/
│   │   ├── main.rs         # Entry point
│   │   ├── lib.rs          # Core logic, system tray, Tauri commands
│   │   └── index.html      # Minimal HTML dashboard
│   ├── Cargo.toml          # Tauri dependencies
│   ├── tauri.conf.json     # Tauri configuration
│   └── icons/              # Application icons
└── data/                   # Runtime data directory
    ├── config.json         # User configuration
    ├── hourly_summary.txt  # Generated summaries
    └── daily_summary.txt   # Daily reports
```

### Data Directory (`src-tauri/data/`)
- **`config.json`**: User context and settings
- **`hourly_summary.txt`**: Hourly activity summaries
- **`daily_summary.txt`**: Daily summary reports

## Tauri Commands (Available to Frontend)

The HTML dashboard can call these Tauri commands:
- **`check_connections`**: Test ActivityWatch and Ollama connectivity
- **`get_current_mode`**: Get current companion mode
- **`set_mode`**: Change companion mode
- **`get_hourly_summary`**: Get current activity summary
- **`generate_hourly_summary`**: Force generate new summary
- **`generate_daily_summary_command`**: Generate daily report
- **`load_user_config`**: Load user configuration
- **`save_user_config`**: Save user configuration
- **`show_connection_help`**: Get help text for connectivity issues

## Development Patterns

### Error Handling
- Uses `anyhow::Result` for error propagation
- Graceful degradation when external services unavailable
- Extensive logging with Tauri's logging plugin

### State Management
- App state managed through Tauri's state management
- Mode changes propagated via Tauri events
- Intervention cooldowns stored in memory

### System Tray Integration
- **Menu Creation**: Dynamic menu based on current mode
- **Event Handling**: Mode switching, window showing, quit functionality
- **Status Updates**: Real-time menu updates when mode changes

## Testing and User Experience

### Primary Interaction Flow
1. **Launch**: `cargo run` starts app in system tray
2. **Mode Selection**: Right-click tray → select mode
3. **Dashboard**: Double-click tray or select "Dashboard" 
4. **Monitoring**: App runs in background monitoring activity
5. **Interventions**: Contextual notifications based on mode and activity

### Connection Testing
The system tray menu includes "Check Ollama and AW" option to test connectivity.

### Log Levels
Set log level via environment variable:
```bash
RUST_LOG=debug cargo run
```

## Dependencies of Note

### Tauri-Specific
- **tauri**: Core Tauri framework with tray-icon feature
- **tauri-plugin-log**: Logging plugin for Tauri applications

### Business Logic  
- **reqwest**: HTTP client for ActivityWatch and Ollama APIs
- **chrono**: Date/time handling with timezone support
- **serde**: JSON serialization for data structures and API responses
- **anyhow**: Error handling and propagation
- **indexmap**: Ordered maps for activity summaries
- **url**: URL parsing for web domain extraction

## Important Implementation Notes

- **System Tray**: Fully implemented with Tauri's tray-icon feature
- **Window Management**: Hides to tray instead of closing, prevents accidental exit
- **No CLI Interface**: This is purely a GUI application, not a command-line tool
- **Self-Contained**: No external web server or Node.js dependencies required
- **Native Performance**: Pure Rust backend with minimal HTML frontend
- **Cross-Platform**: Tauri applications work on Windows, macOS, and Linux

## Build and Distribution

### Development Build
```bash
cargo run  # Starts in development mode with hot reload
```

### Release Build
```bash
cargo build --release  # Creates optimized binary
```

### Tauri Bundle (Future)
```bash
cd src-tauri && cargo tauri build  # Creates platform-specific installers
```

## Known Limitations

- **Web Event Processing**: Disabled due to timing issues with ActivityWatch web watchers
- **Advanced CLI Features**: Removed in favor of GUI/tray interface
- **Platform-Specific**: Some system tray behaviors may vary across platforms