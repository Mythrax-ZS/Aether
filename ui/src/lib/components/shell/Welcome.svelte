<script lang="ts">
  import { ArrowRight, Download, KeyRound, Plus, Zap } from '@lucide/svelte';
  import { connectHost, connectTarget } from '$lib/actions';
  import { keymap } from '$lib/commands.svelte';
  import { hosts } from '$lib/stores/hosts.svelte';
  import { ui } from '$lib/stores/ui.svelte';
  import { formatRelative } from '$lib/util/format';
  import { searchHosts } from '$lib/util/tree';
  import Avatar from '$lib/components/ui/Avatar.svelte';
  import Kbd from '$lib/components/ui/Kbd.svelte';
  import Logo from '$lib/components/ui/Logo.svelte';

  let query = $state('');
  let active = $state(0);

  const matches = $derived(query.trim() ? searchHosts(hosts.data, query).slice(0, 5) : []);
  const featured = $derived.by(() => {
    const seen = new Set<string>();
    const out = [];
    for (const h of [...hosts.recent, ...hosts.favorites, ...hosts.data.hosts]) {
      if (seen.has(h.id)) continue;
      seen.add(h.id);
      out.push(h);
      if (out.length === 6) break;
    }
    return out;
  });
  const recentTargets = $derived(hosts.recents.slice(0, 3));

  $effect(() => {
    void matches.length;
    active = 0;
  });

  function submit() {
    const q = query.trim();
    if (!q) {
      ui.openPalette('connect');
      return;
    }
    if (matches[active]) connectHost(matches[active].item);
    else connectTarget(q);
    query = '';
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' && matches.length) {
      e.preventDefault();
      active = (active + 1) % matches.length;
    } else if (e.key === 'ArrowUp' && matches.length) {
      e.preventDefault();
      active = (active - 1 + matches.length) % matches.length;
    } else if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    } else if (e.key === 'Escape') {
      query = '';
    }
  }
</script>

