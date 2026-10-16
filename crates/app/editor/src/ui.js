// Toasts, modal dialogs and popover menus. Native alert/confirm/prompt are
// avoided on purpose: a WKWebView without a UI delegate silently drops them.
//
// Motion: popovers and dialogs scale in from 0.97 at their trigger and leave
// faster than they came. When the last input was a key press, CSS turns the
// transitions off (see [data-input="key"] in styles.css).

import { h, clear } from "./util.js";
import { icon } from "./icons.js";

const EXIT_MS = 110;

// ---------------------------------------------------------------- toasts

let toastRoot = null;

export function toast(message, opts = {}) {
  const { kind = "info", title = null, mono = false, timeout, action = null } = opts;
  if (!toastRoot) toastRoot = document.body.appendChild(h("div", { class: "toasts", role: "status", "aria-live": "polite" }));
  const text = String(message ?? "").trim();
  const body = h("div", { class: "toast-body" + (mono ? " mono" : "") }, text.length > 1200 ? text.slice(0, 1200) + "…" : text);
  const el = h(
    "div",
    { class: `toast toast-${kind}` },
    icon(kind === "error" ? "warning" : kind === "success" ? "check" : "git", "toast-ic"),
    h(
      "div",
      { class: "toast-main" },
      title && h("div", { class: "toast-title" }, title),
      text && body,
      action &&
        h(
          "div",
          { class: "toast-actions" },
          h(
            "button",
            {
              class: "btn btn-sm",
              onClick: () => {
                dismiss();
                action.run();
              },
            },
            action.label,
          ),
        ),
    ),
    h("button", { class: "toast-x", title: "Dismiss", onClick: () => dismiss() }, icon("close")),
  );
  toastRoot.append(el);
  while (toastRoot.children.length > 4) toastRoot.firstChild.remove();
  el.getBoundingClientRect();
  el.classList.add("in");
  let timer = 0;
  const ms = timeout ?? (kind === "error" ? 8000 : action ? 7000 : mono ? 6000 : 3200);
  const arm = () => (timer = setTimeout(dismiss, ms));
  el.addEventListener("mouseenter", () => clearTimeout(timer));
  el.addEventListener("mouseleave", arm);
  arm();
  function dismiss() {
    clearTimeout(timer);
    el.classList.remove("in");
    el.classList.add("out");
    setTimeout(() => el.remove(), 180);
  }
  return dismiss;
}

export const toastError = (e, title) => toast(e?.message || String(e), { kind: "error", title });

// ---------------------------------------------------------------- modal

const modals = []; // open scrims, topmost last
export const isModalOpen = () => modals.length > 0;

// build(box, done) returns {submit, focus, onKey?}. onKey sees key presses
// first and returns true when it handled one. Resolves to what done() got.
export function openModal(build, { cls = "" } = {}) {
  return new Promise((resolve) => {
    closeMenu();
    const prevFocus = document.activeElement;
    const scrim = h("div", { class: "scrim" });
    modals.push(scrim);
    const box = h("div", { class: "dialog" + (cls ? " " + cls : ""), role: "dialog", "aria-modal": "true" });
    scrim.append(box);
    let finished = false;
    const done = (v) => {
      if (finished) return;
      finished = true;
      modals.splice(modals.indexOf(scrim), 1);
      scrim.classList.remove("in");
      scrim.classList.add("out");
      document.removeEventListener("keydown", onKey, true);
      setTimeout(() => scrim.remove(), EXIT_MS);
      if (prevFocus && prevFocus.focus && prevFocus.isConnected) prevFocus.focus();
      resolve(v);
    };
    const ctl = build(box, done);
    const onKey = (e) => {
      if (finished || modals[modals.length - 1] !== scrim) return; // a dialog on top of this one has the keys
      if (ctl.onKey?.(e)) return;
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        if (isMenuOpen()) closeMenu();
        else done(null);
      } else if (e.key === "Enter" && !e.shiftKey && !e.isComposing && e.target?.tagName !== "TEXTAREA") {
        e.preventDefault();
        e.stopPropagation();
        ctl.submit();
      } else if (e.key === "Tab") {
        const f = [...box.querySelectorAll("button:not(:disabled), input:not(:disabled), textarea, [tabindex='0']")].filter((x) => x.offsetParent);
        const i = f.indexOf(document.activeElement);
        e.preventDefault();
        f[(i + (e.shiftKey ? -1 : 1) + f.length) % f.length]?.focus();
      }
    };
    document.addEventListener("keydown", onKey, true);
    scrim.addEventListener("mousedown", (e) => e.target === scrim && done(null));
    document.body.append(scrim);
    scrim.getBoundingClientRect();
    scrim.classList.add("in");
    ctl.focus();
  });
}

