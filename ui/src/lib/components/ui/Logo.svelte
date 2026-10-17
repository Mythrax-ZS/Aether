<script lang="ts">
  // The app icon as inline SVG so it can animate. Mirrors assets/aether-icon.svg.
  /** `true` animates for as long as it is shown (a progress cue); `'ambient'` is decoration that rests when idle. */
  let { size = 24, animated = false, tile = true }: { size?: number; animated?: boolean | 'ambient'; tile?: boolean } = $props();

  const uid = Math.random().toString(36).slice(2, 8);
</script>

<svg
  class="logo"
  class:animated={!!animated}
  class:ambient={animated === 'ambient'}
  width={size}
  height={size}
  viewBox={tile ? '64 64 896 896' : '180 180 664 664'}
  role="img"
  aria-label="Aether"
>
  <defs>
    <linearGradient id="bg-{uid}" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#1b2140" />
      <stop offset="0.55" stop-color="#0f1226" />
      <stop offset="1" stop-color="#090b17" />
    </linearGradient>
    <radialGradient id="glow-{uid}" cx="0.5" cy="0.46" r="0.55">
      <stop offset="0" stop-color="#5b8cff" stop-opacity="0.42" />
      <stop offset="0.55" stop-color="#7a5cff" stop-opacity="0.16" />
      <stop offset="1" stop-color="#7a5cff" stop-opacity="0" />
    </radialGradient>
    <linearGradient id="ring-{uid}" x1="0.1" y1="0.9" x2="0.9" y2="0.1">
      <stop offset="0" stop-color="#46e0ff" />
      <stop offset="0.5" stop-color="#6f8cff" />
      <stop offset="1" stop-color="#b98cff" />
    </linearGradient>
    <linearGradient id="chev-{uid}" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#ffffff" />
      <stop offset="1" stop-color="#cfe6ff" />
    </linearGradient>
    <clipPath id="tile-{uid}"><rect x="64" y="64" width="896" height="896" rx="212" /></clipPath>
  </defs>

  {#if tile}
    <rect x="64" y="64" width="896" height="896" rx="212" fill="url(#bg-{uid})" />
    <g clip-path="url(#tile-{uid})">
      <rect x="64" y="64" width="896" height="896" fill="url(#glow-{uid})" />
    </g>
    <rect x="64.5" y="64.5" width="895" height="895" rx="211.5" fill="none" stroke="#ffffff" stroke-opacity="0.1" />
  {/if}

  <!-- The tilt is an attribute; the slow spin lives on an inner group so CSS never overrides it. -->
  <g transform="rotate(-38 512 512)">
    <g class="orbit">
      <circle cx="512" cy="512" r="292" fill="none" stroke="url(#ring-{uid})" stroke-width="44" stroke-linecap="round" stroke-dasharray="1330 506" />
    </g>
  </g>
  <circle class="particle" cx="755" cy="282" r="30" fill="#ffffff" />
  <path d="M 420 404 L 574 520 L 420 636" fill="none" stroke="url(#chev-{uid})" stroke-width="66" stroke-linecap="round" stroke-linejoin="round" />
  <rect class="cursor" x="606" y="610" width="124" height="56" rx="28" fill="url(#chev-{uid})" />
</svg>

<style>
  .logo {
    display: block;
    flex-shrink: 0;
  }
  .animated .orbit {
    /* Spin around the ring's own centre, whatever the viewBox origin is. */
    transform-box: fill-box;
    transform-origin: center;
    animation: ring-sweep 18s linear infinite;
  }
  .animated .cursor {
    animation: pulse-dot 1.6s var(--ease-in-out) infinite;
  }
</style>
