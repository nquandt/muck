import type { IndexedRepo, LinkTemplate } from '../api'

interface LinkTokens {
  org: string
  repoName: string
  branch: string
  version: string
  path: string
  line?: number
}

/** Substitutes `{org}`/`{repoName}`/`{branch}`/`{version}`/`{path}`/`{line}` tokens in a
 * caller-supplied link template. `{line}` resolves to an empty string when no line is known
 * (e.g. a plain file link, no specific match) — templates that always want a trailing
 * `#L{line}` should omit that suffix when opening a file without one. */
export function resolveLinkTemplate(template: string, tokens: LinkTokens): string {
  return template
    .replaceAll('{org}', encodeURIComponent(tokens.org))
    .replaceAll('{repoName}', encodeURIComponent(tokens.repoName))
    .replaceAll('{branch}', encodeURIComponent(tokens.branch))
    .replaceAll('{version}', encodeURIComponent(tokens.version))
    .replaceAll('{path}', tokens.path.split('/').map(encodeURIComponent).join('/'))
    .replaceAll('{line}', tokens.line != null ? String(tokens.line) : '')
}

export function resolveRepoLinks(repo: Pick<IndexedRepo, 'links' | 'org' | 'repoName' | 'branch' | 'version'>, path: string, line?: number) {
  return repo.links.map((link: LinkTemplate) => ({
    name: link.name,
    url: resolveLinkTemplate(link.urlTemplate, {
      org: repo.org,
      repoName: repo.repoName,
      branch: repo.branch,
      version: repo.version,
      path,
      line,
    }),
  }))
}
