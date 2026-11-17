import { IS_MAC } from "./shortcuts";

/**
 * Turns off the web view's browser behaviour, which has no place in a desktop app: WebKit's
 * own right-click menu (Open Image in New Window, Download Image, Reload, …), dragging
 * images and links out of the window, and ⌘A selecting the whole page. Text fields and the
 * terminal keep their Cut/Copy/Paste menu, and so does text the user has selected.
 * (Which text can be selected at all is set in styles.css.)
 */
export function installDesktopBehaviour() {
  window.addEventListener("contextmenu", (e) => {
    // Termlab's own menus have already taken the event.
    if (e.defaultPrevented) return;
    if (keepsNativeMenu(e.target)) return;
    e.preventDefault();
  });

  window.addEventListener("dragstart", (e) => {
    const t = e.target;
    if (t instanceof HTMLImageElement || t instanceof HTMLAnchorElement) e.preventDefault();
  });

  window.addEventListener("keydown", (e) => {
    const mod = IS_MAC ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
    if (mod && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "a" && !isTextField(document.activeElement)) {
      e.preventDefault();
    }
  });
}

const TEXT_INPUTS = new Set(["", "text", "search", "url", "email", "password", "number", "tel"]);

function isTextField(el: Element | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) return TEXT_INPUTS.has(el.type);
  return el.isContentEditable;
}

function keepsNativeMenu(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  // The terminal's menu copies its selection and pastes into the session.
  if (isTextField(target) || target.closest(".xterm")) return true;
  const selection = window.getSelection();
  return (
    !!selection &&
    !selection.isCollapsed &&
    selection.toString().trim() !== "" &&
    selection.containsNode(target, true)
  );
}
