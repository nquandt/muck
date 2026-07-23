import { createContext, useContext, useEffect, useState, type ReactNode } from 'react'
import type { TreeThemeStyles } from '@pierre/trees'
import { getIndexStatus, type IndexedRepo } from './api'
import { usePierreTheme, type ColorMode } from './usePierreTheme'

interface AppContextValue {
  colorMode: ColorMode
  toggleColorMode: () => void
  pierreThemeOptions: ReturnType<typeof usePierreTheme>['pierreThemeOptions']
  treeStyles: TreeThemeStyles
  repos: IndexedRepo[]
}

const AppCtx = createContext<AppContextValue | null>(null)

const COLOR_MODE_STORAGE_KEY = 'muck-color-mode'

function readStoredColorMode(): ColorMode {
  const stored = localStorage.getItem(COLOR_MODE_STORAGE_KEY)
  if (stored === 'light' || stored === 'dark') {
    return stored
  }
  return window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
}

export function AppContextProvider({ children }: { children: ReactNode }) {
  const [colorMode, setColorMode] = useState<ColorMode>(readStoredColorMode)
  const [repos, setRepos] = useState<IndexedRepo[]>([])
  const { pierreThemeOptions, treeStyles } = usePierreTheme(colorMode)

  useEffect(() => {
    localStorage.setItem(COLOR_MODE_STORAGE_KEY, colorMode)
  }, [colorMode])

  useEffect(() => {
    void getIndexStatus()
      .then((status) => setRepos(status.repositories))
      .catch(() => {
        // Best-effort — repo picker/dashboard just stays empty if this fails.
      })
  }, [])

  const value: AppContextValue = {
    colorMode,
    toggleColorMode: () => setColorMode((m) => (m === 'dark' ? 'light' : 'dark')),
    pierreThemeOptions,
    treeStyles,
    repos,
  }

  return <AppCtx.Provider value={value}>{children}</AppCtx.Provider>
}

export function useAppContext(): AppContextValue {
  const ctx = useContext(AppCtx)
  if (!ctx) {
    throw new Error('useAppContext must be used within AppContextProvider')
  }
  return ctx
}
