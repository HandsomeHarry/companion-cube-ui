import { Home, Settings, Ghost, Coffee, BookOpen, Brain, BarChart2 } from 'lucide-react'
import { getThemeClasses } from '../utils/theme'

interface SidebarProps {
  currentMode: string
  onModeChange: (mode: string) => void
  isDarkMode: boolean
  onToggleTheme: () => void
  currentPage: string
  onPageChange: (page: string) => void
}

const modes = [
  { id: 'ghost', name: 'Ghost Mode', icon: Ghost },
  { id: 'chill', name: 'Chill Mode', icon: Coffee },
  { id: 'study_buddy', name: 'Study Mode', icon: BookOpen },
  { id: 'coach', name: 'Coach Mode', icon: Brain }
]

function Sidebar({ currentMode, onModeChange, currentPage, onPageChange, isDarkMode }: SidebarProps) {
  const themeClasses = getThemeClasses(currentMode, isDarkMode)

  return (
    <div className={`w-48 ${themeClasses.background} flex flex-col h-screen ${themeClasses.border} border-r`}>
      {/* Header with Logo */}
      <div className="p-4">
        <div className="flex items-center space-x-2">
          {/* Logo - Circle with "C" using theme primary color */}
          <div 
            className="w-8 h-8 rounded-full flex items-center justify-center"
            style={{ backgroundColor: themeClasses.primary }}
          >
            <span className="text-white font-bold text-sm">C</span>
          </div>
          <h1 className={`text-lg font-bold ${themeClasses.textPrimary}`}>
            Companion Cube
          </h1>
        </div>
      </div>

      {/* Navigation */}
      <div className="px-3 pb-4">
        <div className="space-y-1">
          <button 
            onClick={() => onPageChange('home')}
            className={`w-full flex items-center space-x-3 px-3 py-2 rounded-lg transition-colors ${
              currentPage === 'home' 
                ? 'text-white shadow-inner' 
                : `${themeClasses.textPrimary} hover:${themeClasses.surfaceSecondary}`
            }`}
            style={currentPage === 'home' ? { backgroundColor: themeClasses.primary } : {}}
          >
            <Home className="w-4 h-4" />
            <span className="font-medium text-sm">Home</span>
          </button>
          <button 
            onClick={() => onPageChange('history')}
            className={`w-full flex items-center space-x-3 px-3 py-2 rounded-lg transition-colors ${
              currentPage === 'history' 
                ? 'text-white shadow-inner' 
                : `${themeClasses.textPrimary} hover:${themeClasses.surfaceSecondary}`
            }`}
            style={currentPage === 'history' ? { backgroundColor: themeClasses.primary } : {}}
          >
            <BarChart2 className="w-4 h-4" />
            <span className="font-medium text-sm">History</span>
          </button>
          <button 
            onClick={() => onPageChange('settings')}
            className={`w-full flex items-center space-x-3 px-3 py-2 rounded-lg transition-colors ${
              currentPage === 'settings' 
                ? 'text-white shadow-inner' 
                : `${themeClasses.textPrimary} hover:${themeClasses.surfaceSecondary}`
            }`}
            style={currentPage === 'settings' ? { backgroundColor: themeClasses.primary } : {}}
          >
            <Settings className="w-4 h-4" />
            <span className="font-medium text-sm">Settings</span>
          </button>
        </div>
      </div>

      {/* Spacer */}
      <div className="flex-1" />

      {/* Modes Section */}
      <div className="px-3 pb-4">
        <div className="mb-3">
          <h2 className={`text-xs font-semibold ${themeClasses.textSecondary} uppercase tracking-wider`}>
            MODES
          </h2>
        </div>
        
        <div className="space-y-1">
          {modes.map((mode) => {
            const Icon = mode.icon
            const isActive = currentMode === mode.id
            const modeTheme = getThemeClasses(mode.id, isDarkMode)
            
            return (
              <button
                key={mode.id}
                onClick={() => onModeChange(mode.id)}
                className={`w-full flex items-center space-x-3 px-3 py-2 transition-colors rounded-lg ${
                  isActive 
                    ? 'text-white shadow-inner' 
                    : `${themeClasses.textPrimary} hover:${themeClasses.surfaceSecondary}`
                }`}
                style={isActive ? { backgroundColor: modeTheme.primary } : {}}
              >
                <Icon className={`w-4 h-4 ${
                  isActive ? 'text-white' : ''
                }`} style={!isActive ? { color: modeTheme.primary } : {}} />
                <span className="font-medium text-sm">{mode.name}</span>
              </button>
            )
          })}
        </div>
      </div>
    </div>
  )
}

export default Sidebar