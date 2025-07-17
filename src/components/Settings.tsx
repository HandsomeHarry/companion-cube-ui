import { useState, useEffect } from 'react'
import { Settings as SettingsIcon, Download, PlayCircle, Save, RefreshCw, ExternalLink } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'
import { getThemeClasses } from '../utils/theme'

interface SettingsProps {
  isDarkMode: boolean
  currentMode: string
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

function Settings({ isDarkMode, currentMode }: SettingsProps) {
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
    notification_webhook: undefined
  })
  const [isLoading, setIsLoading] = useState(false)
  const [isInstallLoading, setIsInstallLoading] = useState({
    ollama: false,
    activitywatch: false
  })

  // Load config on mount
  useEffect(() => {
    const loadConfig = async () => {
      try {
        const userConfig = await invoke('load_user_config') as UserConfig
        setConfig(userConfig)
      } catch (error) {
        console.error('Failed to load config:', error)
      }
    }
    loadConfig()
  }, [])

  const handleSaveConfig = async () => {
    setIsLoading(true)
    try {
      await invoke('save_user_config', { config })
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

  return (
    <div className={`flex-1 ${themeClasses.background} h-screen flex flex-col`}>
      {/* Header */}
      <div className={`p-6 ${themeClasses.background} ${themeClasses.border} border-b flex-shrink-0`}>
        <div className="flex items-center space-x-3">
          <div className={`w-6 h-6 ${themeClasses.surfaceSecondary} rounded flex items-center justify-center`}>
            <SettingsIcon className={`w-4 h-4 ${themeClasses.textSecondary}`} />
          </div>
          <h1 className={`text-3xl font-bold ${themeClasses.textPrimary}`}>Settings</h1>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 p-6 overflow-y-auto min-h-0">
        <div className="space-y-6 pb-6">
          {/* Prompt Configuration */}
          <div className={`${themeClasses.surface} rounded-xl shadow-lg p-6`}>
            <h2 className={`text-xl font-semibold ${themeClasses.textPrimary} mb-4`}>
              Prompt Configuration
            </h2>
            <p className={`${themeClasses.textSecondary} mb-6`}>
              Customize notification prompts for each mode to match your preferences.
            </p>
            
            <div className="space-y-6">
              {/* Notifications Toggle */}
              <div className="flex items-center justify-between">
                <div>
                  <h3 className={`text-lg font-medium ${themeClasses.textPrimary} mb-1`}>
                    Enable Notifications
                  </h3>
                  <p className={`text-sm ${themeClasses.textSecondary}`}>
                    Turn on/off all mode-specific notifications
                  </p>
                </div>
                <label className="relative inline-flex items-center cursor-pointer">
                  <input
                    type="checkbox"
                    checked={config.notifications_enabled}
                    onChange={(e) => setConfig({...config, notifications_enabled: e.target.checked})}
                    className="sr-only peer"
                  />
                  <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-orange-300 rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-orange-600"></div>
                </label>
              </div>

              {/* Chill Mode Prompt */}
              <div>
                <label className={`block text-sm font-medium ${themeClasses.textPrimary} mb-2`}>
                  Chill Mode Notification
                </label>
                <textarea
                  value={config.chill_notification_prompt}
                  onChange={(e) => setConfig({...config, chill_notification_prompt: e.target.value})}
                  rows={3}
                  className={`w-full px-3 py-2 ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500`}
                  placeholder="Enter notification message for chill mode..."
                />
                <p className={`mt-1 text-xs ${themeClasses.textSecondary}`}>
                  Shown when user has been having too much fun (gaming, YouTube, etc.)
                </p>
              </div>

              {/* Study Mode Prompt */}
              <div>
                <label className={`block text-sm font-medium ${themeClasses.textPrimary} mb-2`}>
                  Study Mode Notification
                </label>
                <textarea
                  value={config.study_notification_prompt}
                  onChange={(e) => setConfig({...config, study_notification_prompt: e.target.value})}
                  rows={3}
                  className={`w-full px-3 py-2 ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500`}
                  placeholder="Enter notification message for study mode..."
                />
                <p className={`mt-1 text-xs ${themeClasses.textSecondary}`}>
                  Shown when user gets distracted from studying
                </p>
              </div>

              {/* Coach Mode Prompt */}
              <div>
                <label className={`block text-sm font-medium ${themeClasses.textPrimary} mb-2`}>
                  Coach Mode Notification
                </label>
                <textarea
                  value={config.coach_notification_prompt}
                  onChange={(e) => setConfig({...config, coach_notification_prompt: e.target.value})}
                  rows={3}
                  className={`w-full px-3 py-2 ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500`}
                  placeholder="Enter notification message for coach mode..."
                />
                <p className={`mt-1 text-xs ${themeClasses.textSecondary}`}>
                  Shown when it's time to check todo list progress
                </p>
              </div>

              {/* Webhook URL */}
              <div>
                <label className={`block text-sm font-medium ${themeClasses.textPrimary} mb-2`}>
                  Webhook URL (Optional)
                </label>
                <input
                  type="url"
                  value={config.notification_webhook || ''}
                  onChange={(e) => setConfig({...config, notification_webhook: e.target.value || undefined})}
                  className={`w-full px-3 py-2 ${themeClasses.surfaceSecondary} ${themeClasses.textPrimary} rounded-lg border border-transparent focus:outline-none focus:ring-2 focus:ring-orange-500`}
                  placeholder="https://your-webhook-url.com/notifications"
                />
                <p className={`mt-1 text-xs ${themeClasses.textSecondary}`}>
                  Send notifications to external service (Discord, Slack, etc.)
                </p>
              </div>

              {/* Test Notification Button */}
              <div className="pt-4">
                <button
                  onClick={handleTestNotification}
                  className={`px-4 py-2 text-white rounded-lg transition-colors hover:opacity-90 flex items-center space-x-2`}
                  style={{ backgroundColor: themeClasses.primary }}
                >
                  <PlayCircle className="w-4 h-4" />
                  <span>Test Notification</span>
                </button>
                <p className={`mt-1 text-xs ${themeClasses.textSecondary}`}>
                  Send a test notification to verify your setup
                </p>
              </div>
            </div>
          </div>

          {/* Port Configuration */}
          <div className={`${themeClasses.surface} rounded-xl shadow-lg p-6`}>
            <h2 className={`text-xl font-semibold ${themeClasses.textPrimary} mb-4`}>
              Port Configuration
            </h2>
            <p className={`${themeClasses.textSecondary} mb-6`}>
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
            </div>
          </div>

          {/* Save Button */}
          <div className="text-center">
            <button
              onClick={handleSaveConfig}
              disabled={isLoading}
              className={`px-6 py-3 text-white rounded-lg transition-colors disabled:opacity-50 flex items-center space-x-2 mx-auto`}
              style={{ backgroundColor: themeClasses.primary }}
            >
              {isLoading ? (
                <RefreshCw className="w-5 h-5 animate-spin" />
              ) : (
                <Save className="w-5 h-5" />
              )}
              <span>{isLoading ? 'Saving...' : 'Save All Settings'}</span>
            </button>
          </div>

          {/* Help Section */}
          <div className={`${themeClasses.surface} rounded-xl shadow-lg p-6`}>
            <h2 className={`text-xl font-semibold ${themeClasses.textPrimary} mb-4`}>
              Help & Setup
            </h2>
            <p className={`${themeClasses.textSecondary} mb-6`}>
              Get started with ActivityWatch and Ollama installation.
            </p>
            
            <div className="space-y-4">
              {/* One-Click Setup */}
              <div className="mb-6">
                <h3 className={`text-lg font-medium ${themeClasses.textPrimary} mb-4`}>
                  One-Click Setup
                </h3>
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div className={`${themeClasses.surfaceSecondary} rounded-lg p-4`}>
                    <div className="flex items-center justify-between mb-3">
                      <div>
                        <h4 className={`text-md font-medium ${themeClasses.textPrimary}`}>ActivityWatch</h4>
                        <p className={`text-sm ${themeClasses.textSecondary}`}>Activity monitoring</p>
                      </div>
                      <button
                        onClick={handleInstallActivityWatch}
                        disabled={isInstallLoading.activitywatch}
                        className={`px-3 py-1 text-white rounded text-sm transition-colors disabled:opacity-50 flex items-center space-x-1`}
                        style={{ backgroundColor: themeClasses.primary }}
                      >
                        {isInstallLoading.activitywatch ? (
                          <RefreshCw className="w-3 h-3 animate-spin" />
                        ) : (
                          <Download className="w-3 h-3" />
                        )}
                        <span>{isInstallLoading.activitywatch ? 'Installing...' : 'Install'}</span>
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
                        disabled={isInstallLoading.ollama}
                        className={`px-3 py-1 text-white rounded text-sm transition-colors disabled:opacity-50 flex items-center space-x-1`}
                        style={{ backgroundColor: themeClasses.primary }}
                      >
                        {isInstallLoading.ollama ? (
                          <RefreshCw className="w-3 h-3 animate-spin" />
                        ) : (
                          <Download className="w-3 h-3" />
                        )}
                        <span>{isInstallLoading.ollama ? 'Installing...' : 'Install'}</span>
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
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
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
          <div className={`${themeClasses.surface} rounded-xl shadow-lg p-6`}>
            <h2 className={`text-xl font-semibold ${themeClasses.textPrimary} mb-4`}>
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
    </div>
  )
}

export default Settings