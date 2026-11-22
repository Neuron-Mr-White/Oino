/** Protocol `Icon` → Lucide (ui-protocol §4.5.2; lucide icons only, DESIGN). */
import type { Component } from 'solid-js';
import { Dynamic } from 'solid-js/web';
import {
  Bot, ChartLine, Check, Clock, Cloud, Diff, Dot, File, Folder, GitBranch, Globe, Info, List, Lock, Pause, Play, Plus,
  RefreshCw, Search, Settings, Square, SquareKanban, Terminal, Trash2, TriangleAlert, X, Zap,
} from 'lucide-solid';
import type { Icon } from '../types';

type IconProps = { size?: number; class?: string; 'aria-hidden'?: boolean | 'true' };

export const ICONS: Record<Icon, Component<IconProps>> = {
  check: Check,
  x: X,
  dot: Dot,
  warn: TriangleAlert,
  info: Info,
  clock: Clock,
  gear: Settings,
  branch: GitBranch,
  bolt: Zap,
  bot: Bot,
  folder: Folder,
  file: File,
  search: Search,
  cloud: Cloud,
  lock: Lock,
  board: SquareKanban,
  terminal: Terminal,
  globe: Globe,
  diff: Diff,
  list: List,
  chart: ChartLine,
  play: Play,
  pause: Pause,
  stop: Square,
  refresh: RefreshCw,
  plus: Plus,
  trash: Trash2,
};

export function IconView(props: { icon: Icon | string | undefined; size?: number; class?: string }) {
  const c = () => (props.icon ? (ICONS as Record<string, Component<IconProps>>)[props.icon] : undefined);
  return <>{c() && <Dynamic component={c()!} size={props.size ?? 16} class={props.class ?? 'ui-icon'} aria-hidden="true" />}</>;
}
