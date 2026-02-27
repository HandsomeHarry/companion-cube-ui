import { useState, useEffect, useCallback } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Star, Trash2, Search, Archive } from 'lucide-react'
import { getThemeClasses } from '../utils/theme'

interface VaultItem {
  id: string
  title: string
  source_app: string
  url: string | null
  saved_at: string
  is_favorited: boolean
  notes: string
}

interface VaultProps {
  isDarkMode: boolean
  currentMode: string
}

function timeAgo(isoString: string): string {
  const diffMs = Date.now() - new Date(isoString).getTime()
  const mins = Math.floor(diffMs / 60000)
  const hours = Math.floor(diffMs / 3600000)
  const days = Math.floor(diffMs / 86400000)
  if (mins < 1) return 'just now'
  if (mins < 60) return `${mins}m ago`
  if (hours < 24) return `${hours}h ago`
  if (days < 7) return `${days}d ago`
  return new Date(isoString).toLocaleDateString()
}

function isStale(isoString: string): boolean {
  return Date.now() - new Date(isoString).getTime() > 7 * 86400000
}

function Vault({ isDarkMode, currentMode }: VaultProps) {
  const [items, setItems] = useState<VaultItem[]>([])
  const [search, setSearch] = useState('')
  const [filter, setFilter] = useState<'all' | 'favorites'>('all')
  // Local notes state so edits are instant without hitting the backend on every keystroke
  const [localNotes, setLocalNotes] = useState<Record<string, string>>({})
  const [isLoading, setIsLoading] = useState(true)

  const themeClasses = getThemeClasses(currentMode, isDarkMode)

  const loadItems = useCallback(async () => {
    try {
      const result = await invoke<VaultItem[]>('get_vault_items')
      setItems(result)
      const notes: Record<string, string> = {}
      result.forEach(item => { notes[item.id] = item.notes })
      setLocalNotes(notes)
    } catch (error) {
      console.error('Failed to load vault items:', error)
    } finally {
      setIsLoading(false)
    }
  }, [])

  useEffect(() => { loadItems() }, [loadItems])

  const handleFavorite = async (item: VaultItem) => {
    try {
      await invoke('update_vault_item', {
        id: item.id,
        is_favorited: !item.is_favorited,
        notes: localNotes[item.id] ?? item.notes,
      })
      setItems(prev => prev.map(i =>
        i.id === item.id ? { ...i, is_favorited: !i.is_favorited } : i
      ))
    } catch (error) {
      console.error('Failed to toggle favorite:', error)
    }
  }

  const handleDelete = async (id: string) => {
    try {
      await invoke('delete_vault_item', { id })
      setItems(prev => prev.filter(i => i.id !== id))
    } catch (error) {
      console.error('Failed to delete vault item:', error)
    }
  }

  // Called on textarea blur — only writes to backend if content changed
  const handleNotesSave = async (item: VaultItem) => {
    const notes = localNotes[item.id] ?? item.notes
    if (notes === item.notes) return
    try {
      await invoke('update_vault_item', {
        id: item.id,
        is_favorited: item.is_favorited,
        notes,
      })
      setItems(prev => prev.map(i => i.id === item.id ? { ...i, notes } : i))
    } catch (error) {
      console.error('Failed to save notes:', error)
    }
  }

  const filtered = items.filter(item => {
    const q = search.trim().toLowerCase()
    const matchesSearch =
      q === '' ||
      item.title.toLowerCase().includes(q) ||
      item.source_app.toLowerCase().includes(q) ||
      (localNotes[item.id] ?? item.notes).toLowerCase().includes(q)
    const matchesFilter = filter === 'all' || item.is_favorited
    return matchesSearch && matchesFilter
  })

  // Styling helpers
  const inputBg = isDarkMode
    ? 'bg-gray-800 border-gray-700 text-white placeholder-gray-500'
    : 'bg-white border-gray-200 text-gray-900 placeholder-gray-400'
  const cardSurface = isDarkMode
    ? 'bg-gray-800/60 border-gray-700/50'
    : 'bg-white border-gray-200'
  const notesBg = isDarkMode
    ? 'bg-gray-700/50 text-gray-300 placeholder-gray-500'
    : 'bg-gray-50 text-gray-600 placeholder-gray-400'

  return (
    <div className={`flex-1 overflow-y-auto h-screen ${themeClasses.background}`}>
      <div className="max-w-2xl mx-auto px-6 py-8">

        {/* Header */}
        <div className="mb-6">
          <div className="flex items-center gap-3 mb-1">
            <Archive className="w-6 h-6" style={{ color: themeClasses.primary }} />
            <h1 className={`text-2xl font-bold ${themeClasses.textPrimary}`}>Vault</h1>
          </div>
          <p className={`text-sm ${themeClasses.textSecondary}`}>
            A quiet place for your rabbit holes. Revisit when you're ready.
          </p>
        </div>

        {/* Search */}
        <div className="relative mb-4">
          <Search className={`absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 ${themeClasses.textSecondary}`} />
          <input
            type="text"
            placeholder="Search vault..."
            value={search}
            onChange={e => setSearch(e.target.value)}
            className={`w-full pl-9 pr-4 py-2.5 rounded-lg border text-sm outline-none ${inputBg}`}
          />
        </div>

        {/* Filter tabs */}
        <div className="flex gap-2 mb-6">
          {(['all', 'favorites'] as const).map(tab => (
            <button
              key={tab}
              onClick={() => setFilter(tab)}
              className={`px-4 py-1.5 rounded-full text-sm font-medium transition-colors ${
                filter === tab
                  ? 'text-white'
                  : `${themeClasses.textSecondary} hover:bg-white/5`
              }`}
              style={filter === tab ? { background: themeClasses.primary } : {}}
            >
              {tab === 'all' ? 'All' : '★ Favorites'}
            </button>
          ))}
        </div>

        {/* Content */}
        {isLoading ? (
          <div className={`text-sm text-center py-12 ${themeClasses.textSecondary}`}>
            Loading...
          </div>
        ) : filtered.length === 0 ? (
          <div className={`text-center py-16 ${themeClasses.textSecondary}`}>
            <Archive className="w-12 h-12 mx-auto mb-3 opacity-20" />
            <p className="text-sm font-medium mb-1">
              {search || filter === 'favorites' ? 'Nothing found' : 'Your vault is empty'}
            </p>
            {!search && filter === 'all' && (
              <p className="text-xs opacity-60 max-w-xs mx-auto">
                When a nudge catches you going down a rabbit hole, tap "Save for Later"
                to park it here.
              </p>
            )}
          </div>
        ) : (
          <div className="space-y-3">
            {filtered.map(item => (
              <div key={item.id} className={`rounded-xl border p-4 ${cardSurface}`}>
                <div className="flex items-start gap-3">

                  {/* Favorite toggle */}
                  <button onClick={() => handleFavorite(item)} className="mt-0.5 flex-shrink-0">
                    <Star
                      className="w-4 h-4 transition-colors"
                      style={{
                        color: item.is_favorited ? themeClasses.primary : undefined,
                        opacity: item.is_favorited ? 1 : 0.3,
                        fill: item.is_favorited ? themeClasses.primary : 'none',
                      }}
                    />
                  </button>

                  {/* Body */}
                  <div className="flex-1 min-w-0">
                    <p className={`text-sm font-medium leading-snug ${themeClasses.textPrimary}`}>
                      {item.title}
                    </p>

                    {/* Meta row */}
                    <div className="flex items-center gap-1.5 mt-1 flex-wrap">
                      <span className={`text-xs ${themeClasses.textSecondary}`}>
                        {item.source_app}
                      </span>
                      <span className={`text-xs opacity-40 ${themeClasses.textSecondary}`}>·</span>
                      <span className={`text-xs ${themeClasses.textSecondary}`}>
                        {timeAgo(item.saved_at)}
                      </span>
                      {isStale(item.saved_at) && (
                        <>
                          <span className={`text-xs opacity-40 ${themeClasses.textSecondary}`}>·</span>
                          <span className="text-xs text-amber-500 font-medium">
                            Still interested?
                          </span>
                        </>
                      )}
                    </div>

                    {/* Notes — auto-saves on blur */}
                    <textarea
                      rows={1}
                      placeholder="Add a note..."
                      value={localNotes[item.id] ?? item.notes}
                      onChange={e =>
                        setLocalNotes(prev => ({ ...prev, [item.id]: e.target.value }))
                      }
                      onBlur={() => handleNotesSave(item)}
                      className={`mt-2 w-full px-2 py-1.5 rounded text-xs border-0 outline-none resize-none ${notesBg}`}
                    />
                  </div>

                  {/* Delete */}
                  <button
                    onClick={() => handleDelete(item.id)}
                    className="mt-0.5 flex-shrink-0 opacity-25 hover:opacity-60 transition-opacity"
                  >
                    <Trash2 className={`w-4 h-4 ${themeClasses.textSecondary}`} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

export default Vault
