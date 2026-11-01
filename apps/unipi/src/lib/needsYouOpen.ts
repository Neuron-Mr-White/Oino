/** The needs-you drawer's open state (FloatingBell owns the drawer; the shell bells only open it). Leaf module: no imports from components. */
import { createSignal } from 'solid-js';

const [needsYouOpen, setNeedsYouOpen] = createSignal(false);
export { needsYouOpen, setNeedsYouOpen };
