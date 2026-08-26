/// <reference types="vite/client" />

declare module "*.vue" {
  import type { DefineComponent } from "vue";
  const component: DefineComponent<Record<string, never>, Record<string, never>, unknown>;
  export default component;
}

interface Window {
  __TAURI__?: {
    core: {
      invoke<T>(
        command: string,
        args?: Record<string, unknown> | number[] | ArrayBuffer | Uint8Array,
        options?: { headers: Record<string, string> },
      ): Promise<T>;
    };
  };
}
