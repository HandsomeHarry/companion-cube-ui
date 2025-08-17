# Project: Companion Cube

## Project Overview

This project, "Companion Cube," is a cross-platform desktop application designed as a productivity assistant for individuals with ADHD. It leverages the Tauri framework, with a Rust backend and a React/TypeScript frontend.

The application monitors user activity using the external tool [ActivityWatch](https://activitywatch.net/) and uses a local AI model via [Ollama](https://ollama.ai/) to analyze productivity patterns and provide supportive, non-judgmental feedback. It features a system tray icon for background operation and a dashboard for visualizing activity, focus scores, and AI-generated summaries.

The UI is built with React, TypeScript, and styled with Tailwind CSS. Data visualization is handled by Chart.js and Recharts.

## Architecture

*   **Backend:** Rust (`src-tauri`)
    *   Manages application state, system tray functionality, and core logic.
    *   Integrates with ActivityWatch and Ollama via HTTP requests.
    *   Handles data processing, activity categorization, and pattern analysis.
    *   Exposes commands to the frontend via the Tauri API.
    *   Key modules include `activity_watch.rs`, `ai_integration.rs`, and `tauri_commands.rs`.
*   **Frontend:** React/TypeScript (`src`)
    *   Provides the user interface, including the main dashboard, charts, settings, and history views.
    *   Communicates with the Rust backend using `@tauri-apps/api`.
    *   Uses Vite for the development server and build process.
    *   Key components include `MainContent.tsx`, `Sidebar.tsx`, and `ActivityChart.tsx`.

## Building and Running

### Prerequisites

1.  **Rust:** Install from [rustup.rs](https://rustup.rs/).
2.  **Node.js:** Install from [nodejs.org](https://nodejs.org/).
3.  **ActivityWatch:** Must be installed and running on `localhost:5600`.
4.  **Ollama:** (Recommended) Must be installed and running for AI features.

### Development

1.  **Install Frontend Dependencies:**
    ```bash
    npm install
    ```

2.  **Run Frontend Dev Server:**
    ```bash
    npm run dev
    ```

3.  **Run Tauri Application (in a separate terminal):**
    ```bash
    cd src-tauri
    cargo run
    ```
    The application will launch in a new window with hot-reloading enabled for both the frontend and backend.

### Production Build

1.  **Build the Frontend:**
    ```bash
    npm run build
    ```

2.  **Build and Run the Tauri App:**
    ```bash
    cd src-tauri
    cargo build --release
    # The executable will be in src-tauri/target/release/
    ```

## Development Conventions

*   **Backend (Rust):**
    *   Code is formatted with `rustfmt`.
    *   Linting is done with `clippy`.
    *   Tests are run with `cargo test`.
    *   Commands:
        ```bash
        cargo fmt --check
        cargo clippy
        cargo test
        ```
*   **Frontend (TypeScript/React):**
    *   The project uses functional components with hooks.
    *   Styling is managed with Tailwind CSS.
    *   No explicit linting or testing scripts are defined in `package.json`, but the presence of `tsconfig.json` implies a standard TypeScript setup.
*   **Commits:** The `README.md` suggests using conventional commit messages.
