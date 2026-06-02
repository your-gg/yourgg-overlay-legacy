import type { Overlay } from '@your-gg/yourgg-core';

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
