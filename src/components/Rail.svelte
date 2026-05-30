<script lang="ts">
  import { activeView } from '$lib/stores';
  import { applyTheme, nextTheme, type ThemeName } from '$lib/theme';

  type View = 'history' | 'vault' | 'rhythm' | 'settings';
  export let onViewChange: (view: View) => void;

  function cycleTheme() {
    const current = (document.documentElement.dataset.theme ?? 'paper') as ThemeName;
    applyTheme(nextTheme(current));
  }
</script>

<nav class="rail">
  <div class="rail__top">
    <button
      class="rail__btn"
      class:active={$activeView === 'history'}
      on:click={() => onViewChange('history')}
      aria-label="History"
      title="History"
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10"/>
        <polyline points="12 6 12 12 16 14"/>
      </svg>
    </button>

    <button
      class="rail__btn"
      class:active={$activeView === 'vault'}
      on:click={() => onViewChange('vault')}
      aria-label="Vault"
      title="Vault"
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
        <polyline points="3.27 6.96 12 12.01 20.73 6.96"/>
        <line x1="12" y1="22.08" x2="12" y2="12"/>
      </svg>
    </button>
    <button
      class="rail__btn"
      class:active={$activeView === 'rhythm'}
      on:click={() => onViewChange('rhythm')}
      aria-label="Rhythm"
      title="Rhythm"
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>
      </svg>
    </button>
  </div>

  <div class="rail__bottom">
    <button
      class="rail__btn"
      on:click={cycleTheme}
      aria-label="Switch theme"
      title="Switch theme"
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="9"/>
        <path d="M12 3a9 9 0 0 0 0 18z" fill="currentColor" stroke="none"/>
      </svg>
    </button>

    <button
      class="rail__btn"
      class:active={$activeView === 'settings'}
      on:click={() => onViewChange('settings')}
      aria-label="Settings"
      title="Settings"
    >
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="3"/>
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
      </svg>
    </button>
  </div>
</nav>

<style>
  .rail {
    width: 52px;
    min-height: 100%;
    background: var(--paper);
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 14px 0;
    border-right: 1px solid var(--divider);
    flex-shrink: 0;
  }

  .rail__top {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .rail__bottom {
    margin-top: auto;
  }

  .rail__btn {
    width: 34px;
    height: 34px;
    border-radius: var(--r-panel);
    display: grid;
    place-items: center;
    color: var(--ink-soft);
    transition: all var(--t-normal) var(--ease);
  }

  .rail__btn:hover {
    background: var(--row-hover);
  }

  .rail__btn.active {
    background: var(--brand-orange);
    color: var(--on-orange);
  }
</style>
