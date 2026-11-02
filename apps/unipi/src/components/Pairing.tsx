import { can, isTauri, platform } from '../lib/platform';
import { QrCode, Link, SquareTerminal, Copy, ClipboardPaste, Smartphone, CameraOff } from 'lucide-solid';
import { openPairPhone } from '../lib/pairPhone';
import crab from '../assets/unicrab-pixel.png';
import Drawer from "corvu/drawer";
import { BoardSheet } from "./board/BoardSheet";
import { createSignal, onMount, onCleanup, Show } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { z } from "zod";
import { call } from "../lib/ipc";
import { parseHostLink } from "../lib/link";
import { HostInfoSchema } from "../lib/schemas";
import { useHosts } from "../hosts/context";
import { useSettings } from "../settings/context";
import { syncSettingsHost } from "../settings/target";
import { ErrorBanner } from "./ErrorBanner";
import { QrScanner } from "./QrScanner";
import type { AppError } from "@unipi/protocol";
export const [addHostOpen, setAddHostOpen] = createSignal(false);

/** What to call this device in the pairing copy: the phone app says "phone". */
export function deviceNoun(): string {
  const p = platform();
  return p.mode === 'tauri-mobile' ? 'phone' : p.mode === 'tauri-desktop' ? 'computer' : 'browser';
}

/** Default "Device name" (shown on the host in `unipi-host devices`). */
export function defaultDeviceName(): string {
  const p = platform();
  if (p.mode === 'tauri-mobile') return 'Phone';
  const os = { linux: 'Linux', windows: 'Windows', macos: 'Mac' }[p.os as 'linux' | 'windows' | 'macos'];
  return os ? `${os} desktop` : 'Desktop';
}

type ScannerModule = typeof import("@tauri-apps/plugin-barcode-scanner");

