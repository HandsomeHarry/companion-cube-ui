<script lang="ts">
  import '../app.css';
  import Rail from '$components/Rail.svelte';
  import Rhythm from '$components/Rhythm.svelte';
  import { activeView, daemonOnline, historyEvents, loading, error, llmConfig, fetchLlmConfig, saveLlmConfig, summaries, summarizing, fetchSummaries, triggerSummarize } from '$lib/stores';
  import { rhythmReport, fetchRhythm } from '$lib/stores';
  import { api } from '$lib/api';
  import { onMount } from 'svelte';
  import type { SessionGroup, EventRow } from '$lib/types';
  import { THEMES, getInitialTheme, applyTheme, type ThemeName } from '$lib/theme';

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
  let theme: ThemeName = 'paper';

  // History state — local mutable copies of groups for drag/rename
  let localGroups: SessionGroup[] = [];
  let expandedGroups: number[] = [];
  let editingGroupIdx: number | null = null;
  let editTitle = '';
  let dragState: { event: EventRow; fromGroup: string } | null = null;
  let dragOverGroup: string | null = null;
  let isDragging = false;
  let dragX = 0;
  let dragY = 0;

  // Pointer-based drag: mousedown on ≡ handle → mousemove tracks hover → mouseup drops
  function startDrag(e: MouseEvent, event: EventRow, groupTitle: string) {
    if (e.button !== 0) return;
    e.preventDefault();
    dragState = { event, fromGroup: groupTitle };
    isDragging = false;

    const onMove = (ev: MouseEvent) => {
      if (!dragState) return cleanup();
      isDragging = true;
      dragX = ev.clientX;
      dragY = ev.clientY;
      const el = document.elementFromPoint(ev.clientX, ev.clientY);
      if (el) {
        const group = el.closest('[data-group-title]');
        dragOverGroup = group ? group.getAttribute('data-group-title') : null;
      }
    };

    const onUp = () => {
      if (dragState && dragOverGroup && dragOverGroup !== dragState.fromGroup) {
        finishDrop(dragState.fromGroup, dragOverGroup, dragState.event);
      }
      cleanup();
    };

    const cleanup = () => {
      dragState = null;
      dragOverGroup = null;
      isDragging = false;
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  }

  async function finishDrop(fromGroupTitle: string, toGroupTitle: string, movedEvent: EventRow) {
    const fromGroup = localGroups.find(g => g.title === fromGroupTitle);
    const toGroup = localGroups.find(g => g.title === toGroupTitle);
    if (!fromGroup || !toGroup) return;

    fromGroup.events = fromGroup.events.filter(e => e.id !== movedEvent.id);
    toGroup.events.push(movedEvent);
    fromGroup.total_duration_ms = fromGroup.events.reduce((s, e) => s + (e.duration_ms ?? 0), 0);
    toGroup.total_duration_ms = toGroup.events.reduce((s, e) => s + (e.duration_ms ?? 0), 0);
    localGroups = localGroups.filter(g => g.events.length > 0);
    localGroups = localGroups;

    try {
      await api.groupCorrection({
        event_id: movedEvent.id,
        from_group: fromGroupTitle,
        to_group: toGroupTitle,
      });
    } catch {}
  }
  let lastGeneratedAt: number = 0;

  function dateCacheKey(sel: Date, mode: string): string {
    if (mode === 'day') return `day:${sel.getFullYear()}-${String(sel.getMonth() + 1).padStart(2, '0')}-${String(sel.getDate()).padStart(2, '0')}`;
    if (mode === 'week') {
      const s = startOfWeek(sel);
      return `week:${s.getFullYear()}-${String(s.getMonth() + 1).padStart(2, '0')}-${String(s.getDate()).padStart(2, '0')}`;
    }
    return `month:${sel.getFullYear()}-${String(sel.getMonth() + 1).padStart(2, '0')}`;
  }

  // Filtered events for the selected date range (flat timeline fallback)
  $: filteredEvents = filterEventsForRange($historyEvents, selectedDate, viewMode);
  // Filtered groups: only keep groups with events in the selected range
  $: displayGroups = localGroups
    .map(g => ({ ...g, events: filterEventsForRange(g.events, selectedDate, viewMode) }))
    .filter(g => g.events.length > 0)
    .map(g => ({ ...g, total_duration_ms: g.events.reduce((s: number, e: EventRow) => s + (e.duration_ms ?? 0), 0) }));

  // Time range navigation
  let viewMode: 'day' | 'week' | 'month' = 'day';
  let today = new Date();
  let selectedDate = new Date();

  // Reactive label — Svelte needs to see selectedDate read directly
  $: navLabel = formatNavLabel(selectedDate, today, viewMode);
  $: canGoFwd = canGoForward(selectedDate, today, viewMode);

  $: if ($summaries?.groups && $summaries.generated_at !== lastGeneratedAt) {
    lastGeneratedAt = $summaries.generated_at;
    localGroups = $summaries.groups.map(g => ({
      ...g,
      events: [...g.events],
    }));
    expandedGroups = localGroups.map((_, i) => i); // expand all by default
  }

  function toggleGroup(idx: number) {
    if (expandedGroups.includes(idx)) {
      expandedGroups = expandedGroups.filter(i => i !== idx);
    } else {
      expandedGroups = [...expandedGroups, idx];
    }
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

  // --- Date navigation helpers ---

  function isSameDay(a: Date, b: Date): boolean {
    return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
  }

  function addDays(date: Date, n: number): Date {
    const d = new Date(date);
    d.setDate(d.getDate() + n);
    d.setHours(0, 0, 0, 0);
    return d;
  }

  function startOfWeek(date: Date): Date {
    const d = new Date(date);
    const day = d.getDay(); // 0=Sun
    d.setDate(d.getDate() - day); // go to Sunday
    d.setHours(0, 0, 0, 0);
    return d;
  }

  function endOfWeek(date: Date): Date {
    const start = startOfWeek(date);
    return addDays(start, 7);
  }

  function startOfMonth(date: Date): Date {
    return new Date(date.getFullYear(), date.getMonth(), 1);
  }

  function endOfMonth(date: Date): Date {
    return new Date(date.getFullYear(), date.getMonth() + 1, 1);
  }

  function formatNavLabel(sel: Date, now: Date, mode: string): string {
    if (mode === 'day') {
      if (isSameDay(sel, now)) return 'Today';
      const yesterday = addDays(now, -1);
      if (isSameDay(sel, yesterday)) return 'Yesterday';
      return sel.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    }
    if (mode === 'week') {
      const start = startOfWeek(sel);
      const end = addDays(start, 6);
      return `${start.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })} – ${end.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}`;
    }
    return sel.toLocaleDateString(undefined, { month: 'long', year: 'numeric' });
  }

  function canGoForward(sel: Date, now: Date, mode: string): boolean {
    if (mode === 'day') return !isSameDay(sel, now);
    if (mode === 'week') return endOfWeek(sel) <= addDays(now, 1);
    return sel.getMonth() < now.getMonth() || sel.getFullYear() < now.getFullYear();
  }

  function goBack() {
    if (viewMode === 'day') selectedDate = addDays(selectedDate, -1);
    else if (viewMode === 'week') selectedDate = addDays(selectedDate, -7);
    else { const d = new Date(selectedDate); d.setMonth(d.getMonth() - 1); selectedDate = d; }
    selectedDate = selectedDate; // reactivity
    refreshHistory();
  }

  function goForward() {
    if (!canGoFwd) return;
    if (viewMode === 'day') selectedDate = addDays(selectedDate, 1);
    else if (viewMode === 'week') selectedDate = addDays(selectedDate, 7);
    else { const d = new Date(selectedDate); d.setMonth(d.getMonth() + 1); selectedDate = d; }
    selectedDate = selectedDate; // reactivity
    refreshHistory();
  }

  function setViewMode(mode: 'day' | 'week' | 'month') {
    viewMode = mode;
    selectedDate = new Date();
    refreshHistory();
  }

  function getRangeHours(): number {
    if (viewMode === 'day') {
      const diffMs = Date.now() - selectedDate.getTime();
      const diffHours = Math.ceil(diffMs / 3_600_000);
      // Always fetch at least 24h of data
      return Math.max(24, diffHours + 24);
    }
    if (viewMode === 'week') return 168; // 7 days
    return 720; // 30 days
  }

  function filterEventsForRange(events: EventRow[], sel: Date, mode: string): EventRow[] {
    let start: Date;
    let end: Date;
    if (mode === 'day') {
      start = new Date(sel); start.setHours(0, 0, 0, 0);
      end = addDays(start, 1);
    } else if (mode === 'week') {
      start = startOfWeek(sel);
      end = endOfWeek(sel);
    } else {
      start = startOfMonth(sel);
      end = endOfMonth(sel);
    }
    const startMs = start.getTime();
    const endMs = end.getTime();
    return events.filter(e => e.ts >= startMs && e.ts < endMs);
  }

  async function refreshHistory() {
    const hours = getRangeHours();
    const rk = dateCacheKey(selectedDate, viewMode);
    try {
      const events = await api.activity(hours);
      historyEvents.set(events.reverse());
    } catch {}
    try {
      await fetchSummaries(rk);
    } catch {}
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
    window.open(url, '_blank');
  }

  // Drag & drop between groups
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
      daemonMsg = 'Daemon should already be running. Checking...';
      const res = await fetch('/api/health');
      if (res.ok) {
        daemonOnline.set(true);
        daemonMsg = 'Daemon is running ✓';
        await refreshDaemonInfo();
        await loadLlmSettings();
      } else {
        daemonMsg = 'Daemon is not responding. Start it manually: ccube-daemon';
      }
    } catch {
      daemonMsg = 'Cannot reach daemon. Start it manually: ccube-daemon';
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
    let sinceMs: number | undefined;
    let untilMs: number | undefined;
    if (viewMode === 'day') {
      const start = new Date(selectedDate); start.setHours(0, 0, 0, 0);
      const end = addDays(start, 1);
      sinceMs = start.getTime();
      untilMs = end.getTime();
    } else if (viewMode === 'week') {
      sinceMs = startOfWeek(selectedDate).getTime();
      untilMs = endOfWeek(selectedDate).getTime();
    } else {
      sinceMs = startOfMonth(selectedDate).getTime();
      untilMs = endOfMonth(selectedDate).getTime();
    }
    const rk = dateCacheKey(selectedDate, viewMode);
    await triggerSummarize(sinceMs, untilMs, rk);
  }

  function autofocus(el: HTMLInputElement) { el.focus(); }

  function handleViewChange(view: 'history' | 'vault' | 'rhythm' | 'settings') {
    $activeView = view;
  }

  function setTheme(t: ThemeName) {
    theme = t;
    applyTheme(t);
  }

  onMount(() => {
    theme = getInitialTheme();
    applyTheme(theme);

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
    refreshHistory().finally(() => { loading.set(false); });
    fetchRhythm(7);

    const refreshInterval = setInterval(() => {
      if ($activeView === 'history') {
        refreshHistory();
      }
      if ($activeView === 'rhythm') {
        fetchRhythm(7);
      }
    }, 30_000);

    // Keep "today" current (rolls over at midnight)
    const dateInterval = setInterval(() => {
      const now = new Date();
      if (!isSameDay(today, now)) {
        today = now;
        if (isSameDay(selectedDate, addDays(now, -1))) selectedDate = now;
      }
    }, 60_000);

    return () => {
      clearInterval(healthInterval);
      clearInterval(refreshInterval);
      clearInterval(dateInterval);
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
          <button class="datenav__btn" aria-label="Previous" on:click={goBack}>←</button>
          <span class="datenav__label">{navLabel}</span>
          <button class="datenav__btn" aria-label="Next" on:click={goForward} disabled={!canGoFwd}>→</button>
        </div>
        <div class="bar__right">
          <button class="btn btn--ghost" on:click={handleSummarize} disabled={$summarizing}>
            {$summarizing ? 'Organizing...' : '⚡ Organize'}
          </button>
          <div class="seg">
            <button class="seg__opt" class:active={viewMode === 'day'} on:click={() => setViewMode('day')}>Day</button>
            <button class="seg__opt" class:active={viewMode === 'week'} on:click={() => setViewMode('week')}>Week</button>
            <button class="seg__opt" class:active={viewMode === 'month'} on:click={() => setViewMode('month')}>Month</button>
          </div>
        </div>
      </div>

      {#if $error}
        <p class="error-msg">{$error}</p>
      {:else if displayGroups.length > 0}
        <!-- GROUPED TIMELINE -->
        <div class="timeline">
          {#each displayGroups as group, idx}
            <div class="tl-group"
              class:drag-over={dragOverGroup === group.title}
              data-group-title={group.title}
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
                  <span class="tl-header__toggle">{expandedGroups.includes(idx) ? '▾' : '▸'}</span>
                </div>

                {#if expandedGroups.includes(idx)}
                  <div class="tl-items">
                    {#each group.events as event (event.id)}
                      <div class="tl-item"
                        class:dragging-source={isDragging && dragState?.event.id === event.id}
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
                        <span class="tl-item__handle" class:active-drag={isDragging && dragState?.event.id === event.id} role="button" tabindex="-1" on:mousedown={(e) => startDrag(e, event, group.title)} on:click|stopPropagation title="Drag to another group">≡</span>
                      </div>
                    {/each}
                  </div>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      {:else if filteredEvents.length > 0}
        <!-- FLAT TIMELINE (no summaries, or viewing past date) -->
        <div class="timeline-flat">
          {#each filteredEvents as event (event.id)}
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

    {:else if $activeView === 'rhythm'}
      <!-- RHYTHM VIEW -->
      <h1 class="heading">Rhythm</h1>
      {#if !$daemonOnline}
        <p style="color:var(--ink-soft)">🔌 Daemon is not running.</p>
      {:else if $rhythmReport}
        <Rhythm report={$rhythmReport} />
      {:else}
        <p class="hint">Loading rhythm…</p>
      {/if}

    {:else if $activeView === 'settings'}
      <!-- SETTINGS VIEW -->
      <h1 class="heading">Settings</h1>

      <!-- Appearance -->
      <div class="card appearance">
        <h2 class="card__title">Appearance</h2>
        <div class="swatches">
          {#each THEMES as t}
            <button
              class="swatch"
              class:active={theme === t.name}
              aria-pressed={theme === t.name}
              aria-label={t.label}
              title={t.label}
              on:click={() => setTheme(t.name)}
            >
              <span class="swatch__chip" style="background: {t.bg};">
                <span class="swatch__dot" style="background: {t.accent};"></span>
              </span>
              <span class="swatch__label">{t.label}</span>
            </button>
          {/each}
        </div>
      </div>

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

{#if isDragging && dragState}
  <div class="drag-chip" style="left: {dragX + 12}px; top: {dragY - 14}px">
    <span class="drag-chip__icon">↕</span>
    <span class="drag-chip__label">{dragState.event.app ?? 'Event'}</span>
  </div>
{/if}

<style>
  .app {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  .content {
    flex: 1;
    padding: 36px 40px;
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
    gap: 0;
    font-size: 14px;
    position: relative;
  }

  .datenav__btn {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 16px;
    color: var(--ink-soft);
    flex-shrink: 0;
    transition: background var(--t-fast) var(--ease);
  }

  .datenav__btn:hover { background: var(--row-hover); }
  .datenav__btn:disabled { opacity: 0.3; cursor: default; }
  .datenav__btn:disabled:hover { background: transparent; }

  .datenav__label {
    color: var(--ink);
    font-size: 14px;
    font-weight: 500;
    min-width: 120px;
    text-align: center;
    user-select: none;
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
  .timeline { padding-top: 12px; }

  .tl-group {
    display: grid;
    grid-template-columns: 56px 1fr;
    margin-bottom: 8px;
    border-radius: var(--r-panel);
    transition: background var(--t-fast) var(--ease);
  }

  .tl-group.drag-over {
    background: rgba(241, 106, 1, 0.08);
    outline: 2px solid var(--brand-orange);
    outline-offset: -2px;
    border-radius: var(--r-panel);
  }

  .tl-gutter {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    min-width: 64px;
    padding-right: 16px;
    padding-top: 10px;
  }

  .tl-gutter__time {
    color: var(--ink-soft);
    font-size: 12px;
    white-space: nowrap;
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
    padding: 10px 12px;
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
    padding-left: 8px;
    animation: slideDown 200ms var(--ease);
  }

  @keyframes slideDown {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .tl-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
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
    margin-left: auto;
  }

  .tl-item__handle:hover {
    background: var(--paper-panel);
    color: var(--ink-soft);
  }

  .tl-item__handle.active-drag {
    background: var(--brand-orange);
    color: var(--on-orange);
  }

  /* Source event dims while being dragged */
  .tl-item.dragging-source {
    opacity: 0.35;
    outline: 2px dashed var(--brand-orange);
    outline-offset: -2px;
  }

  /* Floating chip that follows cursor */
  .drag-chip {
    position: fixed;
    z-index: 9999;
    pointer-events: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 12px;
    background: var(--brand-orange);
    color: var(--on-orange);
    border-radius: var(--r-pill);
    font: 600 12px var(--font-ui);
    box-shadow: 0 4px 16px rgba(241, 106, 1, 0.35);
    white-space: nowrap;
    transition: none;
  }

  .drag-chip__icon {
    font-size: 11px;
  }

  /* ---- Flat timeline (fallback) ---- */
  .timeline-flat { padding-top: 12px; }

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
    min-width: 72px;
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

  .appearance .swatches {
    display: flex;
    gap: 12px;
  }

  .swatch {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    flex: 1;
    padding: 0;
    background: none;
  }

  .swatch__chip {
    width: 100%;
    height: 48px;
    border-radius: var(--r-panel);
    border: 1px solid var(--divider);
    display: grid;
    place-items: center;
    transition: border-color var(--t-fast) var(--ease), box-shadow var(--t-fast) var(--ease);
  }

  .swatch__dot {
    width: 16px;
    height: 16px;
    border-radius: var(--r-pill);
  }

  .swatch__label {
    font-size: 13px;
    color: var(--ink-soft);
    transition: color var(--t-fast) var(--ease);
  }

  .swatch:hover .swatch__chip {
    border-color: var(--brand-orange);
  }

  .swatch.active .swatch__chip {
    border-color: var(--brand-orange);
    box-shadow: 0 0 0 2px var(--brand-orange);
  }

  .swatch.active .swatch__label {
    color: var(--ink);
    font-weight: 600;
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
