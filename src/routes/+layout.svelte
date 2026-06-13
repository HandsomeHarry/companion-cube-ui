<script lang="ts">
  import '../app.css';
  import Rail from '$components/Rail.svelte';
  import Rhythm from '$components/Rhythm.svelte';
  import { activeView, daemonOnline, historyEvents, loading, error, llmConfig, fetchLlmConfig, saveLlmConfig, summaries, summarizing, fetchSummaries, triggerSummarize } from '$lib/stores';
  import { rhythmReport, fetchRhythm, llmHealth, fetchLlmHealth } from '$lib/stores';
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

  // History state — local mutable copies of groups for drag/rename.
  // Sessions have stable IDs; all local state is keyed by ID so background
  // refreshes can merge instead of stomping.
  let localGroups: SessionGroup[] = [];
  let collapsedSessions = new Set<number>(); // default expanded; remember collapses
  let editingSessionId: number | null = null;
  let editTitle = '';
  let dragState: { event: EventRow; fromSessionId: number | null } | null = null;
  let dragOverTarget: number | 'new' | null = null; // session id or the new-session zone
  let isDragging = false;
  let dragX = 0;
  let dragY = 0;
  let undoMove: { event: EventRow; toSessionId: number | null; timer: number } | null = null;

  // Pointer-based drag: mousedown on ≡ handle → mousemove tracks hover →
  // mouseup drops. Esc cancels.
  function startDrag(e: MouseEvent, event: EventRow, fromSessionId: number | null) {
    if (e.button !== 0) return;
    e.preventDefault();
    dragState = { event, fromSessionId };
    isDragging = false;

    const onMove = (ev: MouseEvent) => {
      if (!dragState) return cleanup();
      isDragging = true;
      dragX = ev.clientX;
      dragY = ev.clientY;
      const el = document.elementFromPoint(ev.clientX, ev.clientY);
      dragOverTarget = null;
      if (el) {
        if (el.closest('[data-drop-new]')) {
          dragOverTarget = 'new';
        } else {
          const group = el.closest('[data-session-id]');
          const sid = group?.getAttribute('data-session-id');
          if (sid) dragOverTarget = Number(sid);
        }
      }
    };

    const onUp = () => {
      if (dragState && isDragging && dragOverTarget !== null
          && dragOverTarget !== dragState.fromSessionId) {
        finishDrop(dragState.event, dragState.fromSessionId, dragOverTarget);
      }
      cleanup();
    };

    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === 'Escape') cleanup();
    };

    const cleanup = () => {
      dragState = null;
      dragOverTarget = null;
      isDragging = false;
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.removeEventListener('keydown', onKey);
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    document.addEventListener('keydown', onKey);
  }

  /** Apply a move locally (optimistic), persist it, arm the undo toast. */
  async function finishDrop(
    movedEvent: EventRow,
    fromSessionId: number | null,
    target: number | 'new',
  ) {
    try {
      const res = await api.groupCorrection({
        event_id: movedEvent.id,
        to_session_id: target === 'new' ? null : target,
        new_session_label: target === 'new' ? 'New session' : undefined,
      });
      armUndo(movedEvent, fromSessionId);
      await refreshHistory(); // server state is truth; merge preserves UI state
      if (target === 'new') startRenameSession(res.session_id);
    } catch (e: any) {
      error.set(e?.message || 'Move failed');
    }
  }

  function armUndo(event: EventRow, toSessionId: number | null) {
    if (undoMove) clearTimeout(undoMove.timer);
    // Undo means "move it back where it was" — only possible if it came
    // from a real session (events from Just now have nowhere to go back to).
    if (toSessionId === null) { undoMove = null; return; }
    undoMove = {
      event,
      toSessionId,
      timer: window.setTimeout(() => { undoMove = null; }, 5000),
    };
  }

  async function performUndo() {
    if (!undoMove) return;
    clearTimeout(undoMove.timer);
    const { event, toSessionId } = undoMove;
    undoMove = null;
    try {
      await api.groupCorrection({
        event_id: event.id,
        to_session_id: toSessionId,
        record: false, // undo is not a teaching signal
      });
      await refreshHistory();
    } catch {}
  }

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

  // Events not in any group — captured after last summarize
  $: groupedEventIds = new Set(displayGroups.flatMap(g => g.events.map(e => e.id)));
  // Away periods (idle_start rows carry the away duration) belong in the
  // timeline: honest time accounting means breaks are visible seams.
  $: ungroupedEvents = filteredEvents.filter(
    e => !groupedEventIds.has(e.id) && (e.kind === 'app_focus' || e.kind === 'idle_start')
  );

  // Capture permissions — when blind, labels degrade to app names and the
  // user deserves to know why (quietly).
  let captureHealth: { accessibility: boolean; screen_recording: boolean } | null = null;
  async function fetchCaptureHealth() {
    try { captureHealth = await api.captureHealth(); } catch { captureHealth = null; }
  }

  // The open session is the live head: new events visually flow into it.
  $: openGroup = displayGroups.find(g => g.open && isSameDay(selectedDate, today));

  /** Human label for an event row. */
  function eventLabel(e: EventRow): string {
    if (e.kind === 'idle_start') return 'Away';
    return e.app ?? e.kind;
  }

  // Time range navigation
  let viewMode: 'day' | 'week' | 'month' = 'day';
  let today = new Date();
  let selectedDate = new Date();

  // Reactive label — Svelte needs to see selectedDate read directly
  $: navLabel = formatNavLabel(selectedDate, today, viewMode);
  $: canGoFwd = canGoForward(selectedDate, today, viewMode);

  // Merge server sessions into local state. Never replaces mid-drag, and
  // collapse state survives because it's keyed by session ID.
  $: if ($summaries?.groups && !isDragging) {
    localGroups = $summaries.groups.map(g => ({ ...g, events: [...g.events] }));
  }
  function toggleGroup(sessionId: number) {
    if (collapsedSessions.has(sessionId)) {
      collapsedSessions.delete(sessionId);
    } else {
      collapsedSessions.add(sessionId);
    }
    collapsedSessions = collapsedSessions; // reactivity
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

  /** Clean a description: if it's raw vision JSON, extract the activity field. */
  function cleanDesc(raw: string | null | undefined, fallback: string | null | undefined = ''): string {
    if (!raw) return fallback || '';
    const trimmed = raw.trim();
    // Legacy vision_desc stored raw JSON like {"activity":"...","category":"..."}
    if (trimmed.startsWith('{')) {
      try {
        const obj = JSON.parse(trimmed);
        if (obj.activity && typeof obj.activity === 'string') return obj.activity;
      } catch { /* not JSON, return as-is */ }
    }
    return trimmed;
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

  // Group rename — persists via PUT /sessions/{id}; renames pin the session.
  function startRenameSession(sessionId: number) {
    editingSessionId = sessionId;
    editTitle = localGroups.find(g => g.id === sessionId)?.title ?? '';
  }

  async function finishRename(sessionId: number) {
    const group = localGroups.find(g => g.id === sessionId);
    const newTitle = editTitle.trim();
    editingSessionId = null;
    if (!group || !newTitle || newTitle === group.title) return;

    group.title = newTitle;
    group.pinned = true;
    localGroups = localGroups;

    try {
      await api.renameSession(sessionId, newTitle);
    } catch (e: any) {
      error.set(e?.message || 'Rename failed');
    }
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

  // Sessions are day-scoped (range_key "day:..."). Organizing under a week:
  // or month: key would create sessions invisible to the daily auto-pass and
  // fight it over the same events, so Organize exists only in Day view —
  // week/month are read-only aggregates.
  async function handleSummarize() {
    if (viewMode !== 'day') return;
    const start = new Date(selectedDate); start.setHours(0, 0, 0, 0);
    const end = addDays(start, 1);
    const rk = dateCacheKey(selectedDate, 'day');
    // ⚡ Organize = full pass: regroups the range but never touches pinned
    // (user-edited) sessions.
    await triggerSummarize(start.getTime(), end.getTime(), rk, true);
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
    fetchLlmHealth();
    fetchCaptureHealth();
    const llmHealthInterval = setInterval(() => { fetchLlmHealth(); fetchCaptureHealth(); }, 30_000);

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
      clearInterval(llmHealthInterval);
    };
  });
</script>

<div class="app">
  <Rail onViewChange={handleViewChange} />
  <main class="content">

    {#if captureHealth && (!captureHealth.accessibility || !captureHealth.screen_recording) && $activeView !== 'settings'}
      <div class="setup-hint" role="status">
        {#if !captureHealth.screen_recording}
          Companion Cube can't see your screen — activity descriptions will be vague.
          Allow it in System Settings → Privacy & Security → <strong>Screen Recording</strong>.
        {:else}
          Companion Cube can't see window titles — grouping works better with them.
          Allow it in System Settings → Privacy & Security → <strong>Accessibility</strong>.
        {/if}
      </div>
    {/if}

    {#if $llmHealth && (!$llmHealth.reachable || $llmHealth.model_present === false) && $activeView !== 'settings'}
      <div class="setup-hint" role="status">
        {#if !$llmHealth.reachable}
          {$llmHealth.provider === 'ollama' ? 'Ollama isn’t running' : 'The AI backend can’t be reached'} — your activity is still recorded, but won’t be organized.
        {:else}
          The model “{$llmHealth.model}” isn’t downloaded yet — run <code>ollama pull {$llmHealth.model}</code> in a terminal.
        {/if}
        <button class="setup-hint__link" on:click={() => handleViewChange('settings')}>Settings</button>
      </div>
    {/if}

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
          {#if viewMode === 'day'}
            <button class="btn btn--ghost" on:click={handleSummarize} disabled={$summarizing}>
              {$summarizing ? 'Organizing...' : '⚡ Organize'}
            </button>
          {/if}
          <div class="seg">
            <button class="seg__opt" class:active={viewMode === 'day'} on:click={() => setViewMode('day')}>Day</button>
            <button class="seg__opt" class:active={viewMode === 'week'} on:click={() => setViewMode('week')}>Week</button>
            <button class="seg__opt" class:active={viewMode === 'month'} on:click={() => setViewMode('month')}>Month</button>
          </div>
        </div>
      </div>

      {#if $error}
        <p class="error-msg">{$error}</p>
      {:else if displayGroups.length > 0 || ungroupedEvents.length > 0}
        <!-- LIVE HEAD — events not yet organized into a session. Only
             rendered when no open session exists to absorb them visually. -->
        {#if !openGroup && ungroupedEvents.length > 0}
          <div class="tl-group tl-group--live">
            <div class="tl-gutter">
              <span class="tl-gutter__time">{formatTime(ungroupedEvents[0]?.ts ?? Date.now())}</span>
              <span class="tl-gutter__dot tl-gutter__dot--live"></span>
            </div>
            <div class="tl-body">
              <div class="tl-header tl-header--live">
                <span class="tl-header__title tl-header__title--live">Just now</span>
                <span class="tl-header__count">{ungroupedEvents.length}</span>
              </div>
              <div class="tl-items">
                {#each ungroupedEvents as event (event.id)}
                  <div class="tl-item"
                    class:dragging-source={isDragging && dragState?.event.id === event.id}
                    on:click={() => openEvent(event)}
                    on:keydown={(e) => { if (e.key === 'Enter') openEvent(event); }}
                    role="button"
                    tabindex="0"
                  >
                    <span class="tl-item__bullet">·</span>
                    <span class="tl-item__app" class:away={event.kind === 'idle_start'}>{eventLabel(event)}</span>
                    <span class="tl-item__title">– {cleanDesc(event.llm_desc, cleanDesc(event.vision_desc, event.title))}</span>
                    {#if event.duration_ms}
                      <span class="tl-item__dur">{formatDuration(event.duration_ms)}</span>
                    {/if}
                    {#if event.kind === 'app_focus'}
                      <span class="tl-item__handle" class:active-drag={isDragging && dragState?.event.id === event.id} role="button" tabindex="-1" on:mousedown={(e) => startDrag(e, event, null)} on:click|stopPropagation title="Drag into a session">≡</span>
                    {/if}
                  </div>
                {/each}
              </div>
            </div>
          </div>
        {/if}

        <!-- NEW SESSION drop zone — only exists while dragging -->
        {#if isDragging}
          <div class="tl-dropnew" class:drag-over={dragOverTarget === 'new'} data-drop-new>
            + Drop here to start a new session
          </div>
        {/if}

        <!-- GROUPED TIMELINE (sessions, newest first) -->
        <div class="timeline">
          {#each displayGroups as group (group.id)}
            <div class="tl-group"
              class:drag-over={dragOverTarget === group.id}
              class:tl-group--open={group.open}
              data-session-id={group.id}
            >
              <div class="tl-gutter">
                <span class="tl-gutter__time">{formatTime(group.events[group.events.length - 1]?.ts ?? Date.now())}</span>
                <span class="tl-gutter__dot" class:distraction={group.distraction} class:tl-gutter__dot--live={group.open}></span>
              </div>
              <div class="tl-body">
                <div class="tl-header" role="button" tabindex="0" on:click={() => toggleGroup(group.id)} on:keydown={handleGroupKeydown}>
                  {#if editingSessionId === group.id}
                    <!-- Inline rename -->
                    <input
                      class="tl-rename"
                      type="text"
                      bind:value={editTitle}
                      on:keydown={(e) => { if (e.key === 'Enter') finishRename(group.id); if (e.key === 'Escape') editingSessionId = null; }}
                      on:blur={() => finishRename(group.id)}
                      on:click|stopPropagation
                      use:autofocus
                    />
                  {:else}
                    <span class="tl-header__title" class:distraction={group.distraction}>
                      {group.title}
                    </span>
                    {#if group.open}
                      <span class="tl-header__now" title="This session is still going — new activity joins it">now</span>
                    {/if}
                    {#if group.pinned}
                      <span class="tl-header__pin" title="You've edited this session — auto-organize won't change it">⌖</span>
                    {/if}
                    <button class="tl-header__edit" on:click|stopPropagation={() => startRenameSession(group.id)} title="Rename group">
                      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/></svg>
                    </button>
                  {/if}
                  <span class="tl-header__count">{group.events.length}</span>
                  <span class="tl-header__dur">{formatDuration(group.total_duration_ms)}</span>
                  <span class="tl-header__toggle">{collapsedSessions.has(group.id) ? '▸' : '▾'}</span>
                </div>

                {#if !collapsedSessions.has(group.id)}
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
                        <span class="tl-item__app" class:away={event.kind === 'idle_start'}>{eventLabel(event)}</span>
                        <span class="tl-item__title">– {cleanDesc(event.llm_desc, cleanDesc(event.vision_desc, event.title))}</span>
                        {#if event.duration_ms}
                          <span class="tl-item__dur">{formatDuration(event.duration_ms)}</span>
                        {/if}
                        <span class="tl-item__handle" class:active-drag={isDragging && dragState?.event.id === event.id} role="button" tabindex="-1" on:mousedown={(e) => startDrag(e, event, group.id)} on:click|stopPropagation title="Drag to another group">≡</span>
                      </div>
                    {/each}
                    {#if group.open}
                      {#each ungroupedEvents as event (event.id)}
                        <div class="tl-item tl-item--pending"
                          on:click={() => openEvent(event)}
                          on:keydown={(e) => { if (e.key === 'Enter') openEvent(event); }}
                          role="button"
                          tabindex="0"
                        >
                          <span class="tl-item__bullet">·</span>
                          <span class="tl-item__app" class:away={event.kind === 'idle_start'}>{eventLabel(event)}</span>
                          <span class="tl-item__title">– {cleanDesc(event.llm_desc, cleanDesc(event.vision_desc, event.title))}</span>
                          {#if event.duration_ms}
                            <span class="tl-item__dur">{formatDuration(event.duration_ms)}</span>
                          {/if}
                        </div>
                      {/each}
                    {/if}
                  </div>
                {/if}
              </div>
            </div>
          {/each}
        </div>

        <!-- UNDO TOAST -->
        {#if undoMove}
          <div class="undo-toast">
            Moved “{undoMove.event.app ?? 'event'}”
            <button class="undo-toast__btn" on:click={performUndo}>Undo</button>
          </div>
        {/if}
      {:else if filteredEvents.length > 0}
        <!-- FLAT TIMELINE (no summaries, or viewing past date) -->
        <div class="timeline-flat">
          {#each filteredEvents as event (event.id)}
            <div class="tl-row">
              <span class="tl-time">{formatTime(event.ts)}</span>
              <span class="tl-dot" style="background: {event.kind === 'app_focus' ? 'var(--brand-orange)' : event.kind === 'idle_start' ? '#aaa' : '#888'}"></span>
              <span class="tl-app" class:away={event.kind === 'idle_start'}>{eventLabel(event)}</span>
              <span class="tl-detail">– {cleanDesc(event.llm_desc, cleanDesc(event.vision_desc, event.title))}</span>
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
                <option value="ollama">Ollama (local)</option>
                <option value="llamacpp">llama.cpp (local)</option>
                <option value="openai-compatible">OpenAI Compatible</option>
                <option value="openai">OpenAI</option>
                <option value="anthropic">Anthropic</option>
              </select>
            </div>

            <div class="field">
              <label class="field__label" for="url">API Base URL</label>
              <input id="url" type="url" class="field__input" bind:value={llmUrl} placeholder="http://localhost:11434/v1" />
            </div>

            <div class="field">
              <label class="field__label" for="model">Model</label>
              <input id="model" type="text" class="field__input" bind:value={llmModel} placeholder="gemma4:e4b" />
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

  /* Live head — not yet organized; quiet gray, no LLM label */
  .tl-gutter__dot--live {
    background: var(--ink-faint);
    box-shadow: 0 0 0 3px rgba(163, 155, 142, 0.12);
  }

  .tl-header--live { cursor: default; }

  .tl-header__title--live {
    color: var(--ink-soft);
    font-weight: 600;
    font-style: italic;
  }

  .tl-header__pin {
    color: var(--ink-faint);
    font-size: 12px;
    cursor: help;
  }

  /* New-session drop zone — exists only mid-drag */
  .tl-dropnew {
    margin: 4px 0 12px 56px;
    padding: 10px 14px;
    border: 1.5px dashed var(--divider);
    border-radius: var(--r-panel);
    color: var(--ink-faint);
    font-size: 13px;
    transition: all var(--t-fast) var(--ease);
  }

  .tl-dropnew.drag-over {
    border-color: var(--brand-orange);
    color: var(--brand-orange);
    background: rgba(241, 106, 1, 0.06);
  }

  /* Undo toast */
  .undo-toast {
    position: fixed;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--card-white);
    border: 1px solid var(--divider);
    border-radius: var(--r-control);
    box-shadow: var(--shadow-float);
    padding: 10px 16px;
    font-size: 13px;
    color: var(--ink);
    display: flex;
    gap: 12px;
    align-items: center;
    z-index: 50;
  }

  .undo-toast__btn {
    border: none;
    background: none;
    color: var(--brand-orange);
    font-weight: 600;
    font-size: 13px;
    cursor: pointer;
    padding: 0;
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

  /* The open (live) session */
  .tl-header__now {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--ink-faint);
    border: 1px solid var(--divider);
    border-radius: var(--r-pill);
    padding: 1px 7px;
    cursor: default;
  }

  .tl-item--pending { opacity: 0.55; }

  /* Away rows read as seams, not activities */
  .tl-item__app.away,
  .tl-app.away {
    font-weight: 400;
    font-style: italic;
    color: var(--ink-faint);
  }

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

  .setup-hint {
    font-size: 13px;
    color: var(--ink-soft);
    background: var(--card-white);
    border: 1px solid var(--brand-orange);
    border-radius: 8px;
    padding: 8px 12px;
    margin-bottom: 16px;
    line-height: 1.4;
  }

  .setup-hint code {
    font-size: 12px;
    user-select: all;
  }

  .setup-hint__link {
    border: none;
    background: none;
    color: var(--brand-orange);
    font-size: 13px;
    cursor: pointer;
    padding: 0;
    margin-left: 6px;
    text-decoration: underline;
  }
</style>
