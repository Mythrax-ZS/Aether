<script lang="ts">
  import { Plus, Trash2 } from '@lucide/svelte';
  import type { AgentMode, HostKeyPolicy } from '$lib/bindings';
  import { settings } from '$lib/stores/settings.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import IconButton from '$lib/components/ui/IconButton.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Segmented from '$lib/components/ui/Segmented.svelte';
  import Select from '$lib/components/ui/Select.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';
  import SettingRow from './SettingRow.svelte';

  const s = $derived(settings.value.ssh);
  const num = (v: string, lo: number, hi: number) => Math.min(hi, Math.max(lo, Math.floor(Number(v) || 0)));

  const POLICY: Record<HostKeyPolicy, string> = {
    ask: 'Ask me the first time I connect to a server, and block if its key ever changes.',
    'accept-new': 'Trust new servers automatically, but still block if a known key changes.',
    strict: 'Only connect to servers I’ve already verified. New servers are refused.',
  };

  function setIdentity(i: number, value: string) {
    const list = [...s.identityFiles];
    list[i] = value;
    settings.patch({ ssh: { identityFiles: list } });
  }
</script>

<div class="fsec">
  <h3>Sign-in</h3>
  <SettingRow title="Default user name" description="Used for hosts that don’t set their own. Empty means your Windows user name.">
    <div class="w220"><Input size="sm" mono value={s.defaultUsername} placeholder="your Windows user" oninput={(e) => settings.patch({ ssh: { defaultUsername: (e.currentTarget as HTMLInputElement).value.trim() } })} /></div>
  </SettingRow>
  <SettingRow title="SSH agent" description="Where Aether finds keys held by an agent.">
    <div class="w220">
      <Select
        value={s.agent}
        size="sm"
        label=""
        options={[
          { value: 'auto', label: 'Automatic' },
          { value: 'open-ssh', label: 'Windows OpenSSH Agent' },
          { value: 'pageant', label: 'Pageant (PuTTY)' },
          { value: 'off', label: 'Don’t use an agent' },
        ]}
        onchange={(v) => settings.patch({ ssh: { agent: v as AgentMode } })}
      />
    </div>
  </SettingRow>
  <div class="ids">
    <div class="flabel">Default key files</div>
    <p class="fhint">Tried for hosts that don’t pick a specific key. <code>~</code> means your user folder.</p>
    <div class="flist">
      {#each s.identityFiles as f, i (i)}
        <div class="id">
          <Input size="sm" mono value={f} placeholder="~/.ssh/id_ed25519" oninput={(e) => setIdentity(i, (e.currentTarget as HTMLInputElement).value)} />
          <IconButton label="Remove" size={26} danger onclick={() => settings.patch({ ssh: { identityFiles: s.identityFiles.filter((_, k) => k !== i) } })}><Trash2 size={14} /></IconButton>
        </div>
      {:else}
        <div class="flist-empty">No default key files.</div>
      {/each}
    </div>
    <div class="add"><Button size="sm" onclick={() => settings.patch({ ssh: { identityFiles: [...s.identityFiles, '~/.ssh/id_rsa'] } })}>{#snippet icon()}<Plus size={13} />{/snippet}Add a key file</Button></div>
  </div>
</div>

<div class="fsec">
  <h3>Trust</h3>
  <SettingRow title="Unknown servers" stacked description={POLICY[s.hostKeyPolicy]}>
    <Segmented
      value={s.hostKeyPolicy}
      label="Host key policy"
      options={[
        { value: 'ask', label: 'Ask' },
        { value: 'accept-new', label: 'Accept new' },
        { value: 'strict', label: 'Strict' },
      ]}
      onchange={(v) => settings.patch({ ssh: { hostKeyPolicy: v as HostKeyPolicy } })}
    />
  </SettingRow>
  <SettingRow title="Also trust ~/.ssh/known_hosts" description="Recognise servers you’ve already verified with OpenSSH. Aether only reads that file; it never changes it.">
    <Switch checked={s.readUserKnownHosts} aria="Read ~/.ssh/known_hosts" onchange={(v) => settings.patch({ ssh: { readUserKnownHosts: v } })} />
  </SettingRow>
  <SettingRow title="Use ~/.ssh/config" description="When you type a name or address, apply the matching Host block (user, port, key, ProxyJump…).">
    <Switch checked={s.useSshConfig} aria="Use ~/.ssh/config" onchange={(v) => settings.patch({ ssh: { useSshConfig: v } })} />
  </SettingRow>
</div>

<div class="fsec">
  <h3>Reliability</h3>
  <SettingRow title="Reconnect automatically" description="When the network drops or the server restarts, keep trying and bring your tunnels back.">
    <Switch checked={s.autoReconnect} aria="Reconnect automatically" onchange={(v) => settings.patch({ ssh: { autoReconnect: v } })} />
  </SettingRow>
  <SettingRow title="Give up after" description="Attempts before Aether stops retrying. 0 keeps trying until you stop it.">
    <div class="w120"><Input size="sm" mono type="number" min={0} max={200} value={s.reconnectMaxAttempts} oninput={(e) => settings.patch({ ssh: { reconnectMaxAttempts: num((e.currentTarget as HTMLInputElement).value, 0, 200) } })} /></div>
  </SettingRow>
  <SettingRow title="Keepalive interval" description="Seconds of silence before checking the connection is alive. 0 turns it off.">
    <div class="w120"><Input size="sm" mono type="number" min={0} max={3600} value={s.keepaliveIntervalSecs} oninput={(e) => settings.patch({ ssh: { keepaliveIntervalSecs: num((e.currentTarget as HTMLInputElement).value, 0, 3600) } })} /></div>
  </SettingRow>
  <SettingRow title="Missed keepalives allowed" description="How many go unanswered before the connection counts as lost.">
    <div class="w120"><Input size="sm" mono type="number" min={1} max={20} value={s.keepaliveMax} oninput={(e) => settings.patch({ ssh: { keepaliveMax: num((e.currentTarget as HTMLInputElement).value, 1, 20) } })} /></div>
  </SettingRow>
  <SettingRow title="Connection timeout" description="Seconds to wait for a server to answer.">
    <div class="w120"><Input size="sm" mono type="number" min={1} max={300} value={s.connectTimeoutSecs} oninput={(e) => settings.patch({ ssh: { connectTimeoutSecs: num((e.currentTarget as HTMLInputElement).value, 1, 300) } })} /></div>
  </SettingRow>
</div>

<div class="fsec">
  <h3>Terminal type</h3>
  <SettingRow title="Default $TERM" description="Reported to servers. xterm-256color is right for almost everything.">
    <div class="w220"><Input size="sm" mono value={s.defaultTerm} oninput={(e) => settings.patch({ ssh: { defaultTerm: (e.currentTarget as HTMLInputElement).value.trim() || 'xterm-256color' } })} /></div>
  </SettingRow>
  <SettingRow title="Low-latency networking" description="Send small packets immediately (TCP_NODELAY). Keeps typing responsive.">
    <Switch checked={s.tcpNodelay} aria="TCP no-delay" onchange={(v) => settings.patch({ ssh: { tcpNodelay: v } })} />
  </SettingRow>
</div>

<div class="fsec">
  <h3>Graphical programs (X11)</h3>
  <SettingRow title="X server address" description="Where an X server runs on this PC (VcXsrv, X410, Xming). Programs started on a host that has X11 turned on will open there. It must accept TCP connections from this PC.">
    <div class="w220"><Input size="sm" mono value={s.x11Display} placeholder="localhost:0" oninput={(e) => settings.patch({ ssh: { x11Display: (e.currentTarget as HTMLInputElement).value.trim() || 'localhost:0' } })} /></div>
  </SettingRow>
</div>

<style>
  .w120 {
    width: 110px;
  }
  .w220 {
    width: 220px;
  }
  .ids {
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .id {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 8px;
    align-items: center;
    padding: 7px 8px;
  }
  .add {
    margin-top: 10px;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
