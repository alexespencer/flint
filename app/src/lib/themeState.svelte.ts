import { applyTheme, theme, type Theme } from "./settings";

/** The theme in force, shared so the switch beside the note and the one in Settings stay in step. */
export const themeState = $state<{ value: Theme }>({ value: theme() });

export function chooseTheme(choice: Theme) {
  themeState.value = choice;
  applyTheme(choice, true);
}
