/** Capture/test scenarios by name (`?scenario=<name>`). Later WPs append theirs. */
import type { MockScenario } from '../host';
import { empty } from './empty';
import { noHost } from './no-host';
import { offline } from './offline';
import { streamBasic } from './stream-basic';
import { streamReconnect } from './stream-reconnect';
import { streamSteer } from './stream-steer';

export const SCENARIOS: Record<string, MockScenario> = {
  empty,
  'no-host': noHost,
  offline,
  'stream-basic': streamBasic,
  'stream-steer': streamSteer,
  'stream-reconnect': streamReconnect,
};
