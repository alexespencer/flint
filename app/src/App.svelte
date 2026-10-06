<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorMessage, type StoreInfo } from "./lib/api";
  import Welcome from "./Welcome.svelte";
  import Unlock from "./Unlock.svelte";
  import Workspace from "./Workspace.svelte";

  let stores = $state<StoreInfo[]>([]);
  let loaded = $state(false);
  let adding = $state(false);
  let fatal = $state("");
  /** Whether the unlock screen asks for Touch ID by itself: when the app starts, but not just after a lock. */
  let askAtOnce = $state(true);

  const active = $derived(stores.find((s) => s.active) ?? stores[0]);

  async function refresh() {
    try {
      stores = await api.listStores();
      fatal = "";
    } catch (e) {
      fatal = errorMessage(e);
    }
    loaded = true;
  }

  /** A store was just created or joined: it becomes the one that is selected (the first one already is). */
  async function added(id: string) {
    adding = false;
    await api.switchStore(id).catch(() => undefined);
    await refresh();
  }

  async function switchTo(id: string) {
    await api.switchStore(id);
    await refresh();
  }

  onMount(refresh);
</script>

{#if !loaded}
  <p class="center dim">Opening…</p>
{:else if fatal}
  <p class="center error">{fatal}</p>
{:else if adding || !active}
  <Welcome {stores} ondone={added} oncancel={() => (adding = false)} />
{:else if !active.unlocked}
  <Unlock {stores} store={active} {askAtOnce} onunlocked={refresh} onswitch={switchTo} onadd={() => (adding = true)} onchange={refresh} />
{:else}
  <!-- keyed by store: switching starts a fresh workspace, so nothing of the other store's notes lingers -->
  {#key active.id}
    <Workspace
      store={active}
      {stores}
      onchange={refresh}
      onswitch={switchTo}
      onadd={() => (adding = true)}
      onlocked={(askNext) => (askAtOnce = askNext)}
    />
  {/key}
{/if}

<style>
  .center {
    display: grid;
    place-items: center;
    height: 100%;
    margin: 0;
  }
</style>
