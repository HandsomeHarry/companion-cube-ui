use sqlx::{Pool, Sqlite, SqlitePool, migrate::MigrateDatabase, Row};
use chrono::{DateTime, Utc};
use crate::modules::pattern_analyzer::{
    InteractionMetrics, UserBaseline, PatternAnalysis, WorkflowPattern,
    MouseMetrics, KeyboardMetrics, ApplicationMetrics
};
use serde_json;

pub struct PatternDatabase {
    pub pool: Pool<Sqlite>,
}

impl PatternDatabase {
    pub async fn new(db_path: &str) -> Result<Self, String> {
        // Create database if it doesn't exist
        if !Sqlite::database_exists(db_path).await.unwrap_or(false) {
            Sqlite::create_database(db_path).await
                .map_err(|e| format!("Failed to create database: {}", e))?;
        }

        let pool = SqlitePool::connect(db_path).await
            .map_err(|e| format!("Failed to connect to database: {}", e))?;

        let db = Self { pool };
        db.initialize_schema().await?;
        Ok(db)
    }

    async fn initialize_schema(&self) -> Result<(), String> {
        let schema = r#"
        -- User baseline table
        CREATE TABLE IF NOT EXISTS user_baseline (
            id INTEGER PRIMARY KEY,
            training_start TIMESTAMP NOT NULL,
            training_end TIMESTAMP,
            is_trained BOOLEAN NOT NULL DEFAULT 0,
            baseline_data TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );

        -- Interaction metrics table (for raw data storage)
        CREATE TABLE IF NOT EXISTS interaction_metrics (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TIMESTAMP NOT NULL,
            mouse_metrics TEXT NOT NULL,
            keyboard_metrics TEXT NOT NULL,
            application_metrics TEXT NOT NULL,
            browser_metrics TEXT,
            workflow_metrics TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );

        -- Pattern analysis results
        CREATE TABLE IF NOT EXISTS pattern_analyses (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TIMESTAMP NOT NULL,
            session_summary TEXT NOT NULL,
            anomalies TEXT,
            workflow_state TEXT,
            focus_score REAL,
            analysis_data TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );

        -- Workflow patterns table
        CREATE TABLE IF NOT EXISTS workflow_patterns (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            app_sequence TEXT NOT NULL,
            average_duration REAL,
            frequency INTEGER,
            time_preferences TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(name)
        );

        -- Daily aggregates for quick lookups
        CREATE TABLE IF NOT EXISTS daily_aggregates (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            date DATE NOT NULL,
            total_active_time REAL,
            focus_score_avg REAL,
            context_switches INTEGER,
            productive_ratio REAL,
            top_applications TEXT,
            summary_data TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(date)
        );

        -- Indices for performance
        CREATE INDEX IF NOT EXISTS idx_metrics_timestamp ON interaction_metrics(timestamp);
        CREATE INDEX IF NOT EXISTS idx_analyses_timestamp ON pattern_analyses(timestamp);
        CREATE INDEX IF NOT EXISTS idx_aggregates_date ON daily_aggregates(date);
        
        -- Raw activities from ActivityWatch
        CREATE TABLE IF NOT EXISTS activities (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TIMESTAMP NOT NULL,
            duration REAL NOT NULL,
            app_name TEXT NOT NULL,
            window_title TEXT NOT NULL,
            category TEXT,
            data TEXT, -- JSON data for additional fields
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(timestamp, app_name, window_title)
        );
        
        -- App categorization table
        CREATE TABLE IF NOT EXISTS app_categories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            app_name TEXT NOT NULL UNIQUE,
            category TEXT NOT NULL,
            subcategory TEXT,
            productivity_score INTEGER DEFAULT 50, -- 0-100, how productive this app is
            auto_detected BOOLEAN DEFAULT 0,
            user_modified BOOLEAN DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        
        -- Daily summaries with full text
        CREATE TABLE IF NOT EXISTS daily_summaries (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            date DATE NOT NULL UNIQUE,
            summary_text TEXT NOT NULL,
            total_active_time INTEGER,
            total_sessions INTEGER,
            top_applications TEXT, -- JSON array
            focus_score REAL,
            work_percentage REAL,
            distraction_percentage REAL,
            neutral_percentage REAL,
            metadata TEXT, -- JSON for additional data
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        
        -- Indices for new tables
        CREATE INDEX IF NOT EXISTS idx_activities_timestamp ON activities(timestamp);
        CREATE INDEX IF NOT EXISTS idx_activities_app ON activities(app_name);
        CREATE INDEX IF NOT EXISTS idx_activities_category ON activities(category);
        CREATE INDEX IF NOT EXISTS idx_summaries_date ON daily_summaries(date);
        "#;

        sqlx::raw_sql(schema)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("Failed to create schema: {}", e))?;

        Ok(())
    }

