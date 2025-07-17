export interface ThemeColors {
  primary: string
  primaryHover: string
  primaryDark: string
  accent: string
  background: string
  surface: string
  text: string
  textSecondary: string
  border: string
}

export const getModeColors = (mode: string, isDarkMode: boolean): ThemeColors => {
  const modeColorMap = {
    ghost: {
      primary: isDarkMode ? '#6B7280' : '#4B5563',
      primaryHover: isDarkMode ? '#9CA3AF' : '#374151',
      primaryDark: isDarkMode ? '#4B5563' : '#1F2937',
      accent: isDarkMode ? '#9CA3AF' : '#6B7280'
    },
    chill: {
      primary: isDarkMode ? '#3B82F6' : '#2563EB',
      primaryHover: isDarkMode ? '#60A5FA' : '#1D4ED8',
      primaryDark: isDarkMode ? '#1D4ED8' : '#1E3A8A',
      accent: isDarkMode ? '#93C5FD' : '#3B82F6'
    },
    study_buddy: {
      primary: isDarkMode ? '#F97316' : '#EA580C',
      primaryHover: isDarkMode ? '#FB923C' : '#C2410C',
      primaryDark: isDarkMode ? '#EA580C' : '#9A3412',
      accent: isDarkMode ? '#FDBA74' : '#F97316'
    },
    coach: {
      primary: isDarkMode ? '#10B981' : '#059669',
      primaryHover: isDarkMode ? '#34D399' : '#047857',
      primaryDark: isDarkMode ? '#059669' : '#065F46',
      accent: isDarkMode ? '#6EE7B7' : '#10B981'
    }
  }

  const colors = modeColorMap[mode as keyof typeof modeColorMap] || modeColorMap.study_buddy

  return {
    ...colors,
    background: isDarkMode ? '#0F172A' : '#F8FAFC',
    surface: isDarkMode ? '#1E293B' : '#FFFFFF',
    text: isDarkMode ? '#E2E8F0' : '#1E293B',
    textSecondary: isDarkMode ? '#94A3B8' : '#64748B',
    border: isDarkMode ? '#334155' : '#E2E8F0'
  }
}

export const getThemeClasses = (mode: string, isDarkMode: boolean) => {
  const colors = getModeColors(mode, isDarkMode)
  
  return {
    // Button styles
    primaryButton: `text-white rounded-lg transition-colors disabled:opacity-50`,
    primaryButtonColors: {
      backgroundColor: colors.primary,
      ':hover': { backgroundColor: colors.primaryHover }
    },
    
    // Background styles
    background: isDarkMode ? 'bg-slate-900' : 'bg-gray-50',
    surface: isDarkMode ? 'bg-slate-800' : 'bg-white',
    surfaceSecondary: isDarkMode ? 'bg-slate-700' : 'bg-gray-100',
    
    // Text styles
    textPrimary: isDarkMode ? 'text-slate-200' : 'text-gray-900',
    textSecondary: isDarkMode ? 'text-slate-400' : 'text-gray-600',
    
    // Border styles
    border: isDarkMode ? 'border-slate-700' : 'border-gray-200',
    
    // Mode-specific colors
    accent: colors.accent,
    primary: colors.primary,
    primaryHover: colors.primaryHover
  }
}