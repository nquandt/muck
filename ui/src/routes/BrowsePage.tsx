import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { FormControl, Select, Spinner } from '@primer/react'
import { ArrowLeftIcon, LinkExternalIcon } from '@primer/octicons-react'
import { File as PierreFile } from '@pierre/diffs/react'
import { FileTree, useFileTree } from '@pierre/trees/react'
import { useNavigate, useParams, useRouter, useSearch } from '@tanstack/react-router'
import { getFile, getTree, type FileContentResponse } from '../api'
import { useAppContext } from '../AppContext'
import { resolveRepoLinks } from '../lib/linkTemplate'
import { pierreTreeIcons } from '../lib/pierreTreeIcons'
import { toPierreFile } from '../lib/pierreFile'

export interface BrowsePageSearch {
  line?: number
}

export function BrowsePage() {
  const { repos, pierreThemeOptions, treeStyles } = useAppContext()
  const { repoId, _splat } = useParams({ from: '/repo/$repoId/tree/$' })
  const { line: highlightLine } = useSearch({ from: '/repo/$repoId/tree/$' })
  const navigate = useNavigate({ from: '/repo/$repoId/tree/$' })
  const router = useRouter()

  const path = _splat || null

  const [fileContent, setFileContent] = useState<FileContentResponse | null>(null)
  const [fileLoading, setFileLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Set when the tree hasn't loaded the target path yet, so the reveal effect can catch up
  // once it does (e.g. deep link straight into a repo+path).
  const [pendingRevealPath, setPendingRevealPath] = useState<string | null>(path)
  const isProgrammaticSelectionRef = useRef(false)
  const onSelectionChangeRef = useRef<(paths: readonly string[]) => void>(() => {})

  const { model } = useFileTree({
    paths: [],
    icons: pierreTreeIcons,
    initialExpansion: 'closed',
    onSelectionChange: (paths) => onSelectionChangeRef.current(paths),
  })

  const openFile = useCallback(async (repo: string, filePath: string) => {
    setFileLoading(true)
    setFileContent(null)
    try {
      setFileContent(await getFile(repo, filePath))
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : 'Failed to load file')
    } finally {
      setFileLoading(false)
    }
  }, [])

  useEffect(() => {
    void getTree(repoId)
      .then((tree) => model.resetPaths(tree.paths))
      .catch(() => model.resetPaths([]))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repoId])

  useEffect(() => {
    if (path) {
      void openFile(repoId, path)
      setPendingRevealPath(path)
    } else {
      setFileContent(null)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repoId, path])

  useEffect(() => {
    if (!pendingRevealPath) {
      return
    }
    let cancelled = false
    let attempts = 0
    const target = pendingRevealPath
    const tryReveal = () => {
      if (cancelled) {
        return
      }
      const segments = target.split('/')
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
      const fileHandle = ancestorsReady ? model.getItem(target) : null
      if (fileHandle) {
        isProgrammaticSelectionRef.current = true
        for (const selectedPath of model.getSelectedPaths()) {
          model.getItem(selectedPath)?.deselect()
        }
        fileHandle.select()
        requestAnimationFrame(() => {
          isProgrammaticSelectionRef.current = false
        })
        model.scrollToPath(target)
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
  }, [pendingRevealPath, repoId, model])

  onSelectionChangeRef.current = (paths) => {
    if (isProgrammaticSelectionRef.current) {
      return
    }
    const selected = paths[0]
    if (selected) {
      void navigate({ to: '/repo/$repoId/tree/$', params: { repoId, _splat: selected }, search: {} })
    }
  }

  const pierreOptions = { ...pierreThemeOptions, disableFileHeader: true, overflow: 'scroll' as const }
  const pierreFile =
    fileContent && !fileContent.isBinary && fileContent.content != null ? toPierreFile(fileContent.path, fileContent.content) : null
  const selectedLines = highlightLine ? { start: highlightLine, end: highlightLine } : null

  const filePaneRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!highlightLine || !pierreFile) {
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
        const targetRect = target.getBoundingClientRect()
        const containerRect = scrollContainer.getBoundingClientRect()
        const offsetWithinContainer = targetRect.top - containerRect.top + scrollContainer.scrollTop
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
  }, [highlightLine, pierreFile])

  const currentRepo = useMemo(() => repos.find((repo) => repo.repoId === repoId), [repos, repoId])
  const externalLinks = useMemo(
    () => (currentRepo && fileContent ? resolveRepoLinks(currentRepo, fileContent.path, highlightLine ?? undefined) : []),
    [currentRepo, fileContent, highlightLine],
  )

  return (
    <div className="appBody">
      <div className="sidebar">
        <FormControl>
          <FormControl.Label visuallyHidden>Repository</FormControl.Label>
          <Select
            value={repoId}
            onChange={(event) =>
              void navigate({ to: '/repo/$repoId/tree/$', params: { repoId: event.target.value, _splat: '' }, search: {} })
            }
          >
            {repos.map((repo) => (
              <Select.Option key={repo.repoId} value={repo.repoId}>
                {repo.repoName}
              </Select.Option>
            ))}
          </Select>
        </FormControl>
        <div className="treePane">
          <FileTree model={model} style={treeStyles} />
        </div>
      </div>

      <div className="mainPane">
        <div className="fileToolbar">
          <button
            type="button"
            className="backToResults"
            onClick={() => {
              if (router.history.canGoBack()) {
                router.history.back()
              } else {
                void navigate({ to: '/' })
              }
            }}
          >
            <ArrowLeftIcon size={16} /> Back to search
          </button>
          {fileContent ? (
            <span className="fileToolbarPath">
              {fileContent.path}
              {highlightLine ? ` — line ${highlightLine}` : ''}
            </span>
          ) : null}
          <div className="fileToolbarGrow" />
          {externalLinks.map((link) => (
            <a key={link.name} className="externalLink" href={link.url} target="_blank" rel="noreferrer">
              <LinkExternalIcon size={14} /> {link.name}
            </a>
          ))}
        </div>
        <div className="filePane" ref={filePaneRef}>
          {error ? <div className="errorBanner">{error}</div> : null}
          {fileLoading ? (
            <div className="emptyState">
              <Spinner /> Loading file...
            </div>
          ) : null}
          {!fileLoading && fileContent?.isBinary ? <div className="emptyState">This is a binary file.</div> : null}
          {!fileLoading && pierreFile ? (
            <PierreFile file={pierreFile} options={pierreOptions} selectedLines={selectedLines} disableWorkerPool />
          ) : null}
          {!fileLoading && !fileContent ? <div className="emptyState">Pick a file from the tree to view its content.</div> : null}
        </div>
      </div>
    </div>
  )
}
