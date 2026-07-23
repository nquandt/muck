import { setCustomExtension } from '@pierre/diffs'

/** One-time Pierre/Shiki extension fixes (safe to call multiple times). */
export function initializePierre(): void {
  // Pierre maps `.yml` to Shiki language id `yml`; normalize to `yaml`.
  setCustomExtension('yml', 'yaml')
}

initializePierre()
