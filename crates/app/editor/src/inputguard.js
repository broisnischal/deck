// WebKit sometimes delivers navigation keys as text: U+001C..U+001F are the
// classic Mac character codes for ← → ↑ ↓ (and other C0 controls can leak
// the same way). Such "typing" must never reach a document. In the editor a
// lone arrow code becomes the cursor move it stood for; in plain text fields
// the characters are dropped.

import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import {
  cursorCharLeft,
  cursorCharRight,
  cursorLineUp,
  cursorLineDown,
  selectCharLeft,
  selectCharRight,
  selectLineUp,
  selectLineDown,
} from "@codemirror/commands";

// C0 controls except tab, newline and carriage return, plus DEL.
export const CONTROL = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/;
const CONTROL_G = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g;

const MOVES = {
  "\x1c": [cursorCharLeft, selectCharLeft],
  "\x1d": [cursorCharRight, selectCharRight],
  "\x1e": [cursorLineUp, selectLineUp],
  "\x1f": [cursorLineDown, selectLineDown],
};

let shift = false;

const inputHandler = EditorView.inputHandler.of((view, from, to, text) => {
  if (!CONTROL.test(text)) return false;
  const move = MOVES[text];
  if (move) {
    move[shift ? 1 : 0](view);
    return true;
  }
  const clean = text.replace(CONTROL_G, "");
  if (clean && !view.state.readOnly)
    view.dispatch({ changes: { from, to, insert: clean }, selection: { anchor: from + clean.length }, userEvent: "input.type", scrollIntoView: true });
  return true;
});

// Anything typed that still carries control characters (another input path)
// is cleaned before it lands. Pastes are left alone.
const filter = EditorState.transactionFilter.of((tr) => {
  if (!tr.docChanged || !tr.isUserEvent("input.type")) return tr;
  let bad = false;
  tr.changes.iterChanges((_fa, _ta, _fb, _tb, ins) => {
    if (!bad && CONTROL.test(ins.toString())) bad = true;
  });
  if (!bad) return tr;
  const changes = [];
  tr.changes.iterChanges((fromA, toA, _fb, _tb, ins) => changes.push({ from: fromA, to: toA, insert: ins.toString().replace(CONTROL_G, "") }));
  return { changes, userEvent: "input.type", scrollIntoView: true };
});

const shiftTracker = EditorView.domEventHandlers({
  keydown(e) {
    shift = e.shiftKey;
    return false;
  },
  keyup(e) {
    shift = e.shiftKey;
    return false;
  },
});

export const controlGuard = [inputHandler, filter, shiftTracker];

// Plain <input> and <textarea>: strip control characters as they arrive.
export function guardTextFields(root = document) {
  root.addEventListener(
    "input",
    (e) => {
      const el = e.target;
      if (!el || (el.tagName !== "TEXTAREA" && el.tagName !== "INPUT") || el.closest?.(".cm-editor")) return;
      const v = el.value;
      if (!CONTROL.test(v)) return;
      let caret = null;
      try {
        caret = el.selectionStart;
      } catch {}
      const before = caret == null ? 0 : v.slice(0, caret).replace(CONTROL_G, "").length;
      el.value = v.replace(CONTROL_G, "");
      try {
        if (caret != null) el.setSelectionRange(before, before);
      } catch {}
    },
    true,
  );
}
