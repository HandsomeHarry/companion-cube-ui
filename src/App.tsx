import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import MainContent from './components/MainContent'
import Sidebar from './components/Sidebar'
import Settings from './components/Settings'
import { getThemeClasses } from './utils/theme'

interface ConnectionStatus {
  activitywatch: boolean
  ollama: boolean
}

interface HourlySummary {
  summary: string
  focus_score: number
  last_updated: string
  period: string
  current_state: string
}

interface DailySummary {
  date: string
  summary: string
  total_active_time: number
  top_applications: string[]
  total_sessions: number
  generated_at: string
}

interface UserConfig {
  user_context: string
  activitywatch_port: number
  ollama_port: number
  // Mode-specific contexts
  study_focus: string
  coach_task: string
  // Notification prompts
  chill_notification_prompt: string
  study_notification_prompt: string
  coach_notification_prompt: string
  // Notification settings
  notifications_enabled: boolean
  notification_webhook?: string
}

interface ActivityClassification {
  work: number
  communication: number
  distraction: number
}

function App() {
  const [isDarkMode, setIsDarkMode] = useState(true)
  const [currentMode, setCurrentMode] = useState('coach')
  const [currentPage, setCurrentPage] = useState('home')
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>({
    activitywatch: true,
    ollama: true
  })
  
  // Persistent summary data
  const [hourlySummary, setHourlySummary] = useState<HourlySummary | null>(null)
  const [dailySummary, setDailySummary] = useState<DailySummary | null>(null)
  const [userConfig, setUserConfig] = useState<UserConfig>({ 
    user_context: '', 
    activitywatch_port: 5600, 
    ollama_port: 11434,
    study_focus: '',
    coach_task: '',
    chill_notification_prompt: "Hey! You've been having fun for a while now. Maybe it's time to take a break or switch to something productive? 🌟",
    study_notification_prompt: "Looks like you got distracted from studying. Let's get back on track! 📚",
    coach_notification_prompt: "Time to check your progress! Please review and update your todo list. ✓",
    notifications_enabled: true,
    notification_webhook: undefined
  })
  const [isGeneratingHourly, setIsGeneratingHourly] = useState(false)
  const [isGeneratingDaily, setIsGeneratingDaily] = useState(false)
  const [activityClassification, setActivityClassification] = useState<ActivityClassification | null>(null)
  const [isClassifying, setIsClassifying] = useState(false)

  useEffect(() => {
    if (isDarkMode) {
      document.documentElement.classList.add('dark')
    } else {
      document.documentElement.classList.remove('dark')
    }
  }, [isDarkMode])

  // Load initial data
  useEffect(() => {
    const loadData = async () => {
      try {
        const [hourlyData, dailyData, config] = await Promise.all([
          invoke('get_hourly_summary'),
          invoke('generate_daily_summary_command'),
          invoke('load_user_config')
        ])
        
        setHourlySummary(hourlyData as HourlySummary)
        setDailySummary(dailyData as DailySummary)
        setUserConfig(config as UserConfig)
        
        // Auto-load activity classification on app start
        handleClassifyActivities()
      } catch (error) {
        console.error('Failed to load data:', error)
      }
    }

    loadData()
  }, [])

  useEffect(() => {
    const checkConnections = async () => {
      try {
        const result = await invoke('check_connections')
        setConnectionStatus(result as ConnectionStatus)
      } catch (error) {
        console.error('Failed to check connections:', error)
        // On error, set both to false to indicate connection issues
        setConnectionStatus({
          activitywatch: false,
          ollama: false
        })
      }
    }

    checkConnections()
    const interval = setInterval(checkConnections, 30000)

    return () => clearInterval(interval)
  }, [])

  useEffect(() => {
    const getCurrentMode = async () => {
      try {
        const mode = await invoke('get_current_mode')
        setCurrentMode(mode as string)
      } catch (error) {
        console.error('Failed to get current mode:', error)
      }
    }

    getCurrentMode()

    const unlisten = listen('set_mode', (event) => {
      setCurrentMode(event.payload as string)
    })

    return () => {
      unlisten.then(f => f())
    }
  }, [])

  // Listen for hourly summary updates from background timer
  useEffect(() => {
    const unlistenSummary = listen('hourly_summary_updated', (event) => {
      console.log('Received hourly summary update:', event.payload)
      const summary = event.payload as HourlySummary
      console.log('Setting hourly summary:', summary)
      setHourlySummary(summary)
      // Force a re-render by updating a dummy state if needed
      console.log('Current hourly summary state after update:', summary)
    })
    
    // Also periodically sync with backend every 30 seconds
    const syncInterval = setInterval(async () => {
      try {
        const hourlyData = await invoke('get_hourly_summary')
        setHourlySummary(hourlyData as HourlySummary)
      } catch (error) {
        console.error('Failed to sync hourly summary:', error)
      }
    }, 30000)

    const unlistenNotification = listen('show_notification', (event) => {
      console.log('Received notification:', event.payload)
      // Could show in-app notification here if desired
    })

    // Also listen for mode changes to refresh summary
    const unlistenModeChange = listen('mode_changed', async (event) => {
      console.log('Mode changed, refreshing summary...')
      try {
        const hourlyData = await invoke('get_hourly_summary')
        setHourlySummary(hourlyData as HourlySummary)
      } catch (error) {
        console.error('Failed to refresh summary after mode change:', error)
      }
    })

    return () => {
      unlistenSummary.then(f => f())
      unlistenNotification.then(f => f())
      unlistenModeChange.then(f => f())
      clearInterval(syncInterval)
    }
  }, [])

  const toggleTheme = () => {
    setIsDarkMode(!isDarkMode)
  }

  const handleModeChange = async (mode: string) => {
    try {
      await invoke('set_mode', { mode })
      setCurrentMode(mode)
    } catch (error) {
      console.error('Failed to set mode:', error)
    }
  }

  const handleGenerateHourly = async () => {
    setIsGeneratingHourly(true)
    try {
      const result = await invoke('generate_hourly_summary')
      setHourlySummary(result as HourlySummary)
      
      // Auto-generate activity classification when hourly summary is generated
      handleClassifyActivities()
    } catch (error) {
      console.error('Failed to generate hourly summary:', error)
    } finally {
      setIsGeneratingHourly(false)
    }
  }

  const handleGenerateDaily = async () => {
    setIsGeneratingDaily(true)
    try {
      const result = await invoke('generate_daily_summary_command')
      setDailySummary(result as DailySummary)
    } catch (error) {
      console.error('Failed to generate daily summary:', error)
    } finally {
      setIsGeneratingDaily(false)
    }
  }

  const handleSaveConfig = async () => {
    try {
      await invoke('save_user_config', { config: userConfig })
      console.log('Configuration saved successfully')
    } catch (error) {
      console.error('Failed to save configuration:', error)
    }
  }

  const handleClassifyActivities = async () => {
    setIsClassifying(true)
    try {
      const result = await invoke('classify_activities')
      setActivityClassification(result as ActivityClassification)
    } catch (error) {
      console.error('Failed to classify activities:', error)
    } finally {
      setIsClassifying(false)
    }
  }

  const renderContent = () => {
    switch (currentPage) {
      case 'settings':
        return <Settings isDarkMode={isDarkMode} currentMode={currentMode} />
      case 'home':
      default:
        return (
          <MainContent 
            connectionStatus={connectionStatus}
            currentMode={currentMode}
            isDarkMode={isDarkMode}
            onToggleTheme={toggleTheme}
            hourlySummary={hourlySummary}
            dailySummary={dailySummary}
            userConfig={userConfig}
            setUserConfig={setUserConfig}
            isGeneratingHourly={isGeneratingHourly}
            isGeneratingDaily={isGeneratingDaily}
            onGenerateHourly={handleGenerateHourly}
            onGenerateDaily={handleGenerateDaily}
            onSaveConfig={handleSaveConfig}
            activityClassification={activityClassification}
            isClassifying={isClassifying}
          />
        )
    }
  }

  const themeClasses = getThemeClasses(currentMode, isDarkMode)

  return (
    <div className={`min-h-screen ${isDarkMode ? 'dark' : ''}`}>
      <div className={`flex h-screen ${themeClasses.background}`}>
        <Sidebar 
          currentMode={currentMode} 
          onModeChange={handleModeChange}
          isDarkMode={isDarkMode}
          onToggleTheme={toggleTheme}
          currentPage={currentPage}
          onPageChange={setCurrentPage}
        />
        {renderContent()}
      </div>
    </div>
  )
}

export default App