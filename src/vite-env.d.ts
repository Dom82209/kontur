/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Только для разработки: "tauri" включает TauriDesktopBridge вместо mock. */
  readonly VITE_KONTUR_BRIDGE?: string;
}
