# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**Companion Cube** is a Tauri-based ADHD productivity assistant that monitors user activity through ActivityWatch and provides supportive interventions using AI (Ollama). The application runs as a native desktop application with system tray functionality, tracking computer usage patterns and offering contextual assistance without judgment.

## Core Architecture

The application is built with:
- **Rust backend** (`src-tauri/src/lib.rs`): Contains ALL business logic, activity monitoring, AI integration, and system tray management
- **React frontend** (`src/`): Minimal UI for dashboard display and settings
- **System tray** as the primary interface - the app runs in background with occasional notifications

### Critical Design Decisions

1. **AFK Filtering**: All activity analysis excludes idle periods using `get_active_window_events()` which cross-references window events with AFK buckets
2. **State Detection**: Five states: `productive` (focused work), `moderate` (decent productivity), `chilling` (relaxed/casual), `unproductive` (distracted), `afk` (user away)
3. **Mode-Specific Timing**: Ghost/Chill (hourly), Study (5-min), Coach (15-min) - enforced by background timer checking every minute
4. **AI Prompt Unification**: All modes use `generate_ai_summary()` for consistent analysis quality

## Development Commands

```bash
# Frontend build (required before running Tauri)
npm run build

# Run the Tauri application (launches GUI with system tray)
cd src-tauri && cargo run

# Development mode with hot reload
npm run dev  # Terminal 1: Frontend dev server
cd src-tauri && cargo run  # Terminal 2: Tauri app

# Production build
npm run build && cd src-tauri && cargo build --release

# Run tests
cargo test
cargo clippy
cargo fmt --check
```

## Key Functions and Data Flow

### Activity Monitoring Pipeline
1. **ActivityWatch Query API** - Uses server-side query language for efficient data retrieval:
   - `get_active_window_events()` - Now uses query API with `filter_period_intersect` for AFK filtering
   - `get_multi_timeframe_data_active()` - Leverages query transforms for better performance
   - `execute_query()` - Central method for running ActivityWatch queries
2. `EventProcessor::prepare_raw_data_for_llm()` - Transforms events into structured timeline and context switches
3. `EventProcessor::create_state_analysis_prompt()` - Generates comprehensive prompt with user context
4. `call_ollama_api()` - Sends to LLM with 0.3 temperature for consistent responses
5. `parse_llm_response()` - Robust JSON extraction handling malformed responses

### ActivityWatch Query Improvements (2024)
The codebase now uses ActivityWatch's query API instead of manual event filtering:
- **Server-side filtering**: AFK periods filtered using `filter_period_intersect`
- **Event merging**: Consecutive events merged with `merge_events_by_keys`
- **Better performance**: Reduced data transfer and processing overhead
- **New methods**: `get_activity_stats()`, `get_categorized_events()`, `get_app_usage_by_category()`

### Mode-Specific Behavior

**Study Mode** (Special handling):
- Uses `generate_study_focused_summary()` with study-specific context
- Immediate summary generation on mode switch via `set_mode()` command
- Context: "Currently studying: [topic]. Focus on study-related activities..."

**Coach Mode**:
- Generates todo lists via `generate_coach_todo_list()` in addition to activity summary
- Todos saved to `data/coach_todos.json` for frontend display

### Critical State Management
- `AppState::latest_hourly_summary`: In-memory cache for immediate UI updates
- Background timer uses cloned Arc references to prevent lifetime issues
- Mode switches trigger immediate analysis for study mode

## Important Implementation Details

### Frontend-Backend Communication
- Events: `hourly_summary_updated`, `mode_changed`, `show_notification`
- Commands: `get_hourly_summary`, `generate_hourly_summary`, `set_mode`, `classify_activities`
- Frontend polls every 30 seconds as fallback for missed events

### Error Handling Patterns
- HTTP clients use `OnceLock` for singleton pattern with connection pooling
- Graceful degradation: When Ollama unavailable, uses `generate_time_based_summary()`
- All mode handlers return `Result<(), String>` for consistent error propagation

### Performance Optimizations
- `get_multi_timeframe_data_active()` fetches data once, filters locally for different timeframes
- Bucket information cached to avoid repeated `get_buckets()` calls
- Study mode context injection avoids re-analyzing when context is known

## Common Pitfalls and Solutions

1. **UTF-8 String Slicing**: Use `chars().take(n).collect()` instead of byte slicing for titles
2. **Event Emission**: Use `app.emit()` not `emit_all` (doesn't exist in this Tauri version)
3. **Prompt Length**: Keep prompts concise - verbose prompts cause JSON parsing failures
4. **Mode Switching**: Study mode requires immediate generation, other modes wait for timer

## Testing Approach

### Manual Testing Flow
1. Ensure ActivityWatch is running (localhost:5600)
2. Start Ollama if available (`ollama serve`)
3. Run app: `npm run build && cd src-tauri && cargo run`
4. Test mode switching via system tray
5. Verify summaries update in GUI
6. Check `data/` directory for generated files

### Key Test Scenarios
- Mode switch to study → Immediate summary with study context
- Manual generation → Uses general context regardless of mode
- Ollama offline → Fallback summaries work
- Long idle period → AFK filtering excludes from analysis

## File Organization

```
src-tauri/
├── src/
│   ├── lib.rs          # 3800+ lines - ALL backend logic
│   └── main.rs         # 2 lines - just calls lib::run()
├── data/               # Runtime data (gitignored)
│   ├── config.json     # User settings and context
│   ├── hourly_summary.txt
│   └── coach_todos.json
└── icons/              # System tray icons

src/                    # React frontend
├── App.tsx             # Main app with event listeners
├── components/
│   ├── MainContent.tsx # Dashboard cards and summaries
│   ├── Sidebar.tsx     # Mode selection
│   └── Terminal.tsx    # Log display
└── utils/
    └── modes.ts        # Mode colors and display names
```

## Debugging Tips

- Enable debug logs: `RUST_LOG=debug cargo run`
- Check browser console for frontend event reception
- Verify ActivityWatch buckets: `curl http://localhost:5600/api/0/buckets/`
- Test Ollama: `curl http://localhost:11434/api/generate -d '{"model":"mistral","prompt":"test"}'`
- Mode timing issues: Check `should_run_summary()` and `last_summary_time` logic