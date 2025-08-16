import { useEffect } from 'react'
import { CheckCircle, AlertCircle, Info, X } from 'lucide-react'
import { transitions } from '../utils/designSystem'

export interface ToastMessage {
  id: string
  type: 'success' | 'error' | 'info'
  message: string
  duration?: number
}

interface ToastProps {
  messages: ToastMessage[]
  onDismiss: (id: string) => void
  isDarkMode: boolean
}

function Toast({ messages, onDismiss, isDarkMode }: ToastProps) {
  useEffect(() => {
    const timers = messages.map(message => {
      if (message.duration !== 0) {
        return setTimeout(() => {
          onDismiss(message.id)
        }, message.duration || 3000)
      }
      return null
    })

    return () => {
      timers.forEach(timer => timer && clearTimeout(timer))
    }
  }, [messages, onDismiss])

  const getToastStyles = (type: ToastMessage['type']) => {
    switch (type) {
      case 'success':
        return 'bg-green-500 text-white'
      case 'error':
        return 'bg-red-500 text-white'
      case 'info':
        return 'bg-blue-500 text-white'
    }
  }

  const getIcon = (type: ToastMessage['type']) => {
    switch (type) {
      case 'success':
        return <CheckCircle className="w-5 h-5" />
      case 'error':
        return <AlertCircle className="w-5 h-5" />
      case 'info':
        return <Info className="w-5 h-5" />
    }
  }

  return (
    <div className="fixed bottom-4 right-4 z-50 space-y-2">
      {messages.map((message, index) => (
        <div
          key={message.id}
          className={`flex items-center space-x-3 px-4 py-3 rounded-lg shadow-lg ${getToastStyles(message.type)} animate-slide-up`}
          style={{ 
            transition: transitions.base,
            animationDelay: `${index * 50}ms`
          }}
        >
          {getIcon(message.type)}
          <span className="text-sm font-medium flex-1">{message.message}</span>
          <button
            onClick={() => onDismiss(message.id)}
            className="p-1 rounded hover:bg-white/20 transition-colors"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      ))}
    </div>
  )
}

export default Toast