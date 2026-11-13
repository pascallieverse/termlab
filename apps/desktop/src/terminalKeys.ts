import type { AgentId } from "./api";

/** xterm otherwise drops Shift on Enter and sends CR, which submits Claude's prompt. */
export function claudeNewlineHandler(agent: () => AgentId, send: (data: string) => void) {
  return (event: KeyboardEvent): boolean => {
    if (
      agent() !== "claude" || event.key !== "Enter" || !event.shiftKey
      || event.altKey || event.ctrlKey || event.metaKey
      || event.isComposing || event.keyCode === 229
    ) return true;

    // ESC + CR is the Option/Alt+Enter newline sequence also used by /terminal-setup.
    // Send once on keydown; suppress xterm's keypress/keyup path and browser defaults.
    if (event.type === "keydown") send("\x1b\r");
    event.preventDefault();
    event.stopPropagation();
    return false;
  };
}
