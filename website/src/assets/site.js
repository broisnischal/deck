// One script for every page. The site and its account server share an
// origin, so signing in sets a first-party cookie these calls can use.

const $ = (s, root = document) => root.querySelector(s);
const $$ = (s, root = document) => [...root.querySelectorAll(s)];
const json = (r) => (r.ok ? r.json() : Promise.reject(r.status));
const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
const watch = "IntersectionObserver" in window;

document.documentElement.classList.add("js");

// -- the menu on small screens ---------------------------------------------------------

const menu = $("[data-menu]");
menu?.addEventListener("click", () => {
  const open = document.body.classList.toggle("nav-open");
  menu.setAttribute("aria-expanded", String(open));
});

for (const a of $$("[data-subscribe]")) a.href = "/subscribe";

// -- theme: automatic (follows macOS), light or dark -----------------------------------

const themes = ["auto", "light", "dark"];
const themeIcons = {
  auto: '<circle cx="10" cy="10" r="7"/><path d="M10 3a7 7 0 0 1 0 14z" fill="currentColor" stroke="none"/>',
  light: '<circle cx="10" cy="10" r="3.2"/><path d="M10 2.5v2M10 15.5v2M2.5 10h2M15.5 10h2M4.7 4.7l1.4 1.4M13.9 13.9l1.4 1.4M4.7 15.3l1.4-1.4M13.9 6.1l1.4-1.4" stroke-linecap="round"/>',
  dark: '<path d="M16 12.2A6.5 6.5 0 0 1 7.8 4a6.5 6.5 0 1 0 8.2 8.2z" stroke-linejoin="round"/>',
};
const themeNow = () => document.documentElement.dataset.theme || "auto";
const showTheme = (btn) => {
  const t = themeNow();
  const label = `Theme: ${t === "auto" ? "automatic" : t}`;
  btn.setAttribute("aria-label", label);
  btn.title = label;
  $("svg", btn).innerHTML = themeIcons[t];
};
for (const btn of $$("[data-theme-toggle]")) {
  showTheme(btn);
  btn.addEventListener("click", () => {
    const next = themes[(themes.indexOf(themeNow()) + 1) % themes.length];
    if (next === "auto") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = next;
    try {
      if (next === "auto") localStorage.removeItem("theme");
      else localStorage.setItem("theme", next);
    } catch {}
    showTheme(btn);
  });
}

// -- things that move once they're on screen ------------------------------------------

const onceSeen = (els, fn, margin = "0px 0px -12% 0px") => {
  if (!watch) return els.forEach(fn);
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        io.unobserve(e.target);
        fn(e.target);
      }
    },
    { rootMargin: margin },
  );
  els.forEach((el) => io.observe(el));
};

onceSeen($$("[data-reveal]"), (el) => el.classList.add("seen"));

// The work log plays its steps in, one after another, the way a turn does.
const logs = $$("[data-play]");
if (!still) {
  for (const log of logs) log.classList.add("playing");
  onceSeen(logs, (log) => {
    const parts = [...$$("li", log), $(".worked", log), $(".reply", log)].filter(Boolean);
    parts.forEach((p, i) => setTimeout(() => p.classList.add("in"), 350 + i * 420 + (i >= parts.length - 2 ? 300 : 0)));
  });
}

// The page beside the code turns dark and back, like the switch it shows.
for (const split of $$("[data-flip]")) {
  const page = $(".page", split);
  if (!page) continue;
  if (still) {
    page.classList.add("dark");
    continue;
  }
  let timer = 0;
  const run = (on) => {
    clearInterval(timer);
    if (on) timer = setInterval(() => page.classList.toggle("dark"), 2600);
  };
  if (watch) new IntersectionObserver(([e]) => run(e.isIntersecting)).observe(split);
  else run(true);
}

// On long pages the contents mark the section you're reading.
const tocLinks = $$(".toc a[href^='#']");
if (tocLinks.length && watch) {
  const byId = new Map(tocLinks.map((a) => [a.getAttribute("href").slice(1), a]));
  const spy = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (!e.isIntersecting) continue;
        for (const a of tocLinks) a.classList.toggle("on", a === byId.get(e.target.id));
      }
    },
    { rootMargin: "-20% 0px -70% 0px" },
  );
  for (const id of byId.keys()) {
    const el = document.getElementById(id);
    if (el) spy.observe(el);
  }
}

// -- who is signed in --------------------------------------------------------------

const session = fetch("/api/auth/get-session", { credentials: "same-origin" })
  .then(json)
  .catch(() => null);

session.then((s) => {
  const link = $("[data-account-link]");
  if (link && s?.user) link.textContent = "Account";
});

// -- the price, from the plan in Dodo -------------------------------------------------

