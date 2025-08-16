import { Home, Settings, Ghost, Coffee, BookOpen, Brain, BarChart2, Info } from 'lucide-react'
import { getThemeClasses } from '../utils/theme'
import { modeDescriptions, spacing, typography, borderRadius, elevation, transitions } from '../utils/designSystem'

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
    <div className={`w-44 ${themeClasses.surface} flex flex-col h-screen ${themeClasses.border} border-r`} style={{ transition: transitions.base }}>
      {/* Header with Logo */}
      <div className="p-3">
        <div className="flex items-start">
          {/* Logo - Gradient icon only */}
          <div 
            className="w-12 h-12 rounded-full flex items-center justify-center ml-2"
            style={{ 
              background: 'linear-gradient(135deg, #3b82f6 0%, #60a5fa 100%)',
              boxShadow: '0 2px 4px rgba(0,0,0,0.1)'
            }}
          >
            <span className="text-white font-bold text-xl">C</span>
          </div>
        </div>
      </div>

      {/* Navigation */}
      <div className="px-3 pb-4">
        <div className="space-y-1">
          <button 
            onClick={() => onPageChange('home')}
            className={`w-full flex items-center space-x-2 px-3 py-2 rounded-lg transition-all duration-150 ${
              currentPage === 'home' 
                ? 'text-white shadow-inner' 
                : `${themeClasses.textPrimary} hover:bg-white/[0.04]`
            }`}
            style={{ 
              backgroundColor: currentPage === 'home' 
                ? themeClasses.primary
                : 'transparent'
            }}
          >
            <Home className="w-5 h-5" strokeWidth={2} />
            <span className="font-medium text-sm">Home</span>
          </button>
          <button 
            onClick={() => onPageChange('history')}
            className={`w-full flex items-center space-x-2 px-3 py-2 rounded-lg transition-all duration-150 ${
              currentPage === 'history' 
                ? 'text-white shadow-inner' 
                : `${themeClasses.textPrimary} hover:bg-white/[0.04]`
            }`}
            style={{ 
              backgroundColor: currentPage === 'history' 
                ? themeClasses.primary
                : 'transparent'
            }}
          >
            <BarChart2 className="w-5 h-5" strokeWidth={2} />
            <span className="font-medium text-sm">History</span>
          </button>
          <button 
            onClick={() => onPageChange('settings')}
            className={`w-full flex items-center space-x-2 px-3 py-2 rounded-lg transition-all duration-150 ${
              currentPage === 'settings' 
                ? 'text-white shadow-inner' 
                : `${themeClasses.textPrimary} hover:bg-white/[0.04]`
            }`}
            style={{ 
              backgroundColor: currentPage === 'settings' 
                ? themeClasses.primary
                : 'transparent'
            }}
          >
            <Settings className="w-5 h-5" strokeWidth={2} />
            <span className="font-medium text-sm">Settings</span>
          </button>
        </div>
      </div>

      {/* Spacer */}
      <div className="flex-1" />

      {/* Modes Section */}
      <div className="px-3 pb-4">
        <div className="mb-3 px-4">
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
              <div className="relative group">
                <button
                  key={mode.id}
                  onClick={() => onModeChange(mode.id)}
                  className={`relative w-full text-left transition-all rounded-full overflow-hidden ${
                    isActive 
                      ? 'text-white shadow-inner' 
                      : `${themeClasses.textPrimary} hover:bg-white/[0.04]`
                  }`}
                  style={{
                    padding: '8px 12px',
                    borderRadius: '8px',
                    backgroundColor: isActive 
                      ? 'transparent' 
                      : isDarkMode ? 'rgba(255,255,255,0.05)' : 'rgba(0,0,0,0.05)',
                    background: isActive 
                      ? `linear-gradient(135deg, ${modeTheme.primary} 0%, ${modeTheme.primaryHover} 100%)` 
                      : undefined,
                    transition: transitions.base
                  }}
                >
                  <div className="flex items-center space-x-3">
                    <Icon className={`w-5 h-5 flex-shrink-0 ${
                      isActive ? 'text-white' : ''
                    }`} style={!isActive ? { color: modeTheme.primary } : {}} />
                    <div className="flex-1">
                      <span className="font-medium text-sm block">
                        {modeDescriptions[mode.id as keyof typeof modeDescriptions].title}
                      </span>
                    </div>
                  </div>
                </button>
                {/* Hover tooltip */}
                <div className="absolute left-full ml-2 top-1/2 transform -translate-y-1/2 w-64 p-3 bg-gray-800 text-white text-sm rounded-lg opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-50" 
                     style={{ boxShadow: elevation.lg }}>
                  <p className="font-medium mb-1">{modeDescriptions[mode.id as keyof typeof modeDescriptions].title}</p>
                  <p className="text-xs opacity-90">{modeDescriptions[mode.id as keyof typeof modeDescriptions].subtitle}</p>
                  <p className="text-xs opacity-75 mt-1">{modeDescriptions[mode.id as keyof typeof modeDescriptions].description}</p>
                </div>
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}

export default Sidebar