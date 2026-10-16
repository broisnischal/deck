// Deck's account server, a Cloudflare Worker: Google sign-in (Better Auth),
// the Pro plan (Dodo Payments), data in D1, and what the desktop app asks:
// who am I, may I run, is there a newer Deck.
//
// The app signs in through the browser (Google refuses embedded web views):
//   1. Deck opens /desktop/sign-in?state=S in the default browser.
//   2. That page starts Google sign-in; Better Auth lands on /desktop/done.
//   3. /desktop/done mints a one-time code and opens deck://auth/callback.
//   4. Deck POSTs the code to /desktop/token and keeps the session token it
//      gets, sending it as a Bearer token from then on.

import { env } from "cloudflare:workers";
import { Hono } from "hono";
import { cors } from "hono/cors";
import { auth, dodo } from "./auth";
import * as pages from "./pages";

const app = new Hono();

app.get("/health", (c) => c.text("ok"));

app.on(["POST", "GET"], "/api/auth/*", (c) => auth.handler(c.req.raw));

// -- the desktop sign-in --------------------------------------------------------

const STATE = /^[0-9a-f]{16,128}$/;
const CODE_TTL_MS = 2 * 60 * 1000;

function randomHex(bytes: number) {
  const b = crypto.getRandomValues(new Uint8Array(bytes));
  return [...b].map((x) => x.toString(16).padStart(2, "0")).join("");
}

app.get("/desktop/sign-in", (c) => {
  const state = c.req.query("state") ?? "";
  if (!STATE.test(state)) return c.text("Open this page from Deck.", 400);
  return c.html(pages.signIn(`/desktop/done?state=${state}`));
});

/** The website's Subscribe: sign in with Google if needed, then checkout. */
app.get("/subscribe", async (c) => {
  const session = await auth.api.getSession({ headers: c.req.raw.headers });
  if (!session) return c.html(pages.signIn("/subscribe"));
  return c.html(pages.checkout(true));
});

app.get("/desktop/done", async (c) => {
  const state = c.req.query("state") ?? "";
  if (!STATE.test(state)) return c.text("Open this page from Deck.", 400);
  const session = await auth.api.getSession({ headers: c.req.raw.headers });
  if (!session) return c.redirect(`/desktop/sign-in?state=${state}`);
  const code = randomHex(32);
  const now = Date.now();
  await env.DB.batch([
    // Old codes go as new ones come.
    env.DB.prepare(`DELETE FROM desktop_code WHERE expiresAt < ?`).bind(now),
    env.DB.prepare(`INSERT INTO desktop_code (code, token, expiresAt) VALUES (?, ?, ?)`).bind(
      code,
      session.session.token,
      now + CODE_TTL_MS,
    ),
  ]);
  return c.html(pages.backToApp(`deck://auth/callback?code=${code}&state=${state}`));
});

app.get("/desktop/failed", (c) =>
  c.html(pages.failed(c.req.query("error")?.replace(/_/g, " ") ?? "Sign-in was cancelled.")),
);

/** One-time code → session token. Each code works once, for two minutes. */
app.post("/desktop/token", async (c) => {
  const body = await c.req.json<{ code?: unknown }>().catch(() => ({}) as { code?: unknown });
  const code = typeof body.code === "string" ? body.code : "";
  if (!/^[0-9a-f]{64}$/.test(code)) return c.json({ error: "bad code" }, 400);
  const row = await env.DB.prepare(
    `DELETE FROM desktop_code WHERE code = ? AND expiresAt > ? RETURNING token`,
  )
    .bind(code, Date.now())
    .first<{ token: string }>();
  if (!row) return c.json({ error: "this sign-in expired; try again" }, 400);
  return c.json({ token: row.token });
});

app.get("/desktop/checkout", (c) => c.html(pages.checkout()));
app.get("/desktop/billing", (c) => c.html(pages.billing()));
app.get("/desktop/paid", (c) => c.html(pages.paid()));

// -- what the app and the website ask ----------------------------------------------

// The website (another origin) reads the price and the newest release.
app.use("/v1/pricing", cors());
app.use("/v1/releases/latest", cors());

type Pricing = {
  name: string;
  amount: number;
  currency: string;
  interval: string;
  every: number;
  trial_days: number;
};
let pricing: { at: number; value: Pricing } | undefined;

