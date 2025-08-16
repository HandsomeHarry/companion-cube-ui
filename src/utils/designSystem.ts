// Design System Tokens
export const spacing = {
  xs: '4px',
  sm: '8px',
  md: '12px',
  lg: '20px',
  xl: '32px',
  xxl: '48px'
} as const

export const typography = {
  h1: {
    fontSize: '28px',
    lineHeight: '36px',
    fontWeight: 600
  },
  h2: {
    fontSize: '24px',
    lineHeight: '32px',
    fontWeight: 600
  },
  h3: {
    fontSize: '16px',
    lineHeight: '24px',
    fontWeight: 600
  },
  body: {
    fontSize: '14px',
    lineHeight: '21px',
    fontWeight: 400
  },
  caption: {
    fontSize: '12px',
    lineHeight: '16px',
    fontWeight: 400
  }
} as const

export const colors = {
  // Primary colors
  primary: {
    50: '#eff6ff',
    100: '#dbeafe',
    200: '#bfdbfe',
    300: '#93c5fd',
    400: '#60a5fa',
    500: '#3b82f6',
    600: '#2563eb',
    700: '#1d4ed8',
    800: '#1e40af',
    900: '#1e3a8a'
  },
  // Semantic colors
  success: {
    50: '#f0fdf4',
    100: '#dcfce7',
    200: '#bbf7d0',
    300: '#86efac',
    400: '#4ade80',
    500: '#22c55e',
    600: '#16a34a',
    700: '#15803d',
    800: '#166534',
    900: '#14532d'
  },
  warning: {
    50: '#fffbeb',
    100: '#fef3c7',
    200: '#fde68a',
    300: '#fcd34d',
    400: '#fbbf24',
    500: '#f59e0b',
    600: '#d97706',
    700: '#b45309',
    800: '#92400e',
    900: '#78350f'
  },
  danger: {
    50: '#fef2f2',
    100: '#fee2e2',
    200: '#fecaca',
    300: '#fca5a5',
    400: '#f87171',
    500: '#ef4444',
    600: '#dc2626',
    700: '#b91c1c',
    800: '#991b1b',
    900: '#7f1d1d'
  },
  neutral: {
    50: '#f9fafb',
    100: '#f3f4f6',
    200: '#e5e7eb',
    300: '#d1d5db',
    400: '#9ca3af',
    500: '#6b7280',
    600: '#4b5563',
    700: '#374151',
    800: '#1f2937',
    900: '#111827'
  }
} as const

export const elevation = {
  none: 'none',
  sm: '0 1px 2px 0 rgba(0, 0, 0, 0.05)',
  base: '0 1px 3px 0 rgba(0, 0, 0, 0.1), 0 1px 2px 0 rgba(0, 0, 0, 0.06)',
  md: '0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06)',
  lg: '0 10px 15px -3px rgba(0, 0, 0, 0.1), 0 4px 6px -2px rgba(0, 0, 0, 0.05)',
  xl: '0 20px 25px -5px rgba(0, 0, 0, 0.1), 0 10px 10px -5px rgba(0, 0, 0, 0.04)'
} as const

export const borderRadius = {
  none: '0',
  sm: '4px',
  base: '8px',
  md: '12px',
  lg: '16px',
  full: '9999px'
} as const

export const transitions = {
  fast: '150ms ease-in-out',
  base: '200ms ease-in-out',
  slow: '300ms ease-in-out'
} as const

// Semantic mappings
export const semanticColors = {
  productive: colors.success[600],
  warning: colors.warning[600],
  distraction: colors.danger[600],
  neutral: colors.neutral[600],
  // WCAG AA compliant text colors
  text: {
    primary: colors.neutral[900],
    secondary: colors.neutral[600],
    tertiary: colors.neutral[500],
    inverse: '#ffffff'
  },
  background: {
    primary: '#ffffff',
    secondary: colors.neutral[50],
    tertiary: colors.neutral[100]
  }
} as const

// Mode descriptions
export const modeDescriptions = {
  ghost: {
    title: 'Ghost Mode',
    subtitle: 'Complete privacy, no tracking',
    description: 'Your activity is not monitored or recorded. Perfect for sensitive work.',
    icon: '👻'
  },
  chill: {
    title: 'Chill Mode',
    subtitle: 'Gentle reminders to stay balanced',
    description: 'Hourly check-ins to help you maintain a healthy work-life balance.',
    icon: '😌'
  },
  study_buddy: {
    title: 'Study Mode',
    subtitle: 'Stay focused on your learning goals',
    description: '5-minute intervals to keep you on track with your studies.',
    icon: '📚'
  },
  coach: {
    title: 'Coach Mode',
    subtitle: 'Accountability partner for productivity',
    description: 'Todo tracking and 15-minute check-ins to maximize your output.',
    icon: '💪'
  }
} as const

// Human-readable state mappings
export const stateLabels = {
  productive: {
    label: 'Productive',
    description: 'Great focus and output!',
    color: colors.success[600]
  },
  moderate: {
    label: 'Moderate',
    description: 'Decent productivity',
    color: colors.primary[600]
  },
  chilling: {
    label: 'Chilling',
    description: 'Taking it easy',
    color: colors.primary[400]
  },
  unproductive: {
    label: 'Unproductive',
    description: 'Getting distracted',
    color: colors.warning[600]
  },
  afk: {
    label: 'Away',
    description: 'Taking a break',
    color: colors.neutral[400]
  },
  unknown: {
    label: 'No Data',
    description: 'Start tracking to see status',
    color: colors.neutral[300]
  }
} as const