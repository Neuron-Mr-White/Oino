/**
 * A tiny in-memory board that answers `board_cli` argv the way the real
 * `unipi-kanboard` CLI does (ext-abi §5.5 shapes) — the mock host's board data
 * (captures, component tests) and the CLI-shaped fake of `lib/board/testing`.
 *
 * It implements the subcommands the Board UI issues: `status`, `project
 * list`, `list --all`, `show`, `add`, `move`, `note`, `edit`, `link`,
 * `unlink`, `order`, `duplicate`, `attach`, `search` — with the user-actor
 * transition rules of `USER_RULES` enforced on `move` so sheets and refusals
 * behave like the real board. Everything else answers `usage`.
 */
import type { BoardCliData } from '@unipi/protocol';
import { displayTitle, USER_RULES, type Task } from '../../lib/board/api';

export type MockBoardProject = {
  slug: string;
  name: string;
  /** Task id prefix (default: the first three letters/digits of `name`, like `project add`). */
  prefix?: string;
  archived?: boolean;
  tasks: Task[];
};

export type MockBoard = {
  /** Answers one `board_cli` call (the §5.5 outcome; never throws). */
  cli: (args: string[], project?: string) => BoardCliData;
  /** The live task list of a project (mutations for test assertions). */
  tasks: (slug: string) => Task[];
};

/** A success outcome; the payload is copied like the CLI's stdout (the app may patch its copy). */
const ok = (payload: unknown): BoardCliData => ({
  ok: true,
  payload: payload === undefined ? payload : (structuredClone(payload) as unknown),
});
const failure = (error: string, kind: 'usage' | 'not_found' | 'rule' | 'io'): BoardCliData => ({ ok: false, error, kind });

/** `--flag value` / `--flag=value` / bare positionals of an argv after the subcommand. */
function parse(argv: string[]): { flags: Record<string, string | true>; positionals: string[] } {
  const flags: Record<string, string | true> = {};
  const positionals: string[] = [];
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]!;
    if (arg === '--') {
      positionals.push(...argv.slice(i + 1));
      break;
    }
    if (arg.startsWith('--')) {
      const [name, inline] = arg.slice(2).split('=', 2) as [string, string | undefined];
      if (inline !== undefined) flags[name] = inline;
      else if (i + 1 < argv.length && !argv[i + 1]!.startsWith('--')) flags[name] = argv[++i]!;
      else flags[name] = true;
    } else {
      positionals.push(arg);
    }
  }
  return { flags, positionals };
}

/** `default_prefix` (store.rs): the first three alphanumerics of the name, upper-cased. */
function prefixOf(project: MockBoardProject): string {
  if (project.prefix) return project.prefix;
  const letters = project.name.replace(/[^a-z0-9]/gi, '').toUpperCase();
  return letters ? letters.slice(0, 3) : 'TASK';
}

