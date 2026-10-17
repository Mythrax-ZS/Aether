import '@fontsource-variable/inter';
import '@fontsource-variable/jetbrains-mono';
import './styles/tokens.css';
import './styles/base.css';
import './styles/animations.css';
import './styles/fonts.css';
import './styles/forms.css';
import { mount } from 'svelte';
import App from './App.svelte';

const app = mount(App, { target: document.getElementById('app')! });

// Development only: a handle on the live stores for poking at the running app from DevTools.
if (import.meta.env.DEV) {
  void Promise.all([
    import('$lib/stores/workspace.svelte'),
    import('$lib/stores/settings.svelte'),
    import('$lib/stores/ui.svelte'),
    import('$lib/stores/sessions.svelte'),
    import('$lib/terminal/registry'),
    import('$lib/stores/prompts.svelte'),
    import('$lib/stores/hosts.svelte'),
    import('$lib/api'),
  ]).then(([w, s, u, ss, r, p, h, a]) => {
    Object.assign(window, {
      __aether: { workspace: w.workspace, settings: s.settings, ui: u.ui, sessions: ss.sessions, terminals: r.terminalRegistry, prompts: p.prompts, hosts: h.hosts, api: a.api },
    });
  });
}

export default app;
