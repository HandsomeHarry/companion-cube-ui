import { writable } from 'svelte/store';
import type { EventRow, VaultItem } from './types';
import { api } from './api';

export const activeView = writable<'history' | 'vault' | 'settings'>('history');
export const daemonOnline = writable(false);
export const historyEvents = writable<EventRow[]>([]);
export const vaultItems = writable<VaultItem[]>([]);
export const loading = writable(false);
export const error = writable<string | null>(null);

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
