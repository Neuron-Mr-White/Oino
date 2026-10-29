/**
 * Phone-side image shrinking for chat attachments (UNI-157): the longest
 * side is capped at `MAX_SIDE`, re-encoded as WebP (q0.8) with a JPEG
 * fallback when the WebView can't encode WebP. Re-encoding (canvas/
 * createImageBitmap) also strips EXIF — nothing reads the orientation tag
 * so a canvas draw already undoes any rotation metadata lost.
 */

/** Longest side (in CSS pixels) kept after shrinking. */
export const MAX_SIDE = 1568;
/** WebP/JPEG quality used for the re-encode. */
export const QUALITY = 0.8;
/** Prompts sent inline (base64 in the JSON line) stay under this; bigger
 * goes through the blob channel instead (see `src/lib/blob.ts`). */
export const INLINE_MAX_BYTES = 700 * 1024;

/** The size an image of `width`×`height` should be shrunk to: unchanged if
 * already within `maxSide`, else scaled so the longest side is `maxSide`
 * (aspect ratio kept, dimensions rounded to whole pixels, never below 1). */
export function targetSize(width: number, height: number, maxSide = MAX_SIDE): { width: number; height: number } {
  if (width <= 0 || height <= 0) return { width: Math.max(1, Math.round(width)), height: Math.max(1, Math.round(height)) };
  const longest = Math.max(width, height);
  if (longest <= maxSide) return { width: Math.round(width), height: Math.round(height) };
  const scale = maxSide / longest;
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

export type ShrunkImage = { blob: Blob; mime: string; width: number; height: number };

/** Whether the WebView's canvas can encode WebP (checked once; some older
 * Android WebViews only do JPEG/PNG). */
let webpSupported: boolean | undefined;
function canEncodeWebp(): boolean {
  if (webpSupported !== undefined) return webpSupported;
  try {
    const canvas = document.createElement('canvas');
    canvas.width = 1;
    canvas.height = 1;
    webpSupported = canvas.toDataURL('image/webp').startsWith('data:image/webp');
  } catch {
    webpSupported = false;
  }
  return webpSupported;
}

/** Shrinks `file` to at most `maxSide` on its longest side, re-encoding as
 * WebP (falling back to JPEG) at `quality`. Re-encoding drops EXIF (canvas
 * never carries it through), so orientation metadata can't survive — this
 * relies on `createImageBitmap`'s decode already applying any `orientation`
 * EXIF tag to the pixels it hands back (every Android WebView does). */
export async function shrinkImage(file: Blob, maxSide = MAX_SIDE, quality = QUALITY): Promise<ShrunkImage> {
  const bitmap = await createImageBitmap(file);
  try {
    const { width, height } = targetSize(bitmap.width, bitmap.height, maxSide);
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('canvas 2d context unavailable');
    ctx.drawImage(bitmap, 0, 0, width, height);
    const mime = canEncodeWebp() ? 'image/webp' : 'image/jpeg';
    const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, mime, quality));
    if (!blob) throw new Error('canvas.toBlob returned null');
    return { blob, mime, width, height };
  } finally {
    bitmap.close?.();
  }
}

/** Base64-encodes a blob (no data: prefix), for prompts sent inline. */
export function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error('read failed'));
    reader.onload = () => {
      const result = String(reader.result ?? '');
      const comma = result.indexOf(',');
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.readAsDataURL(blob);
  });
}
