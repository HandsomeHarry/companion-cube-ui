<script lang="ts">
  import type { VaultItem } from '$lib/types';
  import VaultRow from './VaultRow.svelte';

  export let items: VaultItem[];
</script>

<div class="toolbar">
  <div class="search">
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="11" cy="11" r="8"/>
      <line x1="21" y1="21" x2="16.65" y2="16.65"/>
    </svg>
    <input type="text" placeholder="Search ideas..." />
  </div>
  <button class="add-btn" aria-label="Add idea">+</button>
</div>

<table class="vault-table">
  <thead>
    <tr>
      <th>Idea</th>
      <th>Items</th>
      <th>Added</th>
      <th></th>
    </tr>
  </thead>
  <tbody>
    {#each items as item (item.id)}
      <VaultRow {item} />
    {/each}
  </tbody>
</table>

<style>
  .toolbar {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    margin: 8px 0 18px;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 240px;
    background: var(--card-white);
    border: 1px solid var(--divider);
    border-radius: var(--r-control);
    padding: 7px 12px;
    transition: border-color var(--t-fast) var(--ease);
  }

  .search:focus-within {
    border-color: var(--brand-orange);
  }

  .search svg {
    color: var(--ink-faint);
    flex-shrink: 0;
  }

  .search input {
    border: none;
    outline: none;
    font-size: 13px;
    color: var(--ink);
    background: transparent;
    width: 100%;
    font-family: var(--font-ui);
  }

  .search input::placeholder {
    color: var(--ink-faint);
  }

  .add-btn {
    width: 32px;
    height: 32px;
    border-radius: var(--r-control);
    background: var(--brand-orange);
    color: var(--on-orange);
    font-size: 18px;
    display: grid;
    place-items: center;
    transition: background var(--t-fast) var(--ease);
  }

  .add-btn:hover {
    background: var(--brand-orange-hov);
  }

  .vault-table {
    width: 100%;
    border-collapse: collapse;
  }

  .vault-table th {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--ink-soft);
    text-align: left;
    padding: 8px 12px;
    border-bottom: 1px solid var(--divider);
  }

  .vault-table th:last-child {
    width: 80px;
  }
</style>
