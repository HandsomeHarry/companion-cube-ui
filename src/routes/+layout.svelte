<script lang="ts">
  import '../app.css';
  import Rail from '$components/Rail.svelte';
  import { activeView, daemonOnline, historyEvents, loading, error, llmConfig, fetchLlmConfig, saveLlmConfig } from '$lib/stores';
  import { api } from '$lib/api';
  import { onMount } from 'svelte';

  let fetchStatus = 'not started';

  // Settings state
  let daemonVersion = '';
  let uptime = '';
  let provider = '';
  let llmUrl = '';
  let llmModel = '';
  let llmToken = '';
  let showToken = false;
  let saving = false;
  let saveMsg = '';
  let daemonStarting = false;
  let daemonMsg = '';

  async function refreshDaemonInfo() {
    try {
      const data = await api.health();
      daemonVersion = data.daemon_version || '';
      const mins = Math.floor(data.uptime_s / 60);
      const hrs = Math.floor(mins / 60);
      uptime = hrs > 0 ? `${hrs}h ${mins % 60}m` : `${mins}m`;
    } catch {}
  }

  async function handleStartDaemon() {
    daemonStarting = true;
    daemonMsg = '';
    try {
      const { invoke } = (window as any).__TAURI__;
      if (!invoke) {
        daemonMsg = 'Tauri not available — start manually: cargo run --bin ccube-daemon';
        return;
      }
      daemonMsg = 'Starting daemon...';
      const result = await invoke('start_daemon');
      daemonMsg = result as string || 'Daemon started';
      await new Promise(r => setTimeout(r, 3000));
      try {
        const res = await fetch('http://127.0.0.1:7431/health');
        if (res.ok) {
          daemonOnline.set(true);
          daemonMsg = 'Daemon is running ✓';
          await refreshDaemonInfo();
          await loadLlmSettings();
        }
      } catch {
        daemonMsg = 'Daemon started but not responding yet. Check again shortly.';
      }
    } catch (e: any) {
      daemonMsg = `Error: ${e?.message || e}`;
    } finally {
      daemonStarting = false;
    }
  }

  async function loadLlmSettings() {
    const config = await fetchLlmConfig();
    if (config) {
      provider = config.provider;
      llmUrl = config.url;
      llmModel = config.model;
    }
  }

  async function handleSaveLlm() {
    saving = true;
    saveMsg = '';
    try {
      const payload: Record<string, string> = {};
      if (provider) payload.provider = provider;
      if (llmUrl) payload.url = llmUrl;
      if (llmModel) payload.model = llmModel;
      if (llmToken) payload.token = llmToken;
      const result = await saveLlmConfig(payload);
      saveMsg = result.message || 'Saved';
      llmToken = '';
    } catch (e: any) {
      saveMsg = `Error: ${e?.message || 'Save failed'}`;
    } finally {
      saving = false;
    }
  }

  function handleViewChange(view: 'history' | 'vault' | 'settings') {
    $activeView = view;
  }

  onMount(() => {
    const checkHealth = async () => {
      try {
        await api.health();
        daemonOnline.set(true);
      } catch {
        daemonOnline.set(false);
      }
    };
    checkHealth();
    const healthInterval = setInterval(checkHealth, 10_000);
    refreshDaemonInfo();
    loadLlmSettings();

    fetchStatus = 'fetching history...';
    loading.set(true);
    api.recent()
      .then((events) => {
        fetchStatus = `got ${events.length} events`;
        historyEvents.set(events.reverse());
      })
      .catch((e) => {
        fetchStatus = `error: ${e?.message || e}`;
        error.set(e?.message || 'Fetch failed');
      })
      .finally(() => {
        loading.set(false);
      });

    const refreshInterval = setInterval(() => {
      if ($activeView === 'history') {
        api.recent()
          .then((events) => { historyEvents.set(events.reverse()); })
          .catch(() => {});
      }
    }, 30_000);

    return () => {
      clearInterval(healthInterval);
      clearInterval(refreshInterval);
    };
  });
</script>