/** Reads the clipboard through the Tauri plugin, falling back to the web API. */
async function readClipboard(): Promise<string | undefined> {
  try {
    if (isTauri()) {
      const clipboard = await import("@tauri-apps/plugin-clipboard-manager");
      return await clipboard.readText();
    }
  } catch {
    // fall through to the web API
  }
  try {
    return await navigator.clipboard.readText();
  } catch {
    return undefined;
  }
}
export function Pairing() {
  const hosts = useHosts();
  // Settings' host-scoped sections follow the selected PC on the phone (PH-04).
  syncSettingsHost(hosts);
  const [settings] = useSettings();
  const navigate = useNavigate();
  const [link, setLink] = createSignal("");
  const [device, setDevice] = createSignal("Phone");
  const [confirm, setConfirm] = createSignal(false);
  const [paste, setPaste] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<AppError>();
  const [toast, setToast] = createSignal("");
  const [hostId, setHostId] = createSignal("");
  const parsed = () => parseHostLink(link());
  const [scanning, setScanning] = createSignal(false);
  const [cameraDenied, setCameraDenied] = createSignal(false);
  let scanner: ScannerModule | undefined;
  const accept = async (value: string) => {
    setLink(value);
    const p = parseHostLink(value);
    if (p.ok) {
      const preview = await call(
        "pairing_preview",
        { link: value.trim() },
        z.object({ id: z.string() }),
      );
      if (!preview.ok) {
        console.warn("[pair] preview failed", preview.error);
        setError(preview.error);
        return;
      }
      setHostId(preview.value.id);
      setDevice((window as Window & {unipiDeviceModel?:string}).unipiDeviceModel || defaultDeviceName());
      setError(undefined);
      setPaste(false);
      // Close the add-host sheet first, then open the confirm sheet.
      // The add-host sheet must not restore focus on close (see
      // restoreFocus below): focus landing back on "Add a host" is
      // "outside" the confirm sheet and dismissed it immediately —
      // the paste → Continue bug.
      setAddHostOpen(false);
      setConfirm(true);
    } else {
      console.warn("[pair] invalid link", p.error);
      setError(p.error);
    }
  };
  const stopScan = () => {
    if (!scanning()) return;
    setScanning(false);
    // The plugin's cancel() drops the pending scan promise without
    // rejecting it, so nothing else waits on it.
    void scanner?.cancel().catch((e) => console.warn("[scan] cancel failed", e));
  };
  /** Camera refused: say so calmly and open the code entry instead. */
  const cameraOff = () => {
    setCameraDenied(true);
    setError(undefined);
    setPaste(true);
    setAddHostOpen(true);
  };
  /** From the scanner: give up on the camera and type the code. */
  const enterCodeInstead = () => {
    stopScan();
    setPaste(true);
    setAddHostOpen(true);
  };
  const startScan = async () => {
    setError(undefined);
    setCameraDenied(false);
    if (!isTauri() || !can("camera")) {
      setError({ code: "E_PAIRING_INVALID", message: isTauri() ? "QR scanning works in Unipi on your phone. Paste the pairing link instead." : "QR scanning needs the Unipi app", retryable: false });
      return;
    }
    try {
      scanner = await import("@tauri-apps/plugin-barcode-scanner");
      let permission = await scanner.checkPermissions();
      if (permission !== "granted") permission = await scanner.requestPermissions();
      if (permission !== "granted") {
        // Not an error: a calm note with the code fallback opened right away.
        cameraOff();
        return;
      }
      setAddHostOpen(false);
      setScanning(true);
      const result = await scanner.scan({ windowed: true, formats: [scanner.Format.QRCode] });
      if (!scanning()) return;
      setScanning(false);
      await accept(result.content);
      if (error()) setAddHostOpen(true);
    } catch (e) {
      const wasScanning = scanning();
      setScanning(false);
      if (String(e).includes("cancel")) return;
      console.warn("[scan] failed", e);
      // A permission refusal can also surface as a rejected promise.
      if (!wasScanning && /permission|denied/i.test(String(e))) {
        cameraOff();
        return;
      }
      setError({ code: "E_PAIRING_INVALID", message: `Scan failed: ${String(e)}`, retryable: false });
      if (wasScanning) setAddHostOpen(true);
    }
  };
  onMount(async () => {
    if (!isTauri() || !can("deepLinks")) return;
    try {
      const { getCurrent, onOpenUrl } =
        await import("@tauri-apps/plugin-deep-link");
      const current = await getCurrent();
      if (current?.[0]) accept(current[0]);
      const unlisten = await onOpenUrl((urls) => {
        if (urls[0]) accept(urls[0]);
      });
      onCleanup(unlisten);
    } catch (e) {
      console.warn("deep link unavailable", e);
    }
  });
  const pair = async () => {
    setBusy(true);
    const relay = settings.connection.relay;
    const result = await call(
      "pair_host",
      {
        link: link(),
        deviceName: device(),
        relay: relay.mode === "custom" ? relay.url : "n0",
      },
      HostInfoSchema,
    );
    setBusy(false);
    if (result.ok) {
      setConfirm(false);
      await hosts.refresh();
      setToast("Host paired");
      setTimeout(() => setToast(""), 2500);
      navigate("/");
    } else setError(result.error);
  };
  return (
    <>
      <Show when={toast()}>
        <div
          role="status"
          class="fixed top-12 left-5 right-5 bg-surface-2 p-3 z-30 rounded-card"
          onClick={() => setToast("")}
        >
          {toast()}
        </div>
      </Show>
      <Show when={scanning()}>
        <QrScanner onClose={() => { stopScan(); setAddHostOpen(true); }} onEnterCode={enterCodeInstead} />
      </Show>
      <BoardSheet open={addHostOpen()} onClose={() => setAddHostOpen(false)} label="Add a PC" testId="add-host-sheet" class="pair-sheet">
            <p class="pair-intro">Pair Unipi with your own PC.</p>
            <Show
              when={can("camera")}
              fallback={
                <>
                  {/* No camera here (desktop app / browser): the link is the way; QR is for the phone. */}
                  <button class="option-card" onClick={() => setPaste(true)}>
                    <span class="option-tile"><Link size={24}/></span><span class="flex-1"><span class="block font-mono text-[11px] tracking-widest text-claw mb-2">RECOMMENDED</span><strong class="block text-base font-medium">Paste pairing link</strong><span class="block text-sm text-text-muted mt-2">Run <code>unipi-host pair</code> on the PC and paste the link it prints.</span><span class="block text-xs text-text-dim mt-3">~1 min</span></span>
                  </button>
                  <Show when={!paste()}>
                    <button disabled class="option-card text-text-dim">
                      <span class="option-tile"><QrCode size={24}/></span><span><strong class="block">Pair with QR</strong><span class="block text-xs mt-2">Scan the code with Unipi on your phone</span></span>
                    </button>
                  </Show>
                </>
              }
            >
              <button class="option-card" onClick={() => void startScan()}>
                <span class="option-tile"><QrCode size={24}/></span><span class="flex-1"><span class="block font-mono text-[11px] tracking-widest text-claw mb-2">RECOMMENDED</span><strong class="block text-base font-medium">Pair with QR</strong><span class="block text-sm text-text-muted mt-2">Run <code>unipi-host pair</code> on your computer and scan the code.</span><span class="block text-xs text-text-dim mt-3">~1 min</span></span>
              </button>
              <button class="option-card" onClick={() => setPaste(true)}>
                <span class="option-tile"><Link size={24}/></span><span><strong class="block text-base font-medium">Paste pairing link</strong><span class="block text-sm text-text-muted mt-2">Copy the link printed by unipi-host pair.</span><span class="block text-xs text-text-dim mt-3">~1 min</span></span>
              </button>
            </Show>
            <Show when={!paste()}><button disabled class="option-card text-text-dim">
              <span class="option-tile"><SquareTerminal size={24}/></span><span><strong class="block">Connect with SSH</strong><span class="block text-xs mt-2">Coming in M3</span></span>
            </button></Show>
            <Show when={cameraDenied()}>
              <div role="status" class="pair-camera-off" data-testid="camera-off-note">
                <CameraOff size={20} class="pair-camera-off-icon" />
                <div class="flex-1">
                  <strong class="block text-base font-medium">The camera is off for Unipi</strong>
                  <span class="block text-sm text-text-muted mt-1">That's fine. Enter the pairing code instead: paste the link <code>unipi-host pair</code> prints, or its two codes.</span>
                  <button
                    type="button"
                    class="pair-camera-off-settings"
                    onClick={() => void scanner?.openAppSettings().catch((e) => console.warn("[scan] settings", e))}
                  >
                    Allow the camera in Settings
                  </button>
                </div>
              </div>
            </Show>
            <Show when={paste()}>
              <div class="pair-paste">
                <div class="relative">
                  <input
                    ref={(el) => setTimeout(() => el.focus(), 50)}
                    aria-label="Pairing link"
                    class="w-full rounded-input bg-surface-3 py-3 pl-3 pr-12 font-mono text-[13px]"
                    value={link()}
                    onInput={(e) => setLink(e.currentTarget.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") void accept(link()); }}
                    placeholder={cameraDenied() ? "Pairing link or code" : "unipi://pair?t=…&c=…"}
                    autocapitalize="off"
                    autocorrect="off"
                    spellcheck={false}
                  />
                  <button
                    type="button"
                    aria-label="Paste from clipboard"
                    class="absolute right-1 top-1/2 grid h-10 w-10 -translate-y-1/2 place-items-center rounded-input text-text-muted"
                    style={{ "min-height": "40px" }}
                    onClick={async () => {
                      const text = await readClipboard();
                      if (text) setLink(text.trim());
                    }}
                  >
                    <ClipboardPaste size={18} />
                  </button>
                </div>
                <button
                  type="button"
                  class="pair-primary mt-3"
                  disabled={link().trim() === ""}
                  onClick={() => void accept(link())}
                >
                  Continue
                </button>
              </div>
            </Show>
            <Show when={error()}>{(e) => <div class="mt-4"><ErrorBanner error={e()} /></div>}</Show>
            {/* UNI-266: the other direction — this PC shows the QR, the phone scans it. */}
            <Show when={can("localSharing")}>
              <button type="button" class="pair-phone-link" data-testid="add-host-pair-phone" onClick={() => { setAddHostOpen(false); openPairPhone(); }}>
                <Smartphone size={16} /> Pair a phone with this PC
              </button>
            </Show>
      </BoardSheet>
      <Drawer open={confirm()} onOpenChange={setConfirm} side="bottom" closeOnOutsideFocus={false}>
        <Drawer.Portal>
          <Drawer.Overlay class="sheet-overlay" />
          <Drawer.Content class="sheet">
            <img src={crab} alt="Unicrab" class="pair-mascot"/><Drawer.Label class="pair-title">
              Pair with{" "}
              {parsed().ok
                ? (parsed() as { ok: true; value: { name?: string } }).value
                    .name || "Host"
                : "Host"}
            </Drawer.Label>
            <Drawer.Description class="text-text-muted my-4 text-center text-sm">
              This {deviceNoun()} will be able to open terminals on{" "}
              {parsed().ok
                ? (parsed() as { ok: true; value: { name?: string } }).value
                    .name || "this host"
                : "this host"}
            </Drawer.Description>
            <div class="bg-surface-2 rounded-input p-4 mb-5 text-[13px]">
              <div class="flex justify-between gap-3"><span class="text-text-muted">Host</span><span>{parsed().ok ? (parsed() as {ok:true,value:{name?:string}}).value.name || 'Host' : 'Host'}</span></div>
              <div class="flex items-center justify-between gap-3 mt-3"><span class="text-text-muted">ID</span><span class="font-mono ml-auto text-right">{hostId().slice(0,8)}</span><button class="text-text-muted w-8 h-8 rounded-pill bg-surface-3 inline-grid place-items-center" style={{"min-height":"32px"}} aria-label="Copy host ID" onClick={()=>navigator.clipboard.writeText(hostId())}><Copy size={14}/></button></div>
            </div>
            <label class="text-sm">
              Device name
              <input
                class="block w-full bg-surface-3 rounded-input p-3 my-3"
                value={device()}
                onInput={(e) => setDevice(e.currentTarget.value)}
              />
            </label>
            <Show when={error()}>{(e) => <ErrorBanner error={e()} />}</Show>
            <button
              disabled={
                busy() || !z.string().min(1).max(64).safeParse(device()).success
              }
              class="pair-primary mt-4"
              onClick={pair}
            >
              {busy() ? "Pairing…" : "Pair"}
            </button>
            <Drawer.Close class="pair-secondary mt-3">Cancel</Drawer.Close>
          </Drawer.Content>
        </Drawer.Portal>
      </Drawer>
    </>
  );
}
