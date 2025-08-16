use std::collections::HashMap;

pub struct AppCategory {
    pub category: &'static str,
    pub subcategory: Option<&'static str>,
    pub productivity_score: i32,
}

/// Get default app categories based on common apps
/// This avoids expensive LLM calls for known applications
pub fn get_default_app_categories() -> HashMap<&'static str, AppCategory> {
    let mut categories = HashMap::new();
    
    // Gaming & Steam
    categories.insert("steamwebhelper", AppCategory { 
        category: "entertainment", 
        subcategory: Some("gaming"), 
        productivity_score: 10 
    });
    categories.insert("steam", AppCategory { 
        category: "entertainment", 
        subcategory: Some("gaming"), 
        productivity_score: 10 
    });
    categories.insert("cs2", AppCategory { 
        category: "entertainment", 
        subcategory: Some("gaming"), 
        productivity_score: 0 
    });
    
    // Development
    categories.insert("code", AppCategory { 
        category: "development", 
        subcategory: Some("ide"), 
        productivity_score: 95 
    });
    categories.insert("devenv", AppCategory { 
        category: "development", 
        subcategory: Some("ide"), 
        productivity_score: 95 
    });
    categories.insert("windowsterminal", AppCategory { 
        category: "development", 
        subcategory: Some("terminal"), 
        productivity_score: 85 
    });
    categories.insert("cmd", AppCategory { 
        category: "development", 
        subcategory: Some("terminal"), 
        productivity_score: 80 
    });
    categories.insert("powershell", AppCategory { 
        category: "development", 
        subcategory: Some("terminal"), 
        productivity_score: 80 
    });
    
    // Browsers
    categories.insert("brave", AppCategory { 
        category: "productivity", 
        subcategory: Some("browser"), 
        productivity_score: 60 
    });
    categories.insert("chrome", AppCategory { 
        category: "productivity", 
        subcategory: Some("browser"), 
        productivity_score: 60 
    });
    categories.insert("firefox", AppCategory { 
        category: "productivity", 
        subcategory: Some("browser"), 
        productivity_score: 60 
    });
    categories.insert("edge", AppCategory { 
        category: "productivity", 
        subcategory: Some("browser"), 
        productivity_score: 60 
    });
    
    // Communication
    categories.insert("discord", AppCategory { 
        category: "communication", 
        subcategory: Some("chat"), 
        productivity_score: 40 
    });
    categories.insert("slack", AppCategory { 
        category: "communication", 
        subcategory: Some("chat"), 
        productivity_score: 50 
    });
    categories.insert("teams", AppCategory { 
        category: "communication", 
        subcategory: Some("chat"), 
        productivity_score: 50 
    });
    categories.insert("zoom", AppCategory { 
        category: "communication", 
        subcategory: Some("video"), 
        productivity_score: 60 
    });
    
    // System
    categories.insert("explorer", AppCategory { 
        category: "system", 
        subcategory: Some("file_manager"), 
        productivity_score: 50 
    });
    categories.insert("taskmgr", AppCategory { 
        category: "system", 
        subcategory: Some("utility"), 
        productivity_score: 50 
    });
    categories.insert("settings", AppCategory { 
        category: "system", 
        subcategory: Some("settings"), 
        productivity_score: 50 
    });
    
    // Productivity
    categories.insert("obsidian", AppCategory { 
        category: "productivity", 
        subcategory: Some("notes"), 
        productivity_score: 85 
    });
    categories.insert("notion", AppCategory { 
        category: "productivity", 
        subcategory: Some("notes"), 
        productivity_score: 85 
    });
    categories.insert("todoist", AppCategory { 
        category: "productivity", 
        subcategory: Some("tasks"), 
        productivity_score: 90 
    });
    
    // Entertainment
    categories.insert("spotify", AppCategory { 
        category: "entertainment", 
        subcategory: Some("music"), 
        productivity_score: 30 
    });
    categories.insert("vlc", AppCategory { 
        category: "entertainment", 
        subcategory: Some("video"), 
        productivity_score: 20 
    });
    
    // Work
    categories.insert("outlook", AppCategory { 
        category: "work", 
        subcategory: Some("email"), 
        productivity_score: 70 
    });
    categories.insert("excel", AppCategory { 
        category: "work", 
        subcategory: Some("office"), 
        productivity_score: 80 
    });
    categories.insert("word", AppCategory { 
        category: "work", 
        subcategory: Some("office"), 
        productivity_score: 80 
    });
    categories.insert("powerpoint", AppCategory { 
        category: "work", 
        subcategory: Some("office"), 
        productivity_score: 70 
    });
    
    // Special case for Companion Cube itself
    categories.insert("app", AppCategory { 
        category: "productivity", 
        subcategory: Some("assistant"), 
        productivity_score: 70 
    });
    
    categories
}

/// Match app name to category (case-insensitive, partial match)
pub fn categorize_app(app_name: &str) -> Option<(&'static str, Option<&'static str>, i32)> {
    let app_lower = app_name.to_lowercase();
    let categories = get_default_app_categories();
    
    // First try exact match
    if let Some(cat) = categories.get(app_lower.as_str()) {
        return Some((cat.category, cat.subcategory, cat.productivity_score));
    }
    
    // Then try partial match
    for (key, cat) in categories.iter() {
        if app_lower.contains(key) {
            return Some((cat.category, cat.subcategory, cat.productivity_score));
        }
    }
    
    // Common patterns
    if app_lower.contains("game") || app_lower.contains("play") {
        return Some(("entertainment", Some("gaming"), 10));
    }
    if app_lower.contains("code") || app_lower.contains("studio") || app_lower.contains("ide") {
        return Some(("development", Some("ide"), 90));
    }
    if app_lower.contains("chat") || app_lower.contains("messenger") {
        return Some(("communication", Some("chat"), 40));
    }
    if app_lower.contains("browser") {
        return Some(("productivity", Some("browser"), 60));
    }
    
    None
}