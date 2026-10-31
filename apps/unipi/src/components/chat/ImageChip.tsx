/**
 * Chat image chips (UNI-157): small inline thumbnails for `ImageRef`s on a
 * user bubble or tool result. A tiny image with `data` already renders
 * right away; an `omitted`/`mediaRef` one shows a placeholder until tapped
 * (fetches on demand with `chat.media()`), then opens the full previewer.
 *
 * `AttachmentChipRow` is the same idea for `details.attachments[]` (agent
 * tool results/cards): each chip fetches its bytes with `chat.media()` on
 * tap (falling back to `blob_get{path}` when the mediaRef has aged out —
 * see `openFilePath`) and opens the previewer routed by mime/extension.
 */
import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from 'solid-js';
import { File as FileIcon, ImageIcon } from 'lucide-solid';
import type { RefChat } from '../../lib/chat/pathStat';
import type { AttachmentRef, ImageRef } from '../../lib/chat/model';
import type { FileMention } from '../../lib/composer/attachments';
import { classifyMedia } from '../../lib/media/previewer';
import { openPreviewer } from '../../lib/media/previewerState';
import { openFilePath, resolvePathToken } from '../../lib/media/pathTokens';

export function ImageChipRow(props: { chat: RefChat; images: ImageRef[] }) {
  return (
    <Show when={props.images.length > 0}>
      <div class="chat-image-row">
        <For each={props.images}>{(image, index) => <ImageChip chat={props.chat} image={image} images={props.images} index={index()} />}</For>
      </div>
    </Show>
  );
}

/** Loaded image `data:` URLs by mediaRef, shared by every chip (a chat
 * re-render, the previewer's siblings and a second bubble showing the same
 * image reuse one fetch). Bounded; oldest dropped first. */
const imageCache = new Map<string, Promise<string>>();
const IMAGE_CACHE_MAX = 60;
/** At most this many thumbnail fetches in flight (a long history full of
 * screenshots must not flood the bridge line). */
const MAX_PARALLEL = 3;
let inFlight = 0;
const queue: (() => void)[] = [];

function slot(): Promise<() => void> {
  return new Promise((resolve) => {
    const take = () => {
      inFlight += 1;
      let released = false;
      resolve(() => {
        if (released) return;
        released = true;
        inFlight -= 1;
        queue.shift()?.();
      });
    };
    if (inFlight < MAX_PARALLEL) take();
    else queue.push(take);
  });
}

/** Fetches (once) the bytes behind an image chip. Failures aren't cached, so
 * a later tap or a reconnect retries. */
export function loadImage(chat: RefChat, image: ImageRef): Promise<string> {
  if (image.data) return Promise.resolve(`data:${image.mime};base64,${image.data}`);
  const ref = image.mediaRef;
  if (!ref) return Promise.reject(new Error('No image data'));
  const cached = imageCache.get(ref);
  if (cached) return cached;
  const job = (async () => {
    const release = await slot();
    try {
      if (!chat.media) throw new Error('No image channel');
      return (await chat.media(ref, image.mime)).dataUrl;
    } finally {
      release();
    }
  })();
  imageCache.set(ref, job);
  job.catch(() => {
    if (imageCache.get(ref) === job) imageCache.delete(ref);
  });
  while (imageCache.size > IMAGE_CACHE_MAX) {
    const oldest = imageCache.keys().next().value;
    if (oldest === undefined) break;
    imageCache.delete(oldest);
  }
  return job;
}

/** Test seam. */
/** UNI-290: an image ref (a sent message's) as a `File` again, to re-attach it to the composer. */
export async function imageRefFile(chat: RefChat, image: ImageRef, name: string): Promise<File> {
  const url = await loadImage(chat, image);
  const m = /^data:([^;,]*)(;base64)?,([\s\S]*)$/.exec(url);
  if (!m) throw new Error('Not a data URL');
  const mime = m[1] || image.mime || 'application/octet-stream';
  let bytes: Uint8Array<ArrayBuffer>;
  if (m[2]) {
    const bin = atob(m[3]!);
    bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  } else bytes = new TextEncoder().encode(decodeURIComponent(m[3]!));
  return new File([bytes], name, { type: mime });
}

