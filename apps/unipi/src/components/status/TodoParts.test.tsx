// @vitest-environment jsdom
import { cleanup, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { TodoList } from './TodoParts';

afterEach(cleanup);

describe('TodoList', () => {
  it('shows badges only for explicit item priorities', () => {
    render(() => <TodoList todos={[
      { content: 'No priority', status: 'pending' },
      { content: 'Urgent', status: 'in_progress', priority: 'high' },
      { content: 'Normal', status: 'pending', priority: 'medium' },
      { content: 'Later', status: 'completed', priority: 'low' },
      { content: 'Also no priority', status: 'pending' },
    ]} />);
    expect(screen.getAllByTestId('todo-priority').map((badge) => badge.textContent)).toEqual(['high', 'medium', 'low']);
  });
});
