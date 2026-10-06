<script lang="ts">
  import { api, errorMessage, isAndroid, type BiometricState, type StoreInfo } from "./lib/api";
  import { backLayer } from "./lib/layers";
  import { AUTO_LOCK_CHOICES, autoLockMinutes, setAutoLockMinutes } from "./lib/settings";

  let {
    store,
    stores,
    onclose,
    onchange,
  }: { store: StoreInfo; stores: StoreInfo[]; onclose: () => void; onchange: () => void } = $props();

  $effect(() => backLayer(() => onclose())); // not tracked: the effect runs once, when the layer opens

  const method = isAndroid ? "fingerprint" : "Touch ID";

  let minutes = $state(autoLockMinutes());
  let biometric = $state<BiometricState | null>(null);
  let busy = $state(false);
  let error = $state("");

  $effect(() => {
    biometric = null;
    api.biometricState(store.id).then((state) => (biometric = state), () => (biometric = { available: false, enabled: false }));
  });

  function chooseMinutes(value: number) {
    minutes = value;
    setAutoLockMinutes(value);
  }

  async function toggleBiometric() {
    if (!biometric) return;
    busy = true;
    error = "";
    try {
      if (biometric.enabled) await api.biometricDisable(store.id);
      else await api.biometricEnable(store.id); // asks for the fingerprint to confirm
      biometric = await api.biometricState(store.id);
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }

  async function forget(target: StoreInfo) {
    if (!confirm(`Forget "${target.name}" on this device? Its notes on S3${isAndroid ? "" : " and in its folder"} are not touched; you can join it again later.`)) return;
    try {
      await api.forgetStore(target.id);
      onchange();
      if (target.id === store.id) onclose();
    } catch (e) {
      error = errorMessage(e);
    }
  }
</script>

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-label="Settings">
    <header>
      <h2>Settings</h2>
      <button onclick={onclose}>Done</button>
    </header>

    <section>
      <h3>Locking</h3>
      <label class="row">
        <span>Lock all stores after being idle for</span>
        <select value={minutes} onchange={(e) => chooseMinutes(Number(e.currentTarget.value))}>
          {#each AUTO_LOCK_CHOICES as choice (choice.minutes)}
            <option value={choice.minutes}>{choice.label}</option>
          {/each}
        </select>
      </label>
      <p class="dim">
        {isAndroid ? "The app also locks as soon as you leave it. " : ""}This is kept on this device only.
      </p>
    </section>

    <section>
      <h3>{method === "Touch ID" ? "Touch ID" : "Fingerprint"} unlock for “{store.name}”</h3>
      {#if store.passwordless}
        <p class="dim">
          “{store.name}” has no password, so there is nothing to unlock with {method}. Anyone who can read its bucket
          or folder can read the notes.
        </p>
      {:else if biometric === null}
        <p class="dim">Checking…</p>
      {:else if !biometric.available}
        <p class="dim">Not available: this device has no {method} set up.</p>
      {:else}
        <div class="row">
          <span>{biometric.enabled ? `On: unlock with ${method} instead of the password.` : `Off: unlock with the password.`}</span>
          <button onclick={toggleBiometric} disabled={busy}>{biometric.enabled ? "Turn off" : "Turn on"}</button>
        </div>
        {#if !isAndroid}
          <p class="dim">One Touch ID unlocks every store that has it turned on.</p>
        {/if}
      {/if}
    </section>

    <section class="stores">
      <h3>Stores on this device ({stores.length})</h3>
      <ul>
        {#each stores as s (s.id)}
          <li>
            <div class="info">
              <strong>{s.name}</strong>
              <span class="dim" title={s.endpoint ?? s.bucket}>
                {s.bucket}{s.endpoint ? ` · ${s.endpoint.replace(/^https?:\/\//, "")}` : ""}
              </span>
              {#if !isAndroid}<span class="dim" title={s.folder}>{s.folder}</span>{/if}
            </div>
            <button onclick={() => forget(s)}>Forget</button>
          </li>
        {/each}
      </ul>
    </section>

    {#if error}<p class="error pad">{error}</p>{/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.45);
    display: grid;
    place-items: center;
    z-index: 10;
  }
  .dialog {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 12px;
    width: min(560px, 94vw);
    max-height: 90vh;
    padding: 4px 20px 16px;
    display: flex;
    flex-direction: column;
    overflow-y: auto; /* only if even the fixed parts do not fit */
  }
  /* The stores list takes whatever height is left and scrolls inside, so the rest of the dialog stays put. */
  .stores {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 0 1 auto;
  }
  /* Two and a half rows are visible, so the half row hints that the list scrolls. Rows are a fixed height
     (long lines are cut short) so that "two and a half" stays true. */
  .stores ul {
    max-height: calc(var(--row) * 2.5);
    min-height: 0;
    overflow-y: auto;
    border-bottom: 1px solid var(--border);
    --row: 4.5rem;
  }
  header,
  section:not(.stores),
  .pad {
    flex: none;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  h2 {
    font-size: 18px;
  }
  h3 {
    margin: 18px 0 8px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--fg-dim);
  }
  p {
    margin: 6px 0 0;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .row select {
    width: auto;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    height: var(--row);
    flex: none;
    border-top: 1px solid var(--border);
  }
  .info {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-size: 13px;
    line-height: 1.4;
  }
  .info > * {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pad {
    margin-top: 12px;
  }
</style>
