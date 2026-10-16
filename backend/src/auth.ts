import { env } from "cloudflare:workers";
import { betterAuth } from "better-auth";
import { bearer } from "better-auth/plugins";
import { checkout, dodopayments, portal } from "@dodopayments/better-auth";
import DodoPayments from "dodopayments";

export const PRO = "pro";

/** The Dodo client, also for reading the plan's price. */
export const dodo = new DodoPayments({
  bearerToken: env.DODO_PAYMENTS_API_KEY,
  // A var, so `wrangler types` reads it as its literal; it is set per deploy.
  environment: (env.DODO_ENVIRONMENT as string) === "live_mode" ? "live_mode" : "test_mode",
});

export const auth = betterAuth({
  baseURL: env.BETTER_AUTH_URL,
  secret: env.BETTER_AUTH_SECRET,
  // The D1 binding; Better Auth picks its SQLite dialect from it.
  database: env.DB,
  // Google is the only way in; the desktop app opens it in the browser.
  socialProviders: {
    google: {
      clientId: env.GOOGLE_CLIENT_ID,
      clientSecret: env.GOOGLE_CLIENT_SECRET,
      prompt: "select_account",
    },
  },
  // The schema comes from migrations/ (`npm run migrate`), and checking it
  // on the first request would touch D1 outside a request on cold start.
  advanced: { database: { validateSchema: false } },
  session: {
    // The app checks in every few hours; a month of quiet signs it out.
    expiresIn: 60 * 60 * 24 * 30,
    updateAge: 60 * 60 * 24,
  },
  plugins: [
    // The app sends its session token as `Authorization: Bearer …`.
    bearer(),
    dodopayments({
      client: dodo,
      // A Dodo customer for every new account, so checkout and the
      // portal know who is paying.
      createCustomerOnSignUp: true,
      use: [
        checkout({
          products: [{ productId: env.DODO_PRODUCT_PRO, slug: PRO }],
          successUrl: "/desktop/paid",
          authenticatedUsersOnly: true,
        }),
        portal(),
      ],
    }),
  ],
});
