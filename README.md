# Companion Cube

Companion cube determines user focus status using data collected from ActivityWatch and a local LLM model and nudges the user when they're drifting off, whether it be mindless YouTube browsing or Wikipedia rabbit-hole exploring. It is mainly designed for people with ADHD who have trouble focusing on tasks.

<img width="1247" height="830" alt="image" src="https://github.com/user-attachments/assets/ea74cf29-b5a4-46e6-9a25-46883ca51c54" />

## Getting Started

### Installation
1. Get [ActivityWatch](https://activitywatch.net/) and follow the setup.
   
2. Install [Ollama](https://ollama.ai) and follow the setup. No need to download the model just yet.

3. Download Companion Cube from [Releases](https://github.com/HandsomeHarry/companion-cube-ui/releases), keep it in a separate folder.



## Usage Modes

There are 4 modes:
- **Ghost Mode**: No interventions, optional hourly summaries
- **Chill Mode**: Hourly checkins and summaries
- **Study Mode**: 5-minute checkins, with study-specific context
- **Coach Mode**: 15-minute checkins, with todo list generation and context

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
```

### System Requirements
- **Memory**: 50-100MB typical usage
- **Storage**: <10MB for application, variable for activity logs, might be 10GB or more if you use a large model model. Gemma 3 and Mistral is recommended

## Data Privacy
It doesn't need internet to function. Everything is kept on your computer.

## License

This project is licensed under the MIT License.
