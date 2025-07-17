import { PieChart, Pie, Cell, ResponsiveContainer, Tooltip, Legend } from 'recharts'
import { useState, useEffect } from 'react'

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
      { name: 'Work', value: 0, color: '#14B8A6' },
      { name: 'Communication', value: 0, color: '#06B6D4' },
      { name: 'Distractions', value: 0, color: '#F97316' }
    ]
  }

  return [
    { name: 'Work', value: classification.work, color: '#14B8A6' },
    { name: 'Communication', value: classification.communication, color: '#06B6D4' },
    { name: 'Distractions', value: classification.distraction, color: '#F97316' }
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

  return (
    <div className="h-64">
      <ResponsiveContainer width="100%" height="100%">
        <PieChart>
          <Pie
            data={data}
            cx="50%"
            cy="50%"
            innerRadius={50}
            outerRadius={90}
            paddingAngle={5}
            dataKey="value"
            animationBegin={0}
            animationDuration={shouldAnimate ? 800 : 0}
            isAnimationActive={shouldAnimate}
          >
            {data.map((entry, index) => (
              <Cell key={`cell-${index}`} fill={entry.color} />
            ))}
          </Pie>
          <Tooltip 
            contentStyle={{
              backgroundColor: '#334155',
              border: '1px solid #475569',
              borderRadius: '8px',
              color: '#E2E8F0'
            }}
          />
          <Legend 
            wrapperStyle={{
              color: '#E2E8F0'
            }}
          />
        </PieChart>
      </ResponsiveContainer>
    </div>
  )
}

export default ActivityChart