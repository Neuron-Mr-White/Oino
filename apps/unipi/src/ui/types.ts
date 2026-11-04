/**
 * TS mirror of the `oino-ui` wire types (ui-protocol v2 §4, §6.1, §9.2;
 * normative JSON Schema: `crates/oino-ui/schema/{node,registry}.schema.json`).
 *
 * Hand-written from the schema rather than generated: the generated form
 * (json-schema-to-typescript) spells every `allOf` as an intersection, which
 * is unreadable at the call sites. Nodes are never zod-parsed (D-U14): unknown
 * props are ignored, unknown `t` renders `fallback`.
 */

// ── Text model (§4.2) ────────────────────────────────────────────────────

export type Color =
  | 'reset' | 'text' | 'muted' | 'dim' | 'accent' | 'primary' | 'info' | 'link' | 'success' | 'warning' | 'error'
  | 'harness' | 'thinking' | 'tool' | 'user' | 'assistant' | 'code' | 'border' | 'borderAccent' | 'borderMuted'
  | 'c1' | 'c2' | 'c3' | 'c4' | 'c5' | 'c6' | 'c7' | 'c8' | 'c9' | 'c10' | 'c11' | 'c12'
  | 'surface1' | 'surface2' | 'surface3' | 'selectedBg' | 'successBg' | 'errorBg' | 'warningBg' | 'infoBg' | 'accentBg'
  | 'diffAdded' | 'diffRemoved' | 'diffContext';

export type Modifier = 'BOLD' | 'DIM' | 'ITALIC' | 'UNDERLINED' | 'SLOW_BLINK' | 'RAPID_BLINK' | 'REVERSED' | 'HIDDEN' | 'CROSSED_OUT';
export type Alignment = 'Left' | 'Center' | 'Right';

export type Style = { fg?: Color; bg?: Color; underlineColor?: Color; addModifier?: Modifier[]; subModifier?: Modifier[] };
export type SpanObj = { content: string; style?: Style; link?: string };
export type Span = string | SpanObj;
export type LineObj = { spans: Span[]; style?: Style; alignment?: Alignment };
export type Line = string | Span[] | LineObj;
export type TextObj = { lines: Line[]; style?: Style; alignment?: Alignment };
export type Text = string | Line[] | TextObj;

export type Icon =
  | 'check' | 'x' | 'dot' | 'warn' | 'info' | 'clock' | 'gear' | 'branch' | 'bolt' | 'bot' | 'folder' | 'file' | 'search'
  | 'cloud' | 'lock' | 'board' | 'terminal' | 'globe' | 'diff' | 'list' | 'chart' | 'play' | 'pause' | 'stop' | 'refresh'
  | 'plus' | 'trash';

// ── Layout (§4.3) ────────────────────────────────────────────────────────

export type Constraint =
  | number
  | { Length: number }
  | { Percentage: number }
  | { Ratio: [number, number] }
  | { Min: number }
  | { Max: number }
  | { Fill: number };
export type Flex = 'Legacy' | 'Start' | 'End' | 'Center' | 'SpaceBetween' | 'SpaceAround';
export type Direction = 'Vertical' | 'Horizontal';
export type Margin = number | { horizontal?: number; vertical?: number };
export type Padding = number | { left?: number; right?: number; top?: number; bottom?: number };

// ── Nodes (§4.5) ─────────────────────────────────────────────────────────

export type Title = Line | { content: Line; alignment?: Alignment; position?: 'Top' | 'Bottom' };
export type Borders = 'ALL' | 'NONE' | Array<'TOP' | 'RIGHT' | 'BOTTOM' | 'LEFT'>;
export type BlockProps = {
  title?: Line | Title[];
  titleAlignment?: Alignment;
  borders?: Borders;
  borderType?: 'Plain' | 'Rounded' | 'Double' | 'Thick';
  borderStyle?: Style;
  titleStyle?: Style;
  padding?: Padding;
  collapsible?: boolean;
  collapsed?: boolean;
  rail?: Color;
  style?: Style;
};

