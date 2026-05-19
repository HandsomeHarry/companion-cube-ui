<script lang="ts">
  import '../app.css';
  import Rail from '$components/Rail.svelte';
  import { activeView, daemonOnline, historyEvents, loading, error, llmConfig, fetchLlmConfig, saveLlmConfig, summaries, summarizing, fetchSummaries, triggerSummarize } from '$lib/stores';
  import { api } from '$lib/api';
  import { onMount } from 'svelte';
  import type { SessionGroup, EventRow } from '$lib/types';

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

  // History state — local mutable copies of groups for drag/rename
  let localGroups: SessionGroup[] = [];
  let expandedGroups = new Set<number>();
  let editingGroupIdx: number | null = null;
  let editTitle = '';
  let dragEvent: { event: EventRow; fromIdx: number } | null = null;
  let dragOverIdx: number | null = null;

  $: if ($summaries?.groups) {
    // Deep-copy so we can mutate locally (drag, rename)
    localGroups = $summaries.groups.map(g => ({
      ...g,
      events: [...g.events],
    }));
    expandedGroups = new Set(localGroups.map((_, i) => i)); // expand all by default
  }

  function toggleGroup(idx: number) {
    if (expandedGroups.has(idx)) expandedGroups.delete(idx);
    else expandedGroups.add(idx);
    expandedGroups = expandedGroups;
  }

  function handleGroupKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      (e.currentTarget as HTMLElement)?.click();
    }
  }

  function formatDuration(ms: number): string {
    const s = Math.round(ms / 1000);
    if (s < 60) return `${s}s`;
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m`;
    return `${Math.floor(m / 60)}h ${m % 60}m`;
  }

  function formatTime(ts: number): string {
    return new Date(ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  }

  // Open a link/file based on event data
  function openEvent(event: EventRow) {
    const title = event.title ?? '';
    const app = event.app ?? '';

    // If title looks like a URL, open it
    if (/^https?:\/\//i.test(title)) {
      openUrl(title);
      return;
    }
    // If it's a browser, try OCR text for a URL
    if (/brave|chrome|safari|firefox|edge/i.test(app) && event.ocr_text) {
      const urlMatch = event.ocr_text.match(/https?:\/\/[^\s]+/);
      if (urlMatch) {
        openUrl(urlMatch[0]);
        return;
      }
    }
    // For files (Word, etc) — title is the filename, we can try to open it
    if (/\.\w{2,4}$/.test(title) && title.length < 200) {
      // Could search for the file, but for now just show a toast
      // TODO: implement file opening via Tauri dialog
    }
    // For URLs in OCR text
    if (event.ocr_text) {
      const urlMatch = event.ocr_text.match(/https?:\/\/[^\s]+/);
      if (urlMatch) {
        openUrl(urlMatch[0]);
        return;
      }
    }
  }

  function openUrl(url: string) {
    const tauri = (window as any).__TAURI__;
    if (tauri?.shell?.open) {
      tauri.shell.open(url);
    } else {
      window.open(url, '_blank');
    }
  }

  // Drag & drop between groups
  function onDragStart(e: DragEvent, event: EventRow, fromIdx: number) {
    dragEvent = { event, fromIdx };
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      e.dataTransfer.setData('text/plain', String(event.id));
    }
  }

  function onDragOver(e: DragEvent, toIdx: number) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverIdx = toIdx;
  }

  function onDragLeave() {
    dragOverIdx = null;
  }

  async function onDrop(toIdx: number) {
    if (!dragEvent || dragEvent.fromIdx === toIdx) {
      dragEvent = null;
      dragOverIdx = null;
      return;
    }

    const { event: movedEvent, fromIdx } = dragEvent;
    const fromTitle = localGroups[fromIdx].title;
    const toTitle = localGroups[toIdx].title;

    // Remove from source, add to target
    localGroups[fromIdx].events = localGroups[fromIdx].events.filter(e => e.id !== movedEvent.id);
    localGroups[toIdx].events.push(movedEvent);

    // Recalc durations
    localGroups[fromIdx].total_duration_ms = localGroups[fromIdx].events.reduce((s, e) => s + (e.duration_ms ?? 0), 0);
    localGroups[toIdx].total_duration_ms = localGroups[toIdx].events.reduce((s, e) => s + (e.duration_ms ?? 0), 0);

    // Remove empty groups
    localGroups = localGroups.filter(g => g.events.length > 0);

    localGroups = localGroups; // reactivity
    dragEvent = null;
    dragOverIdx = null;

    // Send correction to backend
    try {
      await api.groupCorrection({
        event_id: movedEvent.id,
        from_group: fromTitle,
        to_group: toTitle,
      });
    } catch {
      // Silently fail — the UI already moved, correction is best-effort
    }
  }

  // Group rename
  function startRename(idx: number) {
    editingGroupIdx = idx;
    editTitle = localGroups[idx].title;
  }

  async function finishRename(idx: number) {
    if (!editTitle.trim() || editTitle === localGroups[idx].title) {
      editingGroupIdx = null;
      return;
    }
    const oldTitle = localGroups[idx].title;
    localGroups[idx].title = editTitle.trim();
    localGroups = localGroups;
    editingGroupIdx = null;

    // Record the rename as a correction
    try {
      await api.groupCorrection({
        event_id: localGroups[idx].events[0]?.id ?? 0,
        from_group: oldTitle,
        to_group: editTitle.trim(),
        renamed_to: editTitle.trim(),
      });
    } catch {}
  }

  // Settings helpers (unchanged)
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

  async function handleSummarize() {
    await triggerSummarize();
  }

  function autofocus(el: HTMLInputElement) { el.focus(); }

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

    loading.set(true);
    api.recent()
      .then((events) => {
        historyEvents.set(events.reverse());
      })
      .catch((e) => {
        error.set(e?.message || 'Fetch failed');
      })
      .finally(() => {
        loading.set(false);
      });

    // Fetch cached summaries
    fetchSummaries();

    const refreshInterval = setInterval(() => {
      if ($activeView === 'history') {
        api.recent()
          .then((events) => { historyEvents.set(events.reverse()); })
          .catch(() => {});
        fetchSummaries();
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

    {#if $activeView === 'history'}
      <!-- ==================== HISTORY VIEW ==================== -->
      <h1 class="heading">History</h1>
      <div class="bar">
        <div class="datenav">
          <button class="datenav__btn" aria-label="Previous day">←</button>
          <span class="datenav__label">Today, {new Date().toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}</span>
          <button class="datenav__btn" aria-label="Next day">→</button>
        </div>
        <div class="bar__right">
          <button class="btn btn--ghost" on:click={handleSummarize} disabled={$summarizing}>
            {$summarizing ? 'Organizing...' : '⚡ Organize'}
          </button>
          <div class="seg">
            <button class="seg__opt active">Day</button>
            <button class="seg__opt">Week</button>
            <button class="seg__opt">Month</button>
          </div>
        </div>
      </div>

      {#if $error}
        <p class="error-msg">{$error}</p>
      {:else if localGroups.length > 0}
        <!-- GROUPED TIMELINE -->
        <div class="timeline">
          {#each localGroups as group, idx}
            <div class="tl-group"
              class:drag-over={dragOverIdx === idx}
              on:dragover={(e) => onDragOver(e, idx)}
              on:dragleave={onDragLeave}
              on:drop={() => onDrop(idx)}
              role="list"
            >
              <div class="tl-gutter">
                <span class="tl-gutter__time">{formatTime(group.events[0]?.ts ?? Date.now())}</span>
                <span class="tl-gutter__dot" class:distraction={group.distraction}></span>
              </div>
              <div class="tl-body">
                <div class="tl-header" role="button" tabindex="0" on:click={() => toggleGroup(idx)} on:keydown={handleGroupKeydown}>
                  {#if editingGroupIdx === idx}
                    <!-- Inline rename -->
                    <input
                      class="tl-rename"
                      type="text"
                      bind:value={editTitle}
                      on:keydown={(e) => { if (e.key === 'Enter') finishRename(idx); if (e.key === 'Escape') editingGroupIdx = null; }}
                      on:blur={() => finishRename(idx)}
                      on:click|stopPropagation
                      use:autofocus
                    />
                  {:else}
                    <span class="tl-header__title" class:distraction={group.distraction}>
                      {group.title}
                    </span>
                    <button class="tl-header__edit" on:click|stopPropagation={() => startRename(idx)} title="Rename group">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/></svg>
                    </button>
                  {/if}
                  <span class="tl-header__count">{group.events.length}</span>
                  <span class="tl-header__dur">{formatDuration(group.total_duration_ms)}</span>
                  <span class="tl-header__toggle">{expandedGroups.has(idx) ? '▾' : '▸'}</span>
                </div>

                {#if expandedGroups.has(idx)}
                  <div class="tl-items">
                    {#each group.events as event (event.id)}
                      <div class="tl-item"
                        on:click={() => openEvent(event)}
                        on:keydown={(e) => { if (e.key === 'Enter') openEvent(event); }}
                        role="button"
                        tabindex="0"
                      >
                        <span class="tl-item__bullet">·</span>
                        <span class="tl-item__app">{event.app ?? event.kind}</span>
                        {#if event.title}
                          <span class="tl-item__title">– {event.title}</span>
                        {/if}
                        {#if event.duration_ms}
                          <span class="tl-item__dur">{formatDuration(event.duration_ms)}</span>
                        {/if}
                        <span class="tl-item__handle" draggable="true" role="button" tabindex="-1" on:dragstart={(e) => onDragStart(e, event, idx)} title="Drag to another group">≡</span>
                      </div>
                    {/each}
                  </div>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      {:else if $historyEvents.length > 0}
        <!-- FLAT TIMELINE (no summaries yet) -->
        <div class="timeline-flat">
          {#each $historyEvents as event (event.id)}
            <div class="tl-row">
              <span class="tl-time">{formatTime(event.ts)}</span>
              <span class="tl-dot" style="background: {event.kind === 'app_focus' ? 'var(--brand-orange)' : event.kind === 'idle_start' ? '#aaa' : '#888'}"></span>
              <span class="tl-app">{event.app ?? event.kind}</span>
              {#if event.title}
                <span class="tl-detail">– {event.title}</span>
              {/if}
              {#if event.duration_ms}
                <span class="tl-dur">{formatDuration(event.duration_ms)}</span>
              {/if}
            </div>
          {/each}
        </div>
      {:else if $loading}
        <p class="hint">Loading...</p>
      {:else}
        <p class="hint">No activity recorded yet. Start the daemon to begin capturing.</p>
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
  <slot />
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
    font-family: var(--font-display);
    font-size: 32px;
    font-weight: 700;
    color: var(--brand-orange-deep);
    margin-bottom: 8px;
  }

  /* ---- History bar ---- */
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
  }

  .bar__right {
    display: flex;
    align-items: center;
    gap: 10px;
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
    transition: background var(--t-fast) var(--ease);
  }

  .datenav__btn:hover { background: var(--row-hover); }

  .datenav__label { color: var(--ink); font-size: 14px; }

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
    transition: all var(--t-fast) var(--ease);
  }

  .seg__opt.active {
    background: var(--card-white);
    color: var(--ink);
    box-shadow: var(--shadow-rest);
  }

  /* ---- Buttons ---- */
  .btn {
    border-radius: var(--r-control);
    padding: 8px 16px;
    font-size: 13px;
    font-weight: 600;
    border: none;
    cursor: pointer;
    transition: all var(--t-fast) var(--ease);
    font-family: var(--font-ui);
  }

  .btn--ghost {
    background: var(--card-white);
    border: 1px solid var(--divider);
    color: var(--ink);
  }

  .btn--ghost:hover { background: var(--row-hover); }
  .btn--ghost:disabled { opacity: 0.5; cursor: not-allowed; }

  .btn--primary {
    background: var(--brand-orange);
    color: var(--on-orange);
  }

  .btn--primary:hover { background: var(--brand-orange-hov); }
  .btn--primary:disabled { opacity: 0.5; cursor: not-allowed; }

  /* ---- Grouped timeline ---- */
  .timeline { padding-top: 4px; }

  .tl-group {
    display: grid;
    grid-template-columns: 56px 1fr;
    margin-bottom: 4px;
    border-radius: var(--r-panel);
    transition: background var(--t-fast) var(--ease);
  }

  .tl-group.drag-over {
    background: var(--row-hover);
    outline: 2px dashed var(--brand-orange);
    outline-offset: -2px;
  }

  .tl-gutter {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    padding-right: 12px;
    padding-top: 10px;
  }

  .tl-gutter__time {
    color: var(--ink-soft);
    font-size: 12px;
  }

  .tl-gutter__dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--brand-orange);
    margin-top: 8px;
    box-shadow: 0 0 0 3px rgba(241, 106, 1, 0.15);
  }

  .tl-gutter__dot.distraction {
    background: var(--ink-faint);
    box-shadow: 0 0 0 3px rgba(163, 155, 142, 0.15);
  }

  .tl-body { min-width: 0; }

  .tl-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--r-control);
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }

  .tl-header:hover { background: var(--row-hover); }

  .tl-header__title {
    font-size: 15px;
    font-weight: 700;
    color: var(--ink);
  }

  .tl-header__title.distraction { color: var(--ink-soft); }

  .tl-header__edit {
    color: var(--ink-faint);
    display: grid;
    place-items: center;
    padding: 2px;
    border-radius: 4px;
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease);
  }

  .tl-header:hover .tl-header__edit { opacity: 1; }
  .tl-header__edit:hover { background: var(--paper-panel); }

  .tl-header__count {
    font-size: 11px;
    color: var(--on-orange);
    background: var(--brand-orange);
    border-radius: var(--r-pill);
    padding: 1px 8px;
    font-weight: 600;
  }

  .tl-header__count:empty { display: none; }

  .tl-header__dur {
    font-size: 12px;
    color: var(--ink-faint);
  }

  .tl-header__toggle {
    font-size: 11px;
    color: var(--ink-faint);
    margin-left: auto;
  }

  .tl-rename {
    font: 700 15px var(--font-ui);
    color: var(--ink);
    border: 1px solid var(--brand-orange);
    border-radius: 6px;
    padding: 2px 8px;
    background: var(--card-white);
    outline: none;
    flex: 1;
  }

  .tl-items {
    padding-left: 6px;
    animation: slideDown 200ms var(--ease);
  }

  @keyframes slideDown {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .tl-item {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 8px;
    border-radius: 6px;
    font-size: 14px;
    color: var(--ink);
    transition: background var(--t-fast) var(--ease);
    cursor: pointer;
  }

  .tl-item:hover { background: var(--row-hover); }

  .tl-item__bullet { color: var(--ink-faint); font-size: 16px; }
  .tl-item__app { font-weight: 600; }

  .tl-item__title {
    color: var(--ink-soft);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }

  .tl-item__dur {
    color: var(--ink-faint);
    font-size: 12px;
    flex-shrink: 0;
  }

  .tl-item__handle {
    color: var(--ink-faint);
    cursor: grab;
    font-size: 16px;
    flex-shrink: 0;
    padding: 2px 6px;
    border-radius: 4px;
    transition: background var(--t-fast) var(--ease);
    user-select: none;
  }

  .tl-item__handle:hover {
    background: var(--paper-panel);
    color: var(--ink-soft);
  }

  /* ---- Flat timeline (fallback) ---- */
  .timeline-flat { padding-top: 8px; }

  .tl-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px;
    border-radius: 8px;
    font-size: 14px;
    transition: background var(--t-fast) var(--ease);
  }

  .tl-row:hover { background: var(--row-hover); }

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

  .tl-app { font-weight: 600; color: var(--ink); }

  .tl-detail {
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

  /* ---- Utilities ---- */
  .error-msg {
    color: #dc2626;
    font-size: 14px;
    padding: 12px;
    background: #fef2f2;
    border-radius: var(--r-control);
  }

  .hint {
    color: var(--ink-soft);
    font-size: 14px;
    padding: 20px 0;
  }

  /* ---- Settings ---- */
  .card {
    background: var(--card-white);
    border-radius: var(--r-panel);
    box-shadow: var(--shadow-rest);
    padding: 20px 24px;
    max-width: 480px;
  }

  .card__title {
    font-family: var(--font-display);
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

  .card__label { font-size: 14px; color: var(--ink-soft); }

  .card__value { font-size: 14px; color: var(--ink); font-weight: 500; }
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

  .field { margin-bottom: 14px; }

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

  .field__input:focus { border-color: var(--brand-orange); }
  .field__input::placeholder { color: var(--ink-faint); }

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

  .field__select:focus { border-color: var(--brand-orange); }

  .token-row { display: flex; gap: 8px; }
  .token-row .field__input { flex: 1; }

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
</style>
