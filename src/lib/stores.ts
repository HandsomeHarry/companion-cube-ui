import { writable } from 'svelte/store';
import type { EventRow, VaultItem, SummariesResponse } from './types';
import { api } from './api';

export const activeView = writable<'history' | 'vault' | 'settings'>('history');
export const daemonOnline = writable(false);
export const historyEvents = writable<EventRow[]>([]);
export const vaultItems = writable<VaultItem[]>([]);
export const loading = writable(false);
export const error = writable<string | null>(null);
export const summaries = writable<SummariesResponse | null>(null);
export const summarizing = writable(false);

export async function fetchSummaries() {
  try {
    const data = await api.summaries();
    summaries.set(data);
    return data;
  } catch {
    return null;
  }
}

export async function triggerSummarize(sinceMs?: number, untilMs?: number) {
  summarizing.set(true);
  try {
    const data = await api.summarize(sinceMs, untilMs);
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