export type Common = { key?: string; style?: Style; block?: BlockProps; fallback?: UiNode };
type N<T extends string, P> = Common & { t: T } & P;

export type ListItemObj = { key?: string; content: Text; style?: Style; detail?: Line; badge?: Line; icon?: Icon; disabled?: boolean };
export type ListItem = Text | ListItemObj;
export type CellObj = { content: Text; style?: Style };
export type Cell = Text | CellObj;
export type Row = { key?: string; cells: Cell[]; height?: number; bottomMargin?: number; style?: Style };
export type TabTitle = { key: string; title: Line; badge?: Line };
export type Bar = { value: number; label?: Line; textValue?: string; style?: Style; valueStyle?: Style };
export type BarGroup = { label?: Line; bars: Bar[] };
export type Axis = { title?: Line; bounds: [number, number]; labels?: Span[]; labelsAlignment?: Alignment; style?: Style };
export type Dataset = {
  name?: Line;
  data: Array<[number, number]>;
  marker?: 'Dot' | 'Braille' | 'Block' | 'Bar' | 'HalfBlock';
  graphType?: 'Scatter' | 'Line' | 'Bar';
  style?: Style;
};
export type TreeNode = { key: string; label: Line; icon?: Icon; badge?: Line; children?: TreeNode[]; expanded?: boolean; lazy?: boolean };
export type Hint = { keys: string; label: string; action?: string };
export type SelectOption = { value: string; label: Line; description?: string };
export type ImageSource = { data: string; mimeType: string } | { path: string } | { blob: string };
export type HighlightSpacing = 'Always' | 'WhenSelected' | 'Never';

