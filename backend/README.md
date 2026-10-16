# Deck backend

The website (`../website`, built into `../website/public`) and the account server run as one Cloudflare Worker named `deck`, so sign-in cookies belong to the site itself. The server's routes are `/api/*`, `/v1/*`, `/desktop/*`, `/subscribe` and `/health`; everything else is a static page. `npm run deploy` builds the site and deploys both.

Accounts, Google sign-in and the paid plan for the Deck app. It runs as a Cloudflare Worker, built with [Hono](https://hono.dev), [Better Auth](https://better-auth.com) and [Dodo Payments](https://dodopayments.com), and keeps its data in Cloudflare D1.

A Deck build made with `DECK_API` pointing here asks people to sign in before it runs, and keeps running while their plan is active. A build without `DECK_API` (one you make yourself) asks for nothing.

## How the app signs in

Google refuses sign-in inside embedded web views, so Deck goes through the browser:

1. Deck opens `/desktop/sign-in?state=…` in the default browser.
2. That page starts Google sign-in, and Better Auth brings the browser back to `/desktop/done`.
3. `/desktop/done` makes a one-time code (in D1, valid for two minutes) and opens `deck://auth/callback?code=…&state=…`.
4. Deck checks the state and posts the code to `/desktop/token`. It keeps the session token it gets back in the Keychain and sends it as `Authorization: Bearer …` from then on.
5. Deck asks `/v1/me` who you are and whether your plan is active. `/v1/me` asks Dodo each time, so there's no copy of the plan here to go stale. Deck asks again every six hours and keeps working for up to a week on its last answer if it can't reach the server.

**Choose a Plan** opens `/desktop/checkout`, which starts Dodo checkout. **Manage Plan** opens `/desktop/billing`, which is Dodo's customer portal. After paying, the browser opens `deck://refresh` so the app picks up the change.

## Endpoints

| | |
| --- | --- |
| `GET/POST /api/auth/*` | Better Auth: Google, sessions, and Dodo's `/dodopayments/checkout-session`, `/dodopayments/customer/portal`, `/dodopayments/customer/subscriptions/list` |
| `GET /desktop/sign-in?state=` | starts Google sign-in in the browser |
| `GET /desktop/done?state=` | makes the one-time code and hands the browser back to Deck |
| `POST /desktop/token` `{code}` | one-time code → `{token}` |
| `GET /desktop/checkout`, `/desktop/billing`, `/desktop/paid` | plan pages |
| `GET /v1/me` (Bearer) | `{email, name, image, plan, active, status, renews}` |
| `GET /v1/releases/latest` | `{version, download, notes_url}` for the app's update check (CORS open, for the website) |
| `GET /v1/pricing` | `{name, amount, currency, interval, every, trial_days}` from the Dodo product, kept 10 minutes (CORS open) |
| `GET /subscribe` | the website's Subscribe: Google sign-in if needed, then Dodo checkout |
| `GET /health` | `ok` |

## Deployed

Live at **https://deck.stroke.click** on the broisnees account, with D1 database `deck` (`71ce4efe-…`). Both migrations are applied, and `BETTER_AUTH_SECRET` is set.

Still placeholders: `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `DODO_PAYMENTS_API_KEY` (all `not-set`) and `DODO_PRODUCT_PRO`. Sign-in and checkout start working once these are filled in:

```sh
npx wrangler secret put GOOGLE_CLIENT_ID
npx wrangler secret put GOOGLE_CLIENT_SECRET
npx wrangler secret put DODO_PAYMENTS_API_KEY
# set DODO_PRODUCT_PRO in wrangler.jsonc, then:
npm run deploy
```

The Google redirect URI to register is `https://deck.stroke.click/api/auth/callback/google`.

## Set up from scratch

```sh
cd backend
npm install
npx wrangler login
npx wrangler d1 create deck        # put the database_id it prints into wrangler.jsonc
npm run migrate                    # Better Auth's tables and desktop_code, in D1
```

In `wrangler.jsonc`, set `BETTER_AUTH_URL` to the Worker's address and `DODO_PRODUCT_PRO` to your product. Then store the secrets:

```sh
npx wrangler secret put BETTER_AUTH_SECRET      # openssl rand -base64 32
npx wrangler secret put GOOGLE_CLIENT_ID
npx wrangler secret put GOOGLE_CLIENT_SECRET
npx wrangler secret put DODO_PAYMENTS_API_KEY
npm run deploy
```

Ship Deck with the Worker's address:

```sh
DECK_API=https://deck.stroke.click scripts/install.sh
```

### Google

In Google Cloud Console, go to **APIs & Services › Credentials › Create credentials › OAuth client ID**, choose **Web application**, and add this authorized redirect URI:

```
${BETTER_AUTH_URL}/api/auth/callback/google
```

### Dodo Payments

1. In the Dodo dashboard, create a subscription product (Deck Pro, monthly). Put its ID in `DODO_PRODUCT_PRO`. Add a trial on the product if you want one.
2. Create an API key under **Developer › API Keys** and store it as `DODO_PAYMENTS_API_KEY`.
3. `DODO_ENVIRONMENT` is `test_mode` while you try it. Set it to `live_mode` to take real payments; test and live keys and products are separate.

A Dodo customer is made for every new account (`createCustomerOnSignUp`), so checkout and the portal know who's paying. No webhook is needed: `/v1/me` reads the subscription from Dodo.

### Updates

`/v1/releases/latest` serves `DECK_LATEST_VERSION`, `DECK_DOWNLOAD_URL` (a zip of `Deck.app`) and `DECK_NOTES_URL` when they're set. Otherwise it serves the newest GitHub release of `broisnischal/deck` with its `.zip` asset, cached for five minutes. The app downloads the zip, checks that the bundle ID is Deck's, and swaps the app in when you click Restart.

## Local development

```sh
cp .dev.vars.example .dev.vars     # fill in; these override wrangler.jsonc locally
npm run migrate:local              # the same migrations, on a local D1
npm run dev                        # http://localhost:8787
DECK_API=http://localhost:8787 scripts/install.sh   # a Deck that signs in here
```

`npm run typecheck` regenerates the Worker types from `wrangler.jsonc` and type-checks.

### Schema changes

The SQL in `migrations/0001_better_auth.sql` comes from Better Auth's CLI, run against `schema.config.ts`, which lists the same plugins on an in-memory SQLite:

```sh
npx @better-auth/cli generate --config schema.config.ts --output migrations/000N_name.sql
```

After adding or changing a plugin in `src/auth.ts`, change `schema.config.ts` the same way and generate a new numbered migration.

## Checked

- Type check passes.
- The migrations apply to a local D1.
- Under `wrangler dev` on that local D1, with a test session:
  - The sign-in page and the Google redirect work.
  - The one-time code is made, works once, and is refused the second time.
  - `/v1/me` refuses callers without a session.
  - Checkout refuses a signed-out caller.
- On Cloudflare (live, 2026-10-15): `/health`, the sign-in page, the Google redirect (to the live callback), `/v1/me` refusing an unsigned call, the token route querying the live D1, checkout refusing a signed-out caller, and `/v1/releases/latest` (404: no release yet).
- Not run yet: Google's own sign-in screen and Dodo checkout. Both need real keys. With a dummy Dodo key, `/v1/me` answers 502 and the app keeps its last answer.
