/** Capture/test scenarios by name (`?scenario=<name>`). Later WPs append theirs. */
import type { MockScenario } from '../host';
import { empty } from './empty';
import { noHost } from './no-host';
import { offline } from './offline';

export const SCENARIOS: Record<string, MockScenario> = {
  empty,
  'no-host': noHost,
  offline,
};
