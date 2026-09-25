const REQUIRED_SERVICE_VARIABLES = [
  "GITHUB_APP_ID",
  "GITHUB_APP_PRIVATE_KEY",
  "GITHUB_APP_SLUG",
  "GITHUB_APP_WEBHOOK_SECRET",
  "INNGEST_EVENT_KEY",
  "INNGEST_SIGNING_KEY",
] as const;

/** Credentials AppConfig requires but tests never exercise; `setup-env.ts` and `.env.test` provide them. */
export function requiredServiceEnvironment(): Record<string, string> {
  return Object.fromEntries(
    REQUIRED_SERVICE_VARIABLES.map((name) => {
      const value = process.env[name];
      if (value === undefined) throw new Error(`Missing test variable ${name}`);
      return [name, value];
    }),
  );
}
