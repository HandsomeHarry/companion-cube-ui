import type { EventRow, SummariesResponse, RhythmReport } from './types';

const BASE = '/api';

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    headers: { 'Accept': 'application/json' },
    ...init,
  });
  if (!res.ok) {
    const text = await res.text().catch(() => '');
    const detail = text ? `: ${text.slice(0, 200)}` : '';
    throw new Error(`daemon ${res.status}${detail}`);
  }
  return res.json();
}

export const api = {
  health: () =>
    request<{ status: string; uptime_s: number; daemon_version: string }>('/health'),

  activity: (hours?: number) =>
    request<EventRow[]>(`/activity${hours ? `?hours=${hours}` : ''}`),

  recent: () =>
    request<EventRow[]>('/activity?hours=24'),

  llmHealth: () =>
    request<{ provider: string; url: string; model: string; reachable: boolean; model_present: boolean | null }>('/llm/health'),

  // LLM Configuration
  getLlmConfig: () =>
    request<{ provider: string; url: string; model: string; has_token: boolean }>('/config/llm'),

  setLlmConfig: (config: { provider?: string; url?: string; model?: string; token?: string }) =>
    request<{ status: string; message: string }>('/config/llm', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(config),
    }),

  // Summaries
  summaries: () =>
    request<SummariesResponse | null>('/summaries'),

  summarize: (sinceMs?: number, untilMs?: number, rangeKey?: string, full = false) =>
    request<SummariesResponse>('/summarize', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ since_ms: sinceMs, until_ms: untilMs, range_key: rangeKey, full }),
    }),

  summariesForKey: (rangeKey: string) =>
    request<SummariesResponse | null>(`/summaries?range_key=${encodeURIComponent(rangeKey)}`),

  // Rhythm analytics
  rhythm: (days?: number) =>
    request<RhythmReport>(`/rhythm${days ? `?days=${days}` : ''}`),

  // Group corrections — moves are applied to the sessions table immediately
  // and pin both ends; `record: false` is for undo (no correction logged).
  groupCorrection: (data: {
    event_id: number;
    to_session_id?: number | null;
    new_session_label?: string;
    record?: boolean;
  }) =>
    request<{ status: string; session_id: number }>('/corrections/group', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    }),

  renameSession: (id: number, label: string) =>
    request<{ status: string; session_id: number }>(`/sessions/${id}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ label }),
    }),
};
