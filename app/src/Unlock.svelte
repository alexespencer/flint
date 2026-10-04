<script lang="ts">
  import { api, errorKind, errorMessage, type BiometricState, type StoreInfo } from "./lib/api";

  let {
    stores,
    store,
    onunlocked,
    onswitch,
    onadd,
    onchange,
  }: {
    stores: StoreInfo[];
    store: StoreInfo;
    onunlocked: () => void;
    onswitch: (id: string) => void;
    onadd: () => void;
    onchange: () => void;
  } = $props();

  let password = $state("");
  let busy = $state(false);
  let error = $state("");
  let input: HTMLInputElement | undefined = $state();
  let biometric = $state<BiometricState>({ available: false, enabled: false });
  let rememberFingerprint = $state(true);

  $effect(() => {
    const id = store.id;
    password = "";
    error = "";
    biometric = { available: false, enabled: false };
    api.biometricState(id).then((state) => {
      if (store.id !== id) return;
      biometric = state;
      if (state.enabled) fingerprint(); // ask straight away; the password is still there as a fallback
      else input?.focus();
    }, () => input?.focus());
  });

  async function fingerprint() {
    busy = true;
    error = "";
    try {
      await api.biometricUnlock(store.id);
      onunlocked();
    } catch (e) {
      const kind = errorKind(e);
      if (kind === "keyInvalidated") {
        biometric = { ...biometric, enabled: false };
        error = "Your fingerprints have changed, so enter your password. You can turn fingerprint unlock on again afterwards.";
      } else if (kind !== "biometricCancelled") {
        error = errorMessage(e);
      }
      input?.focus();
    } finally {
      busy = false;
    }
  }

  async function submit(event: Event) {
    event.preventDefault();
    busy = true;
    error = "";
    try {
      await api.unlock(store.id, password);
      password = "";
      if (biometric.available && !biometric.enabled && rememberFingerprint) {
        // wraps the key under the fingerprint-gated hardware key; declining the prompt just skips it
        await api.biometricEnable(store.id).catch(() => undefined);
      }
      onunlocked();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }

  async function forget() {
    if (confirm(`Forget "${store.name}" on this device? Its folder and its notes on S3 are not touched.`)) {
      await api.forgetStore(store.id);
      onchange();
    }
  }
</script>

<main>
  <form onsubmit={submit}>
    <h1>Flint</h1>
    {#if stores.length > 1}
      <select value={store.id} onchange={(e) => onswitch(e.currentTarget.value)} aria-label="Store">
        {#each stores as s (s.id)}
          <option value={s.id}>{s.name}</option>
        {/each}
      </select>
    {:else}
      <p class="name">{store.name}</p>
    {/if}
    <p class="dim">{store.folder}</p>
    <input
      type="password"
      placeholder="Password"
      autocomplete="current-password"
      bind:value={password}
      bind:this={input}
      disabled={busy}
    />
    {#if error}<p class="error">{error}</p>{/if}
    <button class="primary" disabled={busy || !password}>{busy ? "Unlocking…" : "Unlock"}</button>
    {#if biometric.enabled}
      <button type="button" onclick={fingerprint} disabled={busy}>Use fingerprint</button>
    {:else if biometric.available}
      <label class="check">
        <input type="checkbox" bind:checked={rememberFingerprint} /> Unlock with my fingerprint next time
      </label>
    {/if}
    <div class="row">
      <button type="button" class="link" onclick={onadd}>Add another store</button>
      <button type="button" class="link" onclick={forget}>Forget this store</button>
    </div>
  </form>
</main>

<style>
  main {
    display: grid;
    place-items: center;
    height: 100%;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 320px;
  }
  h1 {
    margin: 0 0 6px;
    font-size: 22px;
  }
  .name {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  p {
    margin: 0;
    word-break: break-all;
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
    color: var(--fg-dim);
  }
  .check input {
    width: auto;
  }
  .row {
    display: flex;
    justify-content: space-between;
    margin-top: 6px;
  }
</style>
