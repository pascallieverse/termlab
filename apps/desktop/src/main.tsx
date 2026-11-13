import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/inter/700.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/600.css";
import "./styles.css";
import { App } from "./App";
import { applyTheme, currentTheme } from "./themes";

// Apply the chosen theme before the first paint.
applyTheme(currentTheme());

async function start() {
  // In a plain browser during development, mock the daemon so the UI can be inspected.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    (await import("./devMock")).install();
  }
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

void start();
