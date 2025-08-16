import { PieChart, Pie, Cell, ResponsiveContainer, Tooltip, Label } from 'recharts'
import { useState, useEffect } from 'react'
import { colors, semanticColors, typography } from '../utils/designSystem'

interface ActivityClassification {
  work: number
  communication: number
  distraction: number
}

interface ActivityChartProps {
  isDarkMode: boolean
  classification?: ActivityClassification | null
}

const getChartData = (classification?: ActivityClassification | null) => {
  if (!classification) {
    return [
      { name: 'Work', value: 0, color: '#10B981' },
      { name: 'Communication', value: 0, color: '#F59E0B' },
      { name: 'Distractions', value: 0, color: '#EF4444' }
    ]
  }

  const total = classification.work + classification.communication + classification.distraction
  return [
    { 
      name: 'Productive Work', 
      value: classification.work, 
      percentage: total > 0 ? Math.round((classification.work / total) * 100) : 0,
      color: '#10B981' // green
    },
    { 
      name: 'Communication', 
      value: classification.communication, 
      percentage: total > 0 ? Math.round((classification.communication / total) * 100) : 0,
      color: '#F59E0B' // yellow
    },
    { 
      name: 'Distractions', 
      value: classification.distraction, 
      percentage: total > 0 ? Math.round((classification.distraction / total) * 100) : 0,
      color: '#EF4444' // red
    }
  ]
}

function ActivityChart({ isDarkMode, classification }: ActivityChartProps) {
  const [shouldAnimate, setShouldAnimate] = useState(true)
  
  useEffect(() => {
    // Disable animation after first render
    const timer = setTimeout(() => {
      setShouldAnimate(false)
    }, 1000)
    
    return () => clearTimeout(timer)
  }, [])

  const data = getChartData(classification)
  const hasData = data.some(item => item.value > 0)

  const CustomTooltip = ({ active, payload }: any) => {
    if (active && payload && payload.length) {
      const data = payload[0].payload
      return (
        <div className="bg-gray-800 text-white p-2 rounded-lg shadow-lg border border-gray-700">
          <p className="font-medium text-sm">{data.name}</p>
          <p className="text-xs mt-1">{data.percentage}% • {data.value} min</p>
        </div>
      )
    }
    return null
  }

  const CustomLabel = () => {
    if (!hasData) return null
    const topCategory = data.reduce((prev, current) => 
      prev.value > current.value ? prev : current
    )
    return (
      <text 
        x="50%" 
        y="50%" 
        textAnchor="middle" 
        dominantBaseline="middle"
        style={{ fontSize: typography.h2.fontSize, fontWeight: typography.h2.fontWeight }}
        fill={isDarkMode ? '#e5e7eb' : '#111827'}
      >
        {topCategory.percentage}%
      </text>
    )
  }

  return (
    <div className="flex flex-col h-full">
      {!hasData ? (
        <div className="h-64 flex items-center justify-center">
          <div className="text-center">
            <p className="text-gray-500 text-sm mb-2">No activity data yet</p>
            <p className="text-gray-400 text-xs">Start tracking to see your productivity breakdown</p>
          </div>
        </div>
      ) : (
        <>
          <div className="h-48 relative">
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <defs>
                  {/* Remove patterns - use solid colors only */}
                </defs>
                <Pie
                  data={data}
                  cx="50%"
                  cy="50%"
                  innerRadius={40}
                  outerRadius={80}
                  paddingAngle={3}
                  dataKey="value"
                  animationBegin={0}
                  animationDuration={shouldAnimate ? 800 : 0}
                  isAnimationActive={shouldAnimate}
                >
                  {data.map((entry, index) => (
                    <Cell 
                      key={`cell-${index}`} 
                      fill={entry.color}
                      className="hover:opacity-80 transition-opacity cursor-pointer"
                    />
                  ))}
                  <Label content={<CustomLabel />} position="center" />
                </Pie>
                <Tooltip content={<CustomTooltip />} />
              </PieChart>
            </ResponsiveContainer>
          </div>
        </>
      )}
    </div>
  )
}

export default ActivityChart