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
    background: isDarkMode ? '#111827' : '#F8FAFC',
    surface: isDarkMode ? '#1F2937' : '#FFFFFF',
    text: isDarkMode ? '#e2e8f0' : '#1E293B',
    textSecondary: isDarkMode ? '#a0aec0' : '#64748B',
    border: isDarkMode ? 'rgba(255,255,255,0.08)' : '#E2E8F0'
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
    
    // Background styles - new dark theme
    background: isDarkMode ? 'bg-[#111827]' : 'bg-gray-50',
    surface: isDarkMode ? 'bg-[#1F2937]' : 'bg-white',
    surfaceSecondary: isDarkMode ? 'bg-[#374151]' : 'bg-gray-100',
    
    // Text styles
    textPrimary: isDarkMode ? 'text-[#e2e8f0]' : 'text-gray-900',
    textSecondary: isDarkMode ? 'text-[#a0aec0]' : 'text-gray-600',
    
    // Border styles
    border: isDarkMode ? 'border-white/[0.08]' : 'border-gray-200',
    
    // Mode-specific colors
    accent: colors.accent,
    primary: colors.primary,
    primaryHover: colors.primaryHover
  }
}