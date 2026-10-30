/**
 * Tool-kind icons + group labels (WP-A8): one map for the fold lead, the
 * group heads and the tool rows (lucide only, QA#10).
 */
import { Bot, Database, FilePen, FileText, Globe, ListChecks, MessageCircleQuestion, Search, SquareTerminal, Wrench, type LucideIcon } from 'lucide-solid';
import type { ToolCategory } from '../../../lib/chat/model';
import type { ToolGroupKind } from '../../../lib/session/turns';

export const TOOL_ICON: Record<ToolCategory, LucideIcon> = {
  read: FileText,
  search: Search,
  edit: FilePen,
  run: SquareTerminal,
  web: Globe,
  ask: MessageCircleQuestion,
  agent: Bot,
  memory: Database,
  board: ListChecks,
  other: Wrench,
};

export const GROUP_LABEL: Record<ToolGroupKind, string> = { execute: 'Execute', explore: 'Explored', changes: 'Changed' };
