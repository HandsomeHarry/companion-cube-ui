import type { EventRow, SummariesResponse } from './types';

const BASE = 'http://127.0.0.1:7431';

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

  summarize: () =>
    request<SummariesResponse>('/summarize', { method: 'POST' }),

  // Group corrections
  groupCorrection: (data: { event_id: number; from_group: string; to_group: string; renamed_to?: string }) =>
    request<{ status: string; message: string }>('/corrections/group', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    }),
};
