import type { FileTreeIconConfig } from '@pierre/trees'

function builtin(token: string): string {
  return `file-tree-builtin-${token}`
}

/** Pierre FileTree icons: full colored set plus a few common extension gaps. */
export const pierreTreeIcons: FileTreeIconConfig = {
  set: 'complete',
  colored: true,
  byFileExtension: {
    yaml: builtin('yml'),
    yml: builtin('yml'),
    cs: builtin('typescript'),
    csx: builtin('typescript'),
    xml: builtin('html'),
    toml: builtin('yml'),
    config: builtin('text'),
    ps1: builtin('bash'),
    sh: builtin('bash'),
  },
  byFileNameContains: {
    dockerfile: builtin('docker'),
  },
}
