import { createAuthClient } from "better-auth/react";
import { inferAdditionalFields, organizationClient } from "better-auth/client/plugins";
import { polarClient } from "@polar-sh/better-auth";

import { sessionAdditionalFields } from "./session-fields";

export const authClient = createAuthClient({
  plugins: [organizationClient(), polarClient(), inferAdditionalFields({ session: sessionAdditionalFields })],
});

export function initializeAuthSession() {
  const session = authClient.$store.atoms["session"];
  if (!session) throw new Error("Better Auth session store is unavailable");
  return new Promise<void>((resolve) => {
    const unsubscribe = session.subscribe((state) => {
      if (!state.isPending) {
        queueMicrotask(() => {
          unsubscribe();
          resolve();
        });
      }
    });
  });
}
