import { getFiletypeFromFileName, type FileContents } from '@pierre/diffs'

/** Strip leading slashes and normalize separators for Pierre file names. */
export function normalizePierreFileName(path: string): string {
  const normalized = path.trim().replace(/^\/+/, '').replace(/\\/g, '/')
  const segments = normalized.split('/')
  const base = segments[segments.length - 1] ?? normalized
  const dotIndex = base.lastIndexOf('.')
  if (dotIndex <= 0) {
    return base
  }

  return `${base.slice(0, dotIndex)}.${base.slice(dotIndex + 1).toLowerCase()}`
}

/** Case-insensitive extension lookup with Pierre extension overrides applied. */
export function detectPierreLanguage(fileName: string): FileContents['lang'] {
  const lang = getFiletypeFromFileName(fileName)
  return lang === 'yml' ? 'yaml' : lang
}

export function toPierreFile(path: string, contents: string): FileContents {
  const name = normalizePierreFileName(path)
  return {
    name,
    contents,
    lang: detectPierreLanguage(name),
  }
}