// Resolves to "confirm", "alt" or null (cancelled).
export function confirmDialog({ title, message, items, confirm = "OK", alt, cancel = "Cancel", danger = false }) {
  return openModal((box, done) => {
    const ok = h("button", { class: "btn " + (danger ? "btn-danger" : "btn-primary"), onClick: () => done("confirm") }, confirm);
    const list = items && items.length
      ? h(
          "ul",
          { class: "dialog-list" },
          items.slice(0, 8).map((s) => h("li", null, s)),
          items.length > 8 && h("li", { class: "more" }, `and ${items.length - 8} more`),
        )
      : null;
    box.append(
      h("div", { class: "dialog-title" }, title),
      message && h("div", { class: "dialog-msg" }, message),
      list,
      h(
        "div",
        { class: "dialog-actions" },
        alt && h("button", { class: "btn btn-ghost alt", onClick: () => done("alt") }, alt),
        h("span", { class: "grow" }),
        h("button", { class: "btn", onClick: () => done(null) }, cancel),
        ok,
      ),
    );
    const submit = () => {
      const a = document.activeElement;
      if (a && a.tagName === "BUTTON" && box.contains(a)) a.click();
      else done("confirm");
    };
    return { submit, focus: () => ok.focus() };
  });
}

export function promptDialog({ title, message, placeholder = "", value = "", confirm = "OK", validate }) {
  return openModal((box, done) => {
    const input = h("input", { class: "field", placeholder, value, spellcheck: false, autocomplete: "off" });
    const err = h("div", { class: "dialog-err" });
    const ok = h("button", { class: "btn btn-primary", onClick: () => submit() }, confirm);
    const check = () => {
      const msg = validate ? validate(input.value.trim()) : null;
      err.textContent = input.value.trim() ? msg || "" : "";
      ok.disabled = !input.value.trim() || !!msg;
      return !ok.disabled;
    };
    input.addEventListener("input", check);
    const submit = () => check() && done(input.value.trim());
    check();
    box.append(
      h("div", { class: "dialog-title" }, title),
      message && h("div", { class: "dialog-msg" }, message),
      input,
      err,
      h("div", { class: "dialog-actions" }, h("span", { class: "grow" }), h("button", { class: "btn", onClick: () => done(null) }, "Cancel"), ok),
    );
    return { submit, focus: () => (input.focus(), input.select()) };
  });
}

// ---------------------------------------------------------------- popovers

let menuEl = null;
let menuAnchor = null;
let menuCleanup = null;

export function closeMenu() {
  if (!menuEl) return;
  const el = menuEl;
  menuEl = null;
  menuAnchor = null;
  menuCleanup?.();
  menuCleanup = null;
  el.classList.remove("in");
  el.classList.add("out");
  el.style.pointerEvents = "none";
  setTimeout(() => el.remove(), EXIT_MS);
}
export const isMenuOpen = () => !!menuEl;
export const menuOpenFor = (anchor) => !!menuEl && menuAnchor === anchor;

export function place(el, anchor, align = "left") {
  const r = anchor.getBoundingClientRect();
  const m = el.getBoundingClientRect();
  let left = align === "right" ? r.right - m.width : r.left;
  left = Math.max(6, Math.min(left, innerWidth - m.width - 6));
  let top = r.bottom + 4;
  let below = true;
  if (top + m.height > innerHeight - 6) {
    const above = r.top - m.height - 4;
    if (above >= 6) {
      top = above;
      below = false;
    } else top = Math.max(6, innerHeight - m.height - 6);
  }
  el.style.left = Math.round(left) + "px";
  el.style.top = Math.round(top) + "px";
  const ox = Math.max(0, Math.min(m.width, r.left + r.width / 2 - left));
  el.style.transformOrigin = `${Math.round(ox)}px ${below ? 0 : Math.round(m.height)}px`;
}

