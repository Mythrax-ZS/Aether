<script lang="ts">
  let { size = 16, label = 'Loading' }: { size?: number; label?: string } = $props();
</script>

<svg class="spin" width={size} height={size} viewBox="0 0 24 24" fill="none" role="status" aria-label={label}>
  <circle cx="12" cy="12" r="9" stroke="currentColor" stroke-opacity="0.2" stroke-width="2.6" />
  <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" />
</svg>

<style>
  .spin {
    animation: spin 0.75s linear infinite;
    flex-shrink: 0;
  }
</style>
