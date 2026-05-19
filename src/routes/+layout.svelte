<script lang="ts">
  import '../app.css';
  import Rail from '$components/Rail.svelte';
  import { activeView, daemonOnline, historyEvents, loading, error } from '$lib/stores';
  import { api } from '$lib/api';
  import { onMount } from 'svelte';

  let fetchStatus = 'not started';

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

    fetchStatus = 'fetching history...';
    loading.set(true);
    api.recent()
      .then((events) => {
        fetchStatus = `got ${events.length} events`;
        historyEvents.set(events);
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
          .then((events) => { historyEvents.set(events); })
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
    max-width: 420px;
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
</style>