export type LayoutNode = N<'Layout', { direction?: Direction; constraints?: Constraint[]; children: UiNode[]; spacing?: number; margin?: Margin; flex?: Flex; collapse?: number }>;
export type BlockNode = N<'Block', BlockProps & { child?: UiNode; onToggle?: string }>;
export type ParagraphNode = N<'Paragraph', { text: Text; wrap?: false | { trim?: boolean }; scroll?: [number, number]; alignment?: Alignment; scrollbar?: boolean }>;
export type ListNode = N<'List', {
  items: ListItem[];
  state?: { selected?: string | string[]; offset?: number };
  highlightStyle?: Style;
  highlightSymbol?: string;
  repeatHighlightSymbol?: boolean;
  highlightSpacing?: HighlightSpacing;
  direction?: 'TopToBottom' | 'BottomToTop';
  scrollPadding?: number;
  multi?: boolean;
  scrollbar?: boolean;
  onSelect?: string;
  onActivate?: string;
}>;
export type TableNode = N<'Table', {
  header?: Row;
  rows: Row[];
  footer?: Row;
  widths?: Constraint[];
  columnSpacing?: number;
  rowHighlightStyle?: Style;
  highlightSymbol?: string;
  highlightSpacing?: HighlightSpacing;
  state?: { selected?: string; offset?: number };
  sticky?: 'FirstColumn';
  scrollbar?: boolean;
  onSelect?: string;
  onActivate?: string;
}>;
export type TabsNode = N<'Tabs', { titles: TabTitle[]; selected?: string; divider?: Span; padding?: [Line, Line]; highlightStyle?: Style; panes?: Record<string, UiNode>; onSelect?: string }>;
export type GaugeNode = N<'Gauge', { ratio?: number; percent?: number; label?: Span; gaugeStyle?: Style; useUnicode?: boolean }>;
export type LineGaugeNode = N<'LineGauge', { ratio: number; label?: Line; filledStyle?: Style; unfilledStyle?: Style; lineSet?: 'Normal' | 'Thick' | 'Double' }>;
export type SparklineNode = N<'Sparkline', { data: Array<number | null>; max?: number; direction?: 'LeftToRight' | 'RightToLeft'; barSet?: 'Nine' | 'Three'; absentValueSymbol?: string }>;
export type BarChartNode = N<'BarChart', {
  data: BarGroup[];
  barWidth?: number;
  barGap?: number;
  groupGap?: number;
  direction?: Direction;
  max?: number;
  barStyle?: Style;
  valueStyle?: Style;
  labelStyle?: Style;
  onSelect?: string;
}>;
export type ChartNode = N<'Chart', { datasets: Dataset[]; xAxis?: Axis; yAxis?: Axis; legendPosition?: 'Top' | 'TopRight' | 'TopLeft' | 'Left' | 'Right' | 'Bottom' | 'BottomRight' | 'BottomLeft' | null }>;
export type ScrollbarNode = N<'Scrollbar', {
  orientation?: 'VerticalRight' | 'VerticalLeft' | 'HorizontalBottom' | 'HorizontalTop';
  state: { contentLength: number; position: number; viewportContentLength?: number };
  thumbStyle?: Style;
  trackStyle?: Style;
  beginSymbol?: string | null;
  endSymbol?: string | null;
}>;
export type MarkdownNode = N<'Markdown', { source: string }>;
export type CodeNode = N<'Code', { source: string; language?: string; lineNumbers?: boolean; startLine?: number; highlightLines?: number[]; wrap?: boolean }>;
export type DiffNode = N<'Diff', { patch?: string; old?: string; new?: string; path?: string; context?: number; mode?: 'Unified' | 'Split' }>;
export type ImageNode = N<'Image', { src: ImageSource; alt: string; fit?: 'Contain' | 'Cover'; maxWidth?: number; maxHeight?: number }>;
export type LogNode = N<'Log', { lines: Line[]; maxLines?: number; follow?: boolean; wrap?: boolean; lineNumbers?: boolean }>;
export type TreeViewNode = N<'Tree', { nodes: TreeNode[]; state?: { selected?: string }; onSelect?: string; onActivate?: string; onToggle?: string }>;
export type KeyHintsNode = N<'KeyHints', { hints: Hint[] }>;
export type InputNode = N<'Input', {
  value?: string;
  placeholder?: string;
  label?: Line;
  multiline?: boolean;
  rows?: number;
  secret?: boolean;
  maxLength?: number;
  disabled?: boolean;
  onSubmit?: string;
  onChange?: string;
}>;
export type ToggleNode = N<'Toggle', { label: Line; value: boolean; disabled?: boolean; onChange?: string }>;
export type SelectNode = N<'Select', { options: SelectOption[]; value?: string; placeholder?: string; disabled?: boolean; onChange?: string }>;
export type ButtonNode = N<'Button', {
  label: Line;
  action: string;
  value?: unknown;
  variant?: 'Primary' | 'Secondary' | 'Ghost' | 'Danger';
  icon?: Icon;
  disabled?: boolean;
  confirm?: string;
}>;
export type SpinnerNode = N<'Spinner', { label?: Line }>;
export type EmptyNode = N<'Empty', { title?: string; message?: string; icon?: Icon; action?: ButtonNode }>;
/** A kind this build doesn't know (forward compatibility, D-U14). */
export type UnknownNode = Common & { t: string; [prop: string]: unknown };

export type UiNode =
  | LayoutNode | BlockNode | ParagraphNode | ListNode | TableNode | TabsNode | GaugeNode | LineGaugeNode
  | SparklineNode | BarChartNode | ChartNode | ScrollbarNode | MarkdownNode | CodeNode | DiffNode | ImageNode
  | LogNode | TreeViewNode | KeyHintsNode | InputNode | ToggleNode | SelectNode | ButtonNode | SpinnerNode | EmptyNode;

export type NodeKind = UiNode['t'];