/** Creates the board over `projects`; `onWrite` fires after every mutation. */
export function createMockBoard(projects: MockBoardProject[], onWrite?: (slug: string) => void): MockBoard {
  const find = (slug: string | undefined) => {
    const project = projects.find((candidate) => candidate.slug === slug);
    if (!project) return undefined;
    return project;
  };
  const changed = (slug: string) => onWrite?.(slug);
  const stamp = () => new Date().toISOString();
  let nextStatus = 1;

  const cli = (args: string[], project?: string): BoardCliData => {
    const [sub, ...rest] = args;
    const { flags, positionals } = parse(rest);
    switch (sub) {
      case 'status':
        return ok({ ok: true, home: '/mock/.oino/state/unipi.kanboard', projects: projects.map((p) => ({ slug: p.slug, name: p.name })) });
      case 'project': {
        if (rest[0] === 'list') {
          return ok(
            projects.map((p) => ({ slug: p.slug, name: p.name, archived: Boolean(p.archived) })),
          );
        }
        if (rest[0] === 'archive' || rest[0] === 'unarchive') {
          const target = find(rest[1]);
          if (!target) return failure(`no project ${rest[1]}`, 'not_found');
          target.archived = rest[0] === 'archive';
          changed(target.slug);
          return ok({ slug: target.slug, name: target.name, archived: target.archived });
        }
        return failure(`project ${rest[0] ?? ''} is not mocked`, 'usage');
      }
      case 'list': {
        const target = find(project);
        if (!target) return failure(`no project registered for ${project ?? '(none)'}`, 'not_found');
        let tasks = [...target.tasks];
        const status = flags.status;
        if (status) tasks = tasks.filter((task) => task.status === status);
        return ok({ tasks, problems: [] });
      }
      case 'show': {
        const target = find(project);
        const task = target?.tasks.find((candidate) => candidate.id === positionals[0]);
        return task ? ok(task) : failure(`no task ${positionals[0] ?? ''}`, 'not_found');
      }
      case 'add': {
        const target = find(project);
        if (!target) return failure(`no project registered for ${project ?? '(none)'}`, 'not_found');
        const prefix = prefixOf(target);
        const id = `${prefix}-${nextStatus++}`;
        const created = {
          id,
          title: positionals[0] ?? '',
          displayTitle: positionals[0] || `Body task ${id}`,
          body: typeof flags.body === 'string' ? flags.body : '',
          status: typeof flags.status === 'string' ? flags.status : 'backlog',
          priority: typeof flags.priority === 'string' ? flags.priority : 'none',
          order: nextStatus * 100,
          deps: [],
          labels: [],
          created: stamp(),
          updated: stamp(),
          activity: [{ at: stamp(), actor: 'user', text: `created in ${typeof flags.status === 'string' ? flags.status : 'backlog'}` }],
          ready: (Array.isArray(flags.after) ? [] : []).length === 0,
        } as Task;
        target.tasks.push(created);
        changed(target.slug);
        return ok(created);
      }
      case 'move': {
        const target = find(project);
        const task = target?.tasks.find((candidate) => candidate.id === positionals[0]);
        if (!task) return failure(`no task ${positionals[0] ?? ''}`, 'not_found');
        const to = positionals[1] ?? '';
        const allowed = task.allowedMoves ?? USER_RULES.allowedMoves[task.status] ?? [];
        if (to === task.status) return failure(`task is already in ${to}`, 'rule');
        if (allowed.length > 0 && !allowed.includes(to)) {
          return failure(`${task.status} → ${to} is not an allowed transition (see the kanboard transition table)`, 'rule');
        }
        const hint = USER_RULES.commentRequired[task.status]?.[to];
        const comment = typeof flags.comment === 'string' ? flags.comment : '';
        if (hint && !comment.trim()) {
          return failure(`${task.status} → ${to} requires --comment (${hint})`, 'rule');
        }
        const from = task.status;
        task.status = to;
        task.updated = stamp();
        task.activity.push({ at: stamp(), actor: 'user', text: comment ? `moved ${from} → ${to}: ${comment}` : `moved ${from} → ${to}` });
        changed(target!.slug);
        return ok(task);
      }
      case 'note': {
        const target = find(project);
        const task = target?.tasks.find((candidate) => candidate.id === positionals[0]);
        if (!task) return failure(`no task ${positionals[0] ?? ''}`, 'not_found');
        task.activity.push({ at: stamp(), actor: 'user', text: positionals.slice(1).join(' ') });
        task.updated = stamp();
        changed(target!.slug);
        return ok(task);
      }
      case 'edit': {
        const target = find(project);
        const task = target?.tasks.find((candidate) => candidate.id === positionals[0]);
        if (!task) return failure(`no task ${positionals[0] ?? ''}`, 'not_found');
        if (typeof flags.title === 'string') task.title = flags.title;
        if (typeof flags.body === 'string') task.body = flags.body;
        if (typeof flags.priority === 'string') task.priority = flags.priority;
        if (typeof flags.labels === 'string') task.labels = flags.labels.split(',').filter(Boolean);
        task.displayTitle = task.title || task.body?.split('\n')[0] || task.id;
        task.updated = stamp();
        changed(target!.slug);
        return ok(task);
      }
      case 'link':
      case 'unlink': {
        const target = find(project);
        const task = target?.tasks.find((candidate) => candidate.id === positionals[0]);
        if (!task) return failure(`no task ${positionals[0] ?? ''}`, 'not_found');
        const dep = typeof flags.after === 'string' ? flags.after : '';
        task.deps = sub === 'link' ? [...new Set([...task.deps, dep])] : task.deps.filter((item) => item !== dep);
        task.updated = stamp();
        changed(target!.slug);
        return ok(task);
      }
      case 'order': {
        const target = find(project);
        const task = target?.tasks.find((candidate) => candidate.id === positionals[0]);
        if (!task) return failure(`no task ${positionals[0] ?? ''}`, 'not_found');
        const lane = target!.tasks.filter((candidate) => candidate.status === task.status).sort((a, b) => a.order - b.order);
        const before = typeof flags.before === 'string' ? lane.find((candidate) => candidate.id === flags.before) : undefined;
        const afterPos = typeof flags['after-pos'] === 'string' ? lane.find((candidate) => candidate.id === flags['after-pos']) : undefined;
        const anchor = before ?? afterPos;
        if (anchor) {
          const gap = before ? -1 : 1;
          task.order = anchor.order + gap;
        } else if (flags.top === true) {
          task.order = (lane[0]?.order ?? 100) - 1;
        } else {
          task.order = (lane.at(-1)?.order ?? 0) + 100;
        }
        task.updated = stamp();
        changed(target!.slug);
        return ok(task);
      }
      case 'duplicate': {
        const target = find(project);
        const task = target?.tasks.find((candidate) => candidate.id === positionals[0]);
        if (!task) return failure(`no task ${positionals[0] ?? ''}`, 'not_found');
        const prefix = prefixOf(target!);
        const id = `${prefix}-${nextStatus++}`;
        const copy: Task = {
          ...task,
          id,
          title: task.title,
          displayTitle: task.title || displayTitle(task),
          status: 'backlog',
          order: nextStatus * 100,
          created: stamp(),
          updated: stamp(),
          activity: [{ at: stamp(), actor: 'user', text: `created in backlog (duplicate of ${task.id})` }],
        };
        target!.tasks.push(copy);
        changed(target!.slug);
        return ok(copy);
      }
      case 'attach': {
        const target = find(project);
        const task = target?.tasks.find((candidate) => candidate.id === positionals[0]);
        if (!task) return failure(`no task ${positionals[0] ?? ''}`, 'not_found');
        const original = typeof flags.name === 'string' ? flags.name : (positionals[1] ?? 'file');
        const name = `aaaa${nextStatus++}-${original}`;
        const attachment = {
          name,
          original,
          ref: `att:${task.id}/${name}`,
          path: `/mock/.oino/state/unipi.kanboard/projects/${target!.slug}/attachments/${task.id}/${name}`,
          size: 1024,
          mime: original.endsWith('.png') ? 'image/png' : 'application/octet-stream',
          kind: (original.endsWith('.png') ? 'image' : 'file') as 'image' | 'file',
          markdown: original.endsWith('.png') ? `![${original}](att:${task.id}/${name})` : `[${original}](att:${task.id}/${name})`,
        };
        task.attachments = [...(task.attachments ?? []), attachment];
        task.updated = stamp();
        changed(target!.slug);
        return ok(attachment);
      }
      case 'search': {
        const needle = (positionals[0] ?? '').toLowerCase();
        const items = projects.flatMap((p) =>
          p.tasks
            .filter((task) => `${task.id} ${task.title} ${task.body ?? ''}`.toLowerCase().includes(needle))
            .map((task) => ({ id: task.id, title: displayTitle(task), status: task.status })),
        );
        return ok({ tasks: items });
      }
      default:
        return failure(`\`${sub ?? ''}\` is not a board_cli subcommand the app may run`, 'usage');
    }
  };

  return { cli, tasks: (slug: string) => find(slug)?.tasks ?? [] };
}
