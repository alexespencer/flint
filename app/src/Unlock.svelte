<script lang="ts">
  import { api, errorMessage, type StoreInfo } from "./lib/api";

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

  $effect(() => {
    store.id;
    password = "";
    error = "";
    input?.focus();
  });

  async function submit(event: Event) {
    event.preventDefault();
    busy = true;
    error = "";
    try {
      await api.unlock(store.id, password);
      password = "";
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
  .row {
    display: flex;
    justify-content: space-between;
    margin-top: 6px;
  }
</style>
