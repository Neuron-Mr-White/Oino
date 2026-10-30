/**
 * `Empty` (ui-protocol §4.5.2): the app's `<EmptyState compact>` — icon (or
 * the crab), title, message, and its `action` Button node (path segment
 * `"action"`, §4.6).
 */
import { EmptyState } from '../../components/EmptyState';
import type { EmptyNode } from '../types';
import { Button } from './Button';
import { IconView } from './icons';
import { childPath, type NodeProps } from './props';

export function Empty(props: NodeProps<EmptyNode>) {
  return (
    <div class="ui-empty" data-mode={props.mode}>
      <EmptyState
        compact
        testId="ui-empty"
        title={props.node.title ?? ''}
        body={props.node.message}
        icon={props.node.icon ? <IconView icon={props.node.icon} size={20} /> : undefined}
        action={props.node.action ? <Button node={props.node.action} path={childPath(props.path, 'action')} mode="auto" /> : undefined}
      />
    </div>
  );
}
