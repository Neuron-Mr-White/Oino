/**
 * Best-effort default device name for pairing (DESIGN.md §3.2): the
 * Android model when it can be read, otherwise the plain word
 * "Phone". There is no Tauri plugin exposing the device model in
 * this stack, so on Android we fall back to parsing it out of the
 * user agent string, which is the only thing available without
 * adding a new native plugin.
 */

function modelFromUserAgent(ua: string): string | undefined {
  // Typical Android UA: "Mozilla/5.0 (Linux; Android 13; Pixel 7) ..."
  const match = ua.match(/Android[^;]*;\s*([^)]+)\)/);
  const model = match?.[1]?.trim();
  return model && model.length > 0 ? model : undefined;
}

/** Resolves a default device name to send during pairing. Never throws. */
export async function defaultDeviceName(): Promise<string> {
  try {
    const { platform } = await import('@tauri-apps/plugin-os');
    if (platform() !== 'android') return 'Phone';
  } catch {
    // Not running under Tauri (browser/tests): fall through to UA sniffing.
  }

  const ua = typeof navigator !== 'undefined' ? navigator.userAgent : '';
  return modelFromUserAgent(ua) ?? 'Phone';
}