export const NODE_KINDS: readonly NodeKind[] = [
  'Layout', 'Block', 'Paragraph', 'List', 'Table', 'Tabs', 'Gauge', 'LineGauge', 'Sparkline', 'BarChart', 'Chart',
  'Scrollbar', 'Markdown', 'Code', 'Diff', 'Image', 'Log', 'Tree', 'KeyHints', 'Input', 'Toggle', 'Select', 'Button',
  'Spinner', 'Empty',
];

// ── Patches (§4.7) ───────────────────────────────────────────────────────

export type NodePath = string[];
export type PatchField = 'children' | 'items' | 'rows' | 'nodes' | 'lines' | 'titles' | 'datasets' | 'data' | 'panes';
export type NodeOp =
  | { op: 'replace'; path: NodePath; node: UiNode }
  | { op: 'set'; path: NodePath; props: Record<string, unknown> }
  | { op: 'splice'; path: NodePath; field: PatchField; index: number; remove: number; insert: unknown[] }
  | { op: 'append'; path: NodePath; field: PatchField; items: unknown[] }
  | { op: 'item'; path: NodePath; field: PatchField; key: string; value: unknown };

// ── Registry (§6.1, §9.2) ────────────────────────────────────────────────

export type Scope = 'session' | 'host';
export type Slot =
  | 'overlay' | 'sidePane' | 'bottomPane' | 'editorReplace' | 'fullscreen' | 'widget' | 'widgetBelow' | 'status'
  | 'turnFooter' | 'header' | 'footer' | 'settings';

export type EntryView =
  | { kind: 'tree'; root: UiNode }
  | { kind: 'lines'; lines: string[] }
  | { kind: 'status'; text: Line }
  | { kind: 'reserved'; component: string; fallback?: UiNode; params?: unknown }
  | { kind: 'renderer'; key: string }
  | { kind: 'settings'; namespace: Record<string, unknown> };

export type EntryKey = { key: string; action?: string; label: string };

export type UiEntry = {
  id: string;
  extensionId: string;
  scope: Scope;
  slot: Slot;
  order: number;
  seq: number;
  title?: string;
  icon?: Icon;
  badge?: Line;
  tone?: Color;
  view: EntryView;
  keys?: EntryKey[];
  focus?: boolean;
  closeOnEscape?: boolean;
  size?: 'small' | 'medium' | 'large' | 'full';
  /** The clients it is meant for (`["tui"]`); absent = every client (`lib/surfaces`). */
  surfaces?: string[];
  rev: number;
};

/** A pi / Oino dialog request (pi `extension_ui_request` shape, v1 §4.3). */
export type UiRequest = { id: string; method: string; [field: string]: unknown };
export type Shortcut = { key: string; label?: string; [field: string]: unknown };
export type UiMeta = { title?: string; workingMessage?: string; workingVisible?: boolean; shortcuts?: Shortcut[] };

export type UiSnapshot = { scope: Scope; sessionId?: string | null; rev: number; entries: UiEntry[]; dialogs: UiRequest[]; meta?: UiMeta };

export type EntryMetaSet = { title?: string; icon?: Icon; badge?: Line; order?: number; keys?: EntryKey[]; tone?: Color; focus?: boolean };
export type RegistryOp =
  | { op: 'put'; entry: UiEntry }
  | { op: 'del'; id: string }
  | { op: 'meta'; id: string; set: EntryMetaSet }
  | { op: 'tree'; id: string; rev: number; ops: NodeOp[] }
  | { op: 'invalidate'; id: string; turns?: string[] }
  | { op: 'dialog'; request: UiRequest }
  | { op: 'dismiss'; id: string; reason: 'answered' | 'timeout' | 'cancelled' }
  | { op: 'sessionMeta'; set: UiMeta }
  | { op: 'effect'; request: UiRequest };

export type UiPatch = { scope: Scope; sessionId?: string | null; rev: number; ops: RegistryOp[] };
