import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import { Button, Checkbox, Spinner, TextInput } from '@primer/react'
import { SearchIcon, XIcon } from '@primer/octicons-react'
import { useNavigate, useSearch } from '@tanstack/react-router'
import { search, type SearchFacet, type SearchResult } from '../api'
import { useAppContext } from '../AppContext'
import { ResultCard } from '../components/ResultCard'
import { THEME_NAMES } from '../usePierreTheme'

export interface SearchPageSearch {
  q?: string
  regex?: boolean
  frepos?: string
  ftypes?: string
  orgs?: string
  branches?: string
  cursor?: string
}

const csvSet = (value: string) => new Set(value.split(',').filter(Boolean))

export function SearchPage() {
  const { colorMode, repos } = useAppContext()
  const navigate = useNavigate({ from: '/' })
  const urlState = useSearch({ from: '/' })

  const [query, setQuery] = useState(urlState.q ?? '')
  const [regex, setRegex] = useState(!!urlState.regex)
  const [results, setResults] = useState<SearchResult[]>([])
  const [facets, setFacets] = useState<SearchFacet[]>([])
  const [selectedRepoFacets, setSelectedRepoFacets] = useState<Set<string>>(() => csvSet(urlState.frepos ?? ''))
  const [selectedFileTypeFacets, setSelectedFileTypeFacets] = useState<Set<string>>(() => csvSet(urlState.ftypes ?? ''))
  const [selectedOrgFacets, setSelectedOrgFacets] = useState<Set<string>>(() => csvSet(urlState.orgs ?? ''))
  const [selectedBranchFacets, setSelectedBranchFacets] = useState<Set<string>>(() => csvSet(urlState.branches ?? ''))
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const [pageCursors, setPageCursors] = useState<(string | undefined)[]>([urlState.cursor || undefined])
  const [pageIndex, setPageIndex] = useState(0)
  const [nextCursor, setNextCursor] = useState<string | null>(null)

  const resultsScrollRef = useRef<HTMLDivElement>(null)

  const activeFilters = useMemo(
    () => ({
      repoIds: selectedRepoFacets.size > 0 ? Array.from(selectedRepoFacets) : undefined,
      fileTypes: selectedFileTypeFacets.size > 0 ? Array.from(selectedFileTypeFacets) : undefined,
      orgs: selectedOrgFacets.size > 0 ? Array.from(selectedOrgFacets) : undefined,
      branches: selectedBranchFacets.size > 0 ? Array.from(selectedBranchFacets) : undefined,
    }),
    [selectedRepoFacets, selectedFileTypeFacets, selectedOrgFacets, selectedBranchFacets],
  )

  const fetchPage = useCallback(
    async (index: number, cursor: string | undefined) => {
      setLoading(true)
      setError(null)
      try {
        const response = await search(query, regex, cursor, activeFilters)
        setResults(response.results)
        setFacets(response.facets)
        setNextCursor(response.nextCursor)
        setPageIndex(index)
      } catch (caught) {
        setError(caught instanceof Error ? caught.message : 'Search failed')
      } finally {
        setLoading(false)
      }
    },
    [query, regex, activeFilters],
  )

  const runSearch = useCallback(async () => {
    if (!query.trim()) {
      setResults([])
      setFacets([])
      setPageCursors([undefined])
      setNextCursor(null)
      return
    }
    setPageCursors([undefined])
    await fetchPage(0, undefined)
  }, [query, fetchPage])

  const didRestoreInitialSearch = useRef(false)
  useEffect(() => {
    if (didRestoreInitialSearch.current) {
      return
    }
    didRestoreInitialSearch.current = true
    if (urlState.q?.trim()) {
      void fetchPage(0, urlState.cursor || undefined)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const isFirstQueryRender = useRef(true)
  useEffect(() => {
    if (isFirstQueryRender.current) {
      isFirstQueryRender.current = false
      return
    }
    if (!query.trim()) {
      setResults([])
      setFacets([])
      setPageCursors([undefined])
      setNextCursor(null)
      return
    }
    const timer = setTimeout(() => {
      setPageCursors([undefined])
      void fetchPage(0, undefined)
    }, 300)
    return () => clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, regex])

  const isFirstFilterRender = useRef(true)
  useEffect(() => {
    if (isFirstFilterRender.current) {
      isFirstFilterRender.current = false
      return
    }
    if (!query.trim()) {
      return
    }
    setPageCursors([undefined])
    void fetchPage(0, undefined)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedRepoFacets, selectedFileTypeFacets, selectedOrgFacets, selectedBranchFacets])

  // Mirror state into the URL search params so the address bar stays shareable.
  useEffect(() => {
    void navigate({
      search: () => ({
        q: query || undefined,
        regex: regex || undefined,
        frepos: selectedRepoFacets.size > 0 ? Array.from(selectedRepoFacets).join(',') : undefined,
        ftypes: selectedFileTypeFacets.size > 0 ? Array.from(selectedFileTypeFacets).join(',') : undefined,
        orgs: selectedOrgFacets.size > 0 ? Array.from(selectedOrgFacets).join(',') : undefined,
        branches: selectedBranchFacets.size > 0 ? Array.from(selectedBranchFacets).join(',') : undefined,
        cursor: pageCursors[pageIndex] || undefined,
      }),
      replace: true,
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, regex, selectedRepoFacets, selectedFileTypeFacets, selectedOrgFacets, selectedBranchFacets, pageCursors, pageIndex])

  const goToNextPage = useCallback(() => {
    if (!nextCursor) {
      return
    }
    setPageCursors((prev) => [...prev.slice(0, pageIndex + 1), nextCursor])
    void fetchPage(pageIndex + 1, nextCursor)
  }, [nextCursor, pageIndex, fetchPage])

  const goToPreviousPage = useCallback(() => {
    if (pageIndex === 0) {
      return
    }
    void fetchPage(pageIndex - 1, pageCursors[pageIndex - 1])
  }, [pageIndex, pageCursors, fetchPage])

  const toggleFacet = useCallback(
    (setter: Dispatch<SetStateAction<Set<string>>>) => (value: string) => {
      setter((prev) => {
        const next = new Set(prev)
        if (next.has(value)) {
          next.delete(value)
        } else {
          next.add(value)
        }
        return next
      })
    },
    [],
  )
  const toggleRepoFacet = useMemo(() => toggleFacet(setSelectedRepoFacets), [toggleFacet])
  const toggleFileTypeFacet = useMemo(() => toggleFacet(setSelectedFileTypeFacets), [toggleFacet])
  const toggleOrgFacet = useMemo(() => toggleFacet(setSelectedOrgFacets), [toggleFacet])
  const toggleBranchFacet = useMemo(() => toggleFacet(setSelectedBranchFacets), [toggleFacet])

  const clearFacetFilters = useCallback(() => {
    setSelectedRepoFacets(new Set())
    setSelectedFileTypeFacets(new Set())
    setSelectedOrgFacets(new Set())
    setSelectedBranchFacets(new Set())
  }, [])

  const repoFacets = useMemo(() => facets.filter((f) => f.type === 'repo'), [facets])
  const fileTypeFacets = useMemo(() => facets.filter((f) => f.type === 'file_type'), [facets])
  const orgFacets = useMemo(() => facets.filter((f) => f.type === 'org'), [facets])
  const branchFacets = useMemo(() => facets.filter((f) => f.type === 'branch'), [facets])
  const hasActiveFilters =
    selectedRepoFacets.size > 0 || selectedFileTypeFacets.size > 0 || selectedOrgFacets.size > 0 || selectedBranchFacets.size > 0

  const handleResultClick = useCallback(
    (result: SearchResult) => {
      void navigate({
        to: '/repo/$repoId/tree/$',
        params: { repoId: result.repoId, _splat: result.path },
        search: { line: result.line },
      })
    },
    [navigate],
  )

  return (
    <div className="searchPage">
      <div className="searchBar">
        <TextInput
          className="searchInput"
          leadingVisual={SearchIcon}
          placeholder="Search across all indexed repositories..."
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              void runSearch()
            }
          }}
          autoFocus
        />
        <label className="regexToggle">
          <Checkbox checked={regex} onChange={(event) => setRegex(event.target.checked)} />
          Regex
        </label>
        <Button variant="primary" onClick={() => void runSearch()} disabled={loading || !query.trim()}>
          {loading ? <Spinner size="small" /> : 'Search'}
        </Button>
      </div>

      {error ? <div className="errorBanner">{error}</div> : null}

      {!query ? (
        <div className="repoDashboard">
          <div className="repoDashboardLabel">Indexed repositories</div>
          {repos.length === 0 ? (
            <div className="emptyState">No repositories indexed yet.</div>
          ) : (
            <div className="repoDashboardGrid">
              {repos.map((repo) => (
                <a
                  key={repo.repoId}
                  className="repoCard"
                  href={`/repo/${encodeURIComponent(repo.repoId)}/tree/`}
                  onClick={(event) => {
                    event.preventDefault()
                    void navigate({ to: '/repo/$repoId/tree/$', params: { repoId: repo.repoId, _splat: '' } })
                  }}
                >
                  <div className="repoCardName">{repo.repoName}</div>
                  <div className="repoCardMeta">
                    {repo.org ? <span>{repo.org}</span> : null}
                    {repo.branch ? <span>{repo.branch}</span> : null}
                    <span className={`repoCardStatus repoCardStatus--${repo.status}`}>{repo.status}</span>
                  </div>
                </a>
              ))}
            </div>
          )}
        </div>
      ) : (
        <div className="searchBody">
          {facets.length > 0 ? (
            <div className="filtersPane">
              <div className="filtersPaneHeader">
                <span>Filters</span>
                {hasActiveFilters ? (
                  <button type="button" className="filtersClear" onClick={clearFacetFilters}>
                    <XIcon size={12} /> Clear
                  </button>
                ) : null}
              </div>
              {repoFacets.length > 0 ? (
                <div className="facetGroup">
                  <div className="facetGroupLabel">Repository</div>
                  {repoFacets.map((facet) => (
                    <label key={facet.name} className="facetRow">
                      <Checkbox checked={selectedRepoFacets.has(facet.name)} onChange={() => toggleRepoFacet(facet.name)} />
                      <span className="facetName">{facet.name}</span>
                      <span className="facetCount">{facet.count}</span>
                    </label>
                  ))}
                </div>
              ) : null}
              {orgFacets.length > 0 ? (
                <div className="facetGroup">
                  <div className="facetGroupLabel">Organization</div>
                  {orgFacets.map((facet) => (
                    <label key={facet.name} className="facetRow">
                      <Checkbox checked={selectedOrgFacets.has(facet.name)} onChange={() => toggleOrgFacet(facet.name)} />
                      <span className="facetName">{facet.name}</span>
                      <span className="facetCount">{facet.count}</span>
                    </label>
                  ))}
                </div>
              ) : null}
              {branchFacets.length > 0 ? (
                <div className="facetGroup">
                  <div className="facetGroupLabel">Branch</div>
                  {branchFacets.map((facet) => (
                    <label key={facet.name} className="facetRow">
                      <Checkbox checked={selectedBranchFacets.has(facet.name)} onChange={() => toggleBranchFacet(facet.name)} />
                      <span className="facetName">{facet.name}</span>
                      <span className="facetCount">{facet.count}</span>
                    </label>
                  ))}
                </div>
              ) : null}
              {fileTypeFacets.length > 0 ? (
                <div className="facetGroup">
                  <div className="facetGroupLabel">File type</div>
                  {fileTypeFacets.map((facet) => (
                    <label key={facet.name} className="facetRow">
                      <Checkbox
                        checked={selectedFileTypeFacets.has(facet.name)}
                        onChange={() => toggleFileTypeFacet(facet.name)}
                      />
                      <span className="facetName">{facet.name}</span>
                      <span className="facetCount">{facet.count}</span>
                    </label>
                  ))}
                </div>
              ) : null}
            </div>
          ) : null}

          <div className="mainPane">
            <div className="resultsScroll" ref={resultsScrollRef}>
              {results.map((result, idx) => (
                <ResultCard
                  key={`${result.repoId}-${result.path}-${result.line}-${idx}`}
                  result={result}
                  codeTheme={THEME_NAMES[colorMode]}
                  onClick={() => handleResultClick(result)}
                />
              ))}
              {!loading && results.length === 0 ? <div className="emptyState">No results found for "{query}"</div> : null}
            </div>
            {!loading && results.length > 0 && (pageIndex > 0 || nextCursor) ? (
              <div className="pagination">
                <Button size="small" onClick={goToPreviousPage} disabled={pageIndex === 0}>
                  Previous
                </Button>
                <span className="paginationPage">Page {pageIndex + 1}</span>
                <Button size="small" onClick={goToNextPage} disabled={!nextCursor}>
                  Next
                </Button>
              </div>
            ) : null}
          </div>
        </div>
      )}
    </div>
  )
}
