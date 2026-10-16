// The few pages the browser shows during sign-in and checkout: plain HTML,
// one script each, no build step.

const shell = (title: string, body: string, script = "") => `<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${title}</title>
<style>
  :root { color-scheme: light dark; --fg: #111; --dim: #666; --bg: #fafafa; --card: #fff; --line: #e5e5e5; }
  @media (prefers-color-scheme: dark) { :root { --fg: #f2f2f2; --dim: #9a9a9a; --bg: #0b0b0b; --card: #161616; --line: #2a2a2a; } }
  body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: var(--bg); color: var(--fg);
         font: 15px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
  main { width: min(380px, calc(100vw - 32px)); padding: 32px; background: var(--card); border: 1px solid var(--line);
         border-radius: 16px; text-align: center; }
  .logo { width: 56px; height: 56px; margin: 0 auto 16px; border-radius: 14px; background: var(--fg); color: var(--bg);
          display: grid; place-items: center; font-weight: 700; font-size: 26px; }
  h1 { font-size: 20px; margin: 0 0 6px; }
  p { color: var(--dim); margin: 0 0 20px; }
  button, a.button { display: block; width: 100%; box-sizing: border-box; padding: 11px 16px; border-radius: 10px; border: 0;
         background: var(--fg); color: var(--bg); font: inherit; font-weight: 600; cursor: pointer; text-decoration: none; }
  button.secondary, a.button.secondary { background: transparent; color: var(--fg); border: 1px solid var(--line); margin-top: 8px; }
  button:disabled { opacity: .6; cursor: default; }
  .error { color: #e5534b; margin-top: 14px; }
</style></head>
<body><main><div class="logo">D</div>${body}</main>${script ? `<script>${script}</script>` : ""}</body></html>`;

const esc = (s: string) =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

/** Starts Google sign-in from the browser, so its state cookie lands here,
 *  and comes back to `done` after. */
export function signIn(done: string) {
  return shell(
    "Sign in to Deck",
    `<h1>Sign in to Deck</h1><p>Continue with your Google account.</p>
     <button id="go">Continue with Google</button><div class="error" id="err"></div>`,
    `const go = document.getElementById("go"), err = document.getElementById("err");
     async function start() {
       go.disabled = true;
       try {
         const r = await fetch("/api/auth/sign-in/social", {
           method: "POST", headers: { "Content-Type": "application/json" }, credentials: "include",
           body: JSON.stringify({ provider: "google", callbackURL: ${JSON.stringify(done)},
                                  errorCallbackURL: "/desktop/failed" }) });
         const data = await r.json();
         if (!data.url) throw new Error(data.message || "no sign-in page");
         location.href = data.url;
       } catch (e) { err.textContent = "Couldn't start sign-in: " + e.message; go.disabled = false; }
     }
     go.onclick = start; start();`,
  );
}

/** Hands the browser back to the app. */
export function backToApp(link: string) {
  return shell(
    "Back to Deck",
    `<h1>You're signed in</h1><p>Deck should open by itself. If it doesn't, use the button.</p>
     <a class="button" href="${esc(link)}">Open Deck</a>`,
    `location.href = ${JSON.stringify(link)};`,
  );
}

export function failed(message: string) {
  return shell("Sign-in failed", `<h1>That didn't work</h1><p>${esc(message)}</p>
    <a class="button" href="deck://auth/callback?error=${encodeURIComponent(message)}">Back to Deck</a>`);
}

/** Starts Dodo Payments checkout for the Pro plan, from the signed-in
 *  browser; right away when `now` (coming from the website's Subscribe). */
export function checkout(now = false) {
  return shell(
    "Deck Pro",
    `<h1>Deck Pro</h1><p>Terminals, a browser and every coding agent in one window.</p>
     <button id="go">Subscribe</button><div class="error" id="err"></div>`,
    `const go = document.getElementById("go"), err = document.getElementById("err");
     const start = async () => {
       go.disabled = true;
       try {
         const r = await fetch("/api/auth/dodopayments/checkout-session", {
           method: "POST", headers: { "Content-Type": "application/json" }, credentials: "include",
           body: JSON.stringify({ slug: "pro" }) });
         if (r.status === 401) { err.textContent = "Sign in from Deck first, then come back here."; go.disabled = false; return; }
         const data = await r.json();
         if (!data.url) throw new Error(data.message || "no checkout page");
         location.href = data.url;
       } catch (e) { err.textContent = "Couldn't open checkout: " + e.message; go.disabled = false; }
     };
     go.onclick = start;${now ? " start();" : ""}`,
  );
}

/** Opens the Dodo Payments customer portal for the signed-in browser. */
export function billing() {
  return shell(
    "Manage your plan",
    `<h1>Manage your plan</h1><p>Opening your billing page…</p><div class="error" id="err"></div>`,
    `fetch("/api/auth/dodopayments/customer/portal", { credentials: "include" })
     .then(r => { if (r.status === 401) throw new Error("sign in from Deck first"); return r.json(); })
     .then(d => { if (d.url) location.href = d.url; else throw new Error(d.message || "no portal"); })
     .catch(e => document.getElementById("err").textContent = "Couldn't open billing: " + e.message);`,
  );
}

export function paid() {
  return shell(
    "Thank you",
    `<h1>You're all set</h1><p>Your plan is active. If Deck is open, it picks this up in a moment.</p>
     <a class="button" href="deck://refresh">Open Deck</a>
     <a class="button secondary" href="/account/" style="margin-top:8px">Your account</a>`,
  );
}