const money = (amount, currency) =>
  new Intl.NumberFormat(undefined, {
    style: "currency",
    currency,
    minimumFractionDigits: amount % 100 === 0 ? 0 : 2,
  }).format(amount / 100);

if ($("[data-price]")) {
  fetch("/v1/pricing")
    .then(json)
    .then((p) => {
      const unit = p.every > 1 ? `${p.every} ${p.interval}s` : p.interval;
      for (const el of $$("[data-price]")) el.textContent = money(p.amount, p.currency);
      for (const el of $$("[data-per]")) el.textContent = `/ ${unit}`;
      for (const el of $$("[data-trial]")) {
        el.textContent = `The first ${p.trial_days} days are free, no card needed. Cancel any time.`;
      }
    })
    .catch(() => {
      for (const el of $$("[data-price-block]")) el.hidden = true;
      for (const el of $$("[data-trial]")) {
        el.textContent = "The first 14 days are free, no card needed. You'll see the price before you pay.";
      }
    });
}

// -- the download, from the newest release ----------------------------------------------

if ($("[data-download]")) {
  fetch("/v1/releases/latest")
    .then(json)
    .then((rel) => {
      const file = rel.dmg || rel.download;
      if (!file) throw new Error("no download yet");
      for (const a of $$("[data-download]")) a.href = file;
      for (const el of $$("[data-version-tag]")) el.textContent = `v${rel.version}`;
      for (const el of $$("[data-version]")) el.textContent = "Ready to download";
      for (const el of $$("[data-file]")) el.textContent = `${decodeURIComponent(file.split("/").pop())} · Apple silicon and Intel`;
    })
    .catch(() => {
      // Buttons that say "try it" keep pointing at the download page, which
      // explains; the download itself says it's coming.
      for (const a of $$("[data-download]:not([data-soft])")) {
        a.setAttribute("aria-disabled", "true");
        a.removeAttribute("href");
        const label = $("[data-download-label]", a);
        if (label) label.textContent = "Coming soon";
      }
      for (const el of $$("[data-version]")) el.textContent = "The first public version is on its way.";
    });
}

// -- ⌘K or /: go anywhere --------------------------------------------------------------

const palette = $("[data-palette]");
if (palette) {
  const input = $("[data-palette-input]", palette);
  const list = $("[data-palette-list]", palette);
  const items = [
    ["Home", "/", "Page"],
    ["Features", "/#features", "Home"],
    ["Your code stays on your Mac", "/#private", "Home"],
    ["Native, all the way down", "/#native", "Home"],
    ["Questions", "/#faq", "Home"],
    ["Everything Deck does", "/features/", "Page"],
    ["Chats", "/features/#chat", "Features"],
    ["Approvals and alerts", "/features/#approvals", "Features"],
    ["Undo a turn", "/features/#undo", "Features"],
    ["Prompt rewriting", "/features/#rewrite", "Features"],
    ["Tasks", "/features/#tasks", "Features"],
    ["Terminals", "/features/#terminal", "Features"],
    ["Browser", "/features/#browser", "Features"],
    ["Snippets", "/features/#snippets", "Features"],
    ["The workspace", "/features/#workspace", "Features"],
    ["Pricing", "/pricing/", "Page"],
    ["Subscribe", "/subscribe", "Action"],
    ["Download", "/download/", "Page"],
    ["Docs", "/docs/", "Page"],
    ["Getting started", "/docs/#start", "Docs"],
    ["The deck command", "/docs/#cli", "Docs"],
    ["Shortcuts", "/docs/#shortcuts", "Docs"],
    ["Troubleshooting", "/docs/#help", "Docs"],
    ["Changelog", "/changelog/", "Page"],
    ["Your account", "/account/", "Page"],
    ["Privacy Policy", "/privacy/", "Legal"],
    ["Terms of Service", "/terms/", "Legal"],
    ["Refund Policy", "/refunds/", "Legal"],
  ];
  let shown = [];
  let at = 0;

  const draw = () => {
    const q = input.value.trim().toLowerCase();
    shown = items.filter(([name, , kind]) => !q || name.toLowerCase().includes(q) || kind.toLowerCase().includes(q));
    at = Math.min(at, Math.max(0, shown.length - 1));
    list.replaceChildren(
      ...shown.map(([name, href, kind], i) => {
        const li = document.createElement("li");
        li.setAttribute("role", "option");
        li.className = i === at ? "on" : "";
        li.innerHTML = "<span></span><small></small>";
        li.firstChild.textContent = name;
        li.lastChild.textContent = kind;
        li.addEventListener("click", () => go(href));
        li.addEventListener("mousemove", () => {
          if (at !== i) {
            at = i;
            draw();
          }
        });
        return li;
      }),
    );
    if (!shown.length) {
      const li = document.createElement("li");
      li.textContent = `Nothing called “${input.value.trim()}”`;
      list.replaceChildren(li);
    }
    list.children[at]?.scrollIntoView({ block: "nearest" });
  };
  const open = () => {
    palette.hidden = false;
    input.value = "";
    at = 0;
    draw();
    input.focus();
  };
  const close = () => (palette.hidden = true);
  const go = (href) => {
    close();
    location.href = href;
  };

  input.addEventListener("input", () => {
    at = 0;
    draw();
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") { at = (at + 1) % Math.max(1, shown.length); draw(); e.preventDefault(); }
    else if (e.key === "ArrowUp") { at = (at - 1 + shown.length) % Math.max(1, shown.length); draw(); e.preventDefault(); }
    else if (e.key === "Enter" && shown[at]) go(shown[at][1]);
    else if (e.key === "Escape") close();
  });
  palette.addEventListener("click", (e) => e.target === palette && close());
  for (const b of $$("[data-palette-open]")) b.addEventListener("click", open);
  addEventListener("keydown", (e) => {
    const typing = /input|textarea/i.test(document.activeElement?.tagName ?? "");
    if ((e.key === "k" && (e.metaKey || e.ctrlKey)) || (e.key === "/" && !typing)) {
      e.preventDefault();
      palette.hidden ? open() : close();
    }
  });
}

