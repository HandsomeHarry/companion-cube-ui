<script lang="ts">
  import type { EventRow } from '$lib/types';
  import TimelineGroup from './TimelineGroup.svelte';

  export let events: EventRow[];

  $: groups = buildGroups(events);

  function buildGroups(events: EventRow[]) {
    if (events.length === 0) return [];

    const result: { time: string; title: string; events: EventRow[] }[] = [];
    let currentGroup: { time: string; title: string; events: EventRow[] } | null = null;

    for (const event of events) {
      const time = new Date(event.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
      const app = event.app ?? 'Unknown';

      if (!currentGroup || currentGroup.title !== app) {
        currentGroup = { time, title: app, events: [event] };
        result.push(currentGroup);
      } else {
        currentGroup.events.push(event);
      }
    }

    return result;
  }
</script>

<div class="timeline">
  {#each groups as group (group.time + group.title)}
    <TimelineGroup time={group.time} title={group.title} events={group.events} />
  {/each}
</div>

<style>
  .timeline {
    padding-top: 8px;
  }
</style>
