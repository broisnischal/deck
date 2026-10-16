// Git status polling and the wrapper every git action goes through.

import { api } from "./api.js";
import { S, setStatus, emit, headSignature } from "./store.js";
import { toast, toastError } from "./ui.js";

let running = null;
let again = false;
let failing = false;

// Fetch status; emits "status" only when something changed. Concurrent calls
// coalesce into one extra round so the caller always sees fresh data. When
// the branch or its ahead/behind counts move, "refs" follows.
export async function refreshStatus() {
  if (!S.git) return;
  if (running) {
    again = true;
    return running;
  }
  running = (async () => {
    do {
      again = false;
      try {
        const st = await api("status");
        failing = false;
        if (setStatus(st)) {
          const sig = headSignature(st);
          const moved = S.headSig && sig !== S.headSig;
          S.headSig = sig;
          emit("status");
          if (moved) emit("refs");
        }
      } catch (e) {
        if (!failing) toastError(e, "Couldn't read git status");
        failing = true;
      }
    } while (again);
  })();
  try {
    await running;
  } finally {
    running = null;
  }
}

let busy = null;
export const gitBusy = () => busy;

// Runs a git op with a busy label, reports output/errors as toasts and
// refreshes status afterwards. Resolves to the reply, or null on failure.
// `refs: true` tells the history and branch lists that refs may have moved.
export async function gitOp(label, op, args = {}, { output = false, success = null, after, refs = false, quiet = false } = {}) {
  if (busy) return null;
  busy = label;
  emit("busy");
  let res = null;
  let error = null;
  try {
    res = await api(op, args);
    const out = String(res?.output || "").trim();
    if (output && out) toast(out, { mono: true, title: success || label });
    else if (success) toast(success, { kind: "success" });
  } catch (e) {
    error = e;
    if (!quiet) toastError(e, `${label.replace(/…$/, "")} failed`);
    res = null;
  } finally {
    busy = null;
    emit("busy");
  }
  await refreshStatus();
  if (refs) emit("refs");
  if (after) await after(res, error);
  if (quiet && error) throw error;
  return res;
}
