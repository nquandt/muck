import { useEffect, useMemo, useState } from 'react'
import { LinkExternalIcon } from '@primer/octicons-react'
import type { SearchResult } from '../api'
import { highlightLines } from '../lib/highlightSnippet'
import { useAppContext } from '../AppContext'
import { resolveRepoLinks } from '../lib/linkTemplate'
import { detectPierreLanguage } from '../lib/pierreFile'

interface ResultCardProps {
  result: SearchResult
  codeTheme: string
  onClick: () => void
}

export function ResultCard({ result, codeTheme, onClick }: ResultCardProps) {
  const { repos } = useAppContext()
  const repo = useMemo(() => repos.find((r) => r.repoId === result.repoId), [repos, result.repoId])
  const externalLinks = useMemo(
    () => (repo ? resolveRepoLinks(repo, result.path, result.line) : []),
    [repo, result.path, result.line],
  )
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
        {externalLinks.map((link) => (
          <a
            key={link.name}
            className="resultExternalLink"
            href={link.url}
            target="_blank"
            rel="noreferrer"
            onClick={(event) => event.stopPropagation()}
            title={link.name}
          >
            <LinkExternalIcon size={12} />
          </a>
        ))}
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