// Shows `el` as the single open popover under `anchor`. onKey gets the key
// presses (after Escape, which closes). Returns a function that re-places it.
export function popover(anchor, el, { align = "left", onKey, onClose } = {}) {
  closeMenu();
  menuEl = el;
  menuAnchor = anchor;
  el.classList.add("pop");
  document.body.append(el);
  place(el, anchor, align);
  el.classList.add("in");
  anchor.classList.add("open");
  const outside = (e) => {
    if (!el.contains(e.target) && !anchor.contains(e.target)) closeMenu();
  };
  const key = (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeMenu();
      anchor.focus?.();
    } else onKey?.(e);
  };
  const reflow = () => closeMenu();
  setTimeout(() => document.addEventListener("mousedown", outside, true));
  document.addEventListener("keydown", key, true);
  window.addEventListener("resize", reflow);
  window.addEventListener("blur", reflow);
  menuCleanup = () => {
    anchor.classList.remove("open");
    document.removeEventListener("mousedown", outside, true);
    document.removeEventListener("keydown", key, true);
    window.removeEventListener("resize", reflow);
    window.removeEventListener("blur", reflow);
    onClose?.();
  };
  return () => menuEl === el && place(el, anchor, align);
}

// items: {label, icon, hint, onSelect, disabled, danger, checked} | "-" | {header}
export function showMenu(anchor, items, { align = "left", cls = "", onClose } = {}) {
  if (menuOpenFor(anchor)) return closeMenu();
  const el = h("div", { class: "menu" + (cls ? " " + cls : ""), role: "menu" });
  const rows = [];
  for (const it of items) {
    if (!it) continue;
    if (it === "-") {
      if (el.lastChild && !el.lastChild.classList.contains("menu-sep")) el.append(h("div", { class: "menu-sep" }));
      continue;
    }
    if (it.header) {
      el.append(h("div", { class: "menu-header" }, it.header));
      continue;
    }
    const row = h(
      "button",
      {
        class: "menu-item" + (it.danger ? " danger" : ""),
        role: it.checked !== undefined ? "menuitemcheckbox" : "menuitem",
        "aria-checked": it.checked !== undefined ? String(!!it.checked) : null,
        disabled: !!it.disabled,
        title: it.title || null,
        onClick: () => {
          closeMenu();
          it.onSelect?.();
        },
        onMouseMove: () => setCur(rows.indexOf(row)),
      },
      it.icon ? icon(it.icon) : h("span", { class: "ic" }),
      h("span", { class: "menu-label" }, it.label),
      it.hint && h("span", { class: "menu-hint" }, it.hint),
      it.checked && icon("check", "menu-check"),
    );
    if (!it.disabled) rows.push(row);
    el.append(row);
  }
  if (el.lastChild?.classList.contains("menu-sep")) el.lastChild.remove();
  let cur = -1;
  const setCur = (i) => {
    rows[cur]?.classList.remove("cur");
    cur = i;
    rows[cur]?.classList.add("cur");
  };
  popover(anchor, el, {
    align,
    onClose,
    onKey: (e) => {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = rows.length;
        setCur(cur < 0 ? (e.key === "ArrowDown" ? 0 : n - 1) : (cur + (e.key === "ArrowDown" ? 1 : -1) + n) % n);
      } else if (e.key === "Enter" && cur >= 0) {
        e.preventDefault();
        rows[cur].click();
      }
    },
  });
}

// A menu at the mouse, for right-clicks.
export function showContextMenu(e, items) {
  e.preventDefault();
  const anchor = h("div", { style: `position:fixed;left:${e.clientX}px;top:${e.clientY}px;width:1px;height:1px;pointer-events:none` });
  document.body.append(anchor);
  showMenu(anchor, items, { onClose: () => setTimeout(() => anchor.remove(), EXIT_MS) });
}

// A small segmented control: options [[value, label, icon, title]].
export function segmented(options, value, onPick, cls = "") {
  return h(
    "div",
    { class: "seg seg-sm" + (cls ? " " + cls : ""), role: "radiogroup" },
    options.map(([v, label, ic, title]) =>
      h(
        "button",
        {
          class: "seg-btn" + (v === value ? " active" : "") + (label ? "" : " icon-only"),
          role: "radio",
          "aria-checked": String(v === value),
          title: title || label,
          onClick: () => v !== value && onPick(v),
        },
        ic && icon(ic),
        label || null,
      ),
    ),
  );
}
