import { BaseStyles, ThemeProvider } from '@primer/react'
import { RouterProvider } from '@tanstack/react-router'
import { AppContextProvider, useAppContext } from './AppContext'
import { router } from './router'

export function App() {
  return (
    <AppContextProvider>
      <ThemedRouter />
    </AppContextProvider>
  )
}

function ThemedRouter() {
  const { colorMode } = useAppContext()
  return (
    <ThemeProvider colorMode={colorMode}>
      <BaseStyles>
        <RouterProvider router={router} />
      </BaseStyles>
    </ThemeProvider>
  )
}
