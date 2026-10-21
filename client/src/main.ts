import { mount } from "svelte";

import App from "./App.svelte";
import "normalize.css";
import "./global.sass";

const target = document.getElementById("app");

if (!target) {
  throw new Error("App target was not found");
}

mount(App, { target });
