# Companion Cube

**Companion Cube** is a Tauri-based ADHD productivity assistant that monitors user activity through ActivityWatch and provides supportive interventions using AI (Ollama). The application runs as a native desktop application with system tray functionality, tracking computer usage patterns and offering contextual assistance without judgment.

## Features

<img width="1247" height="830" alt="image" src="https://github.com/user-attachments/assets/009ed6ab-1492-4abc-86c7-abb8acfe4b38" />

### Core Functionality
- **Real-time Activity Monitoring**: Integrates with ActivityWatch for comprehensive computer usage tracking
- **AI-Powered Analysis**: Uses local Ollama LLM for intelligent activity analysis and personalized insights
- **Adaptive Modes**: Four distinct productivity modes (Ghost, Chill, Study, Coach) with mode-specific behavior
- **System Tray Integration**: Runs in background with minimal interface disruption
- **Smart Categorization**: Automatic app categorization with productivity scoring
- **AFK Filtering**: Excludes idle periods for accurate productivity analysis
(this part is AI generated, maybe I'll update it later...)

### Dashboard Features
- **Activity Classification**: Real-time breakdown of productive vs. distraction time
- **Focus Scoring**: Quantitative productivity metrics with visual indicators
- **Hourly/Daily Summaries**: AI-generated insights about activity patterns
- **Mode-Specific Contexts**: Personalized prompts for study topics or tasks
- **Activity History**: Comprehensive charts and statistics
- **Dark/Light Theme**: Fully themed interface with Segoe UI typography

## Architecture

### Technology Stack
- **Backend**: Rust with Tauri for cross-platform desktop application
- **Frontend**: React 18 with TypeScript for modern UI components
- **Styling**: Tailwind CSS with custom design system
- **Charts**: Chart.js and Recharts for data visualization
- **State Management**: React hooks with Tauri state management
- **Build System**: Vite for fast development and optimized builds

### Project Structure
```
companion-cube/
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── lib.rs          # Main application logic (3800+ lines)
│   │   ├── main.rs         # Entry point
│   │   └── modules/        # Modular components
│   │       ├── activity_watch.rs      # ActivityWatch API integration
│   │       ├── ai_integration.rs      # Ollama LLM integration
│   │       ├── app_state.rs          # Application state management
│   │       ├── default_categories.rs  # App categorization system
│   │       ├── enhanced_processor.rs  # Data processing pipeline
│   │       ├── mode_handlers.rs      # Mode-specific logic
│   │       ├── pattern_analyzer.rs   # Productivity pattern analysis
│   │       └── tauri_commands.rs     # Frontend-backend API
│   ├── data/               # Runtime data (gitignored)
│   │   ├── config.json     # User settings and context
│   │   ├── hourly_summary.txt
│   │   ├── daily_summary.json
│   │   └── coach_todos.json
│   └── icons/              # System tray and app icons
├── src/                    # React frontend
│   ├── App.tsx             # Main app with event listeners
│   ├── components/
│   │   ├── MainContent.tsx # Dashboard with cards and summaries
│   │   ├── Sidebar.tsx     # Mode selection and navigation
│   │   ├── ActivityChart.tsx # Productivity visualization
│   │   ├── History.tsx     # Activity history and charts
│   │   ├── Settings.tsx    # Configuration interface
│   │   └── Terminal.tsx    # Debug log display
│   └── utils/
│       ├── designSystem.ts # Typography and spacing constants
│       └── theme.ts        # Mode-based theming system
└── examples/               # Sample ActivityWatch data
```

## Getting Started

### Prerequisites
1. **ActivityWatch**: Download and install from [activitywatch.net](https://activitywatch.net/)
   - Must be running on `localhost:5600`
   - Required for activity monitoring
   
2. **Ollama** (recommended): Install from [ollama.ai](https://ollama.ai)
   - Used for AI-powered analysis
   - Run `ollama serve` to start the service
   - Pull a model: `ollama pull mistral` or `ollama pull llama3.2`

3. **Development Tools**:
   - **Rust**: Install from [rustup.rs](https://rustup.rs/)
   - **Node.js**: Install from [nodejs.org](https://nodejs.org/)

### Installation & Development

```bash
# Clone the repository
git clone <repository-url>
cd companion-cube

# Install frontend dependencies
npm install

# Frontend development (Terminal 1)
npm run dev

# Tauri application (Terminal 2)
cd src-tauri
cargo run
```

### Production Build

```bash
# Build frontend
npm run build

# Build and run production Tauri app
cd src-tauri
cargo build --release
cargo run --release
```

## Usage Modes

The application provides four distinct productivity modes, each with unique timing and behavior:

### Available Modes
- **Ghost Mode**: Minimal interventions, hourly summaries only
- **Chill Mode**: Relaxed monitoring with hourly check-ins
- **Study Mode**: Frequent 5-minute analysis with study-specific context
- **Coach Mode**: Balanced 15-minute intervals with todo list generation

### Mode-Specific Features
- **Study Mode**: 
  - Immediate summary generation on mode switch
  - Study topic context integration
  - 5-minute state analysis
- **Coach Mode**: 
  - Todo list generation and management
  - Task-focused context prompts
  - 15-minute productivity summaries

## Key Features

### Activity Analysis Pipeline
1. **Data Collection**: ActivityWatch query API with server-side filtering
2. **AFK Filtering**: Automatic exclusion of idle periods using `filter_period_intersect`
3. **App Categorization**: Local classification system with productivity scoring
4. **State Detection**: Five states - productive, moderate, chilling, unproductive, afk
5. **LLM Analysis**: Comprehensive prompt generation with user context integration

### Smart Categorization
The system uses LLMs for extensive app categorization
- **Event Merging**: Consecutive events merged for better analysis
- **Multi-timeframe Analysis**: Hour, day, and week-level insights
- **Context Switching Detection**: Rapid app switching analysis

## Configuration

### User Context
Personalize AI responses through mode-specific contexts

### App Categories
Customize app categorization through the Settings interface:
- Productivity scoring (0-100)
- Category assignment
- Subcategory classification
- Bulk category updates

## Development

### Commands
```bash
# Development
npm run dev              # Frontend dev server
cd src-tauri && cargo run  # Tauri app with hot reload

# Building
npm run build           # Production frontend build
cargo build --release  # Production Tauri build

# Testing
cargo test             # Rust tests
cargo clippy           # Rust linting
cargo fmt --check      # Rust formatting check
```

### System Requirements
- **Memory**: 50-100MB typical usage
- **Storage**: <10MB for application, variable for activity logs, might be 10GB or more if you use a bigger model

## Troubleshooting

### Common Issues
1. **ActivityWatch Not Connected**: 
   - Ensure ActivityWatch is running on port 5600
   - Check firewall settings
   - Verify bucket creation

2. **Ollama Not Available**: 
   - Install and start Ollama service
   - Pull required model
   - Application falls back to time-based summaries

3. **Build Errors**: 
   - Check Rust and Node.js versions
   - Run `cargo clean` and `npm ci`
   - Ensure all dependencies are installed

### Debug Mode
Enable comprehensive logging:
```bash
RUST_LOG=debug cargo run
```

## Data Privacy

### Local-First Design
- **Local AI**: Ollama integration runs locally
- **User Control**: Complete control over data collection and analysis
- **No External APIs**: No data transmitted outside your machine

### Data Storage
- **Activity Logs**: Processed locally, stored in `data/` directory
- **User Config**: Preferences stored in local JSON files
- **Summaries**: AI-generated insights stored locally

## Contributing

### Development Setup
1. Fork the repository
2. Create a feature branch
3. Follow existing code patterns and naming conventions
4. Test with both ActivityWatch and Ollama integrations
5. Submit pull request with detailed description

### Code Style
- **Rust**: Follow standard conventions with `rustfmt`
- **TypeScript**: Use functional components with proper typing
- **Commits**: Use conventional commit messages
- **Documentation**: Update README for significant changes

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
