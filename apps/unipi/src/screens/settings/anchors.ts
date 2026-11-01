/**
 * Settings anchor ids (WP-A25): the rows the built-in routing and the search
 * land on. Kept next to the catalog so a rename shows up as a test failure
 * (`reachability.test.ts` checks `/changelog` and `/bug` resolve to About).
 */

/** Rows inside About. */
export const ABOUT_ANCHORS = {
  components: 'about-components',
  'whats-new': 'about-whats-new',
  'report-bug': 'about-report-bug',
} as const;

export type AboutAnchor = keyof typeof ABOUT_ANCHORS;