    /// Store interaction metrics
    pub async fn store_metrics(&self, metrics: &InteractionMetrics) -> Result<i64, String> {
        let mouse_json = serde_json::to_string(&metrics.mouse)
            .map_err(|e| format!("Failed to serialize mouse metrics: {}", e))?;
        let keyboard_json = serde_json::to_string(&metrics.keyboard)
            .map_err(|e| format!("Failed to serialize keyboard metrics: {}", e))?;
        let app_json = serde_json::to_string(&metrics.application)
            .map_err(|e| format!("Failed to serialize app metrics: {}", e))?;
        let browser_json = metrics.browser.as_ref()
            .map(|b| serde_json::to_string(b))
            .transpose()
            .map_err(|e| format!("Failed to serialize browser metrics: {}", e))?;
        let workflow_json = serde_json::to_string(&metrics.workflow)
            .map_err(|e| format!("Failed to serialize workflow metrics: {}", e))?;

        let result = sqlx::query(
            r#"
            INSERT INTO interaction_metrics 
            (timestamp, mouse_metrics, keyboard_metrics, application_metrics, browser_metrics, workflow_metrics)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#
        )
        .bind(metrics.timestamp)
        .bind(mouse_json)
        .bind(keyboard_json)
        .bind(app_json)
        .bind(browser_json)
        .bind(workflow_json)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to insert metrics: {}", e))?;

        Ok(result.last_insert_rowid())
    }

    /// Store or update user baseline
    pub async fn store_baseline(&self, baseline: &UserBaseline) -> Result<(), String> {
        let baseline_json = serde_json::to_string(baseline)
            .map_err(|e| format!("Failed to serialize baseline: {}", e))?;

        sqlx::query(
            r#"
            INSERT INTO user_baseline (training_start, training_end, is_trained, baseline_data)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(id) DO UPDATE SET
                training_end = excluded.training_end,
                is_trained = excluded.is_trained,
                baseline_data = excluded.baseline_data,
                updated_at = CURRENT_TIMESTAMP
            "#
        )
        .bind(baseline.training_start)
        .bind(baseline.training_end)
        .bind(baseline.is_trained)
        .bind(baseline_json)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to store baseline: {}", e))?;

        Ok(())
    }

    /// Retrieve current user baseline
    pub async fn get_baseline(&self) -> Result<Option<UserBaseline>, String> {
        let row = sqlx::query(
            "SELECT baseline_data FROM user_baseline WHERE is_trained = 1 ORDER BY updated_at DESC LIMIT 1"
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("Failed to fetch baseline: {}", e))?;

        match row {
            Some(r) => {
                let baseline_data: String = r.try_get("baseline_data")
                    .map_err(|e| format!("Failed to get baseline_data: {}", e))?;
                let baseline: UserBaseline = serde_json::from_str(&baseline_data)
                    .map_err(|e| format!("Failed to deserialize baseline: {}", e))?;
                Ok(Some(baseline))
            }
            None => Ok(None)
        }
    }

