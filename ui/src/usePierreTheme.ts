import { useEffect, useMemo, useState } from 'react'
import { preloadHighlighter, resolveTheme } from '@pierre/diffs'
import { themeToTreeStyles, type TreeThemeStyles } from '@pierre/trees'

export type ColorMode = 'light' | 'dark'

// GitHub-flavored Shiki themes — keeps the code panes feeling familiar rather than
// introducing a whole theme-picker (this is a small local dev tool, not a full app).
export const THEME_NAMES: Record<ColorMode, string> = {
  light: 'github-light',
  dark: 'github-dark',
}

export function usePierreTheme(mode: ColorMode) {
  const themeName = THEME_NAMES[mode]

  const pierreThemeOptions = useMemo(
    () => ({ themeType: mode, theme: { light: themeName, dark: themeName } }),
    [mode, themeName],
  )

  const [treeStyles, setTreeStyles] = useState<TreeThemeStyles>({ height: '100%' })

  useEffect(() => {
    let cancelled = false

    void resolveTheme(themeName).then((theme) => {
      if (!cancelled) {
        setTreeStyles({ height: '100%', ...themeToTreeStyles(theme) })
      }
    })

    void preloadHighlighter({ themes: [themeName], langs: ['text'] })

    return () => {
      cancelled = true
    }
  }, [themeName])

  return { pierreThemeOptions, treeStyles }
}
