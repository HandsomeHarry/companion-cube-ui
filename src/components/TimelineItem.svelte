<script lang="ts">
  import type { EventRow } from '$lib/types';

  export let event: EventRow;

  $: app = event.app ?? 'Unknown';
  $: title = event.title ?? 'No title';
</script>

<div class="item">
  <span class="item__dot">·</span>
  <span class="item__text">{app} – {title}</span>
  <span class="item__handle">≡</span>
</div>

<style>
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-radius: 8px;
    cursor: pointer;
    transition: background var(--t-fast) var(--ease);
  }

  .item:hover {
    background: var(--row-hover);
  }

  .item__dot {
    color: var(--ink-faint);
    flex-shrink: 0;
  }

  .item__text {
    flex: 1;
    font-size: 14px;
    color: var(--ink);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .item__handle {
    color: var(--ink-faint);
    opacity: 0;
    cursor: grab;
    transition: opacity var(--t-fast) var(--ease);
    flex-shrink: 0;
  }

  .item:hover .item__handle {
    opacity: 1;
  }
</style>
