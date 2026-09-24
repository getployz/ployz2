import { Effect } from "effect";
import type { ReadPairingChanges } from "#/modules/runtime/organization-runtime.server";

/** The pairing change reader for runtimes whose tests never change the pairing. */
export const noPairingChanges: ReadPairingChanges = () => Effect.succeed({ cursor: "0", changed: false });
