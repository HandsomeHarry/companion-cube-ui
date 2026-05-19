<script lang="ts">
  import type { VaultItem } from '$lib/types';

  export let item: VaultItem;

  function relativeDate(ts: number): string {
    const now = new Date();
    const d = new Date(ts);
    const diffDays = Math.floor((now.getTime() - d.getTime()) / (1000 * 60 * 60 * 24));

    if (diffDays === 0) return `Today, ${d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}`;
    if (diffDays === 1) return 'Yesterday';
    return d.toLocaleDateString();
  }
</script>

<tr class="row">
  <td class="row__idea">
    {#if item.favorited}
      <span class="row__star filled">★</span>
    {/if}
    {item.idea}
  </td>
  <td class="row__items">{item.items}</td>
  <td class="row__added">{relativeDate(item.ts)}</td>
  <td class="row__actions">
    <button
      class="row__star-btn"
      class:filled={item.favorited}
      aria-label={item.favorited ? 'Unfavorite' : 'Favorite'}
    >
      {item.favorited ? '★' : '☆'}
    </button>
    <button class="row__kebab" aria-label="More options">⋮</button>
  </td>
</tr>

<style>
  .row td {
    padding: 12px;
    border-bottom: 1px solid var(--divider);
    font-size: 14px;
    vertical-align: middle;
  }

  .row:hover td {
    background: var(--row-hover);
  }

  .row__idea {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--ink);
    font-weight: 400;
  }

  .row__star {
    color: var(--gold-star);
    font-size: 14px;
  }

  .row__items {
    color: var(--ink-soft);
    font-size: 13px;
  }

  .row__added {
    color: var(--ink-faint);
    font-size: 13px;
  }

  .row__actions {
    display: flex;
    align-items: center;
    gap: 4px;
    justify-content: flex-end;
  }

  .row__star-btn,
  .row__kebab {
    width: 28px;
    height: 28px;
    border-radius: 6px;
    display: grid;
    place-items: center;
    font-size: 14px;
    color: var(--ink-faint);
    opacity: 0;
    transition: all var(--t-fast) var(--ease);
  }

  .row:hover .row__star-btn,
  .row:hover .row__kebab,
  .row__star-btn.filled {
    opacity: 1;
  }

  .row__star-btn.filled {
    color: var(--gold-star);
  }

  .row__star-btn:hover,
  .row__kebab:hover {
    background: var(--paper-panel);
  }
</style>
