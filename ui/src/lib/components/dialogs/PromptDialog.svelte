<script lang="ts">
  import { Copy, KeyRound, Lock, ShieldAlert, ShieldQuestion } from '@lucide/svelte';
  import { copyText } from '$lib/actions';
  import type { HostKeyDecision, HostKeyRequest, PromptRequest } from '$lib/bindings';
  import { prompts } from '$lib/stores/prompts.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Modal from '$lib/components/ui/Modal.svelte';

  const current = $derived(prompts.current);

  let answers = $state<string[]>([]);
  let remember = $state(false);
  let verified = $state(false);
  let busy = $state(false);

  // Fresh state for every prompt (and for each retry of the same one).
  $effect(() => {
    const p = current;
    answers = p?.payload.type === 'secret' && p.payload.request.kind.type === 'keyboard-interactive' ? p.payload.request.kind.prompts.map(() => '') : [''];
    // Storing a secret is opt-in: nothing is saved unless the user ticks the box.
    remember = false;
    verified = false;
    busy = false;
  });

  function hopLabel(hop: { username: string; host: string; port: number; total: number; index: number; label: string }): string {
    const base = `${hop.username ? hop.username + '@' : ''}${hop.host}${hop.port !== 22 ? ':' + hop.port : ''}`;
    return hop.total > 1 ? `${base} · hop ${hop.index + 1} of ${hop.total}` : base;
  }

  async function decide(decision: HostKeyDecision, id: string) {
    // The same half second covers the mouse: a double-click on one question must not land its second click on the
    // next question's "Trust and connect", which appears in the very same place. Saying no is always allowed.
    if (decision !== 'reject' && performance.now() - openedAt < GUARD_MS) return;
    busy = true;
    await prompts.respond({ type: 'host-key', decision }, id);
  }

  async function submit(id: string, e?: Event) {
    e?.preventDefault();
    // An empty password is a real password on some devices, so it is allowed. What it must not be is an accident: an
    // Enter pressed for something else, or the second click of a double-click that now lands on the next question.
    // Nobody fills in a prompt within half a second of seeing it, so nothing is submitted in that time.
    if (busy || performance.now() - openedAt < GUARD_MS) return;
    busy = true;
    await prompts.respond({ type: 'answers', answers: [...answers], remember }, id);
  }

  function secretTitle(r: PromptRequest): string {
    switch (r.kind.type) {
      case 'password':
        return 'Password required';
      case 'key-passphrase':
        return 'Unlock your key';
      default:
        return r.kind.name?.trim() || 'Verification required';
    }
  }

  const art = (r: HostKeyRequest) => r.randomart.replace(/\s+$/, '');

  // A question appears on its own, often while the person is typing into the terminal behind it. Whatever Enter or
  // Space they were in the middle of was meant for the terminal, not for "Trust and connect" — so for a question about
  // trusting a server those two keys do nothing for the first half second; and no answer of any kind is accepted in that
  // time (see `submit` and `decide`), whether it comes from the keyboard or the mouse.
  const GUARD_MS = 500;
  let openedAt = 0;
  $effect(() => {
    void current?.id;
    openedAt = performance.now();
  });
  function guardKeys(e: KeyboardEvent) {
    if (current?.payload.type !== 'host-key') return;
    if ((e.key === 'Enter' || e.key === ' ') && performance.now() - openedAt < GUARD_MS) {
      e.preventDefault();
      e.stopImmediatePropagation();
    }
  }
</script>

<svelte:window onkeydowncapture={guardKeys} onkeyupcapture={guardKeys} onkeypresscapture={guardKeys} />

