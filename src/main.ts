import { createApp } from "vue";
import "@fontsource-variable/inter/wght.css";
import "@fontsource-variable/inter/wght-italic.css";
import App from "./App.vue";
import { initializeVault } from "./stores/vault";
import "./styles/main.css";

void initializeVault();
createApp(App).mount("#root");