<div class="welcome">
  <div class="grid" aria-hidden="true"></div>
  <div class="glow ambient" aria-hidden="true"><i class="a"></i><i class="b"></i></div>

  <div class="content">
    <div class="hero">
      <div class="mark"><Logo size={92} animated="ambient" /></div>
      <h1>Aether</h1>
      <p class="tag">Secure shells, beautifully crafted.</p>
    </div>

    <div class="quick" class:open={matches.length > 0}>
      <label class="field">
        <span class="qi"><Zap size={17} /></span>
        <input
          bind:value={query}
          type="text"
          placeholder="Connect to user@host, or search saved hosts"
          spellcheck="false"
          autocomplete="off"
          aria-label="Quick connect"
          onkeydown={onKey}
        />
        <button type="button" class="go" aria-label="Connect" onclick={submit}><ArrowRight size={17} /></button>
      </label>
      {#if matches.length}
        <ul class="suggest" role="listbox">
          {#each matches as m, i (m.item.id)}
            <li>
              <button type="button" role="option" aria-selected={i === active} class:on={i === active} onpointerenter={() => (active = i)} onclick={() => connectHost(m.item)}>
                <Avatar name={m.item.label || m.item.hostname} color={m.item.color} size={24} />
                <span class="n">{m.item.label || m.item.hostname}</span>
                <span class="a">{m.item.username ? m.item.username + '@' : ''}{m.item.hostname}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    {#if hosts.isEmpty}
      <div class="start">
        <button type="button" class="card" onclick={() => ui.newHost()}>
          <span class="ci"><Plus size={20} /></span>
          <span class="ct">Add your first host</span>
          <span class="cd">Save a server with its address, user and key.</span>
        </button>
        <button type="button" class="card" onclick={() => ui.openDialog({ type: 'import-ssh' })}>
          <span class="ci"><Download size={20} /></span>
          <span class="ct">Import from ssh config</span>
          <span class="cd">Bring over everything in your ~/.ssh/config.</span>
        </button>
        <button type="button" class="card" onclick={() => ui.openDialog({ type: 'keys' })}>
          <span class="ci"><KeyRound size={20} /></span>
          <span class="ct">Set up an SSH key</span>
          <span class="cd">Generate a modern key or import one you have.</span>
        </button>
      </div>
    {:else if featured.length}
      <section class="recent" aria-label="Hosts">
        <h2>{hosts.recent.length ? 'Jump back in' : 'Your hosts'}</h2>
        <div class="cards">
          {#each featured as h (h.id)}
            <button type="button" class="host" onclick={() => connectHost(h)}>
              <Avatar name={h.label || h.hostname} color={h.color} size={34} />
              <span class="ht">
                <span class="hn">{h.label || h.hostname}</span>
                <span class="ha">{h.username ? h.username + '@' : ''}{h.hostname}</span>
              </span>
              <span class="hw">{h.lastConnectedAt ? formatRelative(h.lastConnectedAt) : ''}</span>
            </button>
          {/each}
        </div>
        {#if recentTargets.length}
          <p class="targets">
            Recent quick connects:
            {#each recentTargets as t (t.target)}<button type="button" onclick={() => connectTarget(t.target)}>{t.target}</button>{/each}
          </p>
        {/if}
      </section>
    {/if}

    <footer class="tips">
      <span><Kbd keys={keymap.display('palette.open')} subtle /> command palette</span>
      <span><Kbd keys={keymap.display('tab.new')} subtle /> new connection</span>
      <span><Kbd keys={keymap.display('app.sidebar')} subtle /> sidebar</span>
      <span><Kbd keys={keymap.display('app.shortcuts')} subtle /> shortcuts</span>
    </footer>
  </div>
</div>

<style>
  .welcome {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    overflow: auto;
    background: var(--bg-app);
  }
  .grid {
    position: absolute;
    inset: 0;
    background-image: radial-gradient(color-mix(in srgb, var(--text) 16%, transparent) 1px, transparent 1.2px);
    background-size: 24px 24px;
    mask-image: radial-gradient(ellipse 60% 55% at 50% 38%, #000 10%, transparent 75%);
    -webkit-mask-image: radial-gradient(ellipse 60% 55% at 50% 38%, #000 10%, transparent 75%);
    opacity: 0.5;
    pointer-events: none;
  }
  .glow {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
    opacity: 0.5;
  }
  /* Soft radial gradients moved by `transform` alone: the compositor just slides two layers around. A CSS `filter: blur()` on
     the container would be re-run over the whole window on every frame, which cost a quarter of a CPU core at idle. */
  .glow i {
    position: absolute;
    border-radius: 50%;
    will-change: transform;
  }
  .glow .a {
    left: 20%;
    top: -10%;
    width: 42%;
    aspect-ratio: 1;
    background: radial-gradient(circle, rgb(70 216 255 / 0.42), transparent 65%);
    animation: aurora-a 19s ease-in-out infinite;
  }
  .glow .b {
    right: 12%;
    top: 14%;
    width: 38%;
    aspect-ratio: 1;
    background: radial-gradient(circle, rgb(155 120 255 / 0.4), transparent 65%);
    animation: aurora-b 24s ease-in-out infinite;
  }
  .content {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 28px;
    width: min(680px, 100%);
    padding: 32px 28px 40px;
    animation: fade-up 520ms var(--ease-out) both;
  }
  .hero {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
  }
  .mark {
    margin-bottom: 8px;
    filter: drop-shadow(0 18px 44px rgb(111 140 255 / 0.45));
  }
  h1 {
    font-size: var(--fs-3xl);
    font-weight: 750;
    letter-spacing: -0.03em;
    background: var(--grad-accent);
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
  }
  .tag {
    color: var(--text-dim);
    font-size: var(--fs-lg);
  }

  .quick {
    position: relative;
    width: 100%;
  }
  .field {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 54px;
    padding: 0 8px 0 18px;
    background: color-mix(in srgb, var(--bg-surface) 94%, transparent);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-xl);
    box-shadow: var(--shadow-md);
    transition:
      border-color var(--dur-fast),
      box-shadow var(--dur);
  }
  .field:focus-within {
    border-color: var(--accent);
    box-shadow: var(--shadow-md), 0 0 0 4px var(--accent-soft);
  }
  .qi {
    display: inline-flex;
    color: var(--accent);
  }
  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    background: none;
    border: 0;
    outline: none;
    color: var(--text);
    font-size: var(--fs-xl);
    cursor: text;
  }
  input::placeholder {
    color: var(--text-faint);
  }
  .go {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    border-radius: var(--r-lg);
    background: var(--grad-accent);
    color: var(--on-accent);
    transition: transform var(--dur-fast) var(--ease-spring);
  }
  .go:hover {
    transform: translateX(2px);
  }
  .suggest {
    position: absolute;
    top: calc(100% + 8px);
    left: 0;
    right: 0;
    z-index: 4;
    margin: 0;
    padding: 6px;
    list-style: none;
    background: color-mix(in srgb, var(--bg-overlay) 97%, transparent);
    border: 1px solid var(--border-strong);
    border-radius: var(--r-lg);
    box-shadow: var(--shadow-lg);
    animation: scale-in 130ms var(--ease-out);
    transform-origin: top;
  }
  .suggest button {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 38px;
    padding: 0 10px;
    border-radius: var(--r-md);
    text-align: left;
  }
  .suggest button.on {
    background: var(--bg-active);
  }
  .suggest .n {
    font-weight: 600;
  }
  .suggest .a {
    margin-left: auto;
    color: var(--text-faint);
    font-size: var(--fs-sm);
    font-family: var(--font-mono);
  }

  .start {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
    width: 100%;
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 18px 16px;
    text-align: left;
    background: color-mix(in srgb, var(--bg-surface) 80%, transparent);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    transition:
      transform var(--dur) var(--ease-out),
      border-color var(--dur-fast),
      background var(--dur-fast);
  }
  .card:hover {
    transform: translateY(-3px);
    border-color: var(--accent-ring);
    background: var(--bg-surface);
  }
  .ci {
    display: grid;
    place-items: center;
    width: 38px;
    height: 38px;
    margin-bottom: 4px;
    border-radius: 11px;
    background: var(--grad-accent-soft);
    color: var(--accent);
  }
  .ct {
    font-weight: 650;
  }
  .cd {
    color: var(--text-dim);
    font-size: var(--fs-sm);
    line-height: 1.45;
  }

  .recent {
    width: 100%;
  }
  .recent h2 {
    margin-bottom: 10px;
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(196px, 1fr));
    gap: 10px;
  }
  .host {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 11px 12px;
    text-align: left;
    background: color-mix(in srgb, var(--bg-surface) 80%, transparent);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    transition:
      transform var(--dur) var(--ease-out),
      border-color var(--dur-fast),
      background var(--dur-fast);
  }
  .host:hover {
    transform: translateY(-2px);
    border-color: var(--accent-ring);
    background: var(--bg-surface);
  }
  .ht {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .hn {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ha {
    font-size: var(--fs-xs);
    color: var(--text-faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hw {
    font-size: var(--fs-2xs);
    color: var(--text-faint);
    white-space: nowrap;
  }
  .targets {
    margin-top: 12px;
    font-size: var(--fs-sm);
    color: var(--text-faint);
  }
  .targets button {
    margin-left: 8px;
    color: var(--accent);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  .targets button:hover {
    text-decoration: underline;
  }

  .tips {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px 22px;
    margin-top: 8px;
    color: var(--text-faint);
    font-size: var(--fs-sm);
  }
  .tips span {
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }

  @media (max-width: 760px) {
    .start {
      grid-template-columns: 1fr;
    }
  }
</style>
