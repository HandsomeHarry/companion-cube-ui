# Companion Cube

**Companion Cube** is a comprehensive ADHD productivity assistant that combines activity monitoring, AI-powered insights, and contextual interventions to help users maintain focus and productivity. The system features both a CLI interface and a modern desktop GUI built with Tauri and React.

## 🎯 Features

### Core Functionality
- **Activity Monitoring**: Integrates with ActivityWatch to track computer usage patterns
- **AI-Powered Analysis**: Uses Ollama LLM for intelligent state analysis and personalized responses
- **Multiple Modes**: Adaptive behavior based on user context (Ghost, Chill, Study, Coach)
- **Smart Interventions**: Context-aware notifications and productivity nudges
- **Real-time Dashboard**: Modern React-based GUI with live activity charts and summaries

### Key Components
- **Hourly Summaries**: AI-generated insights about recent activity patterns
- **Daily Reports**: Comprehensive analysis of productivity and focus sessions
- **Focus Scoring**: Quantitative metrics for productivity assessment
- **Dark/Light Mode**: Fully themed interface with custom scrollbars
- **Responsive Design**: Adapts to different screen sizes with flexible card layouts

## 🏗️ Architecture

### Technology Stack
- **Backend**: Rust with Tauri for cross-platform desktop application
- **Frontend**: React with TypeScript for modern UI components
- **Styling**: Tailwind CSS with custom theme system
- **State Management**: React hooks with context-aware theming
- **Build System**: Vite for fast development and optimized builds

### Project Structure
```
companion-cube/
├── src/                    # Rust source code
│   ├── main.rs            # CLI entry point
│   ├── lib.rs             # Library interface
│   ├── companion_cube.rs  # Core logic and LLM integration
│   ├── activitywatch_client.rs  # ActivityWatch API client
│   ├── event_processor.rs # Data analysis and pattern detection
│   └── components/        # React UI components
│       ├── App.tsx        # Main application component
│       ├── MainContent.tsx # Dashboard content
│       ├── Sidebar.tsx    # Navigation and mode selection
│       ├── Terminal.tsx   # Log display
│       └── ActivityChart.tsx # Data visualization
├── src-tauri/             # Tauri desktop app backend
│   ├── src/lib.rs         # Tauri commands and GUI integration
│   └── tauri.conf.json    # Desktop app configuration
├── data/                  # Runtime data storage
│   ├── config.json        # User preferences and context
│   ├── hourly_summary.txt # Activity summaries
│   └── daily_summary.txt  # Daily reports
└── utils/                 # Shared utilities
    └── modes.ts           # Mode-based styling and display logic
```

## 🚀 Getting Started

### Prerequisites
1. **ActivityWatch**: Download and install from https://activitywatch.net/
   - Must be running on `localhost:5600`
   - Required for activity monitoring
   
2. **Ollama** (optional but recommended): Install from https://ollama.ai
   - Used for AI-powered analysis and interventions
   - Run `ollama serve` to start the service
   - Pull desired model: `ollama pull mistral`

3. **Rust**: Install from https://rustup.rs/
4. **Node.js**: Install from https://nodejs.org/

### Installation
```bash
# Clone the repository
git clone <repository-url>
cd companion-cube

# Install frontend dependencies
npm install

# Build the Rust backend
cargo build --release

# Run the desktop application
npm run tauri dev
```

### CLI Usage
```bash
# Run with default settings (coach mode)
cargo run

# Run with specific mode and interval
cargo run -- --mode study_buddy --interval 30

# Test mode (single check)
cargo run -- --test

# Verbose mode with detailed LLM analysis
cargo run -- --verbose

# Test connections to ActivityWatch and Ollama
cargo run -- --test-connections

# Generate daily summary
cargo run -- --daily-summary
```

## 🎮 Usage Modes

### Available Modes
- **Ghost Mode**: Minimal interventions, monitoring only
- **Chill Mode**: Relaxed productivity assistance
- **Study Mode**: Frequent check-ins and study support
- **Coach Mode**: Balanced interventions for focus and productivity

