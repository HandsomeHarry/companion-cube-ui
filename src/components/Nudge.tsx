import { useState, useRef, useEffect, useCallback } from 'react'
import { Archive, AlarmClock, Coffee } from 'lucide-react'
import { getThemeClasses } from '../utils/theme'

export interface NudgeMessage {
  id: string
  title: string
  body: string
  timestamp: string
  snoozeCount: number
}

interface NudgeProps {
  nudge: NudgeMessage | null
  onSnooze: (id: string) => void
  onSaveForLater: (nudge: NudgeMessage) => void
  currentMode: string
  isDarkMode: boolean
}

const HOLD_DURATION_MS = 3000

function Nudge({ nudge, onSnooze, onSaveForLater, currentMode, isDarkMode }: NudgeProps) {
  const [holdProgress, setHoldProgress] = useState(0) // 0–100
  const holdTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const progressTimer = useRef<ReturnType<typeof setInterval> | null>(null)
  const holdStart = useRef<number>(0)

  const themeClasses = getThemeClasses(currentMode, isDarkMode)

  // Reset progress when a new nudge arrives
  useEffect(() => {
    setHoldProgress(0)
  }, [nudge?.id])

  const cancelHold = useCallback(() => {
    if (holdTimer.current) clearTimeout(holdTimer.current)
    if (progressTimer.current) clearInterval(progressTimer.current)
    holdTimer.current = null
    progressTimer.current = null
    setHoldProgress(0)
  }, [])

  const startHold = useCallback(() => {
    if (!nudge) return
    holdStart.current = Date.now()

    progressTimer.current = setInterval(() => {
      const elapsed = Date.now() - holdStart.current
      setHoldProgress(Math.min((elapsed / HOLD_DURATION_MS) * 100, 100))
    }, 50)

    holdTimer.current = setTimeout(() => {
      clearInterval(progressTimer.current!)
      progressTimer.current = null
      setHoldProgress(0)
      onSnooze(nudge.id)
    }, HOLD_DURATION_MS)
  }, [nudge, onSnooze])

  // Cleanup on unmount
  useEffect(() => () => cancelHold(), [cancelHold])

  if (!nudge) return null

  const isSuggestingBreak = nudge.snoozeCount >= 3

  const surface = isDarkMode
    ? 'bg-gray-900 border-gray-700/60'
    : 'bg-white border-gray-200'
  const textPrimary = isDarkMode ? 'text-white' : 'text-gray-900'
  const textSecondary = isDarkMode ? 'text-gray-400' : 'text-gray-500'
  const actionBorder = isDarkMode
    ? 'border-gray-700 hover:bg-white/5'
    : 'border-gray-200 hover:bg-gray-50'

  return (
    <div
      className={`fixed bottom-6 right-6 z-50 w-80 rounded-xl shadow-2xl border ${surface} animate-slide-up`}
    >
      {/* Header */}
      <div className="px-4 pt-4 pb-2 space-y-1">
        {isSuggestingBreak ? (
          <>
            <div className="flex items-center gap-2">
              <Coffee className="w-4 h-4 text-blue-400" />
              <span className="text-xs font-semibold text-blue-400 uppercase tracking-wider">
                Maybe a break?
              </span>
            </div>
            <p className={`text-sm leading-snug ${textPrimary}`}>
              You've snoozed {nudge.snoozeCount} times. A short break might help
              more than another snooze.
            </p>
          </>
        ) : (
          <>
            <div className="flex items-center justify-between">
              <span
                className="text-xs font-semibold uppercase tracking-wider"
                style={{ color: themeClasses.primary }}
              >
                {nudge.title}
              </span>
              <span className={`text-xs ${textSecondary}`}>{nudge.timestamp}</span>
            </div>
            <p className={`text-sm leading-snug ${textPrimary}`}>{nudge.body}</p>
          </>
        )}
      </div>

      {/* Actions */}
      <div className="px-4 pb-4 pt-2 flex gap-2">
        {!isSuggestingBreak && (
          <button
            onClick={() => onSaveForLater(nudge)}
            className={`flex-1 flex items-center justify-center gap-1.5 px-3 py-2 rounded-lg text-sm font-medium border ${textPrimary} ${actionBorder} transition-colors`}
          >
            <Archive className="w-3.5 h-3.5" />
            Save for Later
          </button>
        )}

        {/* Hold-to-snooze: user must hold for 3 seconds */}
        <button
          className={`${isSuggestingBreak ? 'flex-1' : ''} relative overflow-hidden flex items-center justify-center gap-1.5 px-3 py-2 rounded-lg text-sm font-medium text-white select-none cursor-pointer`}
          style={{ background: themeClasses.primary, minWidth: '130px' }}
          onPointerDown={startHold}
          onPointerUp={cancelHold}
          onPointerLeave={cancelHold}
        >
          {/* Darkening overlay that fills left-to-right as the hold progresses */}
          <div
            className="absolute inset-0 bg-black/30 origin-left"
            style={{
              transform: `scaleX(${holdProgress / 100})`,
              transformOrigin: 'left',
            }}
          />
          <AlarmClock className="w-3.5 h-3.5 relative z-10 flex-shrink-0" />
          <span className="relative z-10 whitespace-nowrap">
            {holdProgress > 0
              ? `${Math.ceil(3 - (holdProgress / 100) * 3) || 1}s...`
              : isSuggestingBreak
              ? 'Snooze anyway'
              : 'Hold to Snooze'}
          </span>
        </button>
      </div>
    </div>
  )
}

export default Nudge
