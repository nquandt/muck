import { codeToHtml, type FileContents } from '@pierre/diffs'

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

/**
 * Syntax-highlights a small block of consecutive source lines for a result-card preview.
 * Returns one HTML string per input line (Shiki wraps each source line in its own top-level
 * `<span class="line">` — that's the granularity we need to keep our own line-number gutter).
 * Falls back to escaped plain text per line if the language is unsupported or the line count
 * doesn't line up (e.g. an unexpected trailing-newline quirk) rather than showing nothing.
 */
export async function highlightLines(
  lines: string[],
  lang: FileContents['lang'],
  theme: string,
): Promise<string[]> {
  if (lines.length === 0) {
    return []
  }

  let html: string
  try {
    html = await codeToHtml(lines.join('\n'), { lang: lang ?? 'text', theme })
  } catch {
    return lines.map(escapeHtml)
  }

  const doc = new DOMParser().parseFromString(html, 'text/html')
  const codeEl = doc.querySelector('code')
  const lineEls = codeEl
    ? Array.from(codeEl.children).filter((el) => el.classList.contains('line'))
    : []

  if (lineEls.length !== lines.length) {
    return lines.map(escapeHtml)
  }

  return lineEls.map((el) => el.innerHTML)
}
