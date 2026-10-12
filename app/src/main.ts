import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";
import { applyTheme, theme } from "./lib/settings";

applyTheme(theme());

mount(App, { target: document.getElementById("app")! });
