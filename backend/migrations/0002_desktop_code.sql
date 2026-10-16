-- One-time codes the desktop app trades for a session: made by
-- /desktop/done, used once by /desktop/token within two minutes.
create table "desktop_code" (
  "code" text not null primary key,
  "token" text not null,
  "expiresAt" integer not null
);
create index "desktop_code_expiresAt_idx" on "desktop_code" ("expiresAt");