export function resetImageCacheForTests(): void {
  imageCache.clear();
  inFlight = 0;
  queue.length = 0;
}

/** Opens the previewer on `images[index]` (UNI-285: an `[image N]` chip in a
 *  sent message), resolving the siblings too so swipe works. Best effort:
  *  an image whose bytes can't be fetched is skipped. */
export async function openImagePreviewer(chat: RefChat, images: readonly ImageRef[], index: number): Promise<void> {
  if (index < 0 || index >= images.length) return;
  const want = images[index]!;
  const resolved = await loadImage(chat, want).catch(() => undefined);
  if (!resolved) return;
  const items = await Promise.all(
    images.map(async (image, i) => {
      if (i === index) return { url: resolved, mime: want.mime };
      try {
        return { url: await loadImage(chat, image), mime: image.mime };
      } catch {
        return undefined;
      }
    }),
  );
  const resolvedItems = items.filter((i): i is { url: string; mime: string } => !!i);
  openPreviewer({ kind: 'image', items: resolvedItems, index: resolvedItems.findIndex((i) => i.url === resolved) });
}

function ImageChip(props: { chat: RefChat; image: ImageRef; images: ImageRef[]; index: number }) {
  const [url, setUrl] = createSignal<string | undefined>(props.image.data ? `data:${props.image.mime};base64,${props.image.data}` : undefined);
  const [loading, setLoading] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  let el: HTMLButtonElement | undefined;

  const ensureLoaded = async (): Promise<string | undefined> => {
    if (url()) return url();
    if (!props.image.mediaRef) return undefined;
    setLoading(true);
    setFailed(false);
    try {
      const got = await loadImage(props.chat, props.image);
      setUrl(got);
      return got;
    } catch {
      setFailed(true);
      return undefined;
    } finally {
      setLoading(false);
    }
  };

  // Show the thumbnail without a tap (UNI-273): fetch once the chip is (about
  // to be) on screen, and again when the chat reconnects after a failure.
  onMount(() => {
    if (url() || !props.image.mediaRef) return;
    if (typeof IntersectionObserver === 'undefined' || !el) {
      void ensureLoaded();
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          io.disconnect();
          void ensureLoaded();
        }
      },
      { rootMargin: '400px' },
    );
    io.observe(el);
    onCleanup(() => io.disconnect());
  });
  createEffect(
    on(
      () => props.chat.state?.conn,
      (conn, prev) => {
        if (conn === 'live' && prev !== undefined && prev !== 'live' && failed() && !url()) void ensureLoaded();
      },
    ),
  );

  const open = async () => {
    if (props.index < 0 || props.index >= props.images.length) return;
    const resolved = await ensureLoaded();
    if (!resolved) return;
    await openImagePreviewer(props.chat, props.images, props.index);
  };

  return (
    <button
      ref={el}
      type="button"
      class="chat-image-chip"
      classList={{ 'is-loading': loading() && !url() }}
      aria-label="Open image"
      aria-busy={loading() ? 'true' : undefined}
      onClick={() => void open()}
    >
      <Show when={url()} fallback={<ImageIcon size={18} class="chat-image-chip-icon" />}>
        {(src) => <img src={src()} alt="" class="chat-image-chip-img" decoding="async" />}
      </Show>
      <Show when={failed() && !url()}>
        <span class="chat-image-chip-error" title="Couldn't load — tap to retry">!</span>
      </Show>
    </button>
  );
}

export function AttachmentChipRow(props: { chat: RefChat; attachments: AttachmentRef[] }) {
  return (
    <Show when={props.attachments.length > 0}>
      <div class="chat-image-row">
        <For each={props.attachments}>{(attachment) => <AttachmentChip chat={props.chat} attachment={attachment} />}</For>
      </div>
    </Show>
  );
}

