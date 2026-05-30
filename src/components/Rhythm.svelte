<script lang="ts">
  import type { RhythmReport } from '$lib/types';

  export let report: RhythmReport;

  function formatDuration(ms: number): string {
    const totalMin = Math.round(ms / 60_000);
    if (totalMin < 60) return `${totalMin}m`;
    const h = Math.floor(totalMin / 60);
    const m = totalMin % 60;
    return m > 0 ? `${h}h ${m}m` : `${h}h`;
  }
</script>

<div class="rhythm">
  <!-- Heatmap -->
  <section class="rhythm__section">
    <h2 class="rhythm__heading">Weekly Activity</h2>
    <div class="heatmap">
      <div class="heatmap__row heatmap__header">
        <span class="heatmap__day-label"></span>
        {#each report.heatmap.hour_labels as label, i}
          <span class="heatmap__hour-label">{i % 6 === 0 ? label : ''}</span>
        {/each}
      </div>
      {#each report.heatmap.day_labels as day, dayIdx}
        <div class="heatmap__row">
          <span class="heatmap__day-label">{day}</span>
          {#each report.heatmap.hour_labels as _hour, hourIdx}
            {@const value = report.heatmap.cells[dayIdx * 24 + hourIdx]}
            {@const ratio = report.heatmap.max_value > 0 ? value / report.heatmap.max_value : 0}
            <span
              class="heatmap__cell"
              style="opacity: {value > 0 ? Math.max(ratio, 0.12) : 1}; background: {value > 0 ? 'var(--brand-orange)' : 'var(--row-hover)'}"
              title="{day} {report.heatmap.hour_labels[hourIdx]}: {value}m active"
            ></span>
          {/each}
        </div>
      {/each}
    </div>
  </section>

  <!-- Best Focus Windows -->
  <section class="rhythm__section">
    <h2 class="rhythm__heading">Best Focus Windows</h2>
    {#if report.focus_windows.length > 0}
      <div class="cards">
        {#each report.focus_windows as fw}
          <div class="focus-card">
            <div class="focus-card__label">{fw.label}</div>
            <div class="focus-card__value">{formatDuration(fw.total_focus_ms)}</div>
          </div>
        {/each}
      </div>
    {:else}
      <p class="rhythm__empty">Not enough focus data yet. Keep working — patterns appear as you go.</p>
    {/if}
  </section>

  <!-- Focus Fingerprint -->
  <section class="rhythm__section">
    <h2 class="rhythm__heading">Focus Fingerprint</h2>
    {#if report.fingerprint.length > 0}
      <ul class="list">
        {#each report.fingerprint as cluster}
          <li class="list__item">
            <span class="list__dot"></span>
            <span class="list__text">{cluster.apps.join(' + ')}</span>
            <span class="list__meta">{cluster.session_count} session{cluster.session_count === 1 ? '' : 's'}</span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="rhythm__empty">No recurring app combos yet.</p>
    {/if}
  </section>

  <!-- Drift Origins -->
  <section class="rhythm__section">
    <h2 class="rhythm__heading">Drift Origins</h2>
    {#if report.drift_origins.length > 0}
      <ul class="list">
        {#each report.drift_origins as drift}
          <li class="list__item">
            <span class="list__dot"></span>
            <span class="list__text">{drift.app}</span>
            <span class="list__meta">← from {drift.from_app} ({drift.count})</span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="rhythm__empty">No drift detected. Nice focus.</p>
    {/if}
  </section>
</div>

<style>
  .rhythm {
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding-bottom: 40px;
    max-width: 680px;
  }

  .rhythm__heading {
    font-size: 15px;
    font-weight: 600;
    color: var(--ink);
    margin-bottom: 8px;
  }

  .rhythm__empty {
    color: var(--ink-faint);
    font-size: 13px;
  }

  /* Heatmap */
  .heatmap {
    display: flex;
    flex-direction: column;
    gap: 2px;
    overflow-x: auto;
  }

  .heatmap__row {
    display: flex;
    gap: 2px;
    align-items: center;
  }

  .heatmap__day-label {
    width: 34px;
    flex-shrink: 0;
    font-size: 11px;
    color: var(--ink-faint);
    text-align: right;
    padding-right: 8px;
  }

  .heatmap__hour-label {
    width: 15px;
    flex-shrink: 0;
    font-size: 9px;
    color: var(--ink-faint);
    text-align: left;
  }

  .heatmap__cell {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    border-radius: 3px;
    transition: transform var(--t-fast) var(--ease);
  }

  .heatmap__cell:hover {
    transform: scale(1.35);
  }

  /* Focus window cards */
  .cards {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }

  .focus-card {
    background: var(--card-white);
    border: 1px solid var(--divider);
    border-radius: var(--r-panel);
    padding: 12px 16px;
    min-width: 130px;
    box-shadow: var(--shadow-rest);
  }

  .focus-card__label {
    font-size: 13px;
    color: var(--ink-soft);
    margin-bottom: 4px;
  }

  .focus-card__value {
    font-size: 18px;
    font-weight: 700;
    color: var(--brand-orange-deep);
  }

  /* Lists */
  .list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
    padding: 0;
  }

  .list__item {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }

  .list__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--brand-orange);
    flex-shrink: 0;
  }

  .list__text {
    color: var(--ink);
    font-weight: 500;
  }

  .list__meta {
    color: var(--ink-faint);
    font-size: 12px;
  }
</style>
