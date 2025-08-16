import { useState, useEffect } from 'react'
import { BarChart2, Clock, Activity, TrendingUp, RefreshCw, Download } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'
import { getThemeClasses } from '../utils/theme'
import { Pie, Bar, Line } from 'react-chartjs-2'
import {
  Chart as ChartJS,
  ArcElement,
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  BarElement,
  Title,
  Tooltip,
  Legend,
  Filler
} from 'chart.js'

// Register ChartJS components
ChartJS.register(
  ArcElement,
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  BarElement,
  Title,
  Tooltip,
  Legend,
  Filler
)

interface HistoryProps {
  isDarkMode: boolean
  currentMode: string
}

interface ActivityData {
  time_range: string
  category_statistics: CategoryStat[]
  hourly_breakdown: HourlyBreakdown[]
  top_apps: TopApp[]
}

interface CategoryStat {
  category: string
  app_count: number
  total_duration: number
  avg_productivity_score: number
}

interface HourlyBreakdown {
  hour: string
  category: string
  duration: number
}

interface TopApp {
  app_name: string
  category: string
  productivity_score: number
  total_duration: number
  session_count: number
}

const categoryColors: Record<string, string> = {
  work: '#10b981',
  development: '#3b82f6',
  communication: '#f59e0b',
  entertainment: '#ef4444',
  productivity: '#06b6d4',
  system: '#8b5cf6',
  other: '#6b7280',
  uncategorized: '#9ca3af'
}

