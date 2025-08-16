import { useState, useEffect } from 'react'
import { Settings as SettingsIcon, Download, PlayCircle, Save, RefreshCw, ExternalLink, Search, Trash2, ChevronDown, ChevronUp } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'
import { getThemeClasses } from '../utils/theme'

interface ConnectionStatus {
  activitywatch: boolean
  ollama: boolean
}

interface SettingsProps {
  isDarkMode: boolean
  currentMode: string
  connectionStatus: ConnectionStatus
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
  ollama_model: string
  keep_model_loaded: boolean
}

interface AppCategory {
  app_name: string
  category: string
  subcategory?: string
  productivity_score: number
}

function Settings({ isDarkMode, currentMode, connectionStatus }: SettingsProps) {
  const themeClasses = getThemeClasses(currentMode, isDarkMode)
  const [config, setConfig] = useState<UserConfig>({
    user_context: '',
    activitywatch_port: 5600,
    ollama_port: 11434,
    study_focus: '',
    coach_task: '',
    chill_notification_prompt: "Hey! You've been having fun for a while now. Maybe it's time to take a break or switch to something productive? 🌟",
    study_notification_prompt: "Looks like you got distracted from studying. Let's get back on track! 📚",
    coach_notification_prompt: "Time to check your progress! Please review and update your todo list. ✓",
    notifications_enabled: true,
    notification_webhook: undefined,
    ollama_model: 'mistral',
    keep_model_loaded: false
  })
  const [isLoading, setIsLoading] = useState(false)
  const [isInstallLoading, setIsInstallLoading] = useState({
    ollama: false,
    activitywatch: false
  })
  const [availableModels, setAvailableModels] = useState<string[]>(['mistral'])
  const [isLoadingModels, setIsLoadingModels] = useState(false)
  const [appCategories, setAppCategories] = useState<AppCategory[]>([])
  const [isLoadingCategories, setIsLoadingCategories] = useState(false)
  const [editedCategories, setEditedCategories] = useState<Map<string, AppCategory>>(new Map())
  const [categorySearch, setCategorySearch] = useState('')
  const [originalConfig, setOriginalConfig] = useState<UserConfig | null>(null)
  const [hasUnsavedChanges, setHasUnsavedChanges] = useState(false)
  const [isPromptConfigOpen, setIsPromptConfigOpen] = useState(false)

  // Load config on mount
  useEffect(() => {
    const loadConfig = async () => {
      try {
        const userConfig = await invoke('load_user_config') as UserConfig
        setConfig(userConfig)
        setOriginalConfig(userConfig)
      } catch (error) {
        console.error('Failed to load config:', error)
      }
    }
    loadConfig()
  }, [])

  // Check for unsaved changes
  useEffect(() => {
    if (originalConfig) {
      const changed = JSON.stringify(config) !== JSON.stringify(originalConfig) || editedCategories.size > 0
      setHasUnsavedChanges(changed)
    }
  }, [config, originalConfig, editedCategories])

  // Load available models when Ollama is connected
  useEffect(() => {
    const loadModels = async () => {
      if (connectionStatus.ollama && !isLoadingModels) {
        setIsLoadingModels(true)
        try {
          const models = await invoke('get_ollama_models') as string[]
          setAvailableModels(models.length > 0 ? models : ['mistral'])
        } catch (error) {
          console.error('Failed to load Ollama models:', error)
          setAvailableModels(['mistral'])
        } finally {
          setIsLoadingModels(false)
        }
      }
    }
    loadModels()
  }, [connectionStatus.ollama])

  // Load app categories
  useEffect(() => {
    const loadCategories = async () => {
      setIsLoadingCategories(true)
      try {
        const categories = await invoke('get_app_categories') as AppCategory[]
        setAppCategories(categories)
      } catch (error) {
        console.error('Failed to load app categories:', error)
      } finally {
        setIsLoadingCategories(false)
      }
    }
    loadCategories()
  }, [])

  const handleSaveConfig = async () => {
    setIsLoading(true)
    try {
      // Save config
      await invoke('save_user_config', { config })
      
      // Save categories if any were edited
      if (editedCategories.size > 0) {
        const updates = Array.from(editedCategories.values())
        await invoke('bulk_update_categories', { updates })
        setEditedCategories(new Map())
      }
      
      setOriginalConfig(config)
      setHasUnsavedChanges(false)
      console.log('Settings saved successfully')
    } catch (error) {
      console.error('Failed to save settings:', error)
    } finally {
      setIsLoading(false)
    }
  }

  const handleInstallOllama = async () => {
    setIsInstallLoading(prev => ({ ...prev, ollama: true }))
    try {
      const result = await invoke('install_ollama') as string
      console.log('Ollama installation result:', result)
      alert(result)
    } catch (error) {
      console.error('Failed to install Ollama:', error)
      alert('Failed to install Ollama: ' + error)
    } finally {
      setIsInstallLoading(prev => ({ ...prev, ollama: false }))
    }
  }

  const handleInstallActivityWatch = async () => {
    setIsInstallLoading(prev => ({ ...prev, activitywatch: true }))
    try {
      const result = await invoke('install_activitywatch') as string
      console.log('ActivityWatch installation result:', result)
      alert(result)
    } catch (error) {
      console.error('Failed to install ActivityWatch:', error)
      alert('Failed to install ActivityWatch: ' + error)
    } finally {
      setIsInstallLoading(prev => ({ ...prev, activitywatch: false }))
    }
  }

  const handleTestNotification = async () => {
    try {
      await invoke('test_notification')
      alert('Test notification sent successfully!')
    } catch (error) {
      console.error('Failed to send test notification:', error)
      alert('Failed to send test notification: ' + error)
    }
  }

  const handleCategoryChange = (appName: string, field: keyof AppCategory, value: string | number) => {
    const existing = appCategories.find(cat => cat.app_name === appName) || editedCategories.get(appName)
    if (existing) {
      const updated = { ...existing, [field]: value }
      setEditedCategories(new Map(editedCategories.set(appName, updated)))
    }
  }

  const handleSaveCategories = async () => {
    if (editedCategories.size === 0) return
    
    setIsLoading(true)
    try {
      const updates = Array.from(editedCategories.values())
      await invoke('bulk_update_categories', { updates })
      
      // Reload categories to reflect saved changes
      const categories = await invoke('get_app_categories') as AppCategory[]
      setAppCategories(categories)
      setEditedCategories(new Map())
      
      console.log('Categories saved successfully')
    } catch (error) {
      console.error('Failed to save categories:', error)
      alert('Failed to save categories: ' + error)
    } finally {
      setIsLoading(false)
    }
  }

  const filteredCategories = appCategories.filter(cat => 
    cat.app_name.toLowerCase().includes(categorySearch.toLowerCase()) ||
    cat.category.toLowerCase().includes(categorySearch.toLowerCase())
  )

  return (
    <div className={`flex-1 ${themeClasses.background} h-full overflow-hidden flex flex-col`}>
      {/* Header */}
      <div className={`p-6 ${themeClasses.background} ${themeClasses.border} border-b flex-shrink-0`}>
        <div className="flex items-center space-x-3">
          <h1 className={`text-3xl font-bold ${themeClasses.textPrimary}`}>Settings</h1>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 p-8 overflow-y-auto overflow-x-hidden min-h-0">
        <div className="space-y-8 pb-8 max-w-4xl mx-auto">

          {/* Port Configuration */}
          <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4`}>
            <h2 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-4`}>
              Port Configuration
            </h2>
            <p className={`${themeClasses.textSecondary} mb-4 text-sm`}>
              Configure the ports for ActivityWatch and Ollama services.
            </p>
            
            <div className="space-y-4">
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                <div>
                  <label className={`block text-sm font-medium ${themeClasses.textPrimary} mb-2`}>
                    ActivityWatch Port
                  </label>
                  <input
                    type="number"
                    value={config.activitywatch_port}
                    onChange={(e) => setConfig({...config, activitywatch_port: parseInt(e.target.value) || 5600})}
                    className={`w-full px-3 py-2 ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500`}
                    placeholder="5600"
                    min="1024"
                    max="65535"
                  />
                  <p className={`mt-1 text-xs ${themeClasses.textSecondary}`}>
                    Default: 5600
                  </p>
                </div>
                
                <div>
                  <label className={`block text-sm font-medium ${themeClasses.textPrimary} mb-2`}>
                    Ollama Port
                  </label>
                  <input
                    type="number"
                    value={config.ollama_port}
                    onChange={(e) => setConfig({...config, ollama_port: parseInt(e.target.value) || 11434})}
                    className={`w-full px-3 py-2 ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500`}
                    placeholder="11434"
                    min="1024"
                    max="65535"
                  />
                  <p className={`mt-1 text-xs ${themeClasses.textSecondary}`}>
                    Default: 11434
                  </p>
                </div>
              </div>
              
              {/* Ollama Model Selection */}
              <div className="space-y-4">
                <div>
                  <label className={`block text-sm font-medium mb-2 ${themeClasses.textPrimary}`}>
                    Ollama Model
                  </label>
                  <div className="flex items-center space-x-2">
                    <select
                      value={config.ollama_model}
                      onChange={(e) => setConfig({...config, ollama_model: e.target.value})}
                      disabled={!connectionStatus.ollama || isLoadingModels}
                      className={`flex-1 px-3 py-2 ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500 ${(!connectionStatus.ollama || isLoadingModels) ? 'opacity-50 cursor-not-allowed' : ''}`}
                    >
                      {availableModels.map(model => (
                        <option key={model} value={model}>{model}</option>
                      ))}
                    </select>
                    <button
                      onClick={async () => {
                        setIsLoadingModels(true)
                        try {
                          const models = await invoke('get_ollama_models') as string[]
                          setAvailableModels(models.length > 0 ? models : ['mistral'])
                        } catch (error) {
                          console.error('Failed to refresh models:', error)
                        } finally {
                          setIsLoadingModels(false)
                        }
                      }}
                      disabled={!connectionStatus.ollama || isLoadingModels}
                      className={`w-10 h-10 ${isDarkMode ? themeClasses.surfaceSecondary : 'bg-gray-200 hover:bg-gray-300'} ${themeClasses.textPrimary} rounded-lg transition-colors ${(!connectionStatus.ollama || isLoadingModels) ? 'opacity-50 cursor-not-allowed' : ''} flex items-center justify-center`}
                      title="Refresh models"
                    >
                      <RefreshCw className={`w-4 h-4 ${isLoadingModels ? 'animate-spin' : ''}`} />
                    </button>
                  </div>
                  <p className={`mt-1 text-xs ${themeClasses.textSecondary}`}>
                    {connectionStatus.ollama 
                      ? `Select the LLM model to use. Default: mistral`
                      : `Connect to Ollama to see available models`}
                  </p>
                  <button
                    onClick={async () => {
                      try {
                        const loaded = await invoke('get_loaded_ollama_model') as string
                        alert(`Ollama Status:\n${loaded}`)
                      } catch (error) {
                        console.error('Failed to check loaded model:', error)
                        alert('Failed to check loaded model')
                      }
                    }}
                    className={`mt-2 px-4 py-2 ${isDarkMode ? themeClasses.surfaceSecondary : 'bg-gray-200 hover:bg-gray-300'} ${themeClasses.textPrimary} rounded-lg transition-colors text-sm`}
                  >
                    Check Loaded Model
                  </button>
                  
                  {/* Keep Model Loaded Toggle */}
                  <div className="mt-4 flex items-center justify-between">
                    <div className="flex-1">
                      <label className={`text-sm font-medium ${themeClasses.textPrimary}`}>
                        Keep Model Loaded in VRAM
                      </label>
                      <p className={`text-xs ${themeClasses.textSecondary} mt-1`}>
                        When enabled, the model stays in VRAM for faster responses but uses more memory
                      </p>
                    </div>
                    <button
                      onClick={() => setConfig({...config, keep_model_loaded: !config.keep_model_loaded})}
                      className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-orange-500 focus:ring-offset-2 ${
                        config.keep_model_loaded ? 'bg-orange-500' : themeClasses.surfaceSecondary
                      }`}
                    >
                      <span
                        className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                          config.keep_model_loaded ? 'translate-x-6' : 'translate-x-1'
                        }`}
                      />
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>

          {/* App Category Editor */}
          <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4`}>
            <h2 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-4`}>
              App Category Management
            </h2>
            <p className={`${themeClasses.textSecondary} mb-4 text-sm`}>
              Categorize your applications to improve productivity analysis. Categories help the AI understand your activity patterns better.
            </p>
            
            <div className="space-y-4">
              {/* Search Bar */}
              <div className="relative">
                <Search className={`absolute left-3 top-1/2 transform -translate-y-1/2 w-4 h-4 ${themeClasses.textSecondary}`} />
                <input
                  type="text"
                  value={categorySearch}
                  onChange={(e) => setCategorySearch(e.target.value)}
                  placeholder="Search apps..."
                  className={`w-full pl-10 pr-3 py-2 ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500`}
                />
              </div>

              {/* Category Table */}
              <div className={`${themeClasses.surfaceSecondary} rounded-lg overflow-hidden`}>
                <div className="overflow-auto max-h-96">
                  <table className="w-full min-w-[500px]">
                    <thead className={`${themeClasses.surface} border-b ${themeClasses.border} sticky top-0`}>
                      <tr>
                        <th className={`text-left px-3 py-2 text-sm font-medium ${themeClasses.textPrimary}`}>App</th>
                        <th className={`text-left px-3 py-2 text-sm font-medium ${themeClasses.textPrimary}`}>Category</th>
                        <th className={`text-center px-3 py-2 text-sm font-medium ${themeClasses.textPrimary}`}>Score</th>
                      </tr>
                    </thead>
                    <tbody>
                      {isLoadingCategories ? (
                        <tr>
                          <td colSpan={3} className={`text-center py-8 ${themeClasses.textSecondary}`}>
                            <RefreshCw className="w-6 h-6 animate-spin mx-auto mb-2" />
                            Loading categories...
                          </td>
                        </tr>
                      ) : filteredCategories.length === 0 ? (
                        <tr>
                          <td colSpan={3} className={`text-center py-8 ${themeClasses.textSecondary}`}>
                            No apps found
                          </td>
                        </tr>
                      ) : (
                        filteredCategories.map((cat) => {
                          const edited = editedCategories.get(cat.app_name)
                          const current = edited || cat
                          const isEdited = editedCategories.has(cat.app_name)
                          
                          return (
                            <tr key={cat.app_name} className={`border-b ${themeClasses.border} ${isEdited ? 'bg-orange-500/10' : ''}`}>
                              <td className={`px-3 py-2 text-sm ${themeClasses.textPrimary} truncate max-w-[200px]`} title={cat.app_name}>
                                {cat.app_name}
                              </td>
                              <td className="px-3 py-2">
                                <select
                                  value={current.category}
                                  onChange={(e) => handleCategoryChange(cat.app_name, 'category', e.target.value)}
                                  className={`w-full px-2 py-1 text-sm ${themeClasses.surface} ${themeClasses.textPrimary} rounded border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500`}
                                >
                                  <option value="work">work</option>
                                  <option value="communication">communication</option>
                                  <option value="entertainment">entertainment</option>
                                  <option value="development">development</option>
                                  <option value="productivity">productivity</option>
                                  <option value="system">system</option>
                                  <option value="other">other</option>
                                </select>
                              </td>
                              <td className="px-3 py-2">
                                <input
                                  type="number"
                                  value={current.productivity_score}
                                  onChange={(e) => handleCategoryChange(cat.app_name, 'productivity_score', parseInt(e.target.value) || 0)}
                                  min="0"
                                  max="100"
                                  className={`w-16 px-2 py-1 text-sm text-center ${themeClasses.surface} ${themeClasses.textPrimary} rounded border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500`}
                                />
                              </td>
                            </tr>
                          )
                        })
                      )}
                    </tbody>
                  </table>
                </div>
              </div>

              {/* Save Categories Button */}
              {editedCategories.size > 0 && (
                <div className="flex justify-end">
                  <button
                    onClick={handleSaveCategories}
                    disabled={isLoading}
                    className={`px-4 py-2 text-white rounded-lg transition-colors disabled:opacity-50 flex items-center space-x-2`}
                    style={{ backgroundColor: themeClasses.primary }}
                  >
                    {isLoading ? (
                      <RefreshCw className="w-4 h-4 animate-spin" />
                    ) : (
                      <Save className="w-4 h-4" />
                    )}
                    <span>{isLoading ? 'Saving...' : `Save ${editedCategories.size} Changes`}</span>
                  </button>
                </div>
              )}

            </div>
          </div>

          {/* Prompt Configuration - Collapsible */}
          <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4`}>
            <button
              onClick={() => setIsPromptConfigOpen(!isPromptConfigOpen)}
              className="w-full flex items-center justify-between text-left"
            >
              <div>
                <h2 className={`text-lg font-semibold ${themeClasses.textPrimary}`}>
                  Prompt Configuration
                </h2>
                <p className={`${themeClasses.textSecondary} text-sm mt-1`}>
                  Customize notification prompts for each mode
                </p>
              </div>
              {isPromptConfigOpen ? (
                <ChevronUp className={`w-5 h-5 ${themeClasses.textSecondary}`} />
              ) : (
                <ChevronDown className={`w-5 h-5 ${themeClasses.textSecondary}`} />
              )}
            </button>
            
            {isPromptConfigOpen && (
              <div className="mt-4 space-y-4 border-t pt-4" style={{ borderColor: isDarkMode ? 'rgba(255,255,255,0.1)' : 'rgba(0,0,0,0.1)' }}>
                {/* Notifications Toggle */}
                <div className="flex items-center justify-between">
                  <div className="flex-1">
                    <h3 className={`text-sm font-medium ${themeClasses.textPrimary}`}>
                      Enable Notifications
                    </h3>
                    <p className={`text-xs ${themeClasses.textSecondary} mt-1`}>
                      Turn on/off all mode-specific notifications
                    </p>
                  </div>
                  <button
                    onClick={() => setConfig({...config, notifications_enabled: !config.notifications_enabled})}
                    className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-orange-500 focus:ring-offset-2 ${
                      config.notifications_enabled ? 'bg-orange-500' : themeClasses.surfaceSecondary
                    }`}
                  >
                    <span
                      className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                        config.notifications_enabled ? 'translate-x-6' : 'translate-x-1'
                      }`}
                    />
                  </button>
                </div>

                {/* Mode-specific prompts */}
                <div className="space-y-3">
                  {/* Chill Mode */}
                  <div>
                    <label className={`block text-sm font-medium ${themeClasses.textPrimary} mb-1`}>
                      Chill Mode
                    </label>
                    <textarea
                      value={config.chill_notification_prompt}
                      onChange={(e) => setConfig({...config, chill_notification_prompt: e.target.value})}
                      rows={2}
                      className={`w-full px-3 py-2 text-sm ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500 resize-none`}
                    />
                  </div>

                  {/* Study Mode */}
                  <div>
                    <label className={`block text-sm font-medium ${themeClasses.textPrimary} mb-1`}>
                      Study Mode
                    </label>
                    <textarea
                      value={config.study_notification_prompt}
                      onChange={(e) => setConfig({...config, study_notification_prompt: e.target.value})}
                      rows={2}
                      className={`w-full px-3 py-2 text-sm ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500 resize-none`}
                    />
                  </div>

                  {/* Coach Mode */}
                  <div>
                    <label className={`block text-sm font-medium ${themeClasses.textPrimary} mb-1`}>
                      Coach Mode
                    </label>
                    <textarea
                      value={config.coach_notification_prompt}
                      onChange={(e) => setConfig({...config, coach_notification_prompt: e.target.value})}
                      rows={2}
                      className={`w-full px-3 py-2 text-sm ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500 resize-none`}
                    />
                  </div>

                  {/* Webhook */}
                  <div>
                    <label className={`block text-sm font-medium ${themeClasses.textPrimary} mb-1`}>
                      Webhook URL
                    </label>
                    <input
                      type="url"
                      value={config.notification_webhook || ''}
                      onChange={(e) => setConfig({...config, notification_webhook: e.target.value || undefined})}
                      className={`w-full px-3 py-2 text-sm ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500`}
                      placeholder="https://hooks.slack.com/..."
                    />
                  </div>
                </div>

                {/* Test Button */}
                <button
                  onClick={handleTestNotification}
                  className={`px-4 py-2 text-white rounded-lg transition-colors hover:opacity-90 flex items-center space-x-2 text-sm ${isDarkMode ? '' : 'shadow-sm'}`}
                  style={{ backgroundColor: themeClasses.primary }}
                >
                  <PlayCircle className="w-4 h-4" />
                  <span>Test Notification</span>
                </button>
              </div>
            )}
          </div>

          {/* Help Section */}
          <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4`}>
            <h2 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-4`}>
              Help & Setup
            </h2>
            <p className={`${themeClasses.textSecondary} mb-4 text-sm`}>
              Get started with ActivityWatch and Ollama installation.
            </p>
            
            <div className="space-y-4">
              {/* One-Click Setup */}
              <div className="mb-6">
                <h3 className={`text-lg font-medium ${themeClasses.textPrimary} mb-4`}>
                  One-Click Setup
                </h3>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                  <div className={`${themeClasses.surfaceSecondary} rounded-lg p-4`}>
                    <div className="flex items-center justify-between mb-3">
                      <div>
                        <h4 className={`text-md font-medium ${themeClasses.textPrimary}`}>ActivityWatch</h4>
                        <p className={`text-sm ${themeClasses.textSecondary}`}>Activity monitoring</p>
                      </div>
                      <button
                        onClick={handleInstallActivityWatch}
                        disabled={isInstallLoading.activitywatch || connectionStatus.activitywatch}
                        className={`px-3 py-1 text-white rounded text-sm transition-colors disabled:opacity-50 flex items-center space-x-1`}
                        style={{ backgroundColor: connectionStatus.activitywatch ? '#6B7280' : themeClasses.primary }}
                      >
                        {isInstallLoading.activitywatch ? (
                          <RefreshCw className="w-3 h-3 animate-spin" />
                        ) : (
                          <Download className="w-3 h-3" />
                        )}
                        <span>
                          {isInstallLoading.activitywatch ? 'Installing...' : 
                           connectionStatus.activitywatch ? 'Connected' : 'Install'}
                        </span>
                      </button>
                    </div>
                  </div>
                  
                  <div className={`${themeClasses.surfaceSecondary} rounded-lg p-4`}>
                    <div className="flex items-center justify-between mb-3">
                      <div>
                        <h4 className={`text-md font-medium ${themeClasses.textPrimary}`}>Ollama</h4>
                        <p className={`text-sm ${themeClasses.textSecondary}`}>Local AI model</p>
                      </div>
                      <button
                        onClick={handleInstallOllama}
                        disabled={isInstallLoading.ollama || connectionStatus.ollama}
                        className={`px-3 py-1 text-white rounded text-sm transition-colors disabled:opacity-50 flex items-center space-x-1`}
                        style={{ backgroundColor: connectionStatus.ollama ? '#6B7280' : themeClasses.primary }}
                      >
                        {isInstallLoading.ollama ? (
                          <RefreshCw className="w-3 h-3 animate-spin" />
                        ) : (
                          <Download className="w-3 h-3" />
                        )}
                        <span>
                          {isInstallLoading.ollama ? 'Installing...' : 
                           connectionStatus.ollama ? 'Connected' : 'Install'}
                        </span>
                      </button>
                    </div>
                  </div>
                </div>
              </div>

              {/* Manual Setup */}
              <div>
                <h3 className={`text-lg font-medium ${themeClasses.textPrimary} mb-3`}>
                  Manual Setup
                </h3>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                  <div className={`${themeClasses.surfaceSecondary} rounded-lg p-4`}>
                    <h4 className={`text-md font-medium ${themeClasses.textPrimary} mb-2`}>ActivityWatch</h4>
                    <div className={`text-sm ${themeClasses.textSecondary} space-y-1`}>
                      <p>1. Visit <a href="https://activitywatch.net" className="text-orange-400 hover:underline">activitywatch.net</a></p>
                      <p>2. Download and install for your platform</p>
                      <p>3. Run on port {config.activitywatch_port}</p>
                    </div>
                  </div>
                  
                  <div className={`${themeClasses.surfaceSecondary} rounded-lg p-4`}>
                    <h4 className={`text-md font-medium ${themeClasses.textPrimary} mb-2`}>Ollama</h4>
                    <div className={`text-sm ${themeClasses.textSecondary} space-y-1`}>
                      <p>1. Visit <a href="https://ollama.ai" className="text-orange-400 hover:underline">ollama.ai</a></p>
                      <p>2. Install and run <code className="bg-slate-700 px-1 rounded">ollama serve</code></p>
                      <p>3. Pull model: <code className="bg-slate-700 px-1 rounded">ollama pull mistral</code></p>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>

          {/* About Section */}
          <div className={`${themeClasses.surface} rounded-xl shadow-lg p-4`}>
            <h2 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-4`}>
              About
            </h2>
            <div className="space-y-4">
              <div className="flex items-center justify-between">
                <span className={`text-sm ${themeClasses.textSecondary}`}>Version</span>
                <span className={`text-sm ${themeClasses.textPrimary} font-mono`}>v0.1.0</span>
              </div>
              <div className="flex items-center justify-between">
                <span className={`text-sm ${themeClasses.textSecondary}`}>Creator</span>
                <a 
                  href="https://github.com/HarryYu31" 
                  className="text-sm text-orange-400 hover:underline"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  @HarryYu31
                </a>
              </div>
              <div className="flex items-center justify-between">
                <span className={`text-sm ${themeClasses.textSecondary}`}>Framework</span>
                <span className={`text-sm ${themeClasses.textPrimary}`}>Tauri + React</span>
              </div>
              <div className="flex items-center justify-between">
                <span className={`text-sm ${themeClasses.textSecondary}`}>License</span>
                <span className={`text-sm ${themeClasses.textPrimary}`}>MIT</span>
              </div>
            </div>
          </div>
        </div>
      </div>
      
      {/* Floating Save Button */}
      {hasUnsavedChanges && (
        <div className="fixed bottom-8 right-8 z-50">
          <button
            onClick={handleSaveConfig}
            disabled={isLoading}
            className={`px-4 py-2 text-white rounded-lg shadow-lg transition-all transform hover:scale-105 disabled:opacity-50 flex items-center space-x-2`}
            style={{ 
              backgroundColor: themeClasses.primary,
              boxShadow: '0 4px 14px 0 rgba(251, 146, 60, 0.5)'
            }}
          >
            {isLoading ? (
              <RefreshCw className="w-5 h-5 animate-spin" />
            ) : (
              <Save className="w-5 h-5" />
            )}
            <span>{isLoading ? 'Saving...' : 'Save Changes'}</span>
          </button>
        </div>
      )}
    </div>
  )
}

export default Settings