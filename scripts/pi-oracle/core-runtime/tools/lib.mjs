// Shared helpers for the oino-tools pi oracle.
import { pathToFileURL } from "node:url";

export const PI =
  process.env.PI_CODING_AGENT_PKG ??
  `${process.env.HOME}/.local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent`;

export const importPi = (rel) => import(pathToFileURL(`${PI}/dist/${rel}`).href);
