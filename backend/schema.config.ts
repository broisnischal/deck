// Only for `npx @better-auth/cli generate`: the same plugins as src/auth.ts
// on an in-memory SQLite, so the SQL it writes matches what D1 needs.
import { betterAuth } from "better-auth";
import { bearer } from "better-auth/plugins";
import { dodopayments, checkout, portal } from "@dodopayments/better-auth";
import DodoPayments from "dodopayments";
import { DatabaseSync } from "node:sqlite";

export const auth = betterAuth({
  database: new DatabaseSync(":memory:"),
  socialProviders: { google: { clientId: "x", clientSecret: "x" } },
  plugins: [
    bearer(),
    dodopayments({
      client: new DodoPayments({ bearerToken: "x", environment: "test_mode" }),
      createCustomerOnSignUp: true,
      use: [checkout({ products: [] }), portal()],
    }),
  ],
});
