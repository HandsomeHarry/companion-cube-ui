mod commands;
mod daemon_client;
mod paths;

use anyhow::Result;
use clap::{Parser, Subcommand};
use commands::memory::{EditTarget, MemoryTarget};

#[derive(Parser)]
#[command(
    name = "ccube",
    version,
    about = "Companion Cube — ADHD focus companion"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the detector once
    Detect {
        /// Show result without delivering a notification
        #[arg(long)]
        dry_run: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Record a correction
    Correct {
        /// Decision ID to correct (shown in notifications and detect output)
        decision_id: i64,
        /// Your verdict (e.g. "wasn't drift", "should have nudged")
        verdict: String,
    },
    /// Show the current briefing the detector would see
    Briefing {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show daemon status
    Status,
    /// Agent operations (curate, reflect)
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },
    /// Data inspection and management
    Data {
        #[command(subcommand)]
        command: DataCommands,
    },
    /// Daemon lifecycle control
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },
}

// ---------------------------------------------------------------------------
// Agent subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
enum AgentCommands {
    /// Run the curator agent
    Curate {
        /// Propose changes without writing to patterns.md
        #[arg(long)]
        dry_run: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Run the reflector agent
    Reflect {
        #[command(subcommand)]
        command: ReflectCommands,
    },
}

#[derive(Subcommand)]
enum ReflectCommands {
    /// Run the reflector to consolidate patterns.md
    Run {
        /// Propose changes without writing to patterns.md
        #[arg(long)]
        dry_run: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Accept a pending reflector rewrite
    Accept,
    /// Reject a pending reflector rewrite
    Reject,
    /// Show pending reflector output (if any)
    Show {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

// ---------------------------------------------------------------------------
// Data subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
enum DataCommands {
    /// Show recent activity events
    Activity {
        /// Number of hours to look back (default: 1)
        #[arg(long, default_value = "1.0")]
        hours: f64,
    },
    /// Show today's activity sessions (open and solidified)
    Sessions,
    /// Delete events older than 14 days
    Prune,
    /// List corrections
    Corrections {
        /// Show only pending corrections
        #[arg(long)]
        pending: bool,
        /// Maximum number of corrections to show
        #[arg(long, default_value = "20")]
        limit: i64,
    },
    /// Show full details for a correction
    Correction {
        /// Correction ID
        id: i64,
    },
    /// Memory file management (profile, patterns)
    Memory {
        #[command(subcommand)]
        command: MemoryCommands,
    },
}

#[derive(Subcommand)]
enum MemoryCommands {
    /// Show memory contents (profile, patterns, or corrections)
    Show {
        /// Which memory layer to display
        target: MemoryTarget,
    },
    /// Open a memory file in your editor
    Edit {
        /// Which memory file to edit
        target: EditTarget,
    },
    /// List history snapshots for a memory file
    History {
        /// Which memory file's history to show
        target: EditTarget,
    },
    /// Restore a memory file from a history snapshot
    Restore {
        /// Which memory file to restore
        target: EditTarget,
        /// Unix timestamp of the snapshot to restore
        timestamp: i64,
    },
    /// Diff two history snapshots
    Diff {
        /// Which memory file to diff
        target: EditTarget,
        /// Unix timestamp of the first (older) snapshot
        ts1: i64,
        /// Unix timestamp of the second (newer) snapshot
        ts2: i64,
    },
}

// ---------------------------------------------------------------------------
// Daemon subcommands
// ---------------------------------------------------------------------------

#[derive(Subcommand)]
enum DaemonCommands {
    /// Start the daemon in the background
    Start,
    /// Stop the running daemon
    Stop,
    /// Show daemon status
    Status,
    /// Show daemon logs
    Logs {
        /// Follow the log file (like tail -f)
        #[arg(long)]
        follow: bool,
        /// Filter by agent (detector, curator, reflector)
        #[arg(long)]
        agent: Option<String>,
    },
    /// Run continuous activity capture (Ctrl+C to stop)
    Capture,
    /// Register daemon to start on logon
    Install,
    /// Remove daemon autostart registration
    Uninstall,
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    match cli.command {
        // --- Top-level shortcuts (daily workflow) ---
        Some(Commands::Detect { dry_run, json }) => {
            let root = paths::DataRoot::resolve()?;
            ccube_core::db::init_databases(&root.data_dir)?;
            commands::detect::handle_detect(&root, dry_run, json).await?;
        }
        Some(Commands::Correct {
            decision_id,
            verdict,
        }) => {
            let root = paths::DataRoot::resolve()?;
            ccube_core::db::init_databases(&root.data_dir)?;
            commands::correct::handle_correct(&root, decision_id, &verdict).await?;
        }
        Some(Commands::Briefing { json }) => {
            let root = paths::DataRoot::resolve()?;
            ccube_core::db::init_databases(&root.data_dir)?;
            commands::detect::handle_briefing(&root, json).await?;
        }
        Some(Commands::Status) => {
            let root = paths::DataRoot::resolve()?;
            commands::daemon::handle_status(&root).await?;
        }

        // --- Agent operations ---
        Some(Commands::Agent { command }) => {
            let root = paths::DataRoot::resolve()?;
            ccube_core::db::init_databases(&root.data_dir)?;
            match command {
                AgentCommands::Curate { dry_run, json } => {
                    commands::curate::handle_curate(&root, dry_run, json).await?;
                }
                AgentCommands::Reflect { command } => match command {
                    ReflectCommands::Run { dry_run, json } => {
                        commands::reflect::handle_reflect(&root, dry_run, json).await?;
                    }
                    ReflectCommands::Accept => {
                        commands::reflect::handle_accept(&root).await?;
                    }
                    ReflectCommands::Reject => {
                        commands::reflect::handle_reject(&root).await?;
                    }
                    ReflectCommands::Show { json } => {
                        commands::reflect::handle_show_pending(&root, json).await?;
                    }
                },
            }
        }

        // --- Data inspection and management ---
        Some(Commands::Data { command }) => {
            let root = paths::DataRoot::resolve()?;
            ccube_core::db::init_databases(&root.data_dir)?;
            match command {
                DataCommands::Activity { hours } => {
                    commands::activity::handle_recent(&root, hours).await?;
                }
                DataCommands::Sessions => {
                    commands::activity::handle_sessions(&root).await?;
                }
                DataCommands::Prune => {
                    commands::activity::handle_prune(&root)?;
                }
                DataCommands::Corrections { pending, limit } => {
                    commands::correct::handle_corrections_list(&root, pending, limit).await?;
                }
                DataCommands::Correction { id } => {
                    commands::correct::handle_corrections_show(&root, id).await?;
                }
                DataCommands::Memory { command } => match command {
                    MemoryCommands::Show { target } => {
                        commands::memory::handle_show(&root, &target).await?;
                    }
                    MemoryCommands::Edit { target } => {
                        commands::memory::handle_edit(&root, &target)?;
                    }
                    MemoryCommands::History { target } => {
                        commands::memory::handle_history(&root, &target)?;
                    }
                    MemoryCommands::Restore { target, timestamp } => {
                        commands::memory::handle_restore(&root, &target, timestamp)?;
                    }
                    MemoryCommands::Diff { target, ts1, ts2 } => {
                        commands::memory::handle_diff(&root, &target, ts1, ts2)?;
                    }
                },
            }
        }

        // --- Daemon lifecycle ---
        Some(Commands::Daemon { command }) => {
            let root = paths::DataRoot::resolve()?;
            match command {
                DaemonCommands::Start => {
                    commands::daemon::handle_start(&root).await?;
                }
                DaemonCommands::Stop => {
                    commands::daemon::handle_stop(&root).await?;
                }
                DaemonCommands::Status => {
                    commands::daemon::handle_status(&root).await?;
                }
                DaemonCommands::Logs { follow, agent } => {
                    commands::daemon::handle_logs(&root, follow, agent.as_deref())?;
                }
                DaemonCommands::Capture => {
                    commands::capture::handle_capture_run(&root).await?;
                }
                DaemonCommands::Install => {
                    commands::daemon::handle_install(&root)?;
                }
                DaemonCommands::Uninstall => {
                    commands::daemon::handle_uninstall()?;
                }
            }
        }

        None => {
            Cli::parse_from(["ccube", "--help"]);
        }
    }

    Ok(())
}