### Dashboard Features
- **Hourly State**: Current productivity state with AI-generated insights
- **Activity Chart**: Visual representation of work vs. distraction time
- **Daily Summary**: Comprehensive analysis of the day's activities
- **Personal Context**: User-defined context passed to AI for personalized responses

## 🔧 Configuration

### User Context
The system supports personalized responses through user-defined context stored in `data/config.json`:

```json
{
  "user_context": "Your personal context, goals, and preferences here"
}
```

### Intervention Settings
- **Cooldown Timers**: Configurable intervals between interventions
- **Focus Threshold**: Minimum time in same app to consider focus session
- **Context Switching**: Detects rapid app switching for productivity nudges

## 🔄 Data Flow

1. **Data Collection**: ActivityWatch client fetches activity data across multiple timeframes
2. **State Analysis**: LLM analyzes raw activity data to determine user state
3. **Intervention Decision**: System decides whether to intervene based on state and cooldown timers
4. **Response Generation**: Creates contextual, ADHD-friendly prompts and responses
5. **Logging & Summaries**: Tracks activity logs and generates periodic reports

## 🎨 Customization

### Theming
The application supports comprehensive theming with:
- **Dark/Light Mode**: System-wide theme switching
- **Mode-Based Colors**: Each productivity mode has distinct colors
- **Custom Scrollbars**: Themed scrollbars that adapt to dark/light mode
- **Responsive Design**: Flexible card layouts that expand with content

### Extending Functionality
- **Add New Modes**: Extend the mode system in `src/utils/modes.ts`
- **Custom Prompts**: Modify LLM prompts in `src/event_processor.rs`
- **UI Components**: Add new React components in `src/components/`

## 📊 Data Management

### Storage
- **Activity Logs**: 5-minute summaries (last 7 days)
- **Daily Summaries**: Daily reports (last 30 days)
- **User Config**: Persistent user preferences and context

### Privacy
- **Local Storage**: All data stored locally, no cloud services
- **Configurable Tracking**: Users control what data is collected
- **No External APIs**: Optional Ollama integration runs locally

## 🛠️ Development

### Building
```bash
# Development build
cargo build
npm run dev

# Production build
cargo build --release
npm run build

# Run tests
cargo test
npm test
```

### Code Quality
- **Rust**: Uses clippy for linting and cargo fmt for formatting
- **TypeScript**: Strict type checking with comprehensive interfaces
- **React**: Modern functional components with hooks
- **Tailwind**: Utility-first CSS with custom theme system

## 🔍 Troubleshooting

### Common Issues
1. **ActivityWatch Not Connected**: Ensure ActivityWatch is running on port 5600
2. **Ollama Not Available**: Install and start Ollama service, or use fallback mode
3. **Build Errors**: Check Rust and Node.js versions, run `cargo clean` and `npm ci`

### Debug Mode
Use `--verbose` flag for detailed logging:
```bash
cargo run -- --verbose --test
```

## 📈 Performance

### Optimization Features
- **Efficient Data Processing**: Minimal memory usage with streaming data
- **Caching**: Intelligent caching of ActivityWatch data
- **Lazy Loading**: Components load only when needed
- **Build Optimization**: Tree-shaking and code splitting in production

### System Requirements
- **Memory**: 50-100MB typical usage
- **CPU**: Low background usage, periodic analysis spikes
- **Storage**: <10MB for application, variable for activity logs

## 🤝 Contributing

### Development Setup
1. Fork the repository
2. Create a feature branch
3. Follow existing code patterns and naming conventions
4. Test thoroughly with both CLI and GUI modes
5. Submit pull request with detailed description

### Code Style
- **Rust**: Follow standard Rust conventions with rustfmt
- **TypeScript**: Use functional components with TypeScript interfaces
- **Commits**: Use conventional commit messages

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## 🙏 Acknowledgments

- **ActivityWatch**: For providing the activity monitoring foundation
- **Ollama**: For local LLM integration capabilities
- **Tauri**: For enabling cross-platform desktop development
- **React**: For the modern UI framework
- **Tailwind CSS**: For the utility-first styling system

---

**Companion Cube** - Your ADHD-friendly productivity companion 🧠✨