<script lang="ts">
  import { onMount } from 'svelte';
  import { filedrop } from '$lib/stores/filedrop.svelte';
  import { terminalRegistry } from '$lib/terminal/registry';
  import { safePaste, shellQuote } from '$lib/terminal/safety';
  import { workspace } from '$lib/stores/workspace.svelte';
  import PaneLayout from '$lib/components/terminal/PaneLayout.svelte';
  import SftpView from '$lib/components/sftp/SftpView.svelte';
  import Welcome from './Welcome.svelte';

  // Dropping files on a terminal types their paths at the cursor, each as one shell word. A file's name is chosen by
  // whoever made the file, so it is quoted the one way in which nothing inside it is special — a file called
  // `a$(touch pwned).txt` has to arrive as a name, not as a command.
  onMount(() =>
    filedrop.register((paths) => {
      const tab = workspace.activeTerminalTab;
      if (!tab || paths.length === 0) return false;
      const controller = terminalRegistry.get(tab.activePane);
      if (!controller) return false;
      const quoted = safePaste(paths.map(shellQuote).join(' '));
      controller.insert(quoted + ' ');
      controller.focus();
      return true;
    }),
  );
</script>

<main class="stage" aria-label="Sessions">
  {#each workspace.tabs as tab (tab.id)}
    {@const visible = tab.id === workspace.activeId}
    <section class="tab" class:visible aria-hidden={!visible} inert={!visible}>
      {#if tab.kind === 'terminal'}
        <PaneLayout {tab} {visible} />
      {:else}
        <SftpView {tab} {visible} />
      {/if}
    </section>
  {/each}

  {#if workspace.tabs.length === 0}
    <Welcome />
  {/if}
</main>

<style>
  .stage {
    grid-area: stage;
    position: relative;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: var(--stage-bg, var(--bg-app));
    border-top: 1px solid var(--border);
    border-top-left-radius: 12px;
  }
  .tab {
    position: absolute;
    inset: 0;
    display: none;
  }
  .tab.visible {
    display: block;
  }
</style>
