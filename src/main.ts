import { createApp } from 'vue';
import '@fontsource-variable/ibm-plex-sans/wght.css';
import '@fontsource-variable/ibm-plex-sans/wght-italic.css';
import '@fontsource-variable/inter/wght.css';
import '@fontsource-variable/inter/wght-italic.css';
import '@fontsource-variable/jetbrains-mono/wght.css';
import '@fontsource-variable/jetbrains-mono/wght-italic.css';
import '@fontsource-variable/source-serif-4/wght.css';
import '@fontsource-variable/source-serif-4/wght-italic.css';
import App from './App.vue';
import { modalScrollLock } from './directives/modalScrollLock';
import { initializeAppearance } from './stores/appearance';
import { initializeVault } from './stores/vault';
import './styles/main.css';

initializeAppearance();
void initializeVault();
createApp( App )
  .directive( 'modal-scroll-lock', modalScrollLock )
  .mount( '#root' );
