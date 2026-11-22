/** `file://` URI helpers (POSIX paths — hosts are PCs). */

export function fileUri(path: string): string {
  return `file://${path.split('/').map(encodeURIComponent).join('/')}`;
}