    /// Store pattern analysis result
    pub async fn store_analysis(&self, analysis: &PatternAnalysis) -> Result<i64, String> {
        let summary_json = serde_json::to_string(&analysis.session_summary)
            .map_err(|e| format!("Failed to serialize summary: {}", e))?;
        let anomalies_json = serde_json::to_string(&analysis.anomalies)
            .map_err(|e| format!("Failed to serialize anomalies: {}", e))?;
        let workflow_json = serde_json::to_string(&analysis.workflow_state)
            .map_err(|e| format!("Failed to serialize workflow: {}", e))?;
        let analysis_json = serde_json::to_string(analysis)
            .map_err(|e| format!("Failed to serialize analysis: {}", e))?;

        let result = sqlx::query(
            r#"
            INSERT INTO pattern_analyses 
            (timestamp, session_summary, anomalies, workflow_state, focus_score, analysis_data)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#
        )
        .bind(analysis.timestamp)
        .bind(summary_json)
        .bind(anomalies_json)
        .bind(workflow_json)
        .bind(analysis.focus_score)
        .bind(analysis_json)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to insert analysis: {}", e))?;

        Ok(result.last_insert_rowid())
    }

    /// Get recent metrics for analysis
    pub async fn get_recent_metrics(&self, hours: i32) -> Result<Vec<InteractionMetrics>, String> {
        let since = Utc::now() - chrono::Duration::hours(hours as i64);
        
        let rows = sqlx::query(
            r#"
            SELECT timestamp, mouse_metrics, keyboard_metrics, application_metrics, 
                   browser_metrics, workflow_metrics
            FROM interaction_metrics
            WHERE timestamp > ?1
            ORDER BY timestamp DESC
            "#
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to fetch metrics: {}", e))?;

        let mut metrics = Vec::new();
        for row in rows {
            let timestamp: DateTime<Utc> = row.try_get("timestamp")
                .map_err(|e| format!("Failed to get timestamp: {}", e))?;
            let mouse_json: String = row.try_get("mouse_metrics")
                .map_err(|e| format!("Failed to get mouse_metrics: {}", e))?;
            let keyboard_json: String = row.try_get("keyboard_metrics")
                .map_err(|e| format!("Failed to get keyboard_metrics: {}", e))?;
            let app_json: String = row.try_get("application_metrics")
                .map_err(|e| format!("Failed to get application_metrics: {}", e))?;
            let browser_json: Option<String> = row.try_get("browser_metrics")
                .map_err(|e| format!("Failed to get browser_metrics: {}", e))?;
            let workflow_json: String = row.try_get("workflow_metrics")
                .map_err(|e| format!("Failed to get workflow_metrics: {}", e))?;
                
            let mouse: MouseMetrics = serde_json::from_str(&mouse_json)
                .map_err(|e| format!("Failed to deserialize mouse metrics: {}", e))?;
            let keyboard: KeyboardMetrics = serde_json::from_str(&keyboard_json)
                .map_err(|e| format!("Failed to deserialize keyboard metrics: {}", e))?;
            let application: ApplicationMetrics = serde_json::from_str(&app_json)
                .map_err(|e| format!("Failed to deserialize app metrics: {}", e))?;
            
            let browser = browser_json
                .as_ref()
                .map(|b| serde_json::from_str(b))
                .transpose()
                .map_err(|e| format!("Failed to deserialize browser metrics: {}", e))?;
            
            let workflow = serde_json::from_str(&workflow_json)
                .map_err(|e| format!("Failed to deserialize workflow metrics: {}", e))?;

            metrics.push(InteractionMetrics {
                timestamp,
                mouse,
                keyboard,
                application,
                browser,
                workflow,
            });
        }

        Ok(metrics)
    }

    /// Store discovered workflow pattern
    pub async fn store_workflow_pattern(&self, pattern: &WorkflowPattern) -> Result<(), String> {
        let app_sequence_json = serde_json::to_string(&pattern.app_sequence)
            .map_err(|e| format!("Failed to serialize app sequence: {}", e))?;
        let time_prefs_json = serde_json::to_string(&pattern.time_of_day_preference)
            .map_err(|e| format!("Failed to serialize time preferences: {}", e))?;

        sqlx::query(
            r#"
            INSERT INTO workflow_patterns (name, app_sequence, average_duration, frequency, time_preferences)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(name) DO UPDATE SET
                app_sequence = excluded.app_sequence,
                average_duration = excluded.average_duration,
                frequency = excluded.frequency + 1,
                time_preferences = excluded.time_preferences
            "#
        )
        .bind(&pattern.name)
        .bind(app_sequence_json)
        .bind(pattern.average_duration)
        .bind(pattern.frequency)
        .bind(time_prefs_json)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to store workflow pattern: {}", e))?;

        Ok(())
    }

    /// Get training data for baseline calculation
    pub async fn get_training_data(&self, days: i32) -> Result<Vec<InteractionMetrics>, String> {
        let since = Utc::now() - chrono::Duration::days(days as i64);
        
        let rows = sqlx::query(
            r#"
            SELECT timestamp, mouse_metrics, keyboard_metrics, application_metrics, 
                   browser_metrics, workflow_metrics
            FROM interaction_metrics
            WHERE timestamp > ?1
            ORDER BY timestamp ASC
            "#
        )
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to fetch training data: {}", e))?;

        let mut metrics = Vec::new();
        for row in rows {
            let timestamp: DateTime<Utc> = row.try_get("timestamp")
                .map_err(|e| format!("Failed to get timestamp: {}", e))?;
            let mouse_json: String = row.try_get("mouse_metrics")
                .map_err(|e| format!("Failed to get mouse_metrics: {}", e))?;
            let keyboard_json: String = row.try_get("keyboard_metrics")
                .map_err(|e| format!("Failed to get keyboard_metrics: {}", e))?;
            let app_json: String = row.try_get("application_metrics")
                .map_err(|e| format!("Failed to get application_metrics: {}", e))?;
            let browser_json: Option<String> = row.try_get("browser_metrics")
                .map_err(|e| format!("Failed to get browser_metrics: {}", e))?;
            let workflow_json: String = row.try_get("workflow_metrics")
                .map_err(|e| format!("Failed to get workflow_metrics: {}", e))?;
                
            let mouse: MouseMetrics = serde_json::from_str(&mouse_json)
                .map_err(|e| format!("Failed to deserialize mouse metrics: {}", e))?;
            let keyboard: KeyboardMetrics = serde_json::from_str(&keyboard_json)
                .map_err(|e| format!("Failed to deserialize keyboard metrics: {}", e))?;
            let application: ApplicationMetrics = serde_json::from_str(&app_json)
                .map_err(|e| format!("Failed to deserialize app metrics: {}", e))?;
            let browser = browser_json.as_ref()
                .map(|b| serde_json::from_str(b))
                .transpose()
                .map_err(|e| format!("Failed to deserialize browser metrics: {}", e))?;
            let workflow = serde_json::from_str(&workflow_json)
                .map_err(|e| format!("Failed to deserialize workflow metrics: {}", e))?;

            metrics.push(InteractionMetrics {
                timestamp,
                mouse,
                keyboard,
                application,
                browser,
                workflow,
            });
        }

        Ok(metrics)
    }

    /// Clean old data to prevent database bloat
    pub async fn cleanup_old_data(&self, days_to_keep: i32) -> Result<u64, String> {
        let cutoff = Utc::now() - chrono::Duration::days(days_to_keep as i64);
        
        let result = sqlx::query(
            "DELETE FROM interaction_metrics WHERE timestamp < ?1"
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to cleanup old data: {}", e))?;

        Ok(result.rows_affected())
    }
    
    // Activity storage functions
    pub async fn store_activities(&self, activities: &[serde_json::Value]) -> Result<u64, String> {
        let mut tx = self.pool.begin().await
            .map_err(|e| format!("Failed to start transaction: {}", e))?;
        
        let mut count = 0u64;
        for activity in activities {
            let timestamp = activity.get("timestamp")
                .and_then(|t| t.as_str())
                .ok_or("Missing timestamp")?;
            let duration = activity.get("duration")
                .and_then(|d| d.as_f64())
                .unwrap_or(0.0);
            let data = activity.get("data")
                .and_then(|d| d.as_object())
                .ok_or("Missing data object")?;
            let app_name = data.get("app")
                .and_then(|a| a.as_str())
                .unwrap_or("unknown");
            let window_title = data.get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            
            // Get category if it exists
            let category = self.get_app_category(app_name).await.ok();
            
            let result = sqlx::query(
                "INSERT OR IGNORE INTO activities (timestamp, duration, app_name, window_title, category, data) 
                 VALUES (?, ?, ?, ?, ?, ?)"
            )
            .bind(timestamp)
            .bind(duration)
            .bind(app_name)
            .bind(window_title)
            .bind(&category)
            .bind(activity.to_string())
            .execute(&mut *tx)
            .await;
            
            if let Ok(r) = result {
                count += r.rows_affected();
            }
        }
        
        tx.commit().await
            .map_err(|e| format!("Failed to commit transaction: {}", e))?;
        
        Ok(count)
    }
    
    // App category management
    pub async fn get_app_category(&self, app_name: &str) -> Result<String, String> {
        let result = sqlx::query("SELECT category FROM app_categories WHERE app_name = ?")
            .bind(app_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| format!("Failed to get app category: {}", e))?;
        
        match result {
            Some(row) => Ok(row.get("category")),
            None => Err(format!("No category found for app: {}", app_name))
        }
    }
    
    pub async fn set_app_category(
        &self, 
        app_name: &str, 
        category: &str,
        subcategory: Option<&str>,
        productivity_score: Option<i32>,
        auto_detected: bool
    ) -> Result<(), String> {
        let now = Utc::now();
        
        sqlx::query(
            "INSERT INTO app_categories (app_name, category, subcategory, productivity_score, auto_detected, updated_at) 
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(app_name) DO UPDATE SET
                category = excluded.category,
                subcategory = excluded.subcategory,
                productivity_score = COALESCE(excluded.productivity_score, app_categories.productivity_score),
                auto_detected = excluded.auto_detected,
                user_modified = CASE WHEN excluded.auto_detected = 0 THEN 1 ELSE app_categories.user_modified END,
                updated_at = excluded.updated_at"
        )
        .bind(app_name)
        .bind(category)
        .bind(subcategory)
        .bind(productivity_score.unwrap_or(50))
        .bind(auto_detected)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to set app category: {}", e))?;
        
        Ok(())
    }
    
    pub async fn get_all_app_categories(&self) -> Result<Vec<(String, String, Option<String>, i32)>, String> {
        let rows = sqlx::query(
            "SELECT app_name, category, subcategory, productivity_score FROM app_categories ORDER BY app_name"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to get app categories: {}", e))?;
        
        let categories = rows.into_iter()
            .map(|row| {
                (
                    row.get("app_name"),
                    row.get("category"),
                    row.get("subcategory"),
                    row.get("productivity_score")
                )
            })
            .collect();
        
        Ok(categories)
    }
    
    pub async fn get_uncategorized_apps(&self) -> Result<Vec<String>, String> {
        let rows = sqlx::query(
            "SELECT DISTINCT app_name FROM activities 
             WHERE (category IS NULL OR category = '')
             AND app_name NOT IN (SELECT app_name FROM app_categories)
             ORDER BY app_name"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to get uncategorized apps: {}", e))?;
        
        let apps = rows.into_iter()
            .map(|row| row.get("app_name"))
            .collect();
        
        Ok(apps)
    }
    
    // Daily summary management
    pub async fn store_daily_summary(
        &self,
        date: &str,
        summary_text: &str,
        total_active_time: i64,
        total_sessions: i32,
        top_apps: &[String],
        focus_score: Option<f64>,
        work_pct: Option<f64>,
        distraction_pct: Option<f64>,
        neutral_pct: Option<f64>
    ) -> Result<(), String> {
        let top_apps_json = serde_json::to_string(top_apps)
            .map_err(|e| format!("Failed to serialize top apps: {}", e))?;
        
        sqlx::query(
            "INSERT INTO daily_summaries 
             (date, summary_text, total_active_time, total_sessions, top_applications, 
              focus_score, work_percentage, distraction_percentage, neutral_percentage)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(date) DO UPDATE SET
                summary_text = excluded.summary_text,
                total_active_time = excluded.total_active_time,
                total_sessions = excluded.total_sessions,
                top_applications = excluded.top_applications,
                focus_score = excluded.focus_score,
                work_percentage = excluded.work_percentage,
                distraction_percentage = excluded.distraction_percentage,
                neutral_percentage = excluded.neutral_percentage"
        )
        .bind(date)
        .bind(summary_text)
        .bind(total_active_time)
        .bind(total_sessions)
        .bind(top_apps_json)
        .bind(focus_score)
        .bind(work_pct)
        .bind(distraction_pct)
        .bind(neutral_pct)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to store daily summary: {}", e))?;
        
        Ok(())
    }
    
    pub async fn get_daily_summary(&self, date: &str) -> Result<Option<serde_json::Value>, String> {
        let result = sqlx::query(
            "SELECT * FROM daily_summaries WHERE date = ?"
        )
        .bind(date)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| format!("Failed to get daily summary: {}", e))?;
        
        match result {
            Some(row) => {
                let top_apps: Vec<String> = serde_json::from_str(row.get("top_applications"))
                    .unwrap_or_else(|_| Vec::new());
                
                Ok(Some(serde_json::json!({
                    "date": row.get::<String, _>("date"),
                    "summary": row.get::<String, _>("summary_text"),
                    "total_active_time": row.get::<i64, _>("total_active_time"),
                    "total_sessions": row.get::<i32, _>("total_sessions"),
                    "top_applications": top_apps,
                    "focus_score": row.get::<Option<f64>, _>("focus_score"),
                    "work_percentage": row.get::<Option<f64>, _>("work_percentage"),
                    "distraction_percentage": row.get::<Option<f64>, _>("distraction_percentage"),
                    "neutral_percentage": row.get::<Option<f64>, _>("neutral_percentage"),
                    "generated_at": row.get::<chrono::DateTime<Utc>, _>("created_at").to_rfc3339()
                })))
            },
            None => Ok(None)
        }
    }
    
    // Get activities with categories for a time range
    pub async fn get_categorized_activities(
        &self, 
        start: DateTime<Utc>, 
        end: DateTime<Utc>
    ) -> Result<Vec<serde_json::Value>, String> {
        let rows = sqlx::query(
            "SELECT a.*, ac.category, ac.subcategory, ac.productivity_score
             FROM activities a
             LEFT JOIN app_categories ac ON a.app_name = ac.app_name
             WHERE datetime(a.timestamp) >= datetime(?) AND datetime(a.timestamp) <= datetime(?)
             ORDER BY a.timestamp"
        )
        .bind(start.to_rfc3339())
        .bind(end.to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to get categorized activities: {}", e))?;
        
        let activities = rows.into_iter()
            .map(|row| {
                serde_json::json!({
                    "timestamp": row.get::<String, _>("timestamp"),
                    "duration": row.get::<f64, _>("duration"),
                    "app_name": row.get::<String, _>("app_name"),
                    "window_title": row.get::<String, _>("window_title"),
                    "category": row.get::<Option<String>, _>("category"),
                    "subcategory": row.get::<Option<String>, _>("subcategory"),
                    "productivity_score": row.get::<Option<i32>, _>("productivity_score")
                })
            })
            .collect();
        
        Ok(activities)
    }
    
    // Get activity statistics by category for a time range
    pub async fn get_category_statistics(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>
    ) -> Result<Vec<serde_json::Value>, String> {
        let rows = sqlx::query(
            "SELECT 
                COALESCE(ac.category, 'uncategorized') as category,
                COUNT(DISTINCT a.app_name) as app_count,
                SUM(a.duration) as total_duration,
                AVG(COALESCE(ac.productivity_score, 50)) as avg_productivity_score
             FROM activities a
             LEFT JOIN app_categories ac ON a.app_name = ac.app_name
             WHERE datetime(a.timestamp) >= datetime(?) AND datetime(a.timestamp) <= datetime(?)
             GROUP BY COALESCE(ac.category, 'uncategorized')
             ORDER BY total_duration DESC"
        )
        .bind(start.to_rfc3339())
        .bind(end.to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to get category statistics: {}", e))?;
        
        let stats = rows.into_iter()
            .map(|row| {
                serde_json::json!({
                    "category": row.get::<String, _>("category"),
                    "app_count": row.get::<i32, _>("app_count"),
                    "total_duration": row.get::<f64, _>("total_duration"),
                    "avg_productivity_score": row.get::<f64, _>("avg_productivity_score")
                })
            })
            .collect();
        
        Ok(stats)
    }
    
    // Get hourly activity breakdown
    pub async fn get_hourly_breakdown(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>
    ) -> Result<Vec<serde_json::Value>, String> {
        let rows = sqlx::query(
            "SELECT 
                strftime('%Y-%m-%dT%H:00:00Z', timestamp) as hour,
                COALESCE(ac.category, 'uncategorized') as category,
                SUM(duration) as total_duration
             FROM activities a
             LEFT JOIN app_categories ac ON a.app_name = ac.app_name
             WHERE datetime(a.timestamp) >= datetime(?) AND datetime(a.timestamp) <= datetime(?)
             GROUP BY hour, COALESCE(ac.category, 'uncategorized')
             ORDER BY hour, COALESCE(ac.category, 'uncategorized')"
        )
        .bind(start.to_rfc3339())
        .bind(end.to_rfc3339())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to get hourly breakdown: {}", e))?;
        
        let breakdown = rows.into_iter()
            .map(|row| {
                serde_json::json!({
                    "hour": row.get::<String, _>("hour"),
                    "category": row.get::<String, _>("category"),
                    "duration": row.get::<f64, _>("total_duration")
                })
            })
            .collect();
        
        Ok(breakdown)
    }
    
    // Get top apps for a time range
    pub async fn get_top_apps(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: i32
    ) -> Result<Vec<serde_json::Value>, String> {
        let rows = sqlx::query(
            "SELECT 
                a.app_name,
                COALESCE(ac.category, 'uncategorized') as category,
                COALESCE(ac.productivity_score, 50) as productivity_score,
                SUM(a.duration) as total_duration,
                COUNT(*) as session_count
             FROM activities a
             LEFT JOIN app_categories ac ON a.app_name = ac.app_name
             WHERE datetime(a.timestamp) >= datetime(?) AND datetime(a.timestamp) <= datetime(?)
             GROUP BY a.app_name, ac.category, ac.productivity_score
             ORDER BY total_duration DESC
             LIMIT ?"
        )
        .bind(start.to_rfc3339())
        .bind(end.to_rfc3339())
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| format!("Failed to get top apps: {}", e))?;
        
        let apps = rows.into_iter()
            .map(|row| {
                serde_json::json!({
                    "app_name": row.get::<String, _>("app_name"),
                    "category": row.get::<String, _>("category"),
                    "productivity_score": row.get::<i32, _>("productivity_score"),
                    "total_duration": row.get::<f64, _>("total_duration"),
                    "session_count": row.get::<i32, _>("session_count")
                })
            })
            .collect();
        
        Ok(apps)
    }
    
    // Get total activity count
    pub async fn get_activity_count(&self) -> Result<i64, String> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM activities")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("Failed to get activity count: {}", e))?;
        
        Ok(row.get("count"))
    }
    
    // Get categorized app count
    pub async fn get_categorized_app_count(&self) -> Result<i64, String> {
        let row = sqlx::query("SELECT COUNT(*) as count FROM app_categories")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| format!("Failed to get categorized app count: {}", e))?;
        
        Ok(row.get("count"))
    }
}