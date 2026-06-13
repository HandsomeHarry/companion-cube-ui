<script lang="ts">
  export let message: string;
  export let icon = '📭';
</script>

<div class="empty">
  <span class="empty__icon">{icon}</span>
  <p class="empty__msg">{message}</p>
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 80px 20px;
    text-align: center;
  }

  .empty__icon {
    font-size: 48px;
    margin-bottom: 16px;
  }

  .empty__msg {
    font-size: 15px;
    color: var(--ink-soft);
    max-width: 320px;
    line-height: 1.6;
  }
</style>
