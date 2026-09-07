import type { Overlay } from '@your-gg/yourgg-core';

export * from './game-overlay/index.js';

/**
 * Describe a window in `Overlay`.
 */
export type OverlayWindow = {
  /**
   * Associated `Overlay` instance.
   */
  overlay: Overlay,

  /**
   * Window id.
   */
  id: number,
};
