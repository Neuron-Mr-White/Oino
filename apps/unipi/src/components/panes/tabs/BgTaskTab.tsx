/**
 * The background-task tab's registration (the view is `BgTaskView`). w2-ui: the view pulls xterm (≈ 340 KB of
 * script), so it loads when the first tab is shown; this module stays in the launch bundle only to register it.
 */
import { lazy } from 'solid-js';
import { setPaneComponent } from '../../../lib/panes/kinds';

setPaneComponent('bgtask', lazy(() => import('./BgTaskView').then((m) => ({ default: m.BgTaskTab }))));
