import { z } from "zod";

/** The wire protocol version implemented by this package. */
export const PROTO_VERSION = "1.20.0";

/**
 * Strict semver string: MAJOR.MINOR.PATCH with optional
 * -prerelease and +build metadata (SemVer 2.0.0).
 */
const SEMVER_RE =
  /^\d+\.\d+\.\d+(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

export const SemverSchema = z
  .string()
  .regex(SEMVER_RE, "must be a strict semver string (e.g. 1.0.0)");
