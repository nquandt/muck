import { createRootRoute, createRoute, createRouter } from '@tanstack/react-router'
import { RootLayout } from './routes/RootLayout'
import { SearchPage, type SearchPageSearch } from './routes/SearchPage'
import { BrowsePage, type BrowsePageSearch } from './routes/BrowsePage'

const rootRoute = createRootRoute({
  component: RootLayout,
})

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: SearchPage,
  validateSearch: (search: Record<string, unknown>): SearchPageSearch => {
    const result: SearchPageSearch = {}
    if (typeof search.q === 'string') result.q = search.q
    if (search.regex === true || search.regex === 'true') result.regex = true
    if (typeof search.frepos === 'string') result.frepos = search.frepos
    if (typeof search.ftypes === 'string') result.ftypes = search.ftypes
    if (typeof search.orgs === 'string') result.orgs = search.orgs
    if (typeof search.branches === 'string') result.branches = search.branches
    if (typeof search.cursor === 'string') result.cursor = search.cursor
    return result
  },
})

const browseRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/repo/$repoId/tree/$',
  component: BrowsePage,
  validateSearch: (search: Record<string, unknown>): BrowsePageSearch =>
    typeof search.line === 'number' ? { line: search.line } : {},
})

const routeTree = rootRoute.addChildren([indexRoute, browseRoute])

export const router = createRouter({ routeTree })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
