import { Sun, Moon, RefreshCw, Eye, Cpu, TrendingUp, AlertCircle, Info } from 'lucide-react'
import ActivityChart from './ActivityChart'
import Terminal from './Terminal'
import { getThemeClasses } from '../utils/theme'
import { spacing, typography, stateLabels, semanticColors, elevation, borderRadius, transitions } from '../utils/designSystem'
import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'

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
  work_score?: number
  distraction_score?: number
  neutral_score?: number
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

  // Load activity classification on mount and when needed
  useEffect(() => {
    if (!activityClassification && connectionStatus.activitywatch) {
      // Activity classification is loaded by the parent App component
      // This is just a placeholder for any future logic
    }
  }, [connectionStatus.activitywatch, activityClassification])

  // Convert HourlySummary scores to ActivityClassification format
  const getActivityClassification = (): ActivityClassification | null => {
    // Always prefer the actual time-based classification
    if (activityClassification) {
      return activityClassification
    }
    
    // Fallback to scores from summary if available
    if (hourlySummary && hourlySummary.work_score !== undefined) {
      return {
        work: hourlySummary.work_score || 0,
        communication: hourlySummary.neutral_score || 0,
        distraction: hourlySummary.distraction_score || 0
      }
    }
    
    return null
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

  // Helper function to get human-readable state
  const getStateInfo = (state: string | undefined) => {
    const cleanState = state?.replace(/^\[|\]$/g, '') || 'unknown'
    return stateLabels[cleanState as keyof typeof stateLabels] || stateLabels.unknown
  }

  const getStateCardTitle = (mode: string) => {
    switch (mode) {
      case 'study_buddy': return '5-minute State'
      case 'coach': return '15-minute Summary'
      default: return 'Hourly State'
    }
  }

  return (
    <div className={`flex-1 ${themeClasses.background} h-full overflow-hidden flex flex-col`}>
      {/* Header */}
      <div className={`p-4 ${themeClasses.surface} ${themeClasses.border} border-b flex-shrink-0`}>
        <div className="flex items-center justify-between">
          <div className="flex items-center space-x-3">
            <h1 style={{ fontSize: typography.h1.fontSize, fontWeight: typography.h1.fontWeight, lineHeight: typography.h1.lineHeight }} className={`${themeClasses.textPrimary}`}>
              {getModeDisplayName(currentMode)}
            </h1>
          </div>
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
              className={`w-8 h-8 rounded-full ${isDarkMode ? 'bg-white/10 hover:bg-white/20' : 'bg-gray-200 hover:bg-gray-300'} transition-all duration-200 flex items-center justify-center`}
            >
              {isDarkMode ? (
                <Sun className="w-4 h-4 text-gray-400" />
              ) : (
                <Moon className="w-4 h-4 text-gray-600" />
              )}
            </button>
          </div>
        </div>
      </div>

      {/* Main Content */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden min-h-0" style={{ padding: spacing.md }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: spacing.md, paddingBottom: spacing.md }} className="animate-fade-in">
          {/* Mode-specific Cards Layout */}
          {currentMode === 'study_buddy' ? (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              {/* Left Column - Study Mode */}
              <div className="space-y-3">
                {/* Study Focus - First Card */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="mb-3">
                    <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary} mb-1`}>
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
                    className={`px-4 py-2 text-white rounded-lg transition-all duration-150 text-sm font-semibold hover:scale-105 ${isDarkMode ? '' : 'shadow-sm'}`}
                    style={{ 
                      backgroundColor: themeClasses.primary,
                      backgroundImage: 'linear-gradient(180deg, rgba(255,255,255,0.1) 0%, transparent 100%)'
                    }}
                  >
                    Save Context
                  </button>
                </div>

                {/* Daily Summary */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="flex items-center justify-between mb-3">
                    <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary}`}>
                      Daily Summary
                    </h3>
                    <button
                      onClick={onGenerateDaily}
                      disabled={isGeneratingDaily}
                      className={`px-4 py-2 text-white rounded-lg transition-all duration-150 disabled:opacity-50 flex items-center space-x-2 text-sm font-semibold hover:scale-105 ${isDarkMode ? '' : 'shadow-sm'}`}
                      style={{ 
                        backgroundColor: themeClasses.primary,
                        backgroundImage: 'linear-gradient(180deg, rgba(255,255,255,0.1) 0%, transparent 100%)'
                      }}
                    >
                      {isGeneratingDaily ? (
                        <RefreshCw className="w-3 h-3 animate-spin" />
                      ) : (
                        <span>Generate</span>
                      )}
                    </button>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-4 overflow-y-auto min-h-[120px]`}>
                    {isGeneratingDaily ? (
                      <div className="space-y-2">
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" />
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" style={{ width: '85%' }} />
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" style={{ width: '95%' }} />
                      </div>
                    ) : (
                      <p className={`${themeClasses.textPrimary} leading-relaxed text-sm animate-fade-in`}>
                        {dailySummary?.summary || 'Generate a daily summary to see your productivity overview'}
                      </p>
                    )}
                  </div>
                </div>
              </div>

              {/* Right Column - Study Mode */}
              <div className="space-y-3">
                {/* 5-minute State - Second Card */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="flex items-center justify-between mb-3">
                    <div>
                      <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary} mb-1`}>
                        {currentMode === 'study_buddy' ? '5-minute State' : 
                         currentMode === 'coach' ? '15-minute State' : 
                         'Hourly State'}
                      </h3>
                      <div className="inline-flex items-center space-x-2 px-3 py-1 rounded-full bg-white/[0.08]">
                        <div className="w-2 h-2 rounded-full" style={{ backgroundColor: getStateInfo(hourlySummary?.current_state).color }} />
                        <p className={`text-xs font-medium ${themeClasses.textSecondary} uppercase tracking-wider`}>
                          {getStateInfo(hourlySummary?.current_state).label}
                        </p>
                      </div>
                    </div>
                    <button
                      onClick={onGenerateHourly}
                      disabled={isGeneratingHourly}
                      className={`px-4 py-2 text-white rounded-lg transition-all duration-150 disabled:opacity-50 flex items-center space-x-2 text-sm font-semibold hover:scale-105 ${isDarkMode ? '' : 'shadow-sm'}`}
                      style={{ 
                        backgroundColor: themeClasses.primary,
                        backgroundImage: 'linear-gradient(180deg, rgba(255,255,255,0.1) 0%, transparent 100%)'
                      }}
                    >
                      {isGeneratingHourly ? (
                        <RefreshCw className="w-3 h-3 animate-spin" />
                      ) : (
                        <span>Generate</span>
                      )}
                    </button>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-4 mb-3 overflow-y-auto min-h-[120px]`}>
                    {isGeneratingHourly ? (
                      <div className="space-y-2">
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" />
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" style={{ width: '90%' }} />
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" style={{ width: '75%' }} />
                      </div>
                    ) : (
                      <p className={`${themeClasses.textPrimary} leading-relaxed text-sm animate-fade-in`}>
                        {hourlySummary?.summary || 'Click generate to analyze your recent activity'}
                      </p>
                    )}
                  </div>
                  
                  <div className="flex justify-between items-center">
                    <span className={`text-sm ${themeClasses.textSecondary}`}>
                      Last Updated: {hourlySummary?.last_updated || '04:32'}
                    </span>
                    <div className="flex items-center space-x-2">
                      <span className={`text-3xl font-bold tabular-nums`} style={{ color: themeClasses.accent }}>
                        {hourlySummary?.focus_score || 45}
                      </span>
                      <div className="group relative">
                        <Info className="w-4 h-4 text-gray-400 cursor-help" />
                        <div className="absolute bottom-full left-1/2 transform -translate-x-1/2 mb-2 w-48 p-2 bg-gray-800 text-white text-xs rounded-lg opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
                          Productivity score based on app usage patterns
                        </div>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Work vs Distractions */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="flex items-center justify-between mb-3">
                    <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary}`}>
                      Work vs Distractions
                    </h3>
                    {isClassifying && (
                      <RefreshCw className="w-4 h-4 animate-spin text-slate-400" />
                    )}
                  </div>
                  <div className="flex-1">
                    <ActivityChart isDarkMode={isDarkMode} classification={getActivityClassification()} />
                  </div>
                </div>
              </div>
            </div>
          ) : currentMode === 'coach' ? (
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              {/* Left Column - Coach Mode */}
              <div className="space-y-3">
                {/* Coach Task - First Card */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="mb-3">
                    <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary} mb-1`}>
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
                    className={`px-4 py-2 text-white rounded-lg transition-all duration-150 text-sm font-semibold hover:scale-105 ${isDarkMode ? '' : 'shadow-sm'}`}
                    style={{ 
                      backgroundColor: themeClasses.primary,
                      backgroundImage: 'linear-gradient(180deg, rgba(255,255,255,0.1) 0%, transparent 100%)'
                    }}
                  >
                    Save Context
                  </button>
                </div>

                {/* To-do List - Second Card */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="mb-3">
                    <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary}`}>
                      To-do List
                    </h3>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-4 overflow-y-auto min-h-[120px]`}>
                    <div className="space-y-2">
                      {/* Placeholder todo items - these should be loaded from backend */}
                      <div className="flex items-center space-x-2 group">
                        <input type="checkbox" className="rounded border-gray-400 text-orange-500 focus:ring-orange-500" />
                        <span className={`text-sm ${themeClasses.textPrimary} group-hover:text-opacity-80`}>Complete project research</span>
                      </div>
                      <div className="flex items-center space-x-2 group">
                        <input type="checkbox" className="rounded border-gray-400 text-orange-500 focus:ring-orange-500" />
                        <span className={`text-sm ${themeClasses.textPrimary} group-hover:text-opacity-80`}>Review progress from yesterday</span>
                      </div>
                      <div className="flex items-center space-x-2 group">
                        <input type="checkbox" className="rounded border-gray-400 text-orange-500 focus:ring-orange-500" />
                        <span className={`text-sm ${themeClasses.textPrimary} group-hover:text-opacity-80`}>Plan next steps</span>
                      </div>
                    </div>
                  </div>
                </div>
              </div>

              {/* Right Column - Coach Mode */}
              <div className="space-y-3">
                {/* 15-minute Summary - Third Card */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="flex items-center justify-between mb-3">
                    <div>
                      <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary} mb-1`}>
                        15-minute Summary
                      </h3>
                      <p className={`text-sm ${themeClasses.textSecondary}`}>
                        Task accomplishments
                      </p>
                    </div>
                    <button
                      onClick={onGenerateHourly}
                      disabled={isGeneratingHourly}
                      className={`px-4 py-2 text-white rounded-lg transition-all duration-150 disabled:opacity-50 flex items-center space-x-2 text-sm font-semibold hover:scale-105 ${isDarkMode ? '' : 'shadow-sm'}`}
                      style={{ 
                        backgroundColor: themeClasses.primary,
                        backgroundImage: 'linear-gradient(180deg, rgba(255,255,255,0.1) 0%, transparent 100%)'
                      }}
                    >
                      {isGeneratingHourly ? (
                        <RefreshCw className="w-3 h-3 animate-spin" />
                      ) : (
                        <span>Generate</span>
                      )}
                    </button>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-4 mb-3 overflow-y-auto min-h-[120px]`}>
                    {isGeneratingHourly ? (
                      <div className="space-y-2">
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" />
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" style={{ width: '90%' }} />
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" style={{ width: '75%' }} />
                      </div>
                    ) : (
                      <p className={`${themeClasses.textPrimary} leading-relaxed text-sm animate-fade-in`}>
                        {hourlySummary?.summary || 'Click generate to analyze your recent activity'}
                      </p>
                    )}
                  </div>
                  
                  <div className="flex justify-between items-center">
                    <span className={`text-sm ${themeClasses.textSecondary}`}>
                      Last Updated: {hourlySummary?.last_updated || '04:32'}
                    </span>
                    <div className="flex items-center space-x-2">
                      <span className={`text-3xl font-bold tabular-nums`} style={{ color: themeClasses.accent }}>
                        {hourlySummary?.focus_score || 45}
                      </span>
                      <div className="group relative">
                        <Info className="w-4 h-4 text-gray-400 cursor-help" />
                        <div className="absolute bottom-full left-1/2 transform -translate-x-1/2 mb-2 w-48 p-2 bg-gray-800 text-white text-xs rounded-lg opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
                          Productivity score based on app usage patterns
                        </div>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Work vs Distractions */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="flex items-center justify-between mb-3">
                    <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary}`}>
                      Work vs Distractions
                    </h3>
                    {isClassifying && (
                      <RefreshCw className="w-4 h-4 animate-spin text-slate-400" />
                    )}
                  </div>
                  <div className="flex-1">
                    <ActivityChart isDarkMode={isDarkMode} classification={getActivityClassification()} />
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
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="flex items-center justify-between mb-3">
                    <div>
                      <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary} mb-1`}>
                        {getStateCardTitle(currentMode)}
                      </h3>
                      <div className="inline-flex items-center space-x-2 px-3 py-1 rounded-full bg-white/[0.08]">
                        <div className="w-2 h-2 rounded-full" style={{ backgroundColor: getStateInfo(hourlySummary?.current_state).color }} />
                        <p className={`text-xs font-medium ${themeClasses.textSecondary} uppercase tracking-wider`}>
                          {getStateInfo(hourlySummary?.current_state).label}
                        </p>
                      </div>
                    </div>
                    <button
                      onClick={onGenerateHourly}
                      disabled={isGeneratingHourly}
                      className={`px-4 py-2 text-white rounded-lg transition-all duration-150 disabled:opacity-50 flex items-center space-x-2 text-sm font-semibold hover:scale-105 ${isDarkMode ? '' : 'shadow-sm'}`}
                      style={{ 
                        backgroundColor: themeClasses.primary,
                        backgroundImage: 'linear-gradient(180deg, rgba(255,255,255,0.1) 0%, transparent 100%)'
                      }}
                    >
                      {isGeneratingHourly ? (
                        <RefreshCw className="w-3 h-3 animate-spin" />
                      ) : (
                        <span>Generate</span>
                      )}
                    </button>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-4 mb-3 overflow-y-auto min-h-[120px]`}>
                    {isGeneratingHourly ? (
                      <div className="space-y-2">
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" />
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" style={{ width: '90%' }} />
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" style={{ width: '75%' }} />
                      </div>
                    ) : (
                      <p className={`${themeClasses.textPrimary} leading-relaxed text-sm animate-fade-in`}>
                        {hourlySummary?.summary || 'Click generate to analyze your recent activity'}
                      </p>
                    )}
                  </div>
                  
                  <div className="flex justify-between items-center">
                    <span className={`text-sm ${themeClasses.textSecondary}`}>
                      Last Updated: {hourlySummary?.last_updated || '04:32'}
                    </span>
                    <div className="flex items-center space-x-2">
                      <span className={`text-3xl font-bold tabular-nums`} style={{ color: themeClasses.accent }}>
                        {hourlySummary?.focus_score || 45}
                      </span>
                      <div className="group relative">
                        <Info className="w-4 h-4 text-gray-400 cursor-help" />
                        <div className="absolute bottom-full left-1/2 transform -translate-x-1/2 mb-2 w-48 p-2 bg-gray-800 text-white text-xs rounded-lg opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
                          Productivity score based on app usage patterns
                        </div>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Daily Summary */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="flex items-center justify-between mb-3">
                    <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary}`}>
                      Daily Summary
                    </h3>
                    <button
                      onClick={onGenerateDaily}
                      disabled={isGeneratingDaily}
                      className={`px-4 py-2 text-white rounded-lg transition-all duration-150 disabled:opacity-50 flex items-center space-x-2 text-sm font-semibold hover:scale-105 ${isDarkMode ? '' : 'shadow-sm'}`}
                      style={{ 
                        backgroundColor: themeClasses.primary,
                        backgroundImage: 'linear-gradient(180deg, rgba(255,255,255,0.1) 0%, transparent 100%)'
                      }}
                    >
                      {isGeneratingDaily ? (
                        <RefreshCw className="w-3 h-3 animate-spin" />
                      ) : (
                        <span>Generate</span>
                      )}
                    </button>
                  </div>
                  
                  <div className={`flex-1 ${themeClasses.surfaceSecondary} rounded-lg p-4 overflow-y-auto min-h-[120px]`}>
                    {isGeneratingDaily ? (
                      <div className="space-y-2">
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" />
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" style={{ width: '85%' }} />
                        <div className="h-4 bg-gray-300 dark:bg-gray-700 rounded animate-pulse" style={{ width: '95%' }} />
                      </div>
                    ) : (
                      <p className={`${themeClasses.textPrimary} leading-relaxed text-sm animate-fade-in`}>
                        {dailySummary?.summary || 'Generate a daily summary to see your productivity overview'}
                      </p>
                    )}
                  </div>
                </div>
              </div>

              {/* Right Column */}
              <div className="space-y-3">
                {/* Work vs Distractions */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="flex items-center justify-between mb-3">
                    <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary}`}>
                      Work vs Distractions
                    </h3>
                    {isClassifying && (
                      <RefreshCw className="w-4 h-4 animate-spin text-slate-400" />
                    )}
                  </div>
                  <div className="flex-1">
                    <ActivityChart isDarkMode={isDarkMode} classification={getActivityClassification()} />
                  </div>
                </div>

                {/* Context Input - Mode-specific */}
                <div className={`${themeClasses.surface} rounded-xl p-5 flex flex-col ${isDarkMode ? 'shadow-lg border border-white/[0.08]' : 'shadow-md'}`}>
                  <div className="mb-3">
                    <h3 style={{ fontSize: typography.h3.fontSize, fontWeight: typography.h3.fontWeight, lineHeight: typography.h3.lineHeight }} className={`${themeClasses.textPrimary} mb-1`}>
                      {getContextTitle(currentMode)}
                    </h3>
                    <p className={`text-sm ${themeClasses.textSecondary}`}>
                      Personal Context (passed to AI)
                    </p>
                  </div>
                  
                  <div className="flex-1 mb-3">
                    <div className={`${themeClasses.surfaceSecondary} rounded-lg p-3 h-32`}>
                      <textarea
                        value={getContextValue(currentMode)}
                        onChange={(e) => handleContextChange(currentMode, e.target.value)}
                        placeholder={getContextPlaceholder(currentMode)}
                        className={`w-full h-full bg-transparent ${themeClasses.textPrimary} font-mono text-sm resize-none focus:outline-none placeholder-slate-400`}
                      />
                    </div>
                  </div>
                  
                  <button
                    onClick={onSaveConfig}
                    className={`px-4 py-2 text-white rounded-lg transition-all duration-150 text-sm font-semibold hover:scale-105 ${isDarkMode ? '' : 'shadow-sm'}`}
                    style={{ 
                      backgroundColor: themeClasses.primary,
                      backgroundImage: 'linear-gradient(180deg, rgba(255,255,255,0.1) 0%, transparent 100%)'
                    }}
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