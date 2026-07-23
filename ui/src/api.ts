export interface SearchResult {
  repoId: string
  repoName: string
  path: string
  line: number
  column: number
  snippet: string
  contextBefore: string[]
  contextAfter: string[]
  score: number
  blobSha: string
  org: string
  branch: string
}

export interface SearchFacet {
  name: string
  type: 'repo' | 'file_type' | 'org' | 'branch'
  count: number
}

export interface SearchResponse {
  results: SearchResult[]
  nextCursor: string | null
  facets: SearchFacet[]
}

export interface SearchFilters {
  repoIds?: string[]
  fileTypes?: string[]
  pathPrefix?: string
  orgs?: string[]
  branches?: string[]
}

export interface IndexedRepo {
  repoId: string
  repoName: string
  version: string
  org: string
  branch: string
  status: string
}

export interface IndexStatusResponse {
  repositories: IndexedRepo[]
  totalRepos: number
}

export interface FileContentResponse {
  path: string
  content?: string
  isBinary: boolean
}

export interface TreeResponse {
  paths: string[]
}

// Same-origin: this SPA is served by the xgrep-server-local binary itself, no base URL needed.

export async function search(
  query: string,
  regex: boolean,
  cursor?: string | null,
  filters?: SearchFilters,
): Promise<SearchResponse> {
  const hasFilters =
    filters &&
    (filters.repoIds?.length ||
      filters.fileTypes?.length ||
      filters.pathPrefix ||
      filters.orgs?.length ||
      filters.branches?.length)
  const response = await fetch('/v1/search', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      query,
      regex,
      cursor: cursor ?? undefined,
      pageSize: 25,
      filters: hasFilters ? filters : undefined,
    }),
  })
  if (!response.ok) {
    throw new Error(`Search failed: ${response.statusText}`)
  }
  return response.json()
}

export async function getIndexStatus(): Promise<IndexStatusResponse> {
  const response = await fetch('/v1/index/status')
  if (!response.ok) {
    throw new Error(`Failed to load index status: ${response.statusText}`)
  }
  return response.json()
}

export async function getFile(repoId: string, path: string): Promise<FileContentResponse> {
  const params = new URLSearchParams({ path })
  const response = await fetch(`/v1/repos/${encodeURIComponent(repoId)}/file?${params.toString()}`)
  if (!response.ok) {
    throw new Error(`Failed to load file: ${response.statusText}`)
  }
  return response.json()
}

export async function getTree(repoId: string): Promise<TreeResponse> {
  const response = await fetch(`/v1/repos/${encodeURIComponent(repoId)}/tree`)
  if (!response.ok) {
    throw new Error(`Failed to load tree: ${response.statusText}`)
  }
  return response.json()
}
