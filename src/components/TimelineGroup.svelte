<script lang="ts">
  import type { EventRow } from '$lib/types';
  import TimelineItem from './TimelineItem.svelte';

  export let time: string;
  export let title: string;
  export let events: EventRow[];
</script>

<div class="group">
  <div class="group__time">
    <span class="group__dot"></span>
    {time}
  </div>
  <div class="group__body">
    <div class="group__title">
      {title}
      <button class="group__edit" aria-label="Rename group" title="Rename">
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <path d="M17 3a2.828 2.828 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5L17 3z"/>
        </svg>
      </button>
    </div>
    <div class="group__items">
      {#each events as event (event.id)}
        <TimelineItem {event} />
      {/each}
    </div>
  </div>
</div>

<style>
  .group {
    display: grid;
    grid-template-columns: 64px 1fr;
    margin-bottom: 22px;
  }

  .group__time {
    position: relative;
    font-size: 13px;
    color: var(--ink-soft);
    padding-right: 12px;
    text-align: right;
    padding-top: 2px;
  }

  .group__dot {
    position: absolute;
    right: -4px;
    top: 5px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--brand-orange);
  }

  .group__body {
    min-width: 0;
  }

  .group__title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 16px;
    font-weight: 700;
    color: var(--ink);
    margin-bottom: 4px;
  }

  .group__edit {
    color: var(--ink-faint);
    display: grid;
    place-items: center;
    padding: 2px;
    border-radius: 4px;
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease);
  }

  .group__title:hover .group__edit {
    opacity: 1;
  }

  .group__items {
    display: flex;
    flex-direction: column;
  }
</style>
