import { IconButton } from '@primer/react'
import { MoonIcon, SunIcon } from '@primer/octicons-react'
import { Link, Outlet } from '@tanstack/react-router'
import { useAppContext } from '../AppContext'

export function RootLayout() {
  const { colorMode, toggleColorMode } = useAppContext()

  return (
    <div className="app">
      <header className="appHeader">
        <Link to="/" search={{}} className="appTitleLink">
          <h1 className="appTitle">muck</h1>
        </Link>
        <div className="appHeaderSpacer" />
        <IconButton
          aria-label={colorMode === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}
          icon={colorMode === 'dark' ? SunIcon : MoonIcon}
          onClick={toggleColorMode}
          variant="invisible"
        />
      </header>
      <Outlet />
    </div>
  )
}