<!-- Every prompt gets its own dialog. Without the key, a retry that arrives while the previous dialog is
     still fading out would bring that same (already inert, unfocused) dialog back instead of a new one. -->
{#key current?.id}
{#if current?.payload.type === 'host-key'}
  {@const req = current.payload.request}
  {@const pid = current.id}
  {#if req.kind.type === 'changed'}
    <!-- The key is different from the one we trust: the safe answer is the default one. -->
    <!-- A server that has keys on record but now shows one of another type isn't "changed" in the strict sense — but an
         impostor would do exactly that to skip the warning, so it gets the same treatment with its own explanation. -->
    {@const newType = !req.kind.known.some((k) => k.algorithm === req.algorithm)}
    <Modal title={newType ? 'Unfamiliar host key' : 'Host key has changed'} subtitle={hopLabel(req.hop)} width={600} onclose={() => decide('reject', pid)}>
      {#snippet icon()}<span class="danger-ico"><ShieldAlert size={18} /></span>{/snippet}
      <div class="alert">
        {#if newType}
          <strong>This server is using a type of key you haven’t verified before.</strong>
          <p>
            You’ve trusted its {[...new Set(req.kind.known.map((k) => k.algorithm))].join(' and ')} key, but this time it identified itself with
            {req.algorithm}. Servers often have several host keys, so this can be harmless — but it is also what a man-in-the-middle does to avoid the
            usual warning. Don’t continue unless you can confirm the fingerprint below.
          </p>
        {:else}
          <strong>Someone may be intercepting this connection.</strong>
          <p>
            This server identified itself differently from last time. That can happen after a reinstall or key rotation — but it is also exactly what a
            man-in-the-middle attack looks like. Don’t continue unless you know why it changed.
          </p>
        {/if}
      </div>

      <div class="compare">
        <section>
          <h4>Trusted before</h4>
          {#each req.kind.known as k (`${k.source}:${k.line}`)}
            <div class="fp"><span class="alg">{k.algorithm}</span><code class="selectable">{k.fingerprint}</code></div>
          {/each}
        </section>
        <section class="now">
          <h4>Presented now</h4>
          <div class="fp">
            <span class="alg">{req.algorithm}{req.bits ? ` · ${req.bits}-bit` : ''}</span>
            <code class="selectable">{req.fingerprint}</code>
          </div>
        </section>
      </div>

      <label class="check">
        <input type="checkbox" bind:checked={verified} />
        <span>I’ve confirmed the new fingerprint with the server’s administrator</span>
      </label>

      {#snippet footer()}
        <Button variant="primary" autofocus onclick={() => decide('reject', pid)}>Cancel connection</Button>
        <Button variant="danger" disabled={!verified || busy} onclick={() => decide('replace-and-trust', pid)}>{newType ? 'Trust this key and connect' : 'Replace saved key and connect'}</Button>
      {/snippet}
    </Modal>
  {:else}
    <Modal title="Trust this server?" subtitle={hopLabel(req.hop)} width={600} onclose={() => decide('reject', pid)}>
      {#snippet icon()}<ShieldQuestion size={18} />{/snippet}
      <p class="lead">
        You haven’t connected to this server before. Check that the fingerprint matches the one its owner gave you — that’s how you know it’s really them.
      </p>
      <div class="print">
        <div class="meta">
          <div class="fp big">
            <span class="alg">{req.algorithm}{req.bits ? ` · ${req.bits}-bit` : ''}</span>
            <div class="code">
              <code class="selectable">{req.fingerprint}</code>
              <button type="button" class="copy" aria-label="Copy fingerprint" onclick={() => copyText(req.fingerprint, 'Fingerprint copied')}><Copy size={14} /></button>
            </div>
          </div>
          <p class="small">Aether remembers it, and warns you loudly if it ever changes.</p>
        </div>
        <pre class="art selectable" aria-label="Fingerprint randomart">{art(req)}</pre>
      </div>
      {#snippet footer()}
        <Button variant="ghost" onclick={() => decide('reject', pid)}>Cancel</Button>
        <Button disabled={busy} onclick={() => decide('trust-once', pid)}>Connect once</Button>
        <Button variant="primary" autofocus disabled={busy} onclick={() => decide('trust-and-save', pid)}>Trust and connect</Button>
      {/snippet}
    </Modal>
  {/if}
{:else if current?.payload.type === 'secret'}
  {@const req = current.payload.request}
  {@const pid = current.id}
  <Modal title={secretTitle(req)} subtitle={hopLabel(req.hop)} width={460} onclose={() => prompts.cancel(pid)}>
    {#snippet icon()}
      {#if req.kind.type === 'key-passphrase'}<KeyRound size={18} />{:else}<Lock size={18} />{/if}
    {/snippet}
    <form onsubmit={(e) => submit(pid, e)} class="form">
      {#if req.error}<p class="error" role="alert">{req.error}</p>{/if}

      {#if req.kind.type === 'keyboard-interactive'}
        {#if req.kind.instructions}<p class="lead">{req.kind.instructions}</p>{/if}
        {#each req.kind.prompts as p, i (i)}
          <Input bind:value={answers[i]} label={p.text.replace(/:\s*$/, '')} type={p.echo ? 'text' : 'password'} autofocus={i === 0} autocomplete={p.echo ? 'off' : 'one-time-code'} />
        {/each}
      {:else if req.kind.type === 'key-passphrase'}
        <p class="lead">Enter the passphrase for <code>{req.kind.key}</code>.</p>
        <Input bind:value={answers[0]} label="Passphrase" type="password" autofocus autocomplete="current-password" />
      {:else}
        <Input bind:value={answers[0]} label="Password" type="password" autofocus autocomplete="current-password" />
      {/if}

      {#if req.canRemember}
        <label class="check">
          <input type="checkbox" bind:checked={remember} />
          <span>Remember in Windows Credential Manager</span>
        </label>
      {/if}
    </form>
    {#snippet footer()}
      <Button variant="ghost" onclick={() => prompts.cancel(pid)}>Cancel</Button>
      <Button variant="primary" loading={busy} onclick={() => submit(pid)}>{req.kind.type === 'password' ? 'Sign in' : 'Continue'}</Button>
    {/snippet}
  </Modal>
{/if}
{/key}

<style>
  .lead {
    color: var(--text-dim);
    line-height: 1.55;
    margin-bottom: 14px;
  }
  .lead code {
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    color: var(--text);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .error {
    padding: 9px 12px;
    border-radius: var(--r-md);
    background: color-mix(in srgb, var(--danger) 14%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 9px;
    color: var(--text-dim);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .check input {
    accent-color: var(--accent);
    width: 15px;
    height: 15px;
  }

  .print {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 16px;
    align-items: start;
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .fp {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 10px 12px;
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
  }
  .fp + .fp {
    margin-top: 6px;
  }
  .alg {
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--accent-2);
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-sm);
    word-break: break-all;
    line-height: 1.5;
  }
  .code {
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }
  .code code {
    flex: 1;
  }
  .copy {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    border-radius: var(--r-sm);
    color: var(--text-faint);
  }
  .copy:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .small {
    color: var(--text-faint);
    font-size: var(--fs-sm);
    line-height: 1.5;
  }
  .art {
    margin: 0;
    padding: 10px 12px;
    font: 600 12px/1.15 var(--font-mono);
    letter-spacing: 0.02em;
    color: var(--accent);
    background: var(--bg-input);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    user-select: text;
  }

  .danger-ico {
    display: inline-flex;
    color: var(--danger);
  }
  .alert {
    padding: 12px 14px;
    margin-bottom: 14px;
    border-radius: var(--r-md);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 38%, transparent);
    border-left: 3px solid var(--danger);
  }
  .alert strong {
    display: block;
    margin-bottom: 4px;
    color: var(--danger);
  }
  .alert p {
    color: var(--text-dim);
    line-height: 1.55;
    font-size: var(--fs-sm);
  }
  .compare {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    margin-bottom: 14px;
  }
  .compare h4 {
    margin-bottom: 6px;
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-faint);
  }
  .now .fp {
    border-color: color-mix(in srgb, var(--danger) 40%, transparent);
  }
</style>