<div class="app">
  <Rail onViewChange={handleViewChange} />
  <main class="content">
    <!-- DEBUG BANNER -->
    <div style="background:#ffe0b2;padding:8px 12px;border-radius:6px;font-size:12px;margin-bottom:12px;font-family:monospace;word-break:break-all">
      <div>fetch: {fetchStatus}</div>
      <div>daemon: {$daemonOnline} | loading: {$loading} | events: {$historyEvents.length} | view: {$activeView}</div>
    </div>

    {#if $activeView === 'history'}
      <!-- HISTORY VIEW -->
      <h1 class="heading">History</h1>
      <div class="bar">
        <div class="datenav">
          <button class="datenav__btn" aria-label="Previous day">←</button>
          <span class="datenav__label">Today, {new Date().toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}</span>
          <button class="datenav__btn" aria-label="Next day">→</button>
        </div>
        <div class="seg">
          <button class="seg__opt active">Day</button>
          <button class="seg__opt">Week</button>
          <button class="seg__opt">Month</button>
        </div>
      </div>

      {#if $error}
        <p style="color:#dc2626">{$error}</p>
      {:else if $historyEvents.length === 0 && !$loading}
        <p style="color:var(--ink-soft)">No activity recorded yet.</p>
      {:else if $loading}
        <p style="color:var(--ink-soft)">Loading...</p>
      {:else}
        <div class="timeline">
          {#each $historyEvents as event (event.id)}
            <div class="tl-row">
              <span class="tl-time">{new Date(event.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>
              <span class="tl-dot" style="background: {event.kind === 'app_focus' ? 'var(--brand-orange)' : event.kind === 'idle_start' ? '#aaa' : '#888'}"></span>
              <span class="tl-app">{event.app ?? event.kind}</span>
              {#if event.title}
                <span class="tl-title">– {event.title}</span>
              {/if}
              {#if event.duration_ms}
                <span class="tl-dur">{Math.round(event.duration_ms / 1000)}s</span>
              {/if}
            </div>
          {/each}
        </div>
      {/if}

    {:else if $activeView === 'vault'}
      <!-- VAULT VIEW -->
      <h1 class="heading">Vault</h1>
      {#if !$daemonOnline}
        <p style="color:var(--ink-soft)">🔌 Daemon is not running.</p>
      {:else}
        <p style="color:var(--ink-soft)">🏦 Your vault is empty. Save distractions here when nudged, or add ideas manually.</p>
      {/if}

    {:else if $activeView === 'settings'}
      <!-- SETTINGS VIEW -->
      <h1 class="heading">Settings</h1>

      <!-- App Info -->
      <div class="card">
        <h2 class="card__title">Companion Cube</h2>
        <div class="card__row">
          <span class="card__label">App version</span>
          <span class="card__value">0.2.1</span>
        </div>
        <div class="card__row">
          <span class="card__label">Daemon</span>
          <span class="card__value" class:online={$daemonOnline} class:offline={!$daemonOnline}>
            {$daemonOnline ? 'Connected' : 'Offline'}
          </span>
        </div>
        {#if $daemonOnline}
          <div class="card__row">
            <span class="card__label">Daemon version</span>
            <span class="card__value">{daemonVersion}</span>
          </div>
          <div class="card__row">
            <span class="card__label">Uptime</span>
            <span class="card__value">{uptime}</span>
          </div>
        {:else}
          <div class="card__actions">
            <button class="btn btn--primary" on:click={handleStartDaemon} disabled={daemonStarting}>
              {daemonStarting ? 'Starting...' : 'Start Daemon'}
            </button>
            {#if daemonMsg}
              <p class="status-msg">{daemonMsg}</p>
            {/if}
          </div>
        {/if}
      </div>

      <!-- LLM Configuration -->
      <div class="card" style="margin-top: 20px;">
        <h2 class="card__title">LLM Configuration</h2>
        <p class="card__desc">
          Configure the AI provider for drift detection, curation, and reflections.
          Changes are written to <code>.env</code> and require a daemon restart.
        </p>

        {#if $llmConfig}
          <form on:submit|preventDefault={handleSaveLlm}>
            <div class="field">
              <label class="field__label" for="provider">Provider</label>
              <select id="provider" class="field__select" bind:value={provider}>
                <option value="openai-compatible">OpenAI Compatible</option>
                <option value="openai">OpenAI</option>
                <option value="anthropic">Anthropic</option>
                <option value="ollama">Ollama (local)</option>
                <option value="llamacpp">llama.cpp (local)</option>
              </select>
            </div>

            <div class="field">
              <label class="field__label" for="url">API Base URL</label>
              <input id="url" type="url" class="field__input" bind:value={llmUrl} placeholder="http://localhost:8080" />
            </div>

            <div class="field">
              <label class="field__label" for="model">Model</label>
              <input id="model" type="text" class="field__input" bind:value={llmModel} placeholder="default" />
            </div>

            <div class="field">
              <label class="field__label" for="token">
                API Token
                {#if $llmConfig?.has_token}
                  <span class="token-badge">● Set</span>
                {/if}
              </label>
              <div class="token-row">
                <input id="token" type={showToken ? 'text' : 'password'} class="field__input" bind:value={llmToken} placeholder="Leave empty to keep current" />
                <button type="button" class="token-toggle" on:click={() => (showToken = !showToken)}>
                  {showToken ? '🙈' : '👁'}
                </button>
              </div>
            </div>

            {#if saveMsg}
              <p class="save-msg">{saveMsg}</p>
            {/if}

            <div class="actions">
              <button type="submit" class="btn btn--primary" disabled={saving}>
                {saving ? 'Saving...' : 'Save Configuration'}
              </button>
            </div>
          </form>
        {:else if $daemonOnline}
          <p class="card__desc">Loading configuration...</p>
        {:else}
          <p class="card__desc">🔌 Start the daemon to configure LLM settings.</p>
        {/if}
      </div>
    {/if}
  </main>
</div>

<style>
  .app {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .content {
    flex: 1;
    padding: 30px;
    overflow-y: auto;
    background: var(--paper);
  }

  .heading {
    font-family: var(--serif);
    font-size: 22px;
    font-weight: 700;
    color: var(--ink);
    margin-bottom: 16px;
  }

  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
  }

  .datenav {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 14px;
  }

  .datenav__btn {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 14px;
    color: var(--ink-soft);
  }

  .datenav__label {
    color: var(--ink);
  }

  .seg {
    display: inline-flex;
    background: var(--paper-panel);
    border-radius: var(--r-pill);
    padding: 3px;
  }

  .seg__opt {
    padding: 6px 14px;
    border-radius: var(--r-pill);
    font-size: 13px;
    font-weight: 600;
    color: var(--ink-soft);
  }

  .seg__opt.active {
    background: var(--card-white);
    color: var(--ink);
    box-shadow: var(--shadow-rest);
  }

  .timeline {
    padding-top: 8px;
  }

  .tl-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px;
    border-radius: 8px;
    font-size: 14px;
  }

  .tl-row:hover {
    background: var(--row-hover);
  }

  .tl-time {
    color: var(--ink-soft);
    font-size: 13px;
    min-width: 50px;
    text-align: right;
  }

  .tl-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .tl-app {
    font-weight: 600;
    color: var(--ink);
  }

  .tl-title {
    color: var(--ink-soft);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tl-dur {
    color: var(--ink-faint);
    font-size: 12px;
    margin-left: auto;
  }

  .card {
    background: var(--card-white);
    border-radius: var(--r-panel);
    box-shadow: var(--shadow-rest);
    padding: 20px 24px;
    max-width: 480px;
  }

  .card__title {
    font-family: var(--serif);
    font-size: 16px;
    font-weight: 700;
    color: var(--ink);
    margin-bottom: 16px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--divider);
  }

  .card__row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 0;
  }

  .card__label {
    font-size: 14px;
    color: var(--ink-soft);
  }

  .card__value {
    font-size: 14px;
    color: var(--ink);
    font-weight: 500;
  }

  .card__value.online { color: #16a34a; }
  .card__value.offline { color: #dc2626; }

  .card__actions {
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--divider);
  }

  .card__desc {
    font-size: 13px;
    color: var(--ink-soft);
    line-height: 1.5;
    margin-bottom: 16px;
  }

  .card__desc code {
    background: var(--paper-panel);
    padding: 2px 6px;
    border-radius: 4px;
    font-size: 12px;
  }

  .field {
    margin-bottom: 14px;
  }

  .field__label {
    display: block;
    font-size: 13px;
    font-weight: 600;
    color: var(--ink);
    margin-bottom: 6px;
  }

  .field__input {
    width: 100%;
    padding: 8px 12px;
    border: 1px solid var(--divider);
    border-radius: var(--r-control);
    font-size: 14px;
    color: var(--ink);
    background: var(--paper);
    outline: none;
    transition: border-color var(--t-fast) var(--ease);
  }

  .field__input:focus {
    border-color: var(--brand-orange);
  }

  .field__input::placeholder {
    color: var(--ink-faint);
  }

  .field__select {
    width: 100%;
    padding: 8px 12px;
    border: 1px solid var(--divider);
    border-radius: var(--r-control);
    font-size: 14px;
    color: var(--ink);
    background: var(--paper);
    outline: none;
    cursor: pointer;
  }

  .field__select:focus {
    border-color: var(--brand-orange);
  }

  .token-row {
    display: flex;
    gap: 8px;
  }

  .token-row .field__input {
    flex: 1;
  }

  .token-toggle {
    width: 36px;
    height: 36px;
    border: 1px solid var(--divider);
    border-radius: var(--r-control);
    background: var(--paper);
    cursor: pointer;
    display: grid;
    place-items: center;
    font-size: 16px;
    flex-shrink: 0;
  }

  .token-badge {
    display: inline-block;
    font-size: 11px;
    color: #16a34a;
    font-weight: 600;
    margin-left: 8px;
  }

  .save-msg {
    font-size: 13px;
    color: var(--brand-orange);
    margin: 8px 0;
  }

  .status-msg {
    font-size: 13px;
    color: var(--ink-soft);
    margin-top: 8px;
    line-height: 1.4;
  }

  .actions {
    margin-top: 16px;
    display: flex;
    justify-content: flex-end;
  }

  .btn {
    border-radius: var(--r-control);
    padding: 10px 20px;
    font-size: 14px;
    font-weight: 600;
    border: none;
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
  }

  .btn--primary {
    background: var(--brand-orange);
    color: var(--on-orange);
  }

  .btn--primary:hover {
    background: var(--brand-orange-hov);
  }

  .btn--primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
