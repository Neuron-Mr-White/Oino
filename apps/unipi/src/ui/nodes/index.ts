/** Node kind → component (ui-protocol §13.1: one file per kind). */
import { BarChart } from './BarChart';
import { Block } from './Block';
import { Button } from './Button';
import { Chart } from './Chart';
import { Code } from './Code';
import { Diff } from './Diff';
import { Empty } from './Empty';
import { Gauge } from './Gauge';
import { Image } from './Image';
import { Input } from './Input';
import { KeyHints } from './KeyHints';
import { Layout } from './Layout';
import { LineGauge } from './LineGauge';
import { List } from './List';
import { Log } from './Log';
import { Markdown } from './Markdown';
import { Paragraph } from './Paragraph';
import { Scrollbar } from './Scrollbar';
import { Select } from './Select';
import { Sparkline } from './Sparkline';
import { Spinner } from './Spinner';
import { Table } from './Table';
import { Tabs } from './Tabs';
import { Toggle } from './Toggle';
import { Tree } from './Tree';
import type { NodeKind } from '../types';
import type { Component } from 'solid-js';
import type { NodeProps } from './props';

export const NODE_COMPONENTS: Record<NodeKind, Component<NodeProps<never>>> = {
  Layout,
  Block,
  Paragraph,
  List,
  Table,
  Tabs,
  Gauge,
  LineGauge,
  Sparkline,
  BarChart,
  Chart,
  Scrollbar,
  Markdown,
  Code,
  Diff,
  Image,
  Log,
  Tree,
  KeyHints,
  Input,
  Toggle,
  Select,
  Button,
  Spinner,
  Empty,
} as Record<NodeKind, Component<NodeProps<never>>>;