function History({ isDarkMode, currentMode }: HistoryProps) {
  const themeClasses = getThemeClasses(currentMode, isDarkMode)
  const [selectedRange, setSelectedRange] = useState<'hour' | 'day' | 'week'>('day')
  const [activityData, setActivityData] = useState<ActivityData | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [isSyncing, setIsSyncing] = useState(false)
  const [syncMessage, setSyncMessage] = useState<string | null>(null)

  useEffect(() => {
    loadActivityData()
  }, [selectedRange])

  const loadActivityData = async () => {
    setIsLoading(true)
    try {
      const data = await invoke('get_activity_history', { timeRange: selectedRange }) as ActivityData
      setActivityData(data)
    } catch (error) {
      console.error('Failed to load activity history:', error)
    } finally {
      setIsLoading(false)
    }
  }

  const handleSyncActivities = async () => {
    setIsSyncing(true)
    setSyncMessage(null)
    try {
      const result = await invoke('sync_all_activities') as string
      setSyncMessage(result)
      // Reload data after sync
      await loadActivityData()
    } catch (error) {
      setSyncMessage(`Error: ${error}`)
      console.error('Failed to sync activities:', error)
    } finally {
      setIsSyncing(false)
      // Clear message after 5 seconds
      setTimeout(() => setSyncMessage(null), 5000)
    }
  }

  const formatDuration = (seconds: number): string => {
    const hours = Math.floor(seconds / 3600)
    const minutes = Math.floor((seconds % 3600) / 60)
    if (hours > 0) {
      return `${hours}h ${minutes}m`
    }
    return `${minutes}m`
  }

  // Prepare pie chart data for categories
  const categoryChartData = activityData ? {
    labels: activityData.category_statistics.map(stat => stat.category),
    datasets: [{
      data: activityData.category_statistics.map(stat => Math.round(stat.total_duration / 60)), // Convert to minutes
      backgroundColor: activityData.category_statistics.map(stat => categoryColors[stat.category] || '#6b7280'),
      borderColor: isDarkMode ? '#1f2937' : '#ffffff',
      borderWidth: 2
    }]
  } : null

  // Prepare timeline chart data
  const timelineData = activityData ? (() => {
    // Group by hour and stack categories
    const hourMap = new Map<string, Map<string, number>>()
    
    activityData.hourly_breakdown.forEach(item => {
      const hour = new Date(item.hour).getHours()
      const hourKey = `${hour}:00`
      
      if (!hourMap.has(hourKey)) {
        hourMap.set(hourKey, new Map())
      }
      hourMap.get(hourKey)!.set(item.category, (item.duration || 0) / 60) // Convert to minutes
    })

    const categories = Array.from(new Set(activityData.hourly_breakdown.map(item => item.category)))
    const hours = Array.from(hourMap.keys()).sort((a, b) => parseInt(a) - parseInt(b))

    return {
      labels: hours,
      datasets: categories.map(category => ({
        label: category,
        data: hours.map(hour => hourMap.get(hour)?.get(category) || 0),
        backgroundColor: categoryColors[category] || '#6b7280',
        borderColor: categoryColors[category] || '#6b7280',
        borderWidth: 2
      }))
    }
  })() : null

  const chartOptions = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
      legend: {
        position: 'bottom' as const,
        labels: {
          color: isDarkMode ? '#e5e7eb' : '#374151',
          padding: 15,
          font: {
            size: 12
          }
        }
      },
      tooltip: {
        callbacks: {
          label: (context: any) => {
            const value = context.parsed
            const label = context.dataset.label || context.label
            return `${label}: ${formatDuration(value * 60)}`
          }
        }
      }
    }
  }

  const barChartOptions = {
    ...chartOptions,
    scales: {
      x: {
        stacked: true,
        grid: {
          display: false
        },
        ticks: {
          color: isDarkMode ? '#9ca3af' : '#6b7280'
        }
      },
      y: {
        stacked: true,
        grid: {
          color: isDarkMode ? '#374151' : '#e5e7eb'
        },
        ticks: {
          color: isDarkMode ? '#9ca3af' : '#6b7280',
          callback: (value: any) => `${value}m`
        }
      }
    }
  }

  return (
    <div className={`flex-1 ${themeClasses.background} h-full overflow-hidden flex flex-col`}>
      {/* Header */}
      <div className={`p-6 ${themeClasses.background} ${themeClasses.border} border-b flex-shrink-0`}>
        <div className="flex items-center justify-between">
          <div className="flex items-center space-x-3">
            <h1 className={`text-3xl font-bold ${themeClasses.textPrimary}`}>Activity History</h1>
          </div>
          
          {/* Time Range Selector and Actions */}
          <div className="flex items-center space-x-4">
            <div className="flex space-x-2">
              {(['hour', 'day', 'week'] as const).map(range => (
                <button
                  key={range}
                  onClick={() => setSelectedRange(range)}
                  className={`px-4 py-2 rounded-lg transition-colors ${
                    selectedRange === range
                      ? 'text-white'
                      : `${themeClasses.textSecondary} ${themeClasses.surface}`
                  }`}
                  style={selectedRange === range ? { backgroundColor: themeClasses.primary } : {}}
                >
                  Past {range.charAt(0).toUpperCase() + range.slice(1)}
                </button>
              ))}
            </div>
            
            <button
              onClick={async () => {
                try {
                  const debug = await invoke('debug_database_state') as string
                  console.log('Database Debug:', debug)
                  alert(debug)
                } catch (e) {
                  console.error('Debug failed:', e)
                }
              }}
              className={`w-10 h-10 rounded-lg transition-colors ${isDarkMode ? themeClasses.surface : 'bg-gray-200 hover:bg-gray-300'} ${themeClasses.textSecondary} flex items-center justify-center`}
              title="Debug Database"
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4" />
              </svg>
            </button>
          </div>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 p-8 overflow-y-auto overflow-x-hidden min-h-0">
        {/* Sync Section */}
        <div className="mb-6">
          <div className="flex items-center justify-between">
            <div>
              <h3 className={`text-lg font-semibold ${themeClasses.textPrimary}`}>Activity Data</h3>
              <p className={`text-sm ${themeClasses.textSecondary} mt-1`}>Sync your activity data from ActivityWatch</p>
            </div>
            <button
              onClick={handleSyncActivities}
              disabled={isSyncing}
              className={`px-4 py-2 ${isDarkMode ? 'bg-blue-600 hover:bg-blue-700' : 'bg-blue-500 hover:bg-blue-600'} text-white rounded-lg transition-colors disabled:opacity-50 flex items-center space-x-2`}
            >
              {isSyncing ? (
                <>
                  <RefreshCw className="w-4 h-4 animate-spin" />
                  <span>Syncing...</span>
                </>
              ) : (
                <>
                  <Download className="w-4 h-4" />
                  <span>Sync Last 30 Days</span>
                </>
              )}
            </button>
          </div>
          {syncMessage && (
            <div className={`mt-3 p-3 rounded-lg text-sm ${syncMessage.startsWith('Error') ? 'bg-red-100 text-red-700' : 'bg-green-100 text-green-700'}`}>
              {syncMessage}
            </div>
          )}
        </div>
        
        {isLoading ? (
          <div className="flex items-center justify-center h-full">
            <div className="text-center animate-fade-in">
              <div className="relative">
                <Activity className={`w-16 h-16 ${themeClasses.textSecondary} animate-spin mx-auto mb-4`} style={{ animationDuration: '2s' }} />
                <div className="absolute inset-0 w-16 h-16 mx-auto rounded-full bg-gradient-to-r from-transparent to-transparent via-current opacity-20 animate-pulse" />
              </div>
              <h3 className={`text-lg font-medium ${themeClasses.textPrimary} mb-2`}>Analyzing your activity...</h3>
              <p className={`text-sm ${themeClasses.textSecondary}`}>This may take a few moments</p>
            </div>
          </div>
        ) : activityData && activityData.category_statistics.length > 0 ? (
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-8">
            {/* Category Distribution */}
            <div className={`${themeClasses.surface} rounded-xl shadow-lg p-6`}>
              <h2 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-4`}>
                Time by Category
              </h2>
              <div className="h-64">
                {categoryChartData && (
                  <Pie data={categoryChartData} options={chartOptions} />
                )}
              </div>
              <div className="mt-4 space-y-2">
                {activityData.category_statistics.map(stat => (
                  <div key={stat.category} className="flex items-center justify-between">
                    <div className="flex items-center space-x-2">
                      <div
                        className="w-3 h-3 rounded"
                        style={{ backgroundColor: categoryColors[stat.category] || '#6b7280' }}
                      />
                      <span className={`text-sm ${themeClasses.textPrimary}`}>
                        {stat.category}
                      </span>
                    </div>
                    <span className={`text-sm ${themeClasses.textSecondary}`}>
                      {formatDuration(stat.total_duration)}
                    </span>
                  </div>
                ))}
              </div>
            </div>

            {/* Timeline */}
            <div className={`${themeClasses.surface} rounded-xl shadow-lg p-6`}>
              <h2 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-4`}>
                Activity Timeline
              </h2>
              <div className="h-64">
                {timelineData && (
                  <Bar data={timelineData} options={barChartOptions} />
                )}
              </div>
            </div>

            {/* Top Applications */}
            <div className={`${themeClasses.surface} rounded-xl shadow-lg p-6 lg:col-span-2`}>
              <h2 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-4`}>
                Top Applications
              </h2>
              <div className="overflow-x-auto">
                <table className="w-full">
                  <thead>
                    <tr className={`border-b ${themeClasses.border}`}>
                      <th className={`text-left py-2 text-sm font-medium ${themeClasses.textSecondary}`}>
                        Application
                      </th>
                      <th className={`text-left py-2 text-sm font-medium ${themeClasses.textSecondary}`}>
                        Category
                      </th>
                      <th className={`text-center py-2 text-sm font-medium ${themeClasses.textSecondary}`}>
                        Productivity
                      </th>
                      <th className={`text-right py-2 text-sm font-medium ${themeClasses.textSecondary}`}>
                        Time
                      </th>
                      <th className={`text-right py-2 text-sm font-medium ${themeClasses.textSecondary}`}>
                        Sessions
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {activityData.top_apps.map((app, index) => (
                      <tr key={index} className={`border-b ${themeClasses.border}`}>
                        <td className={`py-3 text-sm ${themeClasses.textPrimary}`}>
                          {app.app_name}
                        </td>
                        <td className="py-3">
                          <span
                            className="inline-block px-2 py-1 text-xs rounded"
                            style={{
                              backgroundColor: `${categoryColors[app.category] || '#6b7280'}20`,
                              color: categoryColors[app.category] || '#6b7280'
                            }}
                          >
                            {app.category}
                          </span>
                        </td>
                        <td className="py-3 text-center">
                          <div className="flex items-center justify-center">
                            <div className="w-24 bg-gray-200 rounded-full h-2">
                              <div
                                className="h-2 rounded-full"
                                style={{
                                  width: `${app.productivity_score}%`,
                                  backgroundColor: app.productivity_score >= 70 ? '#10b981' :
                                                 app.productivity_score >= 40 ? '#f59e0b' : '#ef4444'
                                }}
                              />
                            </div>
                            <span className={`ml-2 text-xs ${themeClasses.textSecondary}`}>
                              {app.productivity_score}%
                            </span>
                          </div>
                        </td>
                        <td className={`py-3 text-right text-sm ${themeClasses.textPrimary}`}>
                          {formatDuration(app.total_duration)}
                        </td>
                        <td className={`py-3 text-right text-sm ${themeClasses.textSecondary}`}>
                          {app.session_count}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>

            {/* Summary Stats */}
            <div className={`${themeClasses.surface} rounded-xl shadow-lg p-6 lg:col-span-2`}>
              <h2 className={`text-lg font-semibold ${themeClasses.textPrimary} mb-4`}>
                Summary Statistics
              </h2>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                <div className={`${themeClasses.surfaceSecondary} rounded-lg p-4`}>
                  <div className="flex items-center space-x-2 mb-2">
                    <Clock className={`w-4 h-4 ${themeClasses.textSecondary}`} />
                    <span className={`text-sm ${themeClasses.textSecondary}`}>Total Active Time</span>
                  </div>
                  <p className={`text-2xl font-bold ${themeClasses.textPrimary}`}>
                    {formatDuration(activityData.category_statistics.reduce((sum, stat) => sum + stat.total_duration, 0))}
                  </p>
                </div>
                
                <div className={`${themeClasses.surfaceSecondary} rounded-lg p-4`}>
                  <div className="flex items-center space-x-2 mb-2">
                    <Activity className={`w-4 h-4 ${themeClasses.textSecondary}`} />
                    <span className={`text-sm ${themeClasses.textSecondary}`}>Unique Apps</span>
                  </div>
                  <p className={`text-2xl font-bold ${themeClasses.textPrimary}`}>
                    {activityData.top_apps.length}
                  </p>
                </div>
                
                <div className={`${themeClasses.surfaceSecondary} rounded-lg p-4`}>
                  <div className="flex items-center space-x-2 mb-2">
                    <TrendingUp className={`w-4 h-4 ${themeClasses.textSecondary}`} />
                    <span className={`text-sm ${themeClasses.textSecondary}`}>Avg Productivity</span>
                  </div>
                  <p className={`text-2xl font-bold ${themeClasses.textPrimary}`}>
                    {Math.round(
                      activityData.category_statistics.reduce((sum, stat) => 
                        sum + (stat.avg_productivity_score * stat.total_duration), 0
                      ) / activityData.category_statistics.reduce((sum, stat) => sum + stat.total_duration, 0)
                    )}%
                  </p>
                </div>
              </div>
            </div>
          </div>
        ) : (
          <div className="flex flex-col items-center justify-center h-full max-w-md mx-auto">
            <div className="text-center animate-fade-in">
              {/* Icon with subtle animation */}
              <div className="relative mb-6">
                <div className={`w-24 h-24 mx-auto rounded-full ${themeClasses.surfaceSecondary} flex items-center justify-center`}>
                  <BarChart2 className={`w-12 h-12 ${themeClasses.textSecondary}`} />
                </div>
                <div className="absolute -bottom-1 -right-1 w-8 h-8 bg-orange-500 rounded-full flex items-center justify-center animate-bounce">
                  <span className="text-white text-lg">!</span>
                </div>
              </div>
              
              {/* Messaging */}
              <h3 className={`text-xl font-semibold ${themeClasses.textPrimary} mb-2`}>
                Let's track your productivity journey
              </h3>
              <p className={`text-sm ${themeClasses.textSecondary} mb-6 leading-relaxed`}>
                Sync your activity data from ActivityWatch to see detailed insights about your computer usage, 
                productivity patterns, and time allocation across different applications.
              </p>
              
              {/* Primary CTA */}
              <button
                onClick={handleSyncActivities}
                disabled={isSyncing}
                className={`px-6 py-3 ${isDarkMode ? 'bg-blue-600 hover:bg-blue-700' : 'bg-blue-500 hover:bg-blue-600'} text-white rounded-lg transition-all transform hover:scale-105 disabled:opacity-50 disabled:hover:scale-100 flex items-center space-x-2 mx-auto mb-4 shadow-lg`}
                style={{ transition: 'all 200ms ease-in-out' }}
              >
                {isSyncing ? (
                  <>
                    <RefreshCw className="w-5 h-5 animate-spin" />
                    <span>Syncing your data...</span>
                  </>
                ) : (
                  <>
                    <Download className="w-5 h-5" />
                    <span>Sync Last 30 Days</span>
                  </>
                )}
              </button>
              
              {/* Help text */}
              <p className={`text-xs ${themeClasses.textSecondary}`}>
                Make sure ActivityWatch is running in the background
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

export default History