// -- 404: say what wasn't found ------------------------------------------------------------

for (const el of $$("[data-missing]")) el.textContent = location.pathname;

// -- the account page ------------------------------------------------------------------------

const account = $("[data-account]");
if (account) {
  const show = (name) => {
    for (const el of $$("[data-state]", account)) el.hidden = el.dataset.state !== name;
  };
  const error = (msg) => {
    for (const el of $$("[data-error]", account)) {
      el.textContent = msg;
      el.hidden = !msg;
    }
  };

  const signIn = async () => {
    error("");
    try {
      const r = await fetch("/api/auth/sign-in/social", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify({ provider: "google", callbackURL: "/account/" }),
      });
      const data = await r.json();
      if (!data.url) throw new Error(data.message || "no sign-in page");
      location.href = data.url;
    } catch (e) {
      error(`Couldn't start sign-in: ${e.message}`);
    }
  };

  const openPortal = async () => {
    error("");
    try {
      const r = await fetch("/api/auth/dodopayments/customer/portal", { credentials: "same-origin" });
      const data = await r.json();
      if (!data.url) throw new Error(data.message || "no billing page");
      location.href = data.url;
    } catch (e) {
      error(`Couldn't open billing: ${e.message}`);
    }
  };

  const signOut = async () => {
    await fetch("/api/auth/sign-out", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      credentials: "same-origin",
      body: "{}",
    }).catch(() => {});
    location.reload();
  };

  $("[data-sign-in]", account)?.addEventListener("click", signIn);
  $("[data-portal]", account)?.addEventListener("click", openPortal);
  $("[data-sign-out]", account)?.addEventListener("click", signOut);

  session.then(async (s) => {
    if (!s?.user) return show("out");
    const u = s.user;
    $("[data-name]", account).textContent = u.name || u.email;
    $("[data-email]", account).textContent = u.email;
    const avatar = $("[data-avatar]", account);
    if (u.image) {
      const img = new Image();
      img.alt = "";
      img.referrerPolicy = "no-referrer";
      img.src = u.image;
      avatar.replaceChildren(img);
    } else {
      avatar.textContent = (u.name || u.email || "?").trim()[0].toUpperCase();
    }
    show("in");
    try {
      const me = await fetch("/v1/me", { credentials: "same-origin" }).then(json);
      const day = (d) => d.toLocaleDateString(undefined, { dateStyle: "long" });
      const trial = me.trial_ends ? new Date(me.trial_ends * 1000) : null;
      const left = trial ? Math.ceil((trial - Date.now()) / 86_400_000) : 0;
      $("[data-plan]", account).innerHTML =
        trial && left > 0
          ? `<span class="status-dot on"></span>Free trial, ${left === 1 ? "last day" : `${left} days left`}`
          : trial
            ? `<span class="status-dot"></span>Free trial ended`
            : me.active
              ? `<span class="status-dot on"></span>${me.plan ?? "Pro"}, active`
              : `<span class="status-dot"></span>No plan yet`;
      $("[data-renews-label]", account).textContent = trial ? "Trial ends" : "Renews";
      $("[data-renews]", account).textContent = trial ? day(trial) : me.renews ? day(new Date(me.renews)) : "—";
      // Billing is there once you pay; until then, the way to pay.
      const paid = me.active && !trial;
      $("[data-portal]", account).hidden = !paid;
      $("[data-choose]", account).hidden = paid;
    } catch {
      $("[data-plan]", account).textContent = "Couldn't load your plan. Refresh to try again.";
    }
  });
}
