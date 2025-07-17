import { useState, useEffect, useRef } from 'react'
import { Terminal as TerminalIcon, Maximize2, Minimize2 } from 'lucide-react'
import { listen } from '@tauri-apps/api/event'

interface TerminalProps {
  className?: string
}

interface LogEvent {
  level: string
  message: string
  timestamp: string
}

function Terminal({ className = '' }: TerminalProps) {
  const [logs, setLogs] = useState<string[]>([])
  const [isFullscreen, setIsFullscreen] = useState(false)
  const terminalRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    // Auto-scroll to bottom when new logs are added
    if (terminalRef.current) {
      terminalRef.current.scrollTop = terminalRef.current.scrollHeight
    }
  }, [logs])

  useEffect(() => {
    // Listen for actual log events from the Tauri backend
    const unlisten = listen('log_message', (event) => {
      const logData = event.payload as LogEvent
      const formattedLog = `[${logData.timestamp}][${logData.level.toUpperCase()}] ${logData.message}`
      
      setLogs(prev => {
        const newLogs = [...prev, formattedLog]
        // Keep only last 50 logs
        return newLogs.slice(-50)
      })
    })

    return () => {
      unlisten.then(f => f())
    }
  }, [])

  return (
    <div className={`bg-slate-800 rounded-xl shadow-lg ${className} ${
      isFullscreen ? 'fixed inset-4 z-50' : ''
    }`}>
      <div className="flex items-center justify-between p-4 border-b border-slate-700">
        <div className="flex items-center space-x-2">
          <TerminalIcon className="w-4 h-4 text-slate-400" />
          <h3 className="text-lg font-semibold text-slate-200">Debug Terminal</h3>
        </div>
        <div className="flex items-center space-x-2">
          <button 
            onClick={() => setIsFullscreen(!isFullscreen)}
            className="text-xs text-slate-400 hover:text-slate-200 px-2 py-1 rounded hover:bg-slate-700"
          >
            {isFullscreen ? <Minimize2 className="w-4 h-4" /> : <Maximize2 className="w-4 h-4" />}
          </button>
          <button 
            onClick={() => setLogs([])}
            className="text-xs text-slate-400 hover:text-slate-200 px-2 py-1 rounded hover:bg-slate-700"
          >
            Clear
          </button>
        </div>
      </div>
      
      <div 
        ref={terminalRef}
        className={`p-4 overflow-y-auto bg-slate-900 rounded-b-xl ${
          isFullscreen ? 'h-full' : 'h-48'
        }`}
      >
        <div className="space-y-1 pb-4">
          {logs.length === 0 ? (
            <div className="text-xs font-mono text-slate-500 italic">
              Waiting for application logs...
            </div>
          ) : (
            logs.map((log, index) => (
              <div key={index} className="text-xs font-mono text-slate-300">
                {log}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  )
}

export default Terminal