import { Sun, Moon, RefreshCw, Eye, Cpu } from 'lucide-react'
import ActivityChart from './ActivityChart'
import Terminal from './Terminal'
import { getThemeClasses } from '../utils/theme'
import { useEffect, useRef } from 'react'

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

interface MainContentProps {
  connectionStatus: ConnectionStatus
  currentMode: string
  isDarkMode: boolean
  onToggleTheme: () => void
  hourlySummary: HourlySummary | null
  dailySummary: DailySummary | null
  userConfig: UserConfig
  setUserConfig: (config: UserConfig) => void
  isGeneratingHourly: boolean
  isGeneratingDaily: boolean
  onGenerateHourly: () => void
  onGenerateDaily: () => void
  onSaveConfig: () => void
  activityClassification: ActivityClassification | null
  isClassifying: boolean
}

function MainContent({ 
  connectionStatus, 
  currentMode, 
  isDarkMode, 
  onToggleTheme,
  hourlySummary,
  dailySummary,
  userConfig,
  setUserConfig,
  isGeneratingHourly,
  isGeneratingDaily,
  onGenerateHourly,
  onGenerateDaily,
  onSaveConfig,
  activityClassification,
  isClassifying
}: MainContentProps) {
  const studyFocusRef = useRef<HTMLTextAreaElement>(null)

  // Auto-focus study mode text box when switching to study mode
  useEffect(() => {
    if (currentMode === 'study_buddy' && studyFocusRef.current) {
      studyFocusRef.current.focus()
    }
  }, [currentMode])

  const getModeDisplayName = (mode: string) => {
    switch (mode) {
      case 'ghost': return 'Ghost Mode'
      case 'chill': return 'Chill Mode'
      case 'study_buddy': return 'Study Mode'
      case 'coach': return 'Coach Mode'
      default: return 'Study Mode'
    }
  }

  const getContextTitle = (mode: string) => {
    switch (mode) {
      case 'study_buddy': return 'So we\'re studying for?'
      case 'coach': return 'Let\'s get it done.'
      default: return 'Something I Should Know'
    }
  }

  const getContextPlaceholder = (mode: string) => {
    switch (mode) {
      case 'study_buddy': return 'What subject or topic are you studying today?'
      case 'coach': return 'What task or goal are you working on?'
      default: return 'Tell me about your current goals, projects, or any context that will help me provide better support...'
    }
  }

  const getContextValue = (mode: string) => {
    switch (mode) {
      case 'study_buddy': return userConfig.study_focus
      case 'coach': return userConfig.coach_task
      default: return userConfig.user_context
    }
  }

  const handleContextChange = (mode: string, value: string) => {
    switch (mode) {
      case 'study_buddy':
        setUserConfig({ ...userConfig, study_focus: value })
        break
      case 'coach':
        setUserConfig({ ...userConfig, coach_task: value })
        break
      default:
        setUserConfig({ ...userConfig, user_context: value })
        break
    }
  }

  const themeClasses = getThemeClasses(currentMode, isDarkMode)

  // Helper function to clean up state display
  const cleanStateDisplay = (state: string | undefined) => {
    if (!state) return 'needs_nudge'
    return state.replace(/^\[|\]$/g, '') || 'needs_nudge'
  }

  const getStateCardTitle = (mode: string) => {
    switch (mode) {
      case 'study_buddy': return '5-minute State'
      case 'coach': return '15-minute Summary'
      default: return 'Hourly State'
    }
  }

  return (
    <div className={`flex-1 ${themeClasses.background} h-screen flex flex-col`}>
      {/* Header */}
      <div className={`p-4 ${themeClasses.background} ${themeClasses.border} border-b flex-shrink-0`}>
        <div className="flex items-center justify-between">
          <h1 className={`text-2xl font-bold ${themeClasses.textPrimary}`}>
            {getModeDisplayName(currentMode)}
          </h1>
          <div className="flex items-center space-x-3">
            {/* Connection Status Icons */}
            <div className="flex items-center space-x-2">
              <div className="flex items-center space-x-1">
                <Eye className={`w-4 h-4 ${
                  connectionStatus.activitywatch ? 'text-slate-400' : 'text-red-400'
                }`} />
                <span className="text-xs text-slate-400">AW</span>
              </div>
              <div className="flex items-center space-x-1">
                <Cpu className={`w-4 h-4 ${
                  connectionStatus.ollama ? 'text-slate-400' : 'text-red-400'
                }`} />
                <span className="text-xs text-slate-400">AI</span>
              </div>
            </div>
            
            {/* Theme Toggle */}
            <button
              onClick={onToggleTheme}
              className={`p-2 rounded-lg ${themeClasses.surfaceSecondary} hover:opacity-75 transition-colors`}
            >
              {isDarkMode ? (
                <Sun className="w-4 h-4 text-slate-400" />
              ) : (
                <Moon className="w-4 h-4 text-slate-400" />
              )}
            </button>
          </div>
        </div>
      </div>

      {/* Main Content */}
      <div className="flex-1 p-3 overflow-y-auto min-h-0">
        <div className="space-y-3 pb-6">
          {/* Mode-specific Cards Layout */}
          {currentMode === 'study_buddy' ? (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              {/* Left Column - Study Mode */}
              <div className="space-y-3">
                {/* Study Focus - First Card */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4 flex flex-col`}>
                  <div className="mb-3">
                    <h3 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-1`}>
                      So we're studying for?
                    </h3>
                    <p className={`text-sm ${themeClasses.textSecondary}`}>
                      Study Focus
                    </p>
                  </div>
                  
                  <div className="flex-1 mb-3">
                    <div className={`${themeClasses.surfaceSecondary} rounded-lg p-3 h-32`}>
                      <textarea
                        ref={studyFocusRef}
                        value={userConfig.study_focus}
                        onChange={(e) => setUserConfig({ ...userConfig, study_focus: e.target.value })}
                        placeholder="What subject or topic are you studying today?"
                        className={`w-full h-full bg-transparent ${themeClasses.textPrimary} font-mono text-sm resize-none focus:outline-none placeholder-slate-400`}
                      />
                    </div>
                  </div>
                  
                  <button
                    onClick={onSaveConfig}
                    className={`px-3 py-1 text-white rounded-lg transition-colors text-sm`}
                    style={{ backgroundColor: themeClasses.primary }}
                  >
                    Save Context
                  </button>
                </div>

                {/* Daily Summary */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4 flex flex-col`}>
                  <div className="flex items-center justify-between mb-3">
                    <h3 className={`text-lg font-semibold ${themeClasses.textPrimary}`}>
                      Daily Summary
                    </h3>
                    <button
                      onClick={onGenerateDaily}
                      disabled={isGeneratingDaily}
                      className={`px-3 py-1 text-white rounded-lg transition-colors disabled:opacity-50 flex items-center space-x-2 text-sm`}
                      style={{ backgroundColor: themeClasses.primary }}
                    >
                      {isGeneratingDaily ? (
                        <RefreshCw className="w-3 h-3 animate-spin" />
                      ) : (
                        <span>Generate</span>
                      )}
                    </button>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-3 overflow-y-auto min-h-[120px]`}>
                    <p className={`${themeClasses.textPrimary} leading-relaxed text-sm`}>
                      {dailySummary?.summary || 'Loading daily summary...'}
                    </p>
                  </div>
                </div>
              </div>

              {/* Right Column - Study Mode */}
              <div className="space-y-3">
                {/* 5-minute State - Second Card */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4 flex flex-col`}>
                  <div className="flex items-center justify-between mb-3">
                    <div>
                      <h3 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-1`}>
                        {currentMode === 'study_buddy' ? '5-minute State' : 
                         currentMode === 'coach' ? '15-minute State' : 
                         'Hourly State'}
                      </h3>
                      <p className={`text-sm ${themeClasses.textSecondary}`}>
                        {cleanStateDisplay(hourlySummary?.current_state)}
                      </p>
                    </div>
                    <button
                      onClick={onGenerateHourly}
                      disabled={isGeneratingHourly}
                      className={`px-3 py-1 text-white rounded-lg transition-colors disabled:opacity-50 flex items-center space-x-2 text-sm`}
                      style={{ backgroundColor: themeClasses.primary }}
                    >
                      {isGeneratingHourly ? (
                        <RefreshCw className="w-3 h-3 animate-spin" />
                      ) : (
                        <span>Generate</span>
                      )}
                    </button>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-3 mb-3 overflow-y-auto min-h-[120px]`}>
                    <p className={`${themeClasses.textPrimary} leading-relaxed text-sm`}>
                      {hourlySummary?.summary || 'Loading activity summary...'}
                    </p>
                  </div>
                  
                  <div className="flex justify-between items-center">
                    <span className={`text-sm ${themeClasses.textSecondary}`}>
                      Last Updated: {hourlySummary?.last_updated || '04:32'}
                    </span>
                    <span className={`text-lg font-bold`} style={{ color: themeClasses.accent }}>
                      {hourlySummary?.focus_score || 45}
                    </span>
                  </div>
                </div>

                {/* Work vs Distractions */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4`}>
                  <div className="flex items-center justify-between mb-3">
                    <h3 className={`text-lg font-semibold ${themeClasses.textPrimary}`}>
                      Work vs Distractions
                    </h3>
                    {isClassifying && (
                      <RefreshCw className="w-4 h-4 animate-spin text-slate-400" />
                    )}
                  </div>
                  <div className="h-64">
                    <ActivityChart isDarkMode={isDarkMode} classification={activityClassification} />
                  </div>
                </div>
              </div>
            </div>
          ) : currentMode === 'coach' ? (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              {/* Left Column - Coach Mode */}
              <div className="space-y-3">
                {/* Coach Task - First Card */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4 flex flex-col`}>
                  <div className="mb-3">
                    <h3 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-1`}>
                      Let's get it done.
                    </h3>
                    <p className={`text-sm ${themeClasses.textSecondary}`}>
                      Current Task
                    </p>
                  </div>
                  
                  <div className="flex-1 mb-3">
                    <div className={`${themeClasses.surfaceSecondary} rounded-lg p-3 h-32`}>
                      <textarea
                        value={userConfig.coach_task}
                        onChange={(e) => setUserConfig({ ...userConfig, coach_task: e.target.value })}
                        placeholder="What task or goal are you working on?"
                        className={`w-full h-full bg-transparent ${themeClasses.textPrimary} font-mono text-sm resize-none focus:outline-none placeholder-slate-400`}
                      />
                    </div>
                  </div>
                  
                  <button
                    onClick={onSaveConfig}
                    className={`px-3 py-1 text-white rounded-lg transition-colors text-sm`}
                    style={{ backgroundColor: themeClasses.primary }}
                  >
                    Save Context
                  </button>
                </div>

                {/* To-do List - Second Card */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4 flex flex-col`}>
                  <div className="mb-3">
                    <h3 className={`text-lg font-semibold ${themeClasses.textPrimary}`}>
                      To-do List
                    </h3>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-3 overflow-y-auto min-h-[120px]`}>
                    <div className="space-y-2">
                      {/* Placeholder todo items - these should be loaded from backend */}
                      <div className="flex items-center space-x-2">
                        <input type="checkbox" className="rounded" />
                        <span className={`text-sm ${themeClasses.textPrimary}`}>Complete project research</span>
                      </div>
                      <div className="flex items-center space-x-2">
                        <input type="checkbox" className="rounded" />
                        <span className={`text-sm ${themeClasses.textPrimary}`}>Review progress from yesterday</span>
                      </div>
                      <div className="flex items-center space-x-2">
                        <input type="checkbox" className="rounded" />
                        <span className={`text-sm ${themeClasses.textPrimary}`}>Plan next steps</span>
                      </div>
                    </div>
                  </div>
                </div>
              </div>

              {/* Right Column - Coach Mode */}
              <div className="space-y-3">
                {/* 15-minute Summary - Third Card */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4 flex flex-col`}>
                  <div className="flex items-center justify-between mb-3">
                    <div>
                      <h3 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-1`}>
                        15-minute Summary
                      </h3>
                      <p className={`text-sm ${themeClasses.textSecondary}`}>
                        Task accomplishments
                      </p>
                    </div>
                    <button
                      onClick={onGenerateHourly}
                      disabled={isGeneratingHourly}
                      className={`px-3 py-1 text-white rounded-lg transition-colors disabled:opacity-50 flex items-center space-x-2 text-sm`}
                      style={{ backgroundColor: themeClasses.primary }}
                    >
                      {isGeneratingHourly ? (
                        <RefreshCw className="w-3 h-3 animate-spin" />
                      ) : (
                        <span>Generate</span>
                      )}
                    </button>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-3 mb-3 overflow-y-auto min-h-[120px]`}>
                    <p className={`${themeClasses.textPrimary} leading-relaxed text-sm`}>
                      {hourlySummary?.summary || 'Loading activity summary...'}
                    </p>
                  </div>
                  
                  <div className="flex justify-between items-center">
                    <span className={`text-sm ${themeClasses.textSecondary}`}>
                      Last Updated: {hourlySummary?.last_updated || '04:32'}
                    </span>
                    <span className={`text-lg font-bold`} style={{ color: themeClasses.accent }}>
                      {hourlySummary?.focus_score || 45}
                    </span>
                  </div>
                </div>

                {/* Work vs Distractions */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4`}>
                  <div className="flex items-center justify-between mb-3">
                    <h3 className={`text-lg font-semibold ${themeClasses.textPrimary}`}>
                      Work vs Distractions
                    </h3>
                    {isClassifying && (
                      <RefreshCw className="w-4 h-4 animate-spin text-slate-400" />
                    )}
                  </div>
                  <div className="h-64">
                    <ActivityChart isDarkMode={isDarkMode} classification={activityClassification} />
                  </div>
                </div>
              </div>
            </div>
          ) : (
            /* Default layout for ghost and chill modes */
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              {/* Left Column */}
              <div className="space-y-3">
                {/* Hourly State */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4 flex flex-col`}>
                  <div className="flex items-center justify-between mb-3">
                    <div>
                      <h3 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-1`}>
                        {getStateCardTitle(currentMode)}
                      </h3>
                      <p className={`text-sm ${themeClasses.textSecondary}`}>
                        {cleanStateDisplay(hourlySummary?.current_state)}
                      </p>
                    </div>
                    <button
                      onClick={onGenerateHourly}
                      disabled={isGeneratingHourly}
                      className={`px-3 py-1 text-white rounded-lg transition-colors disabled:opacity-50 flex items-center space-x-2 text-sm`}
                      style={{ backgroundColor: themeClasses.primary }}
                    >
                      {isGeneratingHourly ? (
                        <RefreshCw className="w-3 h-3 animate-spin" />
                      ) : (
                        <span>Generate</span>
                      )}
                    </button>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-3 mb-3 overflow-y-auto min-h-[120px]`}>
                    <p className={`${themeClasses.textPrimary} leading-relaxed text-sm`}>
                      {hourlySummary?.summary || 'Loading activity summary...'}
                    </p>
                  </div>
                  
                  <div className="flex justify-between items-center">
                    <span className={`text-sm ${themeClasses.textSecondary}`}>
                      Last Updated: {hourlySummary?.last_updated || '04:32'}
                    </span>
                    <span className={`text-lg font-bold`} style={{ color: themeClasses.accent }}>
                      {hourlySummary?.focus_score || 45}
                    </span>
                  </div>
                </div>

                {/* Daily Summary */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4 flex flex-col`}>
                  <div className="flex items-center justify-between mb-3">
                    <h3 className={`text-lg font-semibold ${themeClasses.textPrimary}`}>
                      Daily Summary
                    </h3>
                    <button
                      onClick={onGenerateDaily}
                      disabled={isGeneratingDaily}
                      className={`px-3 py-1 text-white rounded-lg transition-colors disabled:opacity-50 flex items-center space-x-2 text-sm`}
                      style={{ backgroundColor: themeClasses.primary }}
                    >
                      {isGeneratingDaily ? (
                        <RefreshCw className="w-3 h-3 animate-spin" />
                      ) : (
                        <span>Generate</span>
                      )}
                    </button>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-3 overflow-y-auto min-h-[120px]`}>
                    <p className={`${themeClasses.textPrimary} leading-relaxed text-sm`}>
                      {dailySummary?.summary || 'Loading daily summary...'}
                    </p>
                  </div>
                </div>
              </div>

              {/* Right Column */}
              <div className="space-y-3">
                {/* Work vs Distractions */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4`}>
                  <div className="flex items-center justify-between mb-3">
                    <h3 className={`text-lg font-semibold ${themeClasses.textPrimary}`}>
                      Work vs Distractions
                    </h3>
                    {isClassifying && (
                      <RefreshCw className="w-4 h-4 animate-spin text-slate-400" />
                    )}
                  </div>
                  <div className="h-64">
                    <ActivityChart isDarkMode={isDarkMode} classification={activityClassification} />
                  </div>
                </div>

                {/* Context Input - Mode-specific */}
                <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4 flex flex-col`}>
                  <div className="mb-3">
                    <h3 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-1`}>
                      {getContextTitle(currentMode)}
                    </h3>
                    <p className={`text-sm ${themeClasses.textSecondary}`}>
                      Personal Context (passed to AI)
                    </p>
                  </div>
                  
                  <div className="flex-1 mb-3">
                    <div className={`${themeClasses.surfaceSecondary} rounded-lg p-3 h-32`}>
                      <textarea
                        value={userConfig.user_context}
                        onChange={(e) => setUserConfig({ ...userConfig, user_context: e.target.value })}
                        placeholder={getContextPlaceholder(currentMode)}
                        className={`w-full h-full bg-transparent ${themeClasses.textPrimary} font-mono text-sm resize-none focus:outline-none placeholder-slate-400`}
                      />
                    </div>
                  </div>
                  
                  <button
                    onClick={onSaveConfig}
                    className={`px-3 py-1 text-white rounded-lg transition-colors text-sm`}
                    style={{ backgroundColor: themeClasses.primary }}
                  >
                    Save Context
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* Terminal Debug */}
          <Terminal className="p-0" />
        </div>
      </div>
    </div>
  )
}

export default MainContent