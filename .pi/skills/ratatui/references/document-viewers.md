# Document, body, schema, and markdown viewer subskill

Use for markdown readers, schema browsers, JSON/body viewers, syntax highlighted previews, record details, links, images, and search overlays.

## Source inspirations
mdfried/mdfrier/ratskin, openapi-tui `ResponseViewer` and schema viewer, fzf-make preview, yozefu record details, oatmeal code blocks.

## Related references

- `theming-polish.md`
- `streaming-async.md`
- `input-focus.md`
- `tables-grids.md`

## Rendering model

- Convert source into semantic `Line`/section/render-block models before draw.
- Cache highlighted lines by source string/version and width where wrapping matters.
- Recompute when body/version/width changes; discard stale parse/highlight results by id.
- Keep scroll state independent from content so top/bottom/page movement is easy.

## Response/body viewer pattern

Openapi-tui `ResponseViewer` is a good template:
- modes: `Normal`, `Search(matches,current)`, `Jq(result,is_error)`;
- content-type tabs influence accept header/request builder;
- line numbers are dim spans with width based on total lines;
- syntax highlighting cache avoids recomputing on every draw;
- jq/search errors render as styled content, not panics.

## Schema viewer pattern

- Resolve `$ref` with recursion guards.
- Merge or group `allOf`; represent `oneOf`/`anyOf` as variant nodes.
- Give every node a stable ID so selection survives re-render.
- Separate render-block construction from widget drawing.

## Record/detail viewer pattern

- Precompute metadata lines when selected record changes.
- Render metadata first (topic/timestamp/offset/size/headers/schema), then highlighted key/value/body.
- Use right-aligned labels for scanability.
- Add top/bottom/line scrolling and visible scrollbars for long payloads.

## Preview pattern

For fuzzy/file pickers:
- Center preview around selected line when possible.
- Hide preview below height threshold.
- Guard binary/huge/missing files with clear placeholder text.
- Use syntax highlighting when extension is known, but fail open to plain text.

## Testing checks

- Highlight cache invalidates on body change.
- Search/jq mode transitions and error display.
- Schema refs/variants do not recurse forever.
- Resize rewrap preserves scroll where possible.
- Long lines and Unicode do not break layout.


## Full markdown reader pattern (mdfried)

For full-screen markdown/document readers, use mdfried’s source-backed section model.

### Model and worker

- Main model stores `scroll`, `cursor`, `input_queue`, `screen_size`, `document`, `document_id`, config, command sender, and event receiver.
- Parsing runs in a worker thread/runtime. Commands carry `(document_id, width, text, image_cache)`.
- Ensure source text ends with `\n` if parser requires it.
- On reload, take existing image protocols/cache out of the document and pass them to the parser so images/headers do not flicker or reload unnecessarily.
- Every worker event includes `document_id`; the model ignores stale parse/image/header events from old documents.
- Send `NewDocument`, streaming `Parsed(section)`, then `ParseDone(last_section_id)`, and image/header load events after placeholders.

### Section rendering

Represent document as sections with height:

```rust
enum SectionContent { Lines, Image, ImagePlaceholder, Header, HeaderPlaceholder }
struct Section { id, height, content }
```

Render by walking sections with `y = -scroll`; skip sections above viewport and break before the status line. This scales better than flattening everything on every draw.

### Link/search overlays

- Store line extras (`Link`, `SearchMatch`) alongside each rendered line.
- Render normal line first, then overlay highlighted link/search spans in their exact rects.
- For wrapped links, compute overlay rects across continuation lines and position cursor at the selected link segment.
- Status line changes by cursor/input mode: selected link URL, `/search`, movement count, cursor positioning command.

### Images and headers

- Image placeholders render immediately; async tasks replace them with terminal image protocols or an error line.
- Header placeholders can be replaced by rendered big text/images when the terminal supports a text-size/image protocol.
- When image load fails, replace placeholder with muted `[error]` lines instead of panicking.

### Padding and reading width

- Support centered reading width via horizontal padding calculated from frame width.
- Status/search line should occupy the final row; document content should not render into it.
