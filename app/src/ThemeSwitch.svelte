<script lang="ts">
  import { THEME_CHOICES } from "./lib/settings";
  import { chooseTheme, themeState } from "./lib/themeState.svelte";

  const glyphs = { auto: "◐", light: "☀", dark: "☾" };
</script>

<div class="theme" role="group" aria-label="Theme">
  {#each THEME_CHOICES as choice (choice.theme)}
    <button
      class:on={themeState.value === choice.theme}
      title={choice.label}
      aria-label={choice.label}
      aria-pressed={themeState.value === choice.theme}
      onclick={() => chooseTheme(choice.theme)}
    >
      {glyphs[choice.theme]}
    </button>
  {/each}
</div>

<style>
  .theme {
    display: flex;
    gap: 1px;
    flex: none;
  }
  button {
    border-color: transparent;
    background: none;
    color: var(--fg-dim);
    font-size: 13px;
    line-height: 1;
    padding: 3px 6px;
  }
  button:hover:not(.on) {
    background: var(--bg-hover);
  }
  button.on {
    background: var(--bg-active);
    color: var(--fg);
  }
</style>
