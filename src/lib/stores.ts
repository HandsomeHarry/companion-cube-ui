import { writable } from 'svelte/store';
import type { EventRow, VaultItem, SummariesResponse, RhythmReport } from './types';
import { api } from './api';

export const activeView = writable<'history' | 'vault' | 'rhythm' | 'settings'>('history');
export const daemonOnline = writable(false);
export const historyEvents = writable<EventRow[]>([]);
export const vaultItems = writable<VaultItem[]>([]);
export const loading = writable(false);
export const error = writable<string | null>(null);
export const summaries = writable<SummariesResponse | null>(null);
export const summarizing = writable(false);
export const rhythmReport = writable<RhythmReport | null>(null);

export async function fetchRhythm(days = 7) {
  try {
    const data = await api.rhythm(days);
    rhythmReport.set(data);
    return data;
  } catch {
    rhythmReport.set(null);
    return null;
  }
}

export async function fetchSummaries(rangeKey?: string) {
  try {
    if (rangeKey) {
      const data = await api.summariesForKey(rangeKey);
      summaries.set(data);
      return data;
    } else {
      const data = await api.summaries();
      summaries.set(data);
      return data;
    }
  } catch {
    return null;
  }
}

export async function triggerSummarize(sinceMs?: number, untilMs?: number, rangeKey?: string) {
  summarizing.set(true);
  try {
    const data = await api.summarize(sinceMs, untilMs, rangeKey);
    summaries.set(data);
    return data;
  } catch (e: any) {
    error.set(e?.message || 'Summarize failed');
    return null;
  } finally {
    summarizing.set(false);
  }
}

// LLM Config
export const llmConfig = writable<{
  provider: string;
  url: string;
  model: string;
  has_token: boolean;
} | null>(null);

export async function fetchLlmConfig() {
  try {
    const config = await api.getLlmConfig();
    llmConfig.set(config);
    return config;
  } catch {
    llmConfig.set(null);
    return null;
  }
}

export async function saveLlmConfig(config: {
  provider?: string;
  url?: string;
  model?: string;
  token?: string;
}) {
  const result = await api.setLlmConfig(config);
  await fetchLlmConfig(); // Refresh
  return result;
}
