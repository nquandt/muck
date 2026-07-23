import { useCallback, useEffect, useMemo, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import { BaseStyles, Button, Checkbox, FormControl, IconButton, Select, Spinner, ThemeProvider, TextInput } from '@primer/react'
import { ArrowLeftIcon, MoonIcon, SearchIcon, SunIcon, XIcon } from '@primer/octicons-react'
import { File as PierreFile } from '@pierre/diffs/react'
import { FileTree, useFileTree } from '@pierre/trees/react'
import {
  getFile,
  getIndexStatus,
  getTree,
  search,
  type FileContentResponse,
  type IndexedRepo,
  type SearchFacet,
  type SearchResult,
} from './api'
import { highlightLines } from './lib/highlightSnippet'
import { detectPierreLanguage, toPierreFile } from './lib/pierreFile'
import { pierreTreeIcons } from './lib/pierreTreeIcons'
import { THEME_NAMES, usePierreTheme, type ColorMode } from './usePierreTheme'

type ViewMode = 'results' | 'file'

interface ResultCardProps {
  result: SearchResult
  codeTheme: string
  onClick: () => void
}

function ResultCard({ result, codeTheme, onClick }: ResultCardProps) {
  const allLines = useMemo(
    () => [...result.contextBefore, result.snippet, ...result.contextAfter],
    [result],
  )
  const matchIndex = result.contextBefore.length
  const startLine = result.line - matchIndex

  const [highlighted, setHighlighted] = useState<string[] | null>(null)

  useEffect(() => {
    let cancelled = false
    setHighlighted(null)
    const lang = detectPierreLanguage(result.path)
    void highlightLines(allLines, lang, codeTheme).then((lines) => {
      if (!cancelled) {
        setHighlighted(lines)
      }
    })
    return () => {
      cancelled = true
    }
  }, [allLines, result.path, codeTheme])

  return (
    <div className="resultCard" onClick={onClick} role="button" tabIndex={0}>
      <div className="resultCardHeader">
        <span className="resultRepo">{result.repoName}</span>
        {result.org ? <span className="resultOrg">{result.org}</span> : null}
        {result.branch ? <span className="resultBranch">{result.branch}</span> : null}
        <span className="resultPath">{result.path}</span>
        <span className="resultLineBadge">line {result.line}</span>
      </div>
      <pre className="resultCardCode">
        {allLines.map((line, i) => (
          <div className={`resultCodeLine${i === matchIndex ? ' resultCodeLine--match' : ''}`} key={i}>
            <span className="resultLineNumber">{startLine + i}</span>
            {highlighted ? (
              <span className="resultLineText" dangerouslySetInnerHTML={{ __html: highlighted[i] }} />
            ) : (
              <span className="resultLineText">{line}</span>
            )}
          </div>
        ))}
      </pre>
    </div>
  )
}

const COLOR_MODE_STORAGE_KEY = 'xgrep-color-mode'

function readStoredColorMode(): ColorMode {
  const stored = localStorage.getItem(COLOR_MODE_STORAGE_KEY)
  if (stored === 'light' || stored === 'dark') {
    return stored
  }
  return window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
}

// --- Shareable URL state -----------------------------------------------------------------
// Every piece of state that changes what's on screen (query, filters, page, open file/line)
// is mirrored into the URL's query string via history.replaceState, and read back out once
// on first load — so copying the address bar reproduces the same view for someone else.

interface UrlState {
  query: string
  regex: boolean
  selectedRepoId: string
  repoFacets: string[]
  fileTypeFacets: string[]
  orgFacets: string[]
  branchFacets: string[]
  cursor: string | null
  viewMode: ViewMode
  filePath: string | null
  highlightLine: number | null
}

function readUrlState(): UrlState {
  const params = new URLSearchParams(window.location.search)
  const csv = (key: string) => params.get(key)?.split(',').filter(Boolean) ?? []
  const line = params.get('line')
  return {
    query: params.get('q') ?? '',
    regex: params.get('regex') === '1',
    selectedRepoId: params.get('repo') ?? '',
    repoFacets: csv('frepos'),
    fileTypeFacets: csv('ftypes'),
    orgFacets: csv('orgs'),
    branchFacets: csv('branches'),
    cursor: params.get('cursor'),
    viewMode: params.get('view') === 'file' ? 'file' : 'results',
    filePath: params.get('path'),
    highlightLine: line ? Number(line) || null : null,
  }
}

const initialUrlState = readUrlState()

export function App() {
  const [colorMode, setColorMode] = useState<ColorMode>(readStoredColorMode)

  useEffect(() => {
    localStorage.setItem(COLOR_MODE_STORAGE_KEY, colorMode)
  }, [colorMode])

  return (
    <ThemeProvider colorMode={colorMode}>
      <BaseStyles>
        <AppContent colorMode={colorMode} onToggleColorMode={() => setColorMode((m) => (m === 'dark' ? 'light' : 'dark'))} />
      </BaseStyles>
    </ThemeProvider>
  )
}

function AppContent({ colorMode, onToggleColorMode }: { colorMode: ColorMode; onToggleColorMode: () => void }) {
  const { pierreThemeOptions, treeStyles } = usePierreTheme(colorMode)

  const [repos, setRepos] = useState<IndexedRepo[]>([])
  const [selectedRepoId, setSelectedRepoId] = useState<string>(initialUrlState.selectedRepoId)

  const [query, setQuery] = useState(initialUrlState.query)
  const [regex, setRegex] = useState(initialUrlState.regex)
  const [results, setResults] = useState<SearchResult[]>([])
  const [facets, setFacets] = useState<SearchFacet[]>([])
  const [selectedRepoFacets, setSelectedRepoFacets] = useState<Set<string>>(new Set(initialUrlState.repoFacets))
  const [selectedFileTypeFacets, setSelectedFileTypeFacets] = useState<Set<string>>(
    new Set(initialUrlState.fileTypeFacets),
  )
  const [selectedOrgFacets, setSelectedOrgFacets] = useState<Set<string>>(new Set(initialUrlState.orgFacets))
  const [selectedBranchFacets, setSelectedBranchFacets] = useState<Set<string>>(
    new Set(initialUrlState.branchFacets),
  )
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // pageCursors[i] is the cursor used to fetch page i (undefined for the first page);
  // pageCursors[pageIndex + 1], if present, is what advances to the next page. A cursor
  // restored from the URL becomes page 1 with no history behind it (Previous is disabled) —
  // it's just the opaque offset xgrep-server gave out, not a real page number.
  const [pageCursors, setPageCursors] = useState<(string | undefined)[]>([
    initialUrlState.cursor ?? undefined,
  ])
  const [pageIndex, setPageIndex] = useState(0)
  const [nextCursor, setNextCursor] = useState<string | null>(null)

  const [viewMode, setViewMode] = useState<ViewMode>(initialUrlState.viewMode)
  // The line to highlight/scroll to in the currently-open file — set when a file is opened
  // from a search result, cleared when opened by clicking the tree directly.
  const [highlightLine, setHighlightLine] = useState<number | null>(initialUrlState.highlightLine)
  const [fileContent, setFileContent] = useState<FileContentResponse | null>(null)
  const [fileLoading, setFileLoading] = useState(false)

  const selectedRepoIdRef = useRef(selectedRepoId)
  selectedRepoIdRef.current = selectedRepoId

  const resultsScrollRef = useRef<HTMLDivElement>(null)
  const savedResultsScrollTop = useRef(0)

  // Set when a file is opened from a search result (rather than a tree click) so the tree can
  // catch up: select + reveal that path once its repo's tree has (re)loaded.
  const [pendingRevealPath, setPendingRevealPath] = useState<string | null>(null)

  // Guards onSelectionChange below from reacting to the reveal effect's own select()/
  // deselect() calls (which fire the same listener a real user click would).
  const isProgrammaticSelectionRef = useRef(false)

  const onSelectionChangeRef = useRef<(paths: readonly string[]) => void>(() => {})
  const { model } = useFileTree({
    paths: [],
    icons: pierreTreeIcons,
    initialExpansion: 'closed',
    onSelectionChange: (paths) => onSelectionChangeRef.current(paths),
  })

  const openFile = useCallback(async (repoId: string, path: string) => {
    setFileLoading(true)
    setFileContent(null)
    try {
      setFileContent(await getFile(repoId, path))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Failed to load file')
    } finally {
      setFileLoading(false)
    }
  }, [])

  useEffect(() => {
    void getIndexStatus()
      .then((status) => {
        setRepos(status.repositories)
        if (!selectedRepoId && status.repositories.length > 0) {
          setSelectedRepoId(status.repositories[0].repoId)
        }
      })
      .catch(() => {
        // Best-effort — repo picker just stays empty if this fails.
      })
    // Restore a deep-linked file view once repos are known to exist (no search results to
    // reveal it via — just open it directly against whatever repo the URL named).
    if (initialUrlState.viewMode === 'file' && initialUrlState.selectedRepoId && initialUrlState.filePath) {
      setPendingRevealPath(initialUrlState.filePath)
      void openFile(initialUrlState.selectedRepoId, initialUrlState.filePath)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    if (!selectedRepoId) {
      model.resetPaths([])
      return
    }
    void getTree(selectedRepoId)
      .then((tree) => model.resetPaths(tree.paths))
      .catch(() => model.resetPaths([]))
  }, [selectedRepoId, model])

  // Expands the ancestor folders of `pendingRevealPath` and selects it in the tree, once it's
  // actually present (tree loading is async and may still be in flight after a cross-repo
  // search-result click switches `selectedRepoId`) — poll briefly rather than racing it.
  useEffect(() => {
    if (!pendingRevealPath) {
      return
    }
    let cancelled = false
    let attempts = 0
    const path = pendingRevealPath
    const tryReveal = () => {
      if (cancelled) {
        return
      }
      const segments = path.split('/')
      let ancestorPath = ''
      let ancestorsReady = true
      for (let i = 0; i < segments.length - 1; i++) {
        ancestorPath += (ancestorPath ? '/' : '') + segments[i]
        const handle = model.getItem(ancestorPath)
        if (!handle) {
          ancestorsReady = false
          break
        }
        if ('isExpanded' in handle && !handle.isExpanded()) {
          handle.expand()
        }
      }
      const fileHandle = ancestorsReady ? model.getItem(path) : null
      if (fileHandle) {
        isProgrammaticSelectionRef.current = true
        // select() only adds to the selection set — clear whatever was selected before so
        // this behaves like the single-select click it's standing in for.
        for (const selectedPath of model.getSelectedPaths()) {
          model.getItem(selectedPath)?.deselect()
        }
        fileHandle.select()
        // Reset on the next frame rather than immediately — guards against the listener
        // firing on a microtask/animation-frame tick rather than synchronously inside select().
        requestAnimationFrame(() => {
          isProgrammaticSelectionRef.current = false
        })
        model.scrollToPath(path)
        setPendingRevealPath(null)
        return
      }
      attempts += 1
      if (attempts < 40) {
        requestAnimationFrame(tryReveal)
      }
    }
    requestAnimationFrame(tryReveal)
    return () => {
      cancelled = true
    }
  }, [pendingRevealPath, selectedRepoId, model])

  onSelectionChangeRef.current = (paths) => {
    if (isProgrammaticSelectionRef.current) {
      return
    }
    const path = paths[0]
    if (path && selectedRepoIdRef.current) {
      setHighlightLine(null)
      setViewMode('file')
      void openFile(selectedRepoIdRef.current, path)
    }
  }

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
    setViewMode('results')
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

  // Restore a deep-linked search (page 1, with whatever cursor/filters the URL had) once on
  // mount, if the URL actually named a query.
  const didRestoreInitialSearch = useRef(false)
  useEffect(() => {
    if (didRestoreInitialSearch.current) {
      return
    }
    didRestoreInitialSearch.current = true
    if (initialUrlState.query.trim()) {
      void fetchPage(0, initialUrlState.cursor ?? undefined)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Search-as-you-type: re-run (from page 1) a short beat after the query stops changing,
  // rather than requiring Enter/the Search button. Enter still fires immediately (see the
  // input's onKeyDown below), which just makes this pending timer redundant, not wrong. Skips
  // the very first render so it doesn't stomp the deep-linked-search restore above.
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
    // Intentionally excludes fetchPage/activeFilters — those changing is handled by the
    // dedicated facet-filter effect below; keying this one on them too would double-fetch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, regex])

  // Re-run the current query (from page 1) whenever the active facet filters change — but
  // not on first mount, and not when there's no query yet to (re)run.
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
    selectedRepoFacets.size > 0 ||
    selectedFileTypeFacets.size > 0 ||
    selectedOrgFacets.size > 0 ||
    selectedBranchFacets.size > 0

  const handleResultClick = useCallback(
    (result: SearchResult) => {
      savedResultsScrollTop.current = resultsScrollRef.current?.scrollTop ?? 0
      setHighlightLine(result.line)
      setViewMode('file')
      // The file tree always tracks one repo at a time — jump it to match whichever repo
      // this result came from, so browsing after a cross-repo search lands in the right tree.
      if (result.repoId !== selectedRepoIdRef.current) {
        setSelectedRepoId(result.repoId)
      }
      setPendingRevealPath(result.path)
      void openFile(result.repoId, result.path)
    },
    [openFile],
  )

  const handleBackToResults = useCallback(() => {
    setViewMode('results')
    // Restore the list's scroll position on the next paint, after it's back in the DOM.
    requestAnimationFrame(() => {
      if (resultsScrollRef.current) {
        resultsScrollRef.current.scrollTop = savedResultsScrollTop.current
      }
    })
  }, [])

  const pierreOptions = useMemo(
    () => ({ ...pierreThemeOptions, disableFileHeader: true, overflow: 'scroll' as const }),
    [pierreThemeOptions],
  )

  const pierreFile = useMemo(() => {
    if (!fileContent || fileContent.isBinary || fileContent.content == null) {
      return null
    }
    return toPierreFile(fileContent.path, fileContent.content)
  }, [fileContent])

  const selectedLines = useMemo(
    () => (highlightLine ? { start: highlightLine, end: highlightLine } : null),
    [highlightLine],
  )

  const filePaneRef = useRef<HTMLDivElement>(null)

  // Pierre renders file content inside a `<diffs-container>` custom element's shadow DOM, so
  // a plain querySelector on the outer pane never finds the line rows — pierce the shadow
  // root explicitly. Rendering/highlighting also happens off a worker pool, so the target
  // line's row may not exist yet on the render that requests it — poll briefly.
  useEffect(() => {
    if (!highlightLine || !pierreFile || viewMode !== 'file') {
      return
    }
    let cancelled = false
    let attempts = 0
    const tryScroll = () => {
      if (cancelled) {
        return
      }
      const scrollContainer = filePaneRef.current
      const shadowRoot = scrollContainer?.querySelector('diffs-container')?.shadowRoot
      const target = shadowRoot?.querySelector(`[data-line="${highlightLine}"]`)
      if (target instanceof HTMLElement && scrollContainer) {
        // `target` lives inside a shadow root, so offsetTop/offsetParent aren't reliable
        // across that boundary — use viewport rects instead, which are shadow-DOM-agnostic.
        const targetRect = target.getBoundingClientRect()
        const containerRect = scrollContainer.getBoundingClientRect()
        const offsetWithinContainer = targetRect.top - containerRect.top + scrollContainer.scrollTop
        // Land the line ~15% down from the top of the pane, not dead center or flush — reads
        // as "here's your match, with context below it", the way GitHub/editors do it.
        const desiredTop = offsetWithinContainer - scrollContainer.clientHeight * 0.15
        scrollContainer.scrollTo({ top: Math.max(0, desiredTop), behavior: 'smooth' })
        return
      }
      attempts += 1
      if (attempts < 60) {
        requestAnimationFrame(tryScroll)
      }
    }
    requestAnimationFrame(tryScroll)
    return () => {
      cancelled = true
    }
  }, [highlightLine, pierreFile, viewMode])

  // Mirror everything that defines "what's on screen" into the URL, so the address bar is
  // always a shareable link back to this exact view. replaceState (not push) — this is meant
  // to travel with a copy/paste, not to build up back-button history for every keystroke.
  useEffect(() => {
    const params = new URLSearchParams()
    if (query) params.set('q', query)
    if (regex) params.set('regex', '1')
    if (selectedRepoId) params.set('repo', selectedRepoId)
    if (selectedRepoFacets.size > 0) params.set('frepos', Array.from(selectedRepoFacets).join(','))
    if (selectedFileTypeFacets.size > 0) params.set('ftypes', Array.from(selectedFileTypeFacets).join(','))
    if (selectedOrgFacets.size > 0) params.set('orgs', Array.from(selectedOrgFacets).join(','))
    if (selectedBranchFacets.size > 0) params.set('branches', Array.from(selectedBranchFacets).join(','))
    const currentCursor = pageCursors[pageIndex]
    if (currentCursor) params.set('cursor', currentCursor)
    if (viewMode === 'file') {
      params.set('view', 'file')
      if (fileContent?.path) params.set('path', fileContent.path)
      if (highlightLine) params.set('line', String(highlightLine))
    }
    const queryString = params.toString()
    const url = queryString ? `${window.location.pathname}?${queryString}` : window.location.pathname
    window.history.replaceState(null, '', url)
  }, [
    query,
    regex,
    selectedRepoId,
    selectedRepoFacets,
    selectedFileTypeFacets,
    selectedOrgFacets,
    selectedBranchFacets,
    pageCursors,
    pageIndex,
    viewMode,
    fileContent,
    highlightLine,
  ])

  return (
    <div className="app">
      <header className="appHeader">
        <h1 className="appTitle">xgrep</h1>
        <FormControl>
          <FormControl.Label visuallyHidden>Repository</FormControl.Label>
          <Select value={selectedRepoId} onChange={(event) => setSelectedRepoId(event.target.value)}>
            {repos.length === 0 ? <Select.Option value="">No indexed repos</Select.Option> : null}
            {repos.map((repo) => (
              <Select.Option key={repo.repoId} value={repo.repoId}>
                {repo.repoName}
              </Select.Option>
            ))}
          </Select>
        </FormControl>
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
        />
        <label className="regexToggle">
          <Checkbox checked={regex} onChange={(event) => setRegex(event.target.checked)} />
          Regex
        </label>
        <Button variant="primary" onClick={() => void runSearch()} disabled={loading || !query.trim()}>
          {loading ? <Spinner size="small" /> : 'Search'}
        </Button>
        <IconButton
          aria-label={colorMode === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}
          icon={colorMode === 'dark' ? SunIcon : MoonIcon}
          onClick={onToggleColorMode}
          variant="invisible"
        />
      </header>

      {error ? <div className="errorBanner">{error}</div> : null}

      <div className="appBody">
        <div className="sidebar">
          <div className="treePane">
            <FileTree model={model} style={treeStyles} />
          </div>
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
                      <Checkbox
                        checked={selectedRepoFacets.has(facet.name)}
                        onChange={() => toggleRepoFacet(facet.name)}
                      />
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
                      <Checkbox
                        checked={selectedBranchFacets.has(facet.name)}
                        onChange={() => toggleBranchFacet(facet.name)}
                      />
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
        </div>

        <div className="mainPane" style={{ display: viewMode === 'results' ? 'flex' : 'none' }}>
          <div className="resultsScroll" ref={resultsScrollRef}>
            {results.map((result, idx) => (
              <ResultCard
                key={`${result.repoId}-${result.path}-${result.line}-${idx}`}
                result={result}
                codeTheme={THEME_NAMES[colorMode]}
                onClick={() => handleResultClick(result)}
              />
            ))}
            {!loading && query && results.length === 0 ? (
              <div className="emptyState">No results found for "{query}"</div>
            ) : null}
            {!query && !loading ? (
              <div className="emptyState">Search across all indexed repositories, or browse the file tree.</div>
            ) : null}
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

        <div className="mainPane" style={{ display: viewMode === 'file' ? 'flex' : 'none' }}>
          <div className="fileToolbar">
            {results.length > 0 ? (
              <button type="button" className="backToResults" onClick={handleBackToResults}>
                <ArrowLeftIcon size={16} /> Back to results
              </button>
            ) : (
              <span className="fileToolbarSpacer" />
            )}
            {fileContent ? (
              <span className="fileToolbarPath">
                {fileContent.path}
                {highlightLine ? ` — line ${highlightLine}` : ''}
              </span>
            ) : null}
          </div>
          <div className="filePane" ref={filePaneRef}>
            {fileLoading ? (
              <div className="emptyState">
                <Spinner /> Loading file...
              </div>
            ) : null}
            {!fileLoading && fileContent?.isBinary ? <div className="emptyState">This is a binary file.</div> : null}
            {!fileLoading && pierreFile ? (
              <PierreFile file={pierreFile} options={pierreOptions} selectedLines={selectedLines} disableWorkerPool />
            ) : null}
            {!fileLoading && !fileContent ? (
              <div className="emptyState">Search or pick a file from the tree to view its content.</div>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  )
}