function AttachmentChip(props: { chat: RefChat; attachment: AttachmentRef }) {
  const kind = () => classifyMedia(props.attachment.mime, props.attachment.path);
  const [loading, setLoading] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  const name = () => props.attachment.path.split('/').pop() ?? props.attachment.path;

  const open = async () => {
    setLoading(true);
    setFailed(false);
    try {
      if (props.attachment.mediaRef) {
        if (!props.chat.media) throw new Error('No media channel');
        const result = await props.chat.media(props.attachment.mediaRef, props.attachment.mime);
        if (kind() === 'image') {
          openPreviewer({ kind: 'image', items: [{ url: result.dataUrl, mime: result.mime, name: name() }], index: 0 });
          return;
        }
        const response = await fetch(result.dataUrl);
        const blob = await response.blob();
        const nonImageKind = kind() as Exclude<ReturnType<typeof kind>, 'image'>;
        if (nonImageKind === 'pdf' || nonImageKind === 'audio' || nonImageKind === 'video') {
          openPreviewer({ kind: nonImageKind, name: name(), mime: result.mime, url: URL.createObjectURL(blob) });
        } else {
          openPreviewer({ kind: nonImageKind, name: name(), mime: result.mime, text: await blob.text() });
        }
        return;
      }
      // No mediaRef (older bridge, or it aged out): fall back to the blob
      // channel by path, registering it first if the host hasn't seen it.
      await openFilePath(props.chat, props.attachment.path);
    } catch {
      setFailed(true);
    } finally {
      setLoading(false);
    }
  };

  return (
    <button type="button" class="chat-image-chip chat-attachment-chip" aria-label={`Open ${name()}`} onClick={() => void open()} disabled={loading()}>
      <Show when={kind() === 'image'} fallback={<FileIcon size={18} class="chat-image-chip-icon" />}>
        <ImageIcon size={18} class="chat-image-chip-icon" />
      </Show>
      <span class="chat-attachment-chip-name truncate">{name()}</span>
      <Show when={failed()}>
        <span class="chat-image-chip-error">!</span>
      </Show>
    </button>
  );
}

/** Chips for `[file: name]@path` mentions in a sent user message (UNI-203):
 * the Composer's generic-file attach now appends these to the outgoing
 * text instead of leaving a raw `@<blob path>` in the draft; the bubble
 * (Transcript.tsx UserBubble) strips the mention lines from its plain
 * text and renders each as a chip here instead, tap → `openFilePath`. */
export function FileMentionChipRow(props: { chat: RefChat; mentions: FileMention[] }) {
  return (
    <Show when={props.mentions.length > 0}>
      <div class="chat-user-files">
        <For each={props.mentions}>{(mention) => <FileMentionChip chat={props.chat} mention={mention} />}</For>
      </div>
    </Show>
  );
}

function FileMentionChip(props: { chat: RefChat; mention: FileMention }) {
  const [loading, setLoading] = createSignal(false);
  const [failed, setFailed] = createSignal(false);
  const open = async () => {
    setLoading(true);
    setFailed(false);
    try {
      const cwd = props.chat.cwd?.();
      await openFilePath(props.chat, resolvePathToken(props.mention.path, cwd));
    } catch {
      setFailed(true);
    } finally {
      setLoading(false);
    }
  };
  return (
    <button type="button" class="chat-image-chip chat-attachment-chip" aria-label={`Open ${props.mention.name}`} onClick={() => void open()} disabled={loading()}>
      <FileIcon size={18} class="chat-image-chip-icon" />
      <span class="chat-attachment-chip-name truncate">{props.mention.name}</span>
      <Show when={failed()}>
        <span class="chat-image-chip-error">!</span>
      </Show>
    </button>
  );
}