/** The Pro plan's price, from Dodo, kept for ten minutes per isolate. */
app.get("/v1/pricing", async (c) => {
  if (pricing && Date.now() - pricing.at < 10 * 60 * 1000) return c.json(pricing.value);
  try {
    const product = await dodo.products.retrieve(env.DODO_PRODUCT_PRO);
    const price = product.price;
    if (price.type !== "recurring_price") return c.json({ error: "the plan is not a subscription" }, 502);
    const value: Pricing = {
      name: product.name ?? "Deck Pro",
      amount: price.price,
      currency: price.currency,
      interval: String(price.payment_frequency_interval).toLowerCase(),
      every: price.payment_frequency_count,
      trial_days: trialDays(),
    };
    pricing = { at: Date.now(), value };
    c.header("Cache-Control", "public, max-age=600");
    return c.json(value);
  } catch (e) {
    return c.json({ error: e instanceof Error ? e.message : "payments did not answer" }, 502);
  }
});

type DodoSubscription = {
  status: string;
  product_id: string;
  next_billing_date?: string | null;
  trial_period_days?: number | null;
};

/** Every account starts with this many days of Deck, no card asked. */
const trialDays = () => Number(env.DECK_TRIAL_DAYS) || 14;

/** The signed-in user and whether Deck runs for them: a paid plan, or the
 *  free trial that starts when the account is made. Dodo is asked each time
 *  (through the plugin's own route), so there is no copy of the plan here to
 *  drift. */
app.get("/v1/me", async (c) => {
  const headers = c.req.raw.headers;
  const session = await auth.api.getSession({ headers });
  if (!session) return c.json({ error: "not signed in" }, 401);
  const { user } = session;
  const trialEnds = new Date(user.createdAt).getTime() + trialDays() * 86_400_000;
  const onTrial = Date.now() < trialEnds;
  const who = { email: user.email, name: user.name ?? "", image: user.image ?? null };
  const trial = {
    ...who,
    plan: onTrial ? "Trial" : null,
    active: onTrial,
    status: onTrial ? "trialing" : null,
    renews: null,
    trial_ends: Math.floor(trialEnds / 1000),
  };

  const res = await auth.handler(
    new Request(
      `${env.BETTER_AUTH_URL}/api/auth/dodopayments/customer/subscriptions/list?status=active`,
      { headers },
    ),
  );
  if (!res.ok) {
    // The trial doesn't need payments to answer. Past it, say so rather
    // than "no plan", so the app keeps its last answer.
    if (onTrial) return c.json(trial);
    return c.json({ error: `payments answered ${res.status}` }, 502);
  }
  const { items = [] } = (await res.json()) as { items?: DodoSubscription[] };
  const live = items.find((s) => s.status === "active");
  if (!live) return c.json(trial);
  return c.json({
    ...who,
    plan: live.product_id === env.DODO_PRODUCT_PRO ? "Pro" : "Paid",
    active: true,
    status: live.status,
    renews: live.next_billing_date ?? null,
    trial_ends: null,
  });
});

type Release = { version: string; download: string | null; dmg: string | null; notes_url: string };

/** The newest Deck: from the Worker's vars, else GitHub's latest release
 *  (scripts/release.sh --publish puts a .dmg and a .zip on it). */
async function latest(): Promise<Release | "none" | "unreachable"> {
  if (env.DECK_LATEST_VERSION) {
    return {
      version: env.DECK_LATEST_VERSION,
      download: env.DECK_DOWNLOAD_URL || null,
      dmg: env.DECK_DMG_URL || null,
      notes_url: env.DECK_NOTES_URL || "",
    };
  }
  const r = await fetch("https://api.github.com/repos/broisnischal/deck/releases/latest", {
    headers: { Accept: "application/vnd.github+json", "User-Agent": "deck-backend" },
    cf: { cacheTtl: 300, cacheEverything: true },
  });
  if (r.status === 404) return "none";
  if (!r.ok) return "unreachable";
  const gh = (await r.json()) as {
    tag_name: string;
    html_url: string;
    assets?: { name: string; browser_download_url: string }[];
  };
  const asset = (ext: string) => gh.assets?.find((a) => a.name.endsWith(ext))?.browser_download_url ?? null;
  return {
    version: gh.tag_name.replace(/^v/, ""),
    // The zip is what the app's updater swaps in; the disk image is for people.
    download: asset(".zip"),
    dmg: asset(".dmg"),
    notes_url: gh.html_url,
  };
}

app.get("/v1/releases/latest", async (c) => {
  const rel = await latest();
  if (rel === "none") return c.json({ error: "no release yet" }, 404);
  if (rel === "unreachable") return c.json({ error: "github is not answering" }, 502);
  return c.json(rel);
});

/** A link that always downloads the newest Deck; the download page until there is one. */
app.get("/download/mac", async (c) => {
  const rel = await latest();
  const to = typeof rel === "object" ? rel.dmg ?? rel.download : null;
  return c.redirect(to ?? "/download/", 302);
});

export default app;
