use crate::modules::pattern_analyzer::{InteractionMetrics, MouseMetrics, KeyboardMetrics, ApplicationMetrics, TypingBurst};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::{AppHandle, Emitter};

const INTERACTION_BUFFER_SIZE: usize = 1000;
const TYPING_BURST_THRESHOLD_MS: f64 = 2000.0;

#[derive(Debug, Clone)]
pub struct InteractionTracker {
    mouse_buffer: Arc<Mutex<VecDeque<MouseEvent>>>,
    keyboard_buffer: Arc<Mutex<VecDeque<KeyboardEvent>>>,
    current_app: Arc<Mutex<Option<ApplicationInfo>>>,
    last_interaction: Arc<Mutex<DateTime<Utc>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MouseEvent {
    timestamp: DateTime<Utc>,
    x: i32,
    y: i32,
    event_type: MouseEventType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum MouseEventType {
    Move,
    Click,
    DoubleClick,
    RightClick,
    Scroll(f32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeyboardEvent {
    timestamp: DateTime<Utc>,
    key_type: KeyType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum KeyType {
    Character,
    Backspace,
    Enter,
    Tab,
    Modifier,
    Navigation,
    Function,
}

#[derive(Debug, Clone)]
struct ApplicationInfo {
    name: String,
    window_title: String,
    start_time: DateTime<Utc>,
    interactions: u32,
}

impl InteractionTracker {
    pub fn new() -> Self {
        Self {
            mouse_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(INTERACTION_BUFFER_SIZE))),
            keyboard_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(INTERACTION_BUFFER_SIZE))),
            current_app: Arc::new(Mutex::new(None)),
            last_interaction: Arc::new(Mutex::new(Utc::now())),
        }
    }

    /// Start tracking interactions
    pub async fn start_tracking(&self, app: AppHandle) -> Result<(), String> {
        // Register global event listeners for mouse and keyboard
        self.setup_mouse_listener(app.clone()).await?;
        self.setup_keyboard_listener(app.clone()).await?;
        
        // Start periodic metric calculation
        let tracker = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
                if let Ok(metrics) = tracker.calculate_metrics().await {
                    // Send metrics to pattern analyzer
                    if let Err(e) = app.emit("interaction_metrics", &metrics) {
                        eprintln!("Failed to emit interaction metrics: {}", e);
                    }
                }
            }
        });

        Ok(())
    }

    /// Record mouse movement
    pub async fn record_mouse_move(&self, x: i32, y: i32) -> Result<(), String> {
        let event = MouseEvent {
            timestamp: Utc::now(),
            x,
            y,
            event_type: MouseEventType::Move,
        };

        let mut buffer = self.mouse_buffer.lock().await;
        if buffer.len() >= INTERACTION_BUFFER_SIZE {
            buffer.pop_front();
        }
        buffer.push_back(event);

        *self.last_interaction.lock().await = Utc::now();
        Ok(())
    }

    /// Record mouse click
    pub async fn record_mouse_click(&self, x: i32, y: i32, button: MouseButton) -> Result<(), String> {
        let event_type = match button {
            MouseButton::Left => MouseEventType::Click,
            MouseButton::Right => MouseEventType::RightClick,
            MouseButton::Middle => MouseEventType::Click,
        };

        let event = MouseEvent {
            timestamp: Utc::now(),
            x,
            y,
            event_type,
        };

        let mut buffer = self.mouse_buffer.lock().await;
        if buffer.len() >= INTERACTION_BUFFER_SIZE {
            buffer.pop_front();
        }
        buffer.push_back(event);

        *self.last_interaction.lock().await = Utc::now();
        Ok(())
    }

    /// Record keyboard event
    pub async fn record_keyboard_event(&self, key_type: KeyType) -> Result<(), String> {
        let event = KeyboardEvent {
            timestamp: Utc::now(),
            key_type,
        };

        let mut buffer = self.keyboard_buffer.lock().await;
        if buffer.len() >= INTERACTION_BUFFER_SIZE {
            buffer.pop_front();
        }
        buffer.push_back(event);

        *self.last_interaction.lock().await = Utc::now();
        Ok(())
    }

    /// Update current application info
    pub async fn update_current_app(&self, app_name: String, window_title: String) -> Result<(), String> {
        let mut current = self.current_app.lock().await;
        
        match current.as_mut() {
            Some(info) if info.name == app_name => {
                info.window_title = window_title;
                info.interactions += 1;
            }
            _ => {
                *current = Some(ApplicationInfo {
                    name: app_name,
                    window_title,
                    start_time: Utc::now(),
                    interactions: 1,
                });
            }
        }

        Ok(())
    }

    /// Calculate current metrics from buffers
    pub async fn calculate_metrics(&self) -> Result<InteractionMetrics, String> {
        let mouse_buffer = self.mouse_buffer.lock().await;
        let keyboard_buffer = self.keyboard_buffer.lock().await;
        let current_app = self.current_app.lock().await;

        let mouse_metrics = self.calculate_mouse_metrics(&mouse_buffer)?;
        let keyboard_metrics = self.calculate_keyboard_metrics(&keyboard_buffer)?;
        let app_metrics = self.calculate_app_metrics(&current_app)?;

        Ok(InteractionMetrics {
            timestamp: Utc::now(),
            mouse: mouse_metrics,
            keyboard: keyboard_metrics,
            application: app_metrics,
            browser: None, // Will be implemented with browser extension
            workflow: Default::default(),
        })
    }

    fn calculate_mouse_metrics(&self, events: &VecDeque<MouseEvent>) -> Result<MouseMetrics, String> {
        if events.is_empty() {
            return Ok(MouseMetrics {
                movement_velocity: 0.0,
                acceleration: 0.0,
                click_frequency: 0,
                click_intervals: vec![],
                idle_time: 60.0,
                distance_traveled: 0.0,
            });
        }

        let mut total_distance = 0.0;
        let mut velocities = Vec::new();
        let mut click_times = Vec::new();
        let mut last_event = events.front().unwrap();

        for event in events.iter().skip(1) {
            match event.event_type {
                MouseEventType::Move => {
                    let dx = (event.x - last_event.x) as f64;
                    let dy = (event.y - last_event.y) as f64;
                    let distance = (dx * dx + dy * dy).sqrt();
                    total_distance += distance;

                    let time_diff = (event.timestamp - last_event.timestamp).num_milliseconds() as f64 / 1000.0;
                    if time_diff > 0.0 {
                        velocities.push(distance / time_diff);
                    }
                }
                MouseEventType::Click | MouseEventType::DoubleClick => {
                    click_times.push(event.timestamp);
                }
                _ => {}
            }
            last_event = event;
        }

        let avg_velocity = if !velocities.is_empty() {
            velocities.iter().sum::<f64>() / velocities.len() as f64
        } else {
            0.0
        };

        let acceleration = if velocities.len() > 1 {
            let mut accel_sum = 0.0;
            for i in 1..velocities.len() {
                accel_sum += (velocities[i] - velocities[i-1]).abs();
            }
            accel_sum / (velocities.len() - 1) as f64
        } else {
            0.0
        };

        let click_intervals = self.calculate_click_intervals(&click_times);
        let click_frequency = (click_times.len() as f64 * 60.0 / 
            (events.back().unwrap().timestamp - events.front().unwrap().timestamp).num_seconds() as f64) as u32;

        Ok(MouseMetrics {
            movement_velocity: avg_velocity,
            acceleration,
            click_frequency,
            click_intervals,
            idle_time: 0.0, // Will be calculated based on gaps
            distance_traveled: total_distance,
        })
    }

    fn calculate_keyboard_metrics(&self, events: &VecDeque<KeyboardEvent>) -> Result<KeyboardMetrics, String> {
        if events.is_empty() {
            return Ok(KeyboardMetrics {
                typing_speed: 0.0,
                burst_patterns: vec![],
                inter_keystroke_timing: vec![],
                correction_rate: 0.0,
                idle_periods: vec![],
            });
        }

        let mut bursts = Vec::new();
        let mut current_burst: Option<TypingBurst> = None;
        let mut keystroke_timings = Vec::new();
        let mut backspace_count = 0;
        let mut char_count = 0;

        let mut last_event = events.front().unwrap();

        for event in events.iter().skip(1) {
            let time_diff = (event.timestamp - last_event.timestamp).num_milliseconds() as f64;
            
            match event.key_type {
                KeyType::Character => {
                    char_count += 1;
                    keystroke_timings.push(time_diff);

                    if time_diff < TYPING_BURST_THRESHOLD_MS {
                        match current_burst.as_mut() {
                            Some(burst) => {
                                burst.keystroke_count += 1;
                                burst.duration = (event.timestamp - burst.start_time).num_seconds() as f64;
                            }
                            None => {
                                current_burst = Some(TypingBurst {
                                    start_time: last_event.timestamp,
                                    duration: time_diff / 1000.0,
                                    keystroke_count: 2,
                                    average_interval: time_diff,
                                });
                            }
                        }
                    } else if let Some(burst) = current_burst.take() {
                        if burst.keystroke_count > 5 {
                            bursts.push(burst);
                        }
                    }
                }
                KeyType::Backspace => {
                    backspace_count += 1;
                }
                _ => {}
            }

            last_event = event;
        }

        if let Some(burst) = current_burst {
            if burst.keystroke_count > 5 {
                bursts.push(burst);
            }
        }

        let total_time = (events.back().unwrap().timestamp - events.front().unwrap().timestamp).num_seconds() as f64 / 60.0;
        let typing_speed = if total_time > 0.0 {
            (char_count as f64 / 5.0) / total_time // Assuming 5 chars per word
        } else {
            0.0
        };

        let correction_rate = if char_count > 0 {
            backspace_count as f64 / char_count as f64
        } else {
            0.0
        };

        Ok(KeyboardMetrics {
            typing_speed,
            burst_patterns: bursts,
            inter_keystroke_timing: keystroke_timings,
            correction_rate,
            idle_periods: vec![], // TODO: Calculate idle periods
        })
    }

    fn calculate_app_metrics(&self, current_app: &Option<ApplicationInfo>) -> Result<ApplicationMetrics, String> {
        match current_app {
            Some(app) => {
                let time_spent = (Utc::now() - app.start_time).num_seconds() as f64;
                let interaction_density = if time_spent > 0.0 {
                    (app.interactions as f64 * 60.0) / time_spent
                } else {
                    0.0
                };

                Ok(ApplicationMetrics {
                    app_name: app.name.clone(),
                    window_title: app.window_title.clone(),
                    time_spent,
                    switch_count: 1, // Will be tracked separately
                    interaction_density,
                })
            }
            None => Ok(ApplicationMetrics {
                app_name: "Unknown".to_string(),
                window_title: "Unknown".to_string(),
                time_spent: 0.0,
                switch_count: 0,
                interaction_density: 0.0,
            })
        }
    }

    fn calculate_click_intervals(&self, click_times: &[DateTime<Utc>]) -> Vec<f64> {
        let mut intervals = Vec::new();
        for i in 1..click_times.len() {
            let interval = (click_times[i] - click_times[i-1]).num_milliseconds() as f64;
            intervals.push(interval);
        }
        intervals
    }

    async fn setup_mouse_listener(&self, _app: AppHandle) -> Result<(), String> {
        // Platform-specific mouse hook implementation
        // This would use native OS APIs or a crate like `device_query` or `rdev`
        Ok(())
    }

    async fn setup_keyboard_listener(&self, _app: AppHandle) -> Result<(), String> {
        // Platform-specific keyboard hook implementation
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl Default for crate::modules::pattern_analyzer::WorkflowMetrics {
    fn default() -> Self {
        Self {
            app_sequence: vec![],
            session_boundaries: vec![],
            efficiency_score: 0.0,
            context_switches: 0,
            productive_periods: vec![],
        }
    }
}