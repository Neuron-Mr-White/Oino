import { can, isTauri } from '../lib/platform';
import { createEffect } from "solid-js";
import { z } from "zod";
import { useSettings } from "../settings/context";
import { call } from "../lib/ipc";
export function RelaySettings() {
  const [settings] = useSettings();
  createEffect(() => {
    const cfg = { ...settings.connection.relay };
    if (isTauri() && can('hosts'))
      void call("set_relay", { cfg }, z.null());
  });
  return null;
}